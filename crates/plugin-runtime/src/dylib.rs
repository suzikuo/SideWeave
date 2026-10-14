//! Trusted Windows dynamic-library plugin loader.

use agile_tiles_plugin_api::{
    ErrorCode, PluginBuffer, PluginDylibApi, PluginDylibEntry, PluginError, PluginEvent,
    PluginHostApi, PluginId, PluginManifest, PluginRequest, PluginResponse, PluginRuntime,
    DYNAMIC_PLUGIN_ABI_VERSION, DYNAMIC_PLUGIN_ENTRY_SYMBOL,
};
use agile_tiles_plugin_sdk::{
    DesktopNotificationOptions, HostError, HostServices, NotificationOptions, ShortcutAction, ShortcutHandler, WindowEvent,
    WindowEventHandler, WindowService, WindowSpec,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::c_void;
use std::path::Path;
use std::sync::{Arc, Mutex};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

const MAX_DYLIB_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_EXTERNAL_READ_CHUNK_BYTES: usize = 768 * 1024;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn FreeLibrary(module: *mut c_void) -> i32;
}

struct LoadedLibrary(*mut c_void);

impl Drop for LoadedLibrary {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { FreeLibrary(self.0) };
        }
    }
}

struct DylibHostContext {
    plugin_id: PluginId,
    permissions: Vec<String>,
    services: Arc<dyn HostServices>,
    event_sink: Arc<Mutex<Option<Arc<dyn Fn(String, Value) + Send + Sync>>>>,
}

struct Invocation {
    api: PluginDylibApi,
    state: *mut c_void,
    host: PluginHostApi,
    lock: Mutex<()>,
}

unsafe impl Send for Invocation {}
unsafe impl Sync for Invocation {}

impl Invocation {
    fn request(&self, request: &PluginRequest) -> Result<PluginResponse, String> {
        let _guard = self.lock.lock().map_err(|_| "dynamic plugin lock is poisoned")?;
        let handle = self
            .api
            .handle
            .ok_or_else(|| "dynamic plugin handle entrypoint is missing".to_string())?;
        let bytes = serde_json::to_vec(request).map_err(|error| error.to_string())?;
        let buffer = unsafe { handle(self.state, bytes.as_ptr(), bytes.len(), self.host) };
        let bytes = read_buffer(buffer, self.api.free_buffer)?;
        serde_json::from_slice(&bytes).map_err(|error| format!("dynamic plugin response is invalid: {error}"))
    }

    fn event(&self, token: String, payload: Value) {
        let Ok(_guard) = self.lock.lock() else { return };
        let Some(handle_event) = self.api.handle_event else { return };
        let event = json!({ "token": token, "payload": payload });
        let Ok(bytes) = serde_json::to_vec(&event) else { return };
        let buffer = unsafe { handle_event(self.state, bytes.as_ptr(), bytes.len(), self.host) };
        let _ = read_buffer(buffer, self.api.free_buffer);
    }
}

pub struct DynamicPlugin {
    _library: LoadedLibrary,
    invocation: Arc<Invocation>,
    host_context: Box<DylibHostContext>,
    manifest: PluginManifest,
}

unsafe impl Send for DynamicPlugin {}

