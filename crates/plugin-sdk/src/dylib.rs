//! C ABI adapter for trusted dynamic-library plugins.
//!
//! Rust trait objects never cross the DLL boundary. The ABI only exchanges
//! JSON bytes and uses the SDK service traits inside the loaded library.

use crate::{
    ClipboardService, EventBus, FileService, GlobalShortcutService, HostError, HostServices,
    DesktopNotificationOptions, NotificationOptions, NotificationService, Plugin, PluginContext, SecretService,
    ShortcutAction, ShortcutHandler, StorageService,
    SystemService, WindowEvent, WindowEventHandler, WindowService, WindowSpec,
};
use agile_tiles_plugin_api::{
    PluginBuffer, PluginDylibApi, PluginEvent, PluginHostApi, PluginId,
    PluginRequest, PluginResponse, DYNAMIC_PLUGIN_ABI_VERSION,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const MAX_DYLIB_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const EXTERNAL_READ_CHUNK_BYTES: usize = 768 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct HostReply {
    ok: bool,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DylibEvent {
    token: String,
    #[serde(default)]
    payload: Value,
}

enum Callback {
    Shortcut(ShortcutHandler),
    Window(Arc<dyn WindowEventHandler>),
}

struct CallbackEntry {
    plugin_id: String,
    callback: Callback,
}

/// Callback storage shared by all Host API handles created for one plugin.
pub struct DylibCallbackRegistry {
    next_token: AtomicU64,
    callbacks: Mutex<HashMap<String, CallbackEntry>>,
}

impl DylibCallbackRegistry {
    pub fn new() -> Self {
        Self {
            next_token: AtomicU64::new(1),
            callbacks: Mutex::new(HashMap::new()),
        }
    }

    fn token(&self) -> String {
        format!("callback-{}", self.next_token.fetch_add(1, Ordering::Relaxed))
    }

    fn add_shortcut(&self, plugin_id: &PluginId, callback: ShortcutHandler) -> String {
        let token = self.token();
        if let Ok(mut callbacks) = self.callbacks.lock() {
            callbacks.insert(
                token.clone(),
                CallbackEntry {
                    plugin_id: plugin_id.to_string(),
                    callback: Callback::Shortcut(callback),
                },
            );
        }
        token
    }

    fn add_window(
        &self,
        plugin_id: &PluginId,
        callback: Arc<dyn WindowEventHandler>,
    ) -> String {
        let token = self.token();
        if let Ok(mut callbacks) = self.callbacks.lock() {
            callbacks.insert(
                token.clone(),
                CallbackEntry {
                    plugin_id: plugin_id.to_string(),
                    callback: Callback::Window(callback),
                },
            );
        }
        token
    }

    fn remove(&self, token: &str) {
        if let Ok(mut callbacks) = self.callbacks.lock() {
            callbacks.remove(token);
        }
    }

    fn remove_plugin(&self, plugin_id: &PluginId) {
        if let Ok(mut callbacks) = self.callbacks.lock() {
            callbacks.retain(|_, entry| entry.plugin_id != plugin_id.as_str());
        }
    }

    pub fn dispatch(&self, token: &str, payload: Value) -> Result<(), HostError> {
        let callbacks = self
            .callbacks
            .lock()
            .map_err(|_| HostError::new("dynamic plugin callback registry is poisoned"))?;
        let Some(entry) = callbacks.get(token) else {
            return Err(HostError::new(format!("unknown dynamic plugin callback: {token}")));
        };
        match &entry.callback {
            Callback::Shortcut(callback) => {
                let action = payload
                    .get("action")
                    .cloned()
                    .ok_or_else(|| HostError::new("shortcut callback is missing action"))?;
                let action: ShortcutAction = serde_json::from_value(action)
                    .map_err(|error| HostError::new(error.to_string()))?;
                callback(action);
            }
            Callback::Window(callback) => {
                let event = payload
                    .get("event")
                    .cloned()
                    .ok_or_else(|| HostError::new("window callback is missing event"))?;
                let event: WindowEvent = serde_json::from_value(event)
                    .map_err(|error| HostError::new(error.to_string()))?;
                callback.on_event(event);
            }
        }
        Ok(())
    }
}

impl Default for DylibCallbackRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// SDK Host API implementation used by a plugin loaded from a DLL.
#[derive(Clone)]
pub struct DylibHostServices {
    host: PluginHostApi,
    callbacks: Arc<DylibCallbackRegistry>,
    plugin_id: PluginId,
}

impl DylibHostServices {
    pub fn new(
        host: PluginHostApi,
        callbacks: Arc<DylibCallbackRegistry>,
        plugin_id: PluginId,
    ) -> Self {
        Self {
            host,
            callbacks,
            plugin_id,
        }
    }

    fn call(&self, method: &str, payload: Value) -> Result<Value, HostError> {
        if self.host.abi_version != DYNAMIC_PLUGIN_ABI_VERSION {
            return Err(HostError::new("unsupported dynamic plugin host ABI"));
        }
        let call = self
            .host
            .call
            .ok_or_else(|| HostError::new("dynamic plugin host callback is unavailable"))?;
        let payload = serde_json::to_vec(&payload)
            .map_err(|error| HostError::new(format!("host payload serialization failed: {error}")))?;
        let bytes = unsafe {
            call(
                self.host.user_data,
                method.as_ptr(),
                method.len(),
                payload.as_ptr(),
                payload.len(),
            )
        };
        let bytes = unsafe { take_buffer(bytes, self.host.free_buffer, self.host.user_data)? };
        let reply: HostReply = serde_json::from_slice(&bytes)
            .map_err(|error| HostError::new(format!("host reply is invalid JSON: {error}")))?;
        if reply.ok {
            Ok(reply.result)
        } else {
            Err(HostError::new(
                reply
                    .error
                    .unwrap_or_else(|| "host request failed".into()),
            ))
        }
    }

    fn result<T: DeserializeOwned>(&self, method: &str, payload: Value) -> Result<T, HostError> {
        serde_json::from_value(self.call(method, payload)?).map_err(|error| {
            HostError::new(format!("invalid result from host method {method}: {error}"))
        })
    }
}

// The host context is allocated by the host and remains alive while the DLL is loaded.
unsafe impl Send for DylibHostServices {}
unsafe impl Sync for DylibHostServices {}

impl StorageService for DylibHostServices {
    fn get(&self, plugin_id: &PluginId, key: &str) -> Result<Option<String>, HostError> {
        self.result("host.storage.get", json!({ "plugin_id": plugin_id, "key": key }))
    }

    fn set(&self, plugin_id: &PluginId, key: &str, value: &str) -> Result<(), HostError> {
        self.call(
            "host.storage.set",
            json!({ "plugin_id": plugin_id, "key": key, "value": value }),
        )
        .map(|_| ())
    }

    fn remove(&self, plugin_id: &PluginId, key: &str) -> Result<(), HostError> {
        self.call(
            "host.storage.remove",
            json!({ "plugin_id": plugin_id, "key": key }),
        )
        .map(|_| ())
    }
}

impl SecretService for DylibHostServices {
    fn get(&self, plugin_id: &PluginId, key: &str) -> Result<Option<String>, HostError> {
        self.result("host.secrets.get", json!({ "plugin_id": plugin_id, "key": key }))
    }

    fn set(&self, plugin_id: &PluginId, key: &str, value: &str) -> Result<(), HostError> {
        self.call(
            "host.secrets.set",
            json!({ "plugin_id": plugin_id, "key": key, "value": value }),
        )
        .map(|_| ())
    }

    fn remove(&self, plugin_id: &PluginId, key: &str) -> Result<(), HostError> {
        self.call(
            "host.secrets.remove",
            json!({ "plugin_id": plugin_id, "key": key }),
        )
        .map(|_| ())
    }
}

impl NotificationService for DylibHostServices {
    fn notify(&self, title: &str, body: &str) -> Result<(), HostError> {
        self.call("host.notification.notify", json!({ "title": title, "body": body }))
            .map(|_| ())
    }

    fn notify_with(&self, notification: &NotificationOptions) -> Result<(), HostError> {
        let payload = serde_json::to_value(notification)
            .map_err(|error| HostError::new(error.to_string()))?;
        self.call("host.notification.notify", payload).map(|_| ())
    }

    fn show_desktop(&self, notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        let payload = serde_json::to_value(notification)
            .map_err(|error| HostError::new(error.to_string()))?;
        self.call("host.notification.show_desktop", payload).map(|_| ())
    }

    fn show_sidebar(&self, notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        let payload = serde_json::to_value(notification)
            .map_err(|error| HostError::new(error.to_string()))?;
        self.call("host.notification.show_sidebar", payload).map(|_| ())
    }

}

impl EventBus for DylibHostServices {
    fn publish(&self, event: PluginEvent) -> Result<(), HostError> {
        self.call("host.events.publish", serde_json::to_value(event).unwrap_or(Value::Null))
            .map(|_| ())
    }
}

impl SystemService for DylibHostServices {
    fn open_external(&self, url: &str) -> Result<(), HostError> {
        self.call("host.system.open_external", json!({ "url": url }))
            .map(|_| ())
    }

    fn spawn(&self, program: &str, args: &[String]) -> Result<(), HostError> {
        self.spawn_with_cwd(program, args, None)
    }

    fn spawn_with_cwd(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<(), HostError> {
        self.call(
            "host.system.spawn",
            json!({ "program": program, "args": args, "cwd": cwd }),
        )
        .map(|_| ())
    }

    fn pick_file(&self, extensions: &[String]) -> Result<Option<String>, HostError> {
        self.result("host.system.pick_file", json!({ "extensions": extensions }))
    }

    fn extract_icon(&self, path: &str) -> Result<Option<String>, HostError> {
        self.result("host.system.extract_icon", json!({ "path": path }))
    }
}

impl FileService for DylibHostServices {
    fn read_private(
        &self,
        plugin_id: &PluginId,
        relative_path: &str,
    ) -> Result<Option<Vec<u8>>, HostError> {
        self.result(
            "host.files.read_private",
            json!({ "plugin_id": plugin_id, "path": relative_path }),
        )
    }

    fn write_private(
        &self,
        plugin_id: &PluginId,
        relative_path: &str,
        bytes: &[u8],
    ) -> Result<(), HostError> {
        self.call(
            "host.files.write_private",
            json!({ "plugin_id": plugin_id, "path": relative_path, "bytes": bytes }),
        )
        .map(|_| ())
    }

    fn remove_private(
        &self,
        plugin_id: &PluginId,
        relative_path: &str,
    ) -> Result<(), HostError> {
        self.call(
            "host.files.remove_private",
            json!({ "plugin_id": plugin_id, "path": relative_path }),
        )
        .map(|_| ())
    }

    fn read_external(&self, path: &str) -> Result<Vec<u8>, HostError> {
        let mut bytes = Vec::new();
        let mut offset = 0_u64;
        loop {
            let chunk: Vec<u8> = self.result(
                "host.files.read_external_range",
                json!({ "path": path, "offset": offset, "max_bytes": EXTERNAL_READ_CHUNK_BYTES }),
            )?;
            if chunk.is_empty() {
                break;
            }
            let chunk_len = chunk.len();
            bytes.extend_from_slice(&chunk);
            offset = offset
                .checked_add(chunk_len as u64)
                .ok_or_else(|| HostError::new("external file is too large to read"))?;
            if chunk_len < EXTERNAL_READ_CHUNK_BYTES {
                break;
            }
        }
        Ok(bytes)
    }

    fn private_path(&self, plugin_id: &PluginId, relative_path: &str) -> Result<String, HostError> {
        self.result(
            "host.files.private_path",
            json!({ "plugin_id": plugin_id, "path": relative_path }),
        )
    }
}

impl ClipboardService for DylibHostServices {
    fn read_text(&self) -> Result<Option<String>, HostError> {
        self.result("host.clipboard.read_text", Value::Null)
    }

    fn write_text(&self, text: &str) -> Result<(), HostError> {
        self.call("host.clipboard.write_text", json!({ "text": text }))
            .map(|_| ())
    }
}

impl GlobalShortcutService for DylibHostServices {
    fn register(
        &self,
        plugin_id: &PluginId,
        bindings: &[(String, ShortcutAction)],
        handler: ShortcutHandler,
    ) -> Result<(), HostError> {
        let token = self.callbacks.add_shortcut(plugin_id, handler);
        let bindings = bindings
            .iter()
            .map(|(key, action)| json!({ "key": key, "action": action }))
            .collect::<Vec<_>>();
        if let Err(error) = self.call(
            "host.shortcuts.register",
            json!({ "plugin_id": plugin_id, "bindings": bindings, "event_token": token }),
        ) {
            self.callbacks.remove(&token);
            return Err(error);
        }
        Ok(())
    }

    fn unregister(&self, plugin_id: &PluginId) -> Result<(), HostError> {
        let result = self.call("host.shortcuts.unregister", json!({ "plugin_id": plugin_id }));
        self.callbacks.remove_plugin(plugin_id);
        result.map(|_| ())
    }
}

impl WindowService for DylibHostServices {
    fn create(
        &self,
        spec: WindowSpec,
        handler: Option<Arc<dyn WindowEventHandler>>,
    ) -> Result<(), HostError> {
        let token = handler.map(|handler| self.callbacks.add_window(&self.plugin_id, handler));
        let result = self.call(
            "host.windows.create",
            json!({ "spec": spec, "event_token": token }),
        );
        if result.is_err() {
            if let Some(token) = token {
                self.callbacks.remove(&token);
            }
        }
        result.map(|_| ())
    }

    fn show(&self, id: &str) -> Result<(), HostError> {
        self.call("host.windows.show", json!({ "id": id })).map(|_| ())
    }

    fn hide(&self, id: &str) -> Result<(), HostError> {
        self.call("host.windows.hide", json!({ "id": id })).map(|_| ())
    }

    fn close(&self, id: &str) -> Result<(), HostError> {
        self.call("host.windows.close", json!({ "id": id })).map(|_| ())
    }

    fn emit(&self, id: &str, topic: &str, payload: Value) -> Result<(), HostError> {
        self.call(
            "host.windows.emit",
            json!({ "id": id, "topic": topic, "payload": payload }),
        )
        .map(|_| ())
    }

    fn set_clickthrough(&self, id: &str, enabled: bool) -> Result<(), HostError> {
        self.call(
            "host.windows.set_clickthrough",
            json!({ "id": id, "enabled": enabled }),
        )
        .map(|_| ())
    }

    fn set_size(&self, id: &str, width: f64, height: f64) -> Result<(), HostError> {
        self.call(
            "host.windows.set_size",
            json!({ "id": id, "width": width, "height": height }),
        )
        .map(|_| ())
    }

    fn set_position(&self, id: &str, x: i32, y: i32) -> Result<(), HostError> {
        self.call(
            "host.windows.set_position",
            json!({ "id": id, "x": x, "y": y }),
        )
        .map(|_| ())
    }

    fn position(&self, id: &str) -> Result<Option<(i32, i32)>, HostError> {
        self.result("host.windows.position", json!({ "id": id }))
    }

    fn focus(&self, id: &str) -> Result<(), HostError> {
        self.call("host.windows.focus", json!({ "id": id })).map(|_| ())
    }
}

impl HostServices for DylibHostServices {
    fn storage(&self) -> &dyn StorageService {
        self
    }
    fn secrets(&self) -> &dyn SecretService {
        self
    }
    fn notification(&self) -> &dyn NotificationService {
        self
    }
    fn events(&self) -> &dyn EventBus {
        self
    }
    fn system(&self) -> &dyn SystemService {
        self
    }
    fn files(&self) -> &dyn FileService {
        self
    }
    fn clipboard(&self) -> &dyn ClipboardService {
        self
    }
    fn shortcuts(&self) -> &dyn GlobalShortcutService {
        self
    }
    fn windows(&self) -> &dyn WindowService {
        self
    }
}

pub struct DylibPluginState<T> {
    plugin: T,
    callbacks: Arc<DylibCallbackRegistry>,
}

pub unsafe fn entry<T>() -> *const PluginDylibApi
where
    T: Plugin + Default + 'static,
{
    Box::into_raw(Box::new(PluginDylibApi {
        abi_version: DYNAMIC_PLUGIN_ABI_VERSION,
        manifest: Some(manifest::<T>),
        create: Some(create::<T>),
        destroy: Some(destroy::<T>),
        handle: Some(handle::<T>),
        handle_event: Some(handle_event::<T>),
        free_buffer: Some(free_buffer),
    }))
}

unsafe extern "C" fn manifest<T>() -> PluginBuffer
where
    T: Plugin + Default,
{
    let bytes = serde_json::to_vec(T::default().manifest()).unwrap_or_default();
    buffer_from_bytes(bytes)
}

unsafe extern "C" fn create<T>() -> *mut c_void
where
    T: Plugin + Default + 'static,
{
    let callbacks = Arc::new(DylibCallbackRegistry::new());
    Box::into_raw(Box::new(DylibPluginState {
        plugin: T::default(),
        callbacks,
    })) as *mut c_void
}

unsafe extern "C" fn destroy<T>(state: *mut c_void)
where
    T: Plugin + Default + 'static,
{
    if !state.is_null() {
        drop(Box::from_raw(state as *mut DylibPluginState<T>));
    }
}

unsafe extern "C" fn handle<T>(
    state: *mut c_void,
    request_ptr: *const u8,
    request_len: usize,
    host: PluginHostApi,
) -> PluginBuffer
where
    T: Plugin + Default + 'static,
{
    if state.is_null() {
        return PluginBuffer::empty();
    }
    let state = &mut *(state as *mut DylibPluginState<T>);
    let Ok(bytes) = input_bytes(request_ptr, request_len) else {
        return PluginBuffer::empty();
    };
    let Ok(request) = serde_json::from_slice::<PluginRequest>(bytes) else {
        return PluginBuffer::empty();
    };
    let plugin_id = request.plugin_id.clone();
    let services: Arc<dyn HostServices> = Arc::new(DylibHostServices::new(
        host,
        state.callbacks.clone(),
        request.plugin_id.clone(),
    ));
    let context = PluginContext::new(plugin_id, services);
    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.plugin.handle(request, &context)
    }))
    .unwrap_or_else(|_| PluginResponse {
        request_id: "dylib-panic".into(),
        plugin_id: context.plugin_id().clone(),
        ok: false,
        result: Value::Null,
        error: Some(agile_tiles_plugin_api::PluginError {
            code: agile_tiles_plugin_api::ErrorCode::Internal,
            message: "plugin panicked while handling request".into(),
        }),
    });
    buffer_from_json(&response)
}