impl DynamicPlugin {
    pub fn load(
        library_path: impl AsRef<Path>,
        expected_manifest: PluginManifest,
        services: Arc<dyn HostServices>,
    ) -> Result<Self, String> {
        expected_manifest
            .validate()
            .map_err(|error| error.to_string())?;
        if expected_manifest.runtime != PluginRuntime::DynamicLibrary {
            return Err("dynamic loader requires dynamic_library runtime".into());
        }
        let library_path = library_path.as_ref();
        let wide = library_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            return Err(format!("failed to load plugin DLL: {}", library_path.display()));
        }
        let library = LoadedLibrary(handle);
        let symbol = unsafe { GetProcAddress(handle, DYNAMIC_PLUGIN_ENTRY_SYMBOL.as_ptr()) };
        let Some(symbol) = symbol else {
            return Err("plugin DLL is missing agile_tiles_plugin_entry".into());
        };
        let entry: PluginDylibEntry = unsafe { std::mem::transmute(symbol) };
        let api_ptr = unsafe { entry() };
        if api_ptr.is_null() {
            return Err("plugin DLL returned a null API table".into());
        }
        let api = unsafe { *api_ptr };
        if api.abi_version != DYNAMIC_PLUGIN_ABI_VERSION {
            return Err(format!("unsupported plugin DLL ABI: {}", api.abi_version));
        }
        let manifest_fn = api
            .manifest
            .ok_or_else(|| "plugin DLL manifest entrypoint is missing".to_string())?;
        let dll_manifest: PluginManifest = serde_json::from_slice(&read_buffer(
            unsafe { manifest_fn() },
            api.free_buffer,
        )?)
        .map_err(|error| format!("plugin DLL manifest is invalid: {error}"))?;
        dll_manifest
            .validate()
            .map_err(|error| format!("plugin DLL manifest is invalid: {error}"))?;
        if dll_manifest.id != expected_manifest.id {
            return Err(format!(
                "plugin DLL id {} does not match package id {}",
                dll_manifest.id, expected_manifest.id
            ));
        }
        if dll_manifest.version != expected_manifest.version {
            return Err(format!(
                "plugin DLL version {} does not match package version {}",
                dll_manifest.version, expected_manifest.version
            ));
        }
        let create = api
            .create
            .ok_or_else(|| "plugin DLL create entrypoint is missing".to_string())?;
        let state = unsafe { create() };
        if state.is_null() {
            return Err("plugin DLL failed to create state".into());
        }
        let event_sink = Arc::new(Mutex::new(None));
        let mut host_context = Box::new(DylibHostContext {
            plugin_id: expected_manifest.id.clone(),
            permissions: expected_manifest.permissions.clone(),
            services,
            event_sink: Arc::clone(&event_sink),
        });
        let host = PluginHostApi {
            abi_version: DYNAMIC_PLUGIN_ABI_VERSION,
            user_data: (&mut *host_context as *mut DylibHostContext).cast(),
            call: Some(host_call),
            free_buffer: Some(free_host_buffer),
        };
        let invocation = Arc::new(Invocation {
            api,
            state,
            host,
            lock: Mutex::new(()),
        });
        let invocation_for_events = Arc::clone(&invocation);
        *event_sink
            .lock()
            .map_err(|_| "dynamic plugin event sink is poisoned")? = Some(Arc::new(
            move |token, payload| invocation_for_events.event(token, payload),
        ));
        let mut manifest = expected_manifest;
        if manifest.hover_preview.is_none() {
            manifest.hover_preview = dll_manifest.hover_preview;
        }
        Ok(Self {
            _library: library,
            invocation,
            host_context,
            manifest,
        })
    }
}

impl agile_tiles_plugin_sdk::Plugin for DynamicPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn handle(&mut self, request: PluginRequest, _ctx: &agile_tiles_plugin_sdk::PluginContext) -> PluginResponse {
        let request_id = request.request_id.clone();
        let plugin_id = request.plugin_id.clone();
        match self.invocation.request(&request) {
            Ok(response) => response,
            Err(message) => PluginResponse {
                request_id,
                plugin_id,
                ok: false,
                result: Value::Null,
                error: Some(PluginError {
                    code: ErrorCode::Internal,
                    message,
                }),
            },
        }
    }
}

impl Drop for DynamicPlugin {
    fn drop(&mut self) {
        if let Ok(mut sink) = self.host_context.event_sink.lock() {
            *sink = None;
        }
        if let Some(destroy) = self.invocation.api.destroy {
            unsafe { destroy(self.invocation.state) };
        }
    }
}

#[derive(Debug, Deserialize)]
struct ShortcutBinding {
    key: String,
    action: ShortcutAction,
}

struct WindowForwarder {
    token: String,
    sink: Arc<Mutex<Option<Arc<dyn Fn(String, Value) + Send + Sync>>>>,
}

impl WindowEventHandler for WindowForwarder {
    fn on_event(&self, event: WindowEvent) {
        let Ok(sink) = self.sink.lock() else { return };
        let Some(sink) = sink.as_ref() else { return };
        sink(self.token.clone(), json!({ "event": event }));
    }
}

fn host_context<'a>(user_data: *mut c_void) -> Result<&'a DylibHostContext, String> {
    if user_data.is_null() {
        return Err("dynamic plugin host context is null".into());
    }
    Ok(unsafe { &*(user_data as *const DylibHostContext) })
}

unsafe extern "C" fn host_call(
    user_data: *mut c_void,
    method_ptr: *const u8,
    method_len: usize,
    payload_ptr: *const u8,
    payload_len: usize,
) -> PluginBuffer {
    let result = (|| {
        if method_len > MAX_DYLIB_MESSAGE_BYTES
            || payload_len > MAX_DYLIB_MESSAGE_BYTES
            || (method_len > 0 && method_ptr.is_null())
            || (payload_len > 0 && payload_ptr.is_null())
        {
            return Err("dynamic plugin host request is invalid or too large".into());
        }
        let method = std::str::from_utf8(std::slice::from_raw_parts(method_ptr, method_len))
            .map_err(|error| error.to_string())?;
        let payload = serde_json::from_slice::<Value>(std::slice::from_raw_parts(
            payload_ptr,
            payload_len,
        ))
        .map_err(|error| error.to_string())?;
        let context = host_context(user_data)?;
        dispatch_host_call(context, method, payload)
    })();
    host_reply(result)
}