unsafe extern "C" fn handle_event<T>(
    state: *mut c_void,
    event_ptr: *const u8,
    event_len: usize,
    _host: PluginHostApi,
) -> PluginBuffer
where
    T: Plugin + Default + 'static,
{
    if state.is_null() {
        return PluginBuffer::empty();
    }
    let state = &*(state as *const DylibPluginState<T>);
    let Ok(bytes) = input_bytes(event_ptr, event_len) else {
        return PluginBuffer::empty();
    };
    let Ok(event) = serde_json::from_slice::<DylibEvent>(bytes) else {
        return PluginBuffer::empty();
    };
    let result = state.callbacks.dispatch(&event.token, event.payload);
    buffer_from_json(&json!({ "ok": result.is_ok() }))
}

unsafe extern "C" fn free_buffer(buffer: PluginBuffer) {
    if !buffer.ptr.is_null() && buffer.capacity >= buffer.len {
        drop(Vec::from_raw_parts(buffer.ptr, buffer.len, buffer.capacity));
    }
}

fn buffer_from_json<T: Serialize>(value: &T) -> PluginBuffer {
    buffer_from_bytes(serde_json::to_vec(value).unwrap_or_default())
}

fn buffer_from_bytes(mut bytes: Vec<u8>) -> PluginBuffer {
    let buffer = PluginBuffer {
        ptr: bytes.as_mut_ptr(),
        len: bytes.len(),
        capacity: bytes.capacity(),
    };
    std::mem::forget(bytes);
    buffer
}

unsafe fn input_bytes<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], HostError> {
    if len > MAX_DYLIB_MESSAGE_BYTES || (len > 0 && ptr.is_null()) {
        return Err(HostError::new("dynamic plugin message is invalid or too large"));
    }
    Ok(std::slice::from_raw_parts(ptr, len))
}

unsafe fn take_buffer(
    buffer: PluginBuffer,
    free: Option<unsafe extern "C" fn(*mut c_void, PluginBuffer)>,
    user_data: *mut c_void,
) -> Result<Vec<u8>, HostError> {
    if buffer.len > MAX_DYLIB_MESSAGE_BYTES
        || (buffer.len > 0 && buffer.ptr.is_null())
        || buffer.capacity < buffer.len
    {
        if let Some(free) = free {
            free(user_data, buffer);
        }
        return Err(HostError::new("host returned an invalid dynamic plugin buffer"));
    }
    let bytes = if buffer.len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(buffer.ptr, buffer.len).to_vec()
    };
    if let Some(free) = free {
        free(user_data, buffer);
    }
    Ok(bytes)
}

#[macro_export]
macro_rules! export_plugin_dylib {
    ($plugin:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn agile_tiles_plugin_entry()
            -> *const agile_tiles_plugin_api::PluginDylibApi
        {
            agile_tiles_plugin_sdk::dylib::entry::<$plugin>()
        }
    };
}