fn dispatch_host_call(context: &DylibHostContext, method: &str, payload: Value) -> Result<Value, String> {
    if let Some(permission) = agile_tiles_plugin_api::permissions::for_host_method(method) {
        if !agile_tiles_plugin_api::permissions::is_declared(&context.permissions, permission) {
            return Err(format!("permission not declared by plugin: {permission}"));
        }
        if !context
            .services
            .permission_granted(&context.plugin_id, permission)
            .map_err(host_error)?
        {
            return Err(format!("permission denied by user: {permission}"));
        }
    }
    let id = &context.plugin_id;
    match method {
        "host.storage.get" => {
            let key = string_field(&payload, "key")?;
            context.services.storage().get(id, &key).map_err(host_error)
                .map(|value| json!(value))
        }
        "host.storage.set" => {
            let key = string_field(&payload, "key")?;
            let value = string_field(&payload, "value")?;
            context.services.storage().set(id, &key, &value).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.storage.remove" => {
            let key = string_field(&payload, "key")?;
            context.services.storage().remove(id, &key).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.secrets.get" => {
            let key = string_field(&payload, "key")?;
            context.services.secrets().get(id, &key).map_err(host_error)
                .map(|value| json!(value))
        }
        "host.secrets.set" => {
            let key = string_field(&payload, "key")?;
            let value = string_field(&payload, "value")?;
            context.services.secrets().set(id, &key, &value).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.secrets.remove" => {
            let key = string_field(&payload, "key")?;
            context.services.secrets().remove(id, &key).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.notification.notify" => {
            let notification: NotificationOptions =
                serde_json::from_value(payload).map_err(|error| error.to_string())?;
            context
                .services
                .notification()
                .notify_with(&notification)
                .map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.notification.show_desktop" => {
            let notification: DesktopNotificationOptions =
                serde_json::from_value(payload).map_err(|error| error.to_string())?;
            context
                .services
                .notification()
                .show_desktop_for_plugin(&context.plugin_id, &notification)
                .map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.notification.show_sidebar" => {
            let notification: DesktopNotificationOptions =
                serde_json::from_value(payload).map_err(|error| error.to_string())?;
            context
                .services
                .notification()
                .show_sidebar_for_plugin(&context.plugin_id, &notification)
                .map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.events.publish" => {
            let event: PluginEvent = serde_json::from_value(payload).map_err(|error| error.to_string())?;
            context.services.events().publish(event).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.system.open_external" => {
            context.services.system().open_external(string_field(&payload, "url")?.as_str()).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.system.spawn" => {
            let program = string_field(&payload, "program")?;
            let args: Vec<String> = value_field(&payload, "args")?;
            let cwd: Option<String> = optional_value(&payload, "cwd")?;
            context.services.system().spawn_with_cwd(&program, &args, cwd.as_deref()).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.system.pick_file" => {
            let extensions: Vec<String> = value_field(&payload, "extensions")?;
            context.services.system().pick_file(&extensions).map_err(host_error).map(|value| json!(value))
        }
        "host.system.extract_icon" => {
            context.services.system().extract_icon(string_field(&payload, "path")?.as_str()).map_err(host_error).map(|value| json!(value))
        }
        "host.files.read_private" => {
            let path = string_field(&payload, "path")?;
            context.services.files().read_private(id, &path).map_err(host_error).map(|value| json!(value))
        }
        "host.files.write_private" => {
            let path = string_field(&payload, "path")?;
            let bytes: Vec<u8> = value_field(&payload, "bytes")?;
            context.services.files().write_private(id, &path, &bytes).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.files.remove_private" => {
            let path = string_field(&payload, "path")?;
            context.services.files().remove_private(id, &path).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.files.read_external" => {
            context.services.files().read_external(string_field(&payload, "path")?.as_str()).map_err(host_error).map(|value| json!(value))
        }
        "host.files.read_external_range" => {
            let path = string_field(&payload, "path")?;
            let offset: u64 = value_field(&payload, "offset")?;
            let max_bytes: usize = value_field(&payload, "max_bytes")?;
            if max_bytes > MAX_EXTERNAL_READ_CHUNK_BYTES {
                return Err("external file read chunk exceeds 512 KiB".into());
            }
            context.services.files().read_external_range(&path, offset, max_bytes).map(|value| json!(value)).map_err(host_error)
        }
        "host.files.private_path" => {
            let path = string_field(&payload, "path")?;
            context.services.files().private_path(id, &path).map_err(host_error).map(|value| json!(value))
        }
        "host.clipboard.read_text" => context.services.clipboard().read_text().map_err(host_error).map(|value| json!(value)),
        "host.clipboard.write_text" => {
            context.services.clipboard().write_text(string_field(&payload, "text")?.as_str()).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.shortcuts.register" => register_shortcuts(context, payload),
        "host.shortcuts.unregister" => {
            context.services.shortcuts().unregister(id).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.windows.create" => create_window(context, payload),
        "host.windows.show" => window_call(context, payload, |service, id| service.show(id)),
        "host.windows.hide" => window_call(context, payload, |service, id| service.hide(id)),
        "host.windows.close" => window_call(context, payload, |service, id| service.close(id)),
        "host.windows.set_clickthrough" => {
            let id = string_field(&payload, "id")?;
            let enabled: bool = value_field(&payload, "enabled")?;
            context.services.windows().set_clickthrough(&id, enabled).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.windows.set_size" => {
            let id = string_field(&payload, "id")?;
            let width: f64 = value_field(&payload, "width")?;
            let height: f64 = value_field(&payload, "height")?;
            context.services.windows().set_size(&id, width, height).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.windows.set_position" => {
            let id = string_field(&payload, "id")?;
            let x: i32 = value_field(&payload, "x")?;
            let y: i32 = value_field(&payload, "y")?;
            context.services.windows().set_position(&id, x, y).map_err(host_error)?;
            Ok(Value::Null)
        }
        "host.windows.position" => {
            context.services.windows().position(string_field(&payload, "id")?.as_str()).map_err(host_error).map(|value| json!(value))
        }
        "host.windows.focus" => window_call(context, payload, |service, id| service.focus(id)),
        "host.windows.emit" => {
            let id = string_field(&payload, "id")?;
            let topic = string_field(&payload, "topic")?;
            let value = payload.get("payload").cloned().unwrap_or(Value::Null);
            context.services.windows().emit(&id, &topic, value).map_err(host_error)?;
            Ok(Value::Null)
        }
        _ => Err(format!("unsupported dynamic plugin host method: {method}")),
    }
}

fn register_shortcuts(context: &DylibHostContext, payload: Value) -> Result<Value, String> {
    let bindings: Vec<ShortcutBinding> = value_field(&payload, "bindings")?;
    let token = string_field(&payload, "event_token")?;
    let sink = Arc::clone(&context.event_sink);
    let callback: ShortcutHandler = Arc::new(move |action| {
        let Ok(sink) = sink.lock() else { return };
        let Some(sink) = sink.as_ref() else { return };
        sink(token.clone(), json!({ "action": action }));
    });
    let bindings = bindings.into_iter().map(|binding| (binding.key, binding.action)).collect::<Vec<_>>();
    context.services.shortcuts().register(&context.plugin_id, &bindings, callback).map_err(host_error)?;
    Ok(Value::Null)
}

fn create_window(context: &DylibHostContext, payload: Value) -> Result<Value, String> {
    let mut spec: WindowSpec = value_field(&payload, "spec")?;
    if !spec.url.contains("://") {
        let resource = spec.url.trim_start_matches('/');
        if resource.is_empty() || resource.contains("..") {
            return Err("dynamic plugin window URL is invalid".into());
        }
        spec.url = format!(
            "agile-plugin://localhost/{}/{}",
            context.plugin_id,
            resource
        );
    }
    let handler = optional_value::<String>(&payload, "event_token")?.map(|token| {
        Arc::new(WindowForwarder {
            token,
            sink: Arc::clone(&context.event_sink),
        }) as Arc<dyn WindowEventHandler>
    });
    context.services.windows().create(spec, handler).map_err(host_error)?;
    Ok(Value::Null)
}

fn window_call<F>(context: &DylibHostContext, payload: Value, call: F) -> Result<Value, String>
where
    F: FnOnce(&dyn WindowService, &str) -> Result<(), HostError>,
{
    let id = string_field(&payload, "id")?;
    call(context.services.windows(), &id).map_err(host_error)?;
    Ok(Value::Null)
}

fn string_field(payload: &Value, key: &str) -> Result<String, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string field: {key}"))
}

fn value_field<T: serde::de::DeserializeOwned>(payload: &Value, key: &str) -> Result<T, String> {
    payload
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing field: {key}"))
        .and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
}

fn optional_value<T: serde::de::DeserializeOwned>(payload: &Value, key: &str) -> Result<Option<T>, String> {
    match payload.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| error.to_string()),
    }
}

fn host_error(error: HostError) -> String {
    error.message
}

fn host_reply(result: Result<Value, String>) -> PluginBuffer {
    let value = match result {
        Ok(result) => json!({ "ok": true, "result": result }),
        Err(error) => json!({ "ok": false, "result": Value::Null, "error": error }),
    };
    buffer_from_bytes(serde_json::to_vec(&value).unwrap_or_default())
}

unsafe extern "C" fn free_host_buffer(_user_data: *mut c_void, buffer: PluginBuffer) {
    if !buffer.ptr.is_null() && buffer.capacity >= buffer.len {
        drop(Vec::from_raw_parts(buffer.ptr, buffer.len, buffer.capacity));
    }
}

fn read_buffer(
    buffer: PluginBuffer,
    free: Option<unsafe extern "C" fn(PluginBuffer)>,
) -> Result<Vec<u8>, String> {
    if buffer.len > MAX_DYLIB_MESSAGE_BYTES
        || (buffer.len > 0 && buffer.ptr.is_null())
        || buffer.capacity < buffer.len
    {
        if let Some(free) = free {
            unsafe { free(buffer) };
        }
        return Err("dynamic plugin returned an invalid buffer".into());
    }
    let bytes = if buffer.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(buffer.ptr, buffer.len).to_vec() }
    };
    if let Some(free) = free {
        unsafe { free(buffer) };
    }
    Ok(bytes)
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

use std::os::windows::ffi::OsStrExt;
