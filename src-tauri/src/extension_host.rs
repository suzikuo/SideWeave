//! Tauri-backed Host API services for the extension runtime.

use sideweave_plugin_api::{PluginEvent, PluginId};
use sideweave_plugin_sdk::{
    ClipboardService, DesktopNotificationOptions, EventBus, FileService, GlobalShortcutService,
    HostError, HostServices, NotificationOptions, NotificationService, SecretService,
    ShortcutAction, ShortcutHandler, StorageService, SystemService,
    WindowEvent as PluginWindowEvent, WindowEventHandler, WindowService, WindowSpec,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowBuilder};
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt as _, ShortcutState};
use tauri_plugin_notification::NotificationExt as _;

pub struct TauriHostServices {
    data_dir: PathBuf,
    app: AppHandle,
    window_bindings: Arc<Mutex<HashMap<String, WindowBinding>>>,
    clickthrough_states: Arc<Mutex<HashMap<String, bool>>>,
    native_windows: Arc<Mutex<HashSet<String>>>,
}

impl TauriHostServices {
    pub fn new(app: AppHandle, data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            app,
            window_bindings: Arc::new(Mutex::new(HashMap::new())),
            clickthrough_states: Arc::new(Mutex::new(HashMap::new())),
            native_windows: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    #[cfg(windows)]
    fn is_native_window(&self, id: &str) -> bool {
        self.native_windows
            .lock()
            .map(|windows| windows.contains(id))
            .unwrap_or(false)
    }

    #[cfg(windows)]
    fn notify_visibility(&self, id: &str, visible: bool) {
        if let Some(slot) = self
            .window_bindings
            .lock()
            .ok()
            .and_then(|bindings| bindings.get(id).map(|binding| Arc::clone(&binding.slot)))
        {
            slot.on_event(PluginWindowEvent::VisibilityChanged { visible });
        }
    }
}

struct WindowHandlerSlot {
    state: Mutex<WindowHandlerState>,
    idle: Condvar,
}

struct WindowHandlerState {
    handler: Option<Arc<dyn WindowEventHandler>>,
    active: usize,
    detached: bool,
}

impl WindowHandlerSlot {
    fn new(handler: Option<Arc<dyn WindowEventHandler>>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(WindowHandlerState {
                handler,
                active: 0,
                detached: false,
            }),
            idle: Condvar::new(),
        })
    }

    fn replace(&self, handler: Option<Arc<dyn WindowEventHandler>>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.detached = false;
        state.handler = handler;
    }

    fn detach(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.detached = true;
        state.handler = None;
        while state.active > 0 {
            state = self
                .idle
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn is_detached(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.detached)
            .unwrap_or(true)
    }
}

impl WindowEventHandler for WindowHandlerSlot {
    fn on_event(&self, event: PluginWindowEvent) {
        let handler = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.detached {
                return;
            }
            state.active += 1;
            state.handler.clone()
        };
        if let Some(handler) = handler {
            handler.on_event(event);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active = state.active.saturating_sub(1);
        if state.active == 0 {
            self.idle.notify_all();
        }
    }
}

struct WindowBinding {
    plugin_id: String,
    slot: Arc<WindowHandlerSlot>,
}

pub fn services(app: AppHandle, data_dir: PathBuf) -> Arc<dyn HostServices> {
    Arc::new(TauriHostServices::new(app, data_dir))
}

impl StorageService for TauriHostServices {
    fn get(&self, plugin_id: &PluginId, key: &str) -> Result<Option<String>, HostError> {
        let path = store_path(&self.data_dir, plugin_id, key)?;
        let value = crate::paths::load_json(&path).unwrap_or_else(|| serde_json::json!({}));
        Ok(value
            .get(key)
            .and_then(|stored| stored.as_str().map(str::to_string)))
    }

    fn set(&self, plugin_id: &PluginId, key: &str, value: &str) -> Result<(), HostError> {
        if value.len() > 256 * 1024 {
            return Err(HostError::new("storage value exceeds 256KB"));
        }
        let path = store_path(&self.data_dir, plugin_id, key)?;
        let mut object = crate::paths::load_json(&path).unwrap_or_else(|| serde_json::json!({}));
        if !object.is_object() {
            object = serde_json::json!({});
        }
        object[key] = Value::String(value.to_string());
        crate::paths::save_json_atomic(&path, &object)
            .map_err(|error| HostError::new(error.to_string()))
    }

    fn remove(&self, plugin_id: &PluginId, key: &str) -> Result<(), HostError> {
        let path = store_path(&self.data_dir, plugin_id, key)?;
        let mut object = crate::paths::load_json(&path).unwrap_or_else(|| serde_json::json!({}));
        if let Some(map) = object.as_object_mut() {
            map.remove(key);
        }
        crate::paths::save_json_atomic(&path, &object)
            .map_err(|error| HostError::new(error.to_string()))
    }
}

impl SecretService for TauriHostServices {
    fn get(&self, plugin_id: &PluginId, key: &str) -> Result<Option<String>, HostError> {
        let path = secret_path(&self.data_dir, plugin_id, key)?;
        let value = crate::paths::load_json(&path).unwrap_or_else(|| serde_json::json!({}));
        let Some(protected) = value.get(key).and_then(Value::as_str) else {
            return Ok(None);
        };
        crate::secret_store::unprotect_current_user(protected)
            .map(Some)
            .map_err(|error| HostError::new(format!("failed to decrypt plugin secret: {error}")))
    }

    fn set(&self, plugin_id: &PluginId, key: &str, value: &str) -> Result<(), HostError> {
        if value.len() > 32 * 1024 {
            return Err(HostError::new("secret value exceeds 32KB"));
        }
        let path = secret_path(&self.data_dir, plugin_id, key)?;
        let protected = crate::secret_store::protect_current_user(value)
            .map_err(|error| HostError::new(format!("failed to encrypt plugin secret: {error}")))?;
        let mut object = crate::paths::load_json(&path).unwrap_or_else(|| serde_json::json!({}));
        if !object.is_object() {
            object = serde_json::json!({});
        }
        object[key] = Value::String(protected);
        crate::paths::save_json_atomic(&path, &object)
            .map_err(|error| HostError::new(error.to_string()))
    }

    fn remove(&self, plugin_id: &PluginId, key: &str) -> Result<(), HostError> {
        let path = secret_path(&self.data_dir, plugin_id, key)?;
        let mut object = crate::paths::load_json(&path).unwrap_or_else(|| serde_json::json!({}));
        if let Some(map) = object.as_object_mut() {
            map.remove(key);
        }
        crate::paths::save_json_atomic(&path, &object)
            .map_err(|error| HostError::new(error.to_string()))
    }
}

impl NotificationService for TauriHostServices {
    fn notify(&self, title: &str, body: &str) -> Result<(), HostError> {
        self.notify_with(&NotificationOptions::new(title, body))
    }

    fn notify_with(&self, notification: &NotificationOptions) -> Result<(), HostError> {
        let mut builder = self
            .app
            .notification()
            .builder()
            .title(notification.title.as_str())
            .body(notification.body.as_str());
        if let Some(icon) = notification.icon.as_deref() {
            builder = builder.icon(icon);
        }
        if let Some(sound) = notification.sound.as_deref() {
            builder = builder.sound(sound);
        }
        builder
            .show()
            .map_err(|error| HostError::new(error.to_string()))
    }

    fn show_desktop(&self, notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        self.app
            .state::<crate::desktop_notifications::DesktopNotificationManager>()
            .show_desktop(self.app.clone(), notification)
    }

    fn show_desktop_for_plugin(
        &self,
        plugin_id: &PluginId,
        notification: &DesktopNotificationOptions,
    ) -> Result<(), HostError> {
        self.app
            .state::<crate::desktop_notifications::DesktopNotificationManager>()
            .show_desktop_for_plugin(self.app.clone(), notification, plugin_id.as_str())
    }

    fn show_sidebar(&self, notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        self.app
            .state::<crate::desktop_notifications::DesktopNotificationManager>()
            .show_sidebar(self.app.clone(), notification)
    }

    fn show_sidebar_for_plugin(
        &self,
        plugin_id: &PluginId,
        notification: &DesktopNotificationOptions,
    ) -> Result<(), HostError> {
        self.app
            .state::<crate::desktop_notifications::DesktopNotificationManager>()
            .show_sidebar_for_plugin(self.app.clone(), notification, plugin_id.as_str())
    }
}

impl EventBus for TauriHostServices {
    fn publish(&self, event: PluginEvent) -> Result<(), HostError> {
        self.app
            .emit("plugin://event", event)
            .map_err(|error| HostError::new(error.to_string()))
    }
}

impl SystemService for TauriHostServices {
    fn open_external(&self, url: &str) -> Result<(), HostError> {
        if !(url.starts_with("http://") || url.starts_with("https://")) || url.contains('"') {
            return Err(HostError::new("only http(s) URLs are supported"));
        }
        #[cfg(windows)]
        {
            Command::new("explorer")
                .arg(url)
                .spawn()
                .map_err(|error| HostError::new(format!("failed to open browser: {error}")))?;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = url;
            Err(HostError::new(
                "external URLs are only supported on Windows",
            ))
        }
    }

    fn spawn(&self, program: &str, args: &[String]) -> Result<(), HostError> {
        if program.trim().is_empty() {
            return Err(HostError::new("program cannot be empty"));
        }
        Command::new(program)
            .args(args)
            .spawn()
            .map(|_| ())
            .map_err(|error| HostError::new(format!("failed to start process: {error}")))
    }

    fn spawn_with_cwd(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<(), HostError> {
        if program.trim().is_empty() {
            return Err(HostError::new("program cannot be empty"));
        }
        let mut command = Command::new(program);
        command.args(args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command
            .spawn()
            .map(|_| ())
            .map_err(|error| HostError::new(format!("failed to start process: {error}")))
    }

    fn pick_file(&self, extensions: &[String]) -> Result<Option<String>, HostError> {
        let filters: Vec<&str> = extensions.iter().map(String::as_str).collect();
        let app = self.app.clone();
        let file = crate::with_detail_not_always_on_top(&app, || {
            let mut dialog = app.dialog().file();
            if let Some(window) = crate::detail_window(&app) {
                dialog = dialog.set_parent(&window);
            }
            dialog.add_filter("Files", &filters).blocking_pick_file()
        });
        file.map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| HostError::new(format!("invalid selected path: {error}")))
        })
        .transpose()
    }

    fn extract_icon(&self, path: &str) -> Result<Option<String>, HostError> {
        #[cfg(windows)]
        {
            Ok(icon::icon_base64(path, &self.data_dir))
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Ok(None)
        }
    }
}

impl FileService for TauriHostServices {
    fn read_private(
        &self,
        plugin_id: &PluginId,
        relative_path: &str,
    ) -> Result<Option<Vec<u8>>, HostError> {
        let path = private_path(&self.data_dir, plugin_id, relative_path)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(file_error("读取私有文件", &path, error)),
        }
    }

    fn write_private(
        &self,
        plugin_id: &PluginId,
        relative_path: &str,
        bytes: &[u8],
    ) -> Result<(), HostError> {
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(HostError::new("private file exceeds 8MB"));
        }
        let path = private_path(&self.data_dir, plugin_id, relative_path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| file_error("创建私有文件目录", parent, error))?;
        }
        std::fs::write(&path, bytes).map_err(|error| file_error("写入私有文件", &path, error))?;
        harden_private_file(&path).map_err(|error| {
            HostError::new(format!("{}（路径：{}）", error.message, path.display()))
        })
    }

    fn remove_private(&self, plugin_id: &PluginId, relative_path: &str) -> Result<(), HostError> {
        let path = private_path(&self.data_dir, plugin_id, relative_path)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(HostError::new(error.to_string())),
        }
    }

    fn read_external(&self, path: &str) -> Result<Vec<u8>, HostError> {
        std::fs::read(path).map_err(|error| HostError::new(error.to_string()))
    }

    fn read_external_range(
        &self,
        path: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Vec<u8>, HostError> {
        const MAX_CHUNK_BYTES: usize = 768 * 1024;
        if max_bytes > MAX_CHUNK_BYTES {
            return Err(HostError::new("external file read chunk exceeds 512 KiB"));
        }
        let mut file =
            std::fs::File::open(path).map_err(|error| HostError::new(error.to_string()))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| HostError::new(error.to_string()))?;
        let mut bytes = Vec::with_capacity(max_bytes);
        file.take(max_bytes as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| HostError::new(error.to_string()))?;
        Ok(bytes)
    }

    fn private_path(&self, plugin_id: &PluginId, relative_path: &str) -> Result<String, HostError> {
        Ok(private_path(&self.data_dir, plugin_id, relative_path)?
            .to_string_lossy()
            .into_owned())
    }
}

fn shortcut_registry() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

impl GlobalShortcutService for TauriHostServices {
    fn register(
        &self,
        plugin_id: &PluginId,
        bindings: &[(String, ShortcutAction)],
        handler: ShortcutHandler,
    ) -> Result<(), HostError> {
        self.unregister(plugin_id)?;
        let mut registered = Vec::new();
        for (raw, action) in bindings {
            if raw.trim().is_empty() {
                continue;
            }
            let shortcut: tauri_plugin_global_shortcut::Shortcut = raw
                .parse()
                .map_err(|error| HostError::new(format!("invalid shortcut {raw}: {error}")))?;
            let callback = Arc::clone(&handler);
            let action = *action;
            self.app
                .global_shortcut()
                .on_shortcut(shortcut, move |_app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        callback(action);
                    }
                })
                .map_err(|error| HostError::new(error.to_string()))?;
            registered.push(raw.clone());
        }
        shortcut_registry()
            .lock()
            .map_err(|_| HostError::new("shortcut registry is poisoned"))?
            .insert(plugin_id.as_str().to_string(), registered);
        Ok(())
    }

    fn unregister(&self, plugin_id: &PluginId) -> Result<(), HostError> {
        let previous = shortcut_registry()
            .lock()
            .map_err(|_| HostError::new("shortcut registry is poisoned"))?
            .remove(plugin_id.as_str())
            .unwrap_or_default();
        for raw in previous {
            if let Ok(shortcut) = raw.parse::<tauri_plugin_global_shortcut::Shortcut>() {
                self.app
                    .global_shortcut()
                    .unregister(shortcut)
                    .map_err(|error| HostError::new(error.to_string()))?;
            }
        }
        Ok(())
    }
}

impl ClipboardService for TauriHostServices {
    fn read_text(&self) -> Result<Option<String>, HostError> {
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|error| HostError::new(format!("failed to access clipboard: {error}")))?;
        match clipboard.get_text() {
            Ok(text) => Ok(Some(text)),
            Err(error) => Err(HostError::new(format!("failed to read clipboard: {error}"))),
        }
    }

    fn write_text(&self, text: &str) -> Result<(), HostError> {
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|error| HostError::new(format!("failed to access clipboard: {error}")))?;
        clipboard
            .set_text(text.to_string())
            .map_err(|error| HostError::new(format!("failed to write clipboard: {error}")))
    }
}

fn create_plugin_window_now(
    app: &AppHandle,
    spec: WindowSpec,
    handler: Option<Arc<WindowHandlerSlot>>,
    clickthrough: bool,
    native_windows: &Arc<Mutex<HashSet<String>>>,
) -> Result<(), HostError> {
    #[cfg(windows)]
    if spec.url.starts_with("agile-native://") {
        let notify_handle = |handler: &Option<Arc<WindowHandlerSlot>>, hwnd: u64| {
            if let Some(handler) = handler {
                handler.on_event(PluginWindowEvent::NativeHandleAvailable { handle: hwnd });
            }
        };
        let notify_visibility = |handler: &Option<Arc<WindowHandlerSlot>>, visible| {
            if let Some(handler) = handler {
                handler.on_event(PluginWindowEvent::VisibilityChanged { visible });
            }
        };
        if let Some(window) = app.get_window(&spec.id) {
            native_windows
                .lock()
                .map_err(|_| HostError::new("native window registry is poisoned"))?
                .insert(spec.id.clone());
            let hwnd = window
                .hwnd()
                .map_err(|error| HostError::new(error.to_string()))?
                .0 as usize as u64;
            notify_handle(&handler, hwnd);
            apply_native_clickthrough(&window, clickthrough)?;
            window
                .show()
                .map_err(|error| HostError::new(error.to_string()))?;
            notify_visibility(&handler, true);
            return Ok(());
        }
        let scale = crate::monitor_scale(app);
        let mut builder = WindowBuilder::new(app, spec.id.clone())
            .title(spec.title)
            .decorations(spec.decorations)
            .transparent(spec.transparent)
            .shadow(false)
            .always_on_top(spec.always_on_top)
            .skip_taskbar(spec.skip_taskbar)
            .resizable(spec.resizable)
            .maximizable(spec.maximizable)
            .focused(false)
            .visible(false)
            .inner_size(spec.width, spec.height);
        if let Some((x, y)) = spec.position_px {
            builder = builder.position(f64::from(x) / scale, f64::from(y) / scale);
        }
        let window = builder.build().map_err(|error| {
            HostError::new(format!("failed to create native plugin window: {error}"))
        })?;
        let hwnd = window
            .hwnd()
            .map_err(|error| HostError::new(error.to_string()))?
            .0;
        native_windows
            .lock()
            .map_err(|_| HostError::new("native window registry is poisoned"))?
            .insert(spec.id.clone());
        if let Some(handler) = handler.as_ref() {
            let close_handler = Arc::clone(handler);
            let close_window = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if let Ok(position) = close_window.outer_position() {
                        close_handler.on_event(PluginWindowEvent::Moved {
                            x: position.x,
                            y: position.y,
                        });
                    }
                    if let Ok(size) = close_window.outer_size() {
                        let scale = close_window.scale_factor().unwrap_or(1.0);
                        close_handler.on_event(PluginWindowEvent::Resized {
                            width: f64::from(size.width) / scale,
                            height: f64::from(size.height) / scale,
                        });
                    }
                    let _ = close_window.hide();
                    close_handler.on_event(PluginWindowEvent::VisibilityChanged { visible: false });
                    close_handler.on_event(PluginWindowEvent::Closed);
                }
            });
            let move_handler = Arc::clone(handler);
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::Moved(position) = event {
                    move_handler.on_event(PluginWindowEvent::Moved {
                        x: position.x,
                        y: position.y,
                    });
                }
            });
            let resize_handler = Arc::clone(handler);
            let resize_window = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::Resized(size) = event {
                    let scale = resize_window.scale_factor().unwrap_or(1.0);
                    resize_handler.on_event(PluginWindowEvent::Resized {
                        width: f64::from(size.width) / scale,
                        height: f64::from(size.height) / scale,
                    });
                }
            });
        }
        notify_handle(&handler, hwnd as usize as u64);
        apply_native_clickthrough(&window, clickthrough)?;
        window
            .show()
            .map_err(|error| HostError::new(error.to_string()))?;
        notify_visibility(&handler, true);
        return Ok(());
    }

    #[cfg(not(windows))]
    let _ = native_windows;

    if let Some(window) = app.get_webview_window(&spec.id) {
        if spec.url.starts_with("agile-plugin://") {
            let target_url = spec
                .url
                .parse()
                .map_err(|error| HostError::new(format!("invalid plugin window URL: {error}")))?;
            if window.url().map(|url| url != target_url).unwrap_or(true) {
                window
                    .navigate(target_url)
                    .map_err(|error| HostError::new(error.to_string()))?;
            }
        }
        apply_plugin_clickthrough(&window, clickthrough)?;
        window
            .show()
            .map_err(|error| HostError::new(error.to_string()))?;
        return Ok(());
    }

    let scale = crate::monitor_scale(app);
    let webview_url = if spec.url.starts_with("agile-plugin://") {
        WebviewUrl::External(
            spec.url
                .parse()
                .map_err(|error| HostError::new(format!("invalid plugin window URL: {error}")))?,
        )
    } else {
        WebviewUrl::App(spec.url.clone().into())
    };
    let mut builder = WebviewWindowBuilder::new(app, spec.id.clone(), webview_url)
        .title(spec.title)
        .decorations(spec.decorations)
        .transparent(spec.transparent)
        .shadow(false)
        .always_on_top(spec.always_on_top)
        .skip_taskbar(spec.skip_taskbar)
        .resizable(spec.resizable)
        .maximizable(spec.maximizable)
        .visible(false)
        .inner_size(spec.width, spec.height);
    if let Some((x, y)) = spec.position_px {
        builder = builder.position(f64::from(x) / scale, f64::from(y) / scale);
    }
    let window = builder
        .build()
        .map_err(|error| HostError::new(format!("failed to create plugin window: {error}")))?;
    crate::apply_interface_scale_to_window(&window, crate::interface_scale(app));

    apply_plugin_clickthrough(&window, clickthrough)?;

    if let Some(handler) = handler.as_ref() {
        let close_handler = Arc::clone(handler);
        let app = app.clone();
        let id = spec.id.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Some(window) = app.get_webview_window(&id) {
                    if let Ok(position) = window.outer_position() {
                        close_handler.on_event(PluginWindowEvent::Moved {
                            x: position.x,
                            y: position.y,
                        });
                    }
                    if let Ok(size) = window.outer_size() {
                        close_handler.on_event(PluginWindowEvent::Resized {
                            width: f64::from(size.width) / scale,
                            height: f64::from(size.height) / scale,
                        });
                    }
                    let _ = window.hide();
                }
                close_handler.on_event(PluginWindowEvent::Closed);
            }
        });

        let move_handler = Arc::clone(handler);
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::Moved(position) = event {
                move_handler.on_event(PluginWindowEvent::Moved {
                    x: position.x,
                    y: position.y,
                });
            }
        });

        let resize_handler = Arc::clone(handler);
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::Resized(size) = event {
                resize_handler.on_event(PluginWindowEvent::Resized {
                    width: f64::from(size.width) / scale,
                    height: f64::from(size.height) / scale,
                });
            }
        });
    }

    window
        .show()
        .map_err(|error| HostError::new(error.to_string()))
}

fn apply_plugin_clickthrough<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    enabled: bool,
) -> Result<(), HostError> {
    #[cfg(windows)]
    {
        let hwnd = window
            .hwnd()
            .map_err(|error| HostError::new(error.to_string()))?;
        crate::platform::set_clickthrough(hwnd.0, enabled).map_err(HostError::new)?;
    }
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|error| HostError::new(error.to_string()))
}

#[cfg(windows)]
fn apply_native_clickthrough<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    enabled: bool,
) -> Result<(), HostError> {
    let hwnd = window
        .hwnd()
        .map_err(|error| HostError::new(error.to_string()))?;
    crate::platform::set_clickthrough(hwnd.0, enabled).map_err(HostError::new)?;
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|error| HostError::new(error.to_string()))
}

impl WindowService for TauriHostServices {
    fn create(
        &self,
        spec: WindowSpec,
        handler: Option<Arc<dyn WindowEventHandler>>,
    ) -> Result<(), HostError> {
        let plugin_id = plugin_id_from_window_url(&spec.url);
        let existing_slot = self.window_bindings.lock().ok().and_then(|bindings| {
            bindings
                .get(&spec.id)
                .map(|binding| Arc::clone(&binding.slot))
        });
        let slot = if let Some(existing_slot) = existing_slot {
            existing_slot.replace(handler);
            existing_slot
        } else {
            WindowHandlerSlot::new(handler)
        };
        let window_bindings = Arc::clone(&self.window_bindings);
        let clickthrough_states = Arc::clone(&self.clickthrough_states);
        let native_windows = Arc::clone(&self.native_windows);
        let binding_slot = Arc::clone(&slot);
        if let Some(plugin_id) = plugin_id.as_ref() {
            window_bindings
                .lock()
                .map_err(|_| HostError::new("plugin window registry is poisoned"))?
                .insert(
                    spec.id.clone(),
                    WindowBinding {
                        plugin_id: plugin_id.clone(),
                        slot: Arc::clone(&slot),
                    },
                );
        }
        let is_existing_native = native_windows
            .lock()
            .map_err(|_| HostError::new("native window registry is poisoned"))?
            .contains(&spec.id);
        if self.app.get_webview_window(&spec.id).is_some() || is_existing_native {
            let spec_id = spec.id.clone();
            let clickthrough = clickthrough_states
                .lock()
                .map_err(|_| HostError::new("click-through window registry is poisoned"))?
                .get(&spec.id)
                .copied()
                .unwrap_or(false);
            let result = create_plugin_window_now(
                &self.app,
                spec,
                Some(Arc::clone(&slot)),
                clickthrough,
                &native_windows,
            );
            if result.is_err() {
                slot.detach();
                if let Ok(mut bindings) = window_bindings.lock() {
                    let same_slot = bindings
                        .get(&spec_id)
                        .is_some_and(|binding| Arc::ptr_eq(&binding.slot, &slot));
                    if same_slot {
                        bindings.remove(&spec_id);
                    }
                }
            }
            return result;
        }

        let dispatch_app = self.app.clone();
        let callback_app = dispatch_app.clone();
        let window_id = spec.id.clone();
        let queue_window_id = window_id.clone();
        let queued_clickthrough_states = Arc::clone(&clickthrough_states);
        let queued_native_windows = Arc::clone(&native_windows);
        std::thread::Builder::new()
            .name(format!("plugin-window-{window_id}"))
            .spawn(move || {
                let queued_bindings = Arc::clone(&window_bindings);
                let queued_slot = Arc::clone(&binding_slot);
                let queued_clickthrough_states = Arc::clone(&queued_clickthrough_states);
                let queued_native_windows = Arc::clone(&queued_native_windows);
                if let Err(error) = dispatch_app.run_on_main_thread(move || {
                    if queued_slot.is_detached() {
                        return;
                    }
                    let clickthrough = queued_clickthrough_states
                        .lock()
                        .ok()
                        .and_then(|states| states.get(&window_id).copied())
                        .unwrap_or(false);
                    if let Err(error) = create_plugin_window_now(
                        &callback_app,
                        spec,
                        Some(Arc::clone(&queued_slot)),
                        clickthrough,
                        &queued_native_windows,
                    ) {
                        eprintln!(
                            "[sideweave_] failed to create plugin window '{}': {}",
                            window_id, error.message
                        );
                        queued_slot.detach();
                        if let Ok(mut bindings) = queued_bindings.lock() {
                            let same_slot = bindings
                                .get(&window_id)
                                .is_some_and(|binding| Arc::ptr_eq(&binding.slot, &queued_slot));
                            if same_slot {
                                bindings.remove(&window_id);
                            }
                        }
                    }
                }) {
                    eprintln!(
                        "[sideweave_] failed to queue plugin window '{}': {error}",
                        queue_window_id
                    );
                    binding_slot.detach();
                    if let Ok(mut bindings) = window_bindings.lock() {
                        let same_slot = bindings
                            .get(&queue_window_id)
                            .is_some_and(|binding| Arc::ptr_eq(&binding.slot, &binding_slot));
                        if same_slot {
                            bindings.remove(&queue_window_id);
                        }
                    }
                }
            })
            .map(|_| ())
            .map_err(|error| HostError::new(format!("failed to queue plugin window: {error}")))
    }

    fn show(&self, id: &str) -> Result<(), HostError> {
        #[cfg(windows)]
        if self.is_native_window(id) {
            if let Some(window) = self.app.get_window(id) {
                window
                    .show()
                    .map_err(|error| HostError::new(error.to_string()))?;
            }
            self.notify_visibility(id, true);
            return Ok(());
        }
        if let Some(window) = self.app.get_webview_window(id) {
            window
                .show()
                .map_err(|error| HostError::new(error.to_string()))?;
        }
        Ok(())
    }

    fn hide(&self, id: &str) -> Result<(), HostError> {
        #[cfg(windows)]
        if self.is_native_window(id) {
            self.notify_visibility(id, false);
            if let Some(window) = self.app.get_window(id) {
                window
                    .hide()
                    .map_err(|error| HostError::new(error.to_string()))?;
            }
            return Ok(());
        }
        if let Some(window) = self.app.get_webview_window(id) {
            window
                .hide()
                .map_err(|error| HostError::new(error.to_string()))?;
        }
        Ok(())
    }

    fn close(&self, id: &str) -> Result<(), HostError> {
        #[cfg(windows)]
        if self.is_native_window(id) {
            if let Some(window) = self.app.get_window(id) {
                if let Ok(hwnd) = window.hwnd() {
                    crate::platform::remove_clickthrough_hook(hwnd.0);
                }
                window
                    .destroy()
                    .map_err(|error| HostError::new(error.to_string()))?;
            }
            if let Ok(mut windows) = self.native_windows.lock() {
                windows.remove(id);
            }
        } else if let Some(window) = self.app.get_webview_window(id) {
            #[cfg(windows)]
            if let Ok(hwnd) = window.hwnd() {
                crate::platform::remove_clickthrough_hook(hwnd.0);
            }
            window
                .destroy()
                .map_err(|error| HostError::new(error.to_string()))?;
        }
        #[cfg(not(windows))]
        if let Some(window) = self.app.get_webview_window(id) {
            window
                .destroy()
                .map_err(|error| HostError::new(error.to_string()))?;
        }
        self.clickthrough_states
            .lock()
            .map_err(|_| HostError::new("click-through window registry is poisoned"))?
            .remove(id);
        let binding = self
            .window_bindings
            .lock()
            .map_err(|_| HostError::new("plugin window registry is poisoned"))?
            .remove(id);
        if let Some(binding) = binding {
            binding.slot.detach();
        }
        Ok(())
    }

    fn emit(&self, id: &str, topic: &str, payload: Value) -> Result<(), HostError> {
        #[cfg(windows)]
        if self.is_native_window(id) {
            let _ = (topic, payload);
            return Err(HostError::new(
                "native plugin windows do not support WebView events",
            ));
        }
        self.app
            .emit_to(id, topic, payload)
            .map_err(|error| HostError::new(error.to_string()))
    }

    fn set_clickthrough(&self, id: &str, enabled: bool) -> Result<(), HostError> {
        self.clickthrough_states
            .lock()
            .map_err(|_| HostError::new("click-through window registry is poisoned"))?
            .insert(id.to_string(), enabled);
        #[cfg(windows)]
        if self.is_native_window(id) {
            if let Some(window) = self.app.get_window(id) {
                apply_native_clickthrough(&window, enabled)?;
            }
            return Ok(());
        }
        if let Some(window) = self.app.get_webview_window(id) {
            apply_plugin_clickthrough(&window, enabled)?;
        }
        Ok(())
    }

    fn set_size(&self, id: &str, width: f64, height: f64) -> Result<(), HostError> {
        #[cfg(windows)]
        if self.is_native_window(id) {
            if let Some(window) = self.app.get_window(id) {
                window
                    .set_size(tauri::LogicalSize::new(width, height))
                    .map_err(|error| HostError::new(error.to_string()))?;
            }
            return Ok(());
        }
        if let Some(window) = self.app.get_webview_window(id) {
            window
                .set_size(tauri::LogicalSize::new(width, height))
                .map_err(|error| HostError::new(error.to_string()))?;
        }
        Ok(())
    }

    fn set_position(&self, id: &str, x: i32, y: i32) -> Result<(), HostError> {
        #[cfg(windows)]
        if self.is_native_window(id) {
            if let Some(window) = self.app.get_window(id) {
                window
                    .set_position(tauri::PhysicalPosition::new(x, y))
                    .map_err(|error| HostError::new(error.to_string()))?;
            }
            return Ok(());
        }
        if let Some(window) = self.app.get_webview_window(id) {
            window
                .set_position(tauri::PhysicalPosition::new(x, y))
                .map_err(|error| HostError::new(error.to_string()))?;
        }
        Ok(())
    }

    fn position(&self, id: &str) -> Result<Option<(i32, i32)>, HostError> {
        #[cfg(windows)]
        if self.is_native_window(id) {
            return self
                .app
                .get_window(id)
                .map(|window| {
                    window
                        .outer_position()
                        .map(|position| (position.x, position.y))
                        .map_err(|error| HostError::new(error.to_string()))
                })
                .transpose();
        }
        self.app
            .get_webview_window(id)
            .map(|window| {
                window
                    .outer_position()
                    .map(|position| (position.x, position.y))
                    .map_err(|error| HostError::new(error.to_string()))
            })
            .transpose()
    }

    fn focus(&self, id: &str) -> Result<(), HostError> {
        #[cfg(windows)]
        if self.is_native_window(id) {
            if let Some(window) = self.app.get_window(id) {
                window
                    .set_focus()
                    .map_err(|error| HostError::new(error.to_string()))?;
            }
            return Ok(());
        }
        if let Some(window) = self.app.get_webview_window(id) {
            window
                .set_focus()
                .map_err(|error| HostError::new(error.to_string()))?;
        }
        Ok(())
    }

    fn close_plugin_windows(&self, plugin_id: &PluginId) -> Result<(), HostError> {
        let window_ids = {
            let mut bindings = self
                .window_bindings
                .lock()
                .map_err(|_| HostError::new("plugin window registry is poisoned"))?;
            let ids = bindings
                .iter()
                .filter(|(_, binding)| binding.plugin_id == plugin_id.as_str())
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in &ids {
                if let Some(binding) = bindings.remove(id) {
                    binding.slot.detach();
                }
            }
            ids
        };

        for id in window_ids {
            self.clickthrough_states
                .lock()
                .map_err(|_| HostError::new("click-through window registry is poisoned"))?
                .remove(&id);
            #[cfg(windows)]
            if self.is_native_window(&id) {
                if let Some(window) = self.app.get_window(&id) {
                    if let Ok(hwnd) = window.hwnd() {
                        crate::platform::remove_clickthrough_hook(hwnd.0);
                    }
                    window
                        .destroy()
                        .map_err(|error| HostError::new(error.to_string()))?;
                }
                if let Ok(mut windows) = self.native_windows.lock() {
                    windows.remove(&id);
                }
                continue;
            }
            if let Some(window) = self.app.get_webview_window(&id) {
                #[cfg(windows)]
                if let Ok(hwnd) = window.hwnd() {
                    crate::platform::remove_clickthrough_hook(hwnd.0);
                }
                window
                    .destroy()
                    .map_err(|error| HostError::new(error.to_string()))?;
            }
        }
        Ok(())
    }
}

impl HostServices for TauriHostServices {
    fn permission_granted(
        &self,
        plugin_id: &PluginId,
        permission: &str,
    ) -> Result<bool, HostError> {
        Ok(crate::plugin_permission_granted(
            &self.app,
            plugin_id,
            permission,
        ))
    }

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

fn plugin_id_from_window_url(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("agile-plugin://")
        .or_else(|| url.strip_prefix("agile-native://"))?;
    let mut segments = path.split('/');
    let _host = segments.next()?;
    let plugin_id = segments.next()?.trim();
    if plugin_id.is_empty() {
        None
    } else {
        Some(plugin_id.to_string())
    }
}

fn store_path(data_dir: &Path, plugin_id: &PluginId, key: &str) -> Result<PathBuf, HostError> {
    if key.is_empty()
        || key.len() > 64
        || !key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(HostError::new("invalid storage key"));
    }
    Ok(data_dir
        .join("plugin-data")
        .join(plugin_id.as_str())
        .join("store.json"))
}

fn secret_path(data_dir: &Path, plugin_id: &PluginId, key: &str) -> Result<PathBuf, HostError> {
    store_path(data_dir, plugin_id, key)?;
    Ok(data_dir
        .join("plugin-data")
        .join(plugin_id.as_str())
        .join("secrets.json"))
}

fn private_path(
    data_dir: &Path,
    plugin_id: &PluginId,
    relative_path: &str,
) -> Result<PathBuf, HostError> {
    let path = Path::new(relative_path);
    if relative_path.is_empty()
        || relative_path.len() > 256
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(HostError::new("invalid private file path"));
    }
    Ok(data_dir
        .join("plugin-data")
        .join(plugin_id.as_str())
        .join("files")
        .join(path))
}

fn file_error(operation: &str, path: &Path, error: std::io::Error) -> HostError {
    let code = error
        .raw_os_error()
        .map(|value| format!("，系统错误码 {value}"))
        .unwrap_or_default();
    HostError::new(format!(
        "{operation}失败：{}{code}（路径：{}）",
        error,
        path.display()
    ))
}

#[cfg(windows)]
fn harden_private_file(path: &Path) -> Result<(), HostError> {
    let output = Command::new("whoami")
        .output()
        .map_err(|error| HostError::new(format!("无法获取当前 Windows 用户：{error}")))?;
    if !output.status.success() {
        return Err(HostError::new("无法获取当前 Windows 用户"));
    }
    let principal = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if principal.is_empty() {
        return Err(HostError::new("无法获取当前 Windows 用户"));
    }
    let grant = format!("{principal}:(M)");
    let output = Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(grant)
        .output()
        .map_err(|error| HostError::new(format!("无法设置私有文件权限：{error}")))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let code = output
            .status
            .code()
            .map(|value| format!("，退出码 {value}"))
            .unwrap_or_default();
        return Err(HostError::new(if detail.is_empty() {
            format!("无法设置私有文件权限{code}")
        } else {
            format!("无法设置私有文件权限：{detail}{code}")
        }));
    }
    Ok(())
}

#[cfg(not(windows))]
fn harden_private_file(_path: &Path) -> Result<(), HostError> {
    Ok(())
}

#[cfg(windows)]
mod icon {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use base64::Engine as _;
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
    };
    use windows_sys::Win32::UI::Shell::SHDefExtractIconW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

    const ICON_SIZE: u32 = 64 | (32 << 16);
    const MEM_CACHE_CAP: usize = 512;

    fn mem_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
        static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn icon_base64(raw: &str, data_dir: &Path) -> Option<String> {
        let exe = expand_env(raw).trim().trim_matches('"').to_string();
        if exe.is_empty() || !Path::new(&exe).is_file() {
            return None;
        }
        let key = exe.to_lowercase();
        if let Some(hit) = lock(mem_cache()).get(&key) {
            return hit.clone();
        }
        let result = load_or_extract(&exe, &data_dir.join("icons"));
        let mut cache = lock(mem_cache());
        if cache.len() >= MEM_CACHE_CAP {
            cache.clear();
        }
        cache.insert(key, result.clone());
        result
    }

    fn expand_env(value: &str) -> String {
        let mut output = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(start) = rest.find('%') {
            output.push_str(&rest[..start]);
            rest = &rest[start + 1..];
            match rest.find('%') {
                Some(end) => {
                    let name = &rest[..end];
                    output.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
                    rest = &rest[end + 1..];
                }
                None => {
                    output.push('%');
                    break;
                }
            }
        }
        output.push_str(rest);
        output
    }

    fn load_or_extract(exe: &str, icons_dir: &Path) -> Option<String> {
        let cache_file = disk_cache_file(icons_dir, Path::new(exe));
        if let Some(file) = &cache_file {
            if let Ok(bytes) = std::fs::read(file) {
                return Some(encode_b64(&bytes));
            }
        }
        let png = extract_png(exe)?;
        if let Some(file) = &cache_file {
            let _ = std::fs::create_dir_all(icons_dir);
            let _ = std::fs::write(file, &png);
        }
        Some(encode_b64(&png))
    }

    fn encode_b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn disk_cache_file(icons_dir: &Path, source: &Path) -> Option<PathBuf> {
        let nanos = std::fs::metadata(source)
            .ok()?
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        let mut input = source.to_string_lossy().into_owned().into_bytes();
        input.extend_from_slice(nanos.to_string().as_bytes());
        Some(icons_dir.join(format!("{:016x}.png", fnv1a64(&input))))
    }

    fn fnv1a64(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    }

    fn extract_png(exe: &str) -> Option<Vec<u8>> {
        let wide: Vec<u16> = exe.encode_utf16().chain(std::iter::once(0)).collect();
        let mut hicon: HICON = std::ptr::null_mut();
        let result = unsafe {
            SHDefExtractIconW(
                wide.as_ptr(),
                0,
                0,
                &mut hicon,
                std::ptr::null_mut(),
                ICON_SIZE,
            )
        };
        if result < 0 || hicon.is_null() {
            return None;
        }
        let mut info: ICONINFO = unsafe { std::mem::zeroed() };
        if unsafe { GetIconInfo(hicon, &mut info) } == 0 {
            unsafe { DestroyIcon(hicon) };
            return None;
        }
        let output = hicon_to_png(&info);
        unsafe { DeleteObject(info.hbmMask) };
        if !info.hbmColor.is_null() {
            unsafe { DeleteObject(info.hbmColor) };
        }
        unsafe { DestroyIcon(hicon) };
        output
    }

    fn hicon_to_png(info: &ICONINFO) -> Option<Vec<u8>> {
        if info.hbmColor.is_null() {
            return None;
        }
        let (width, height) = bitmap_size(info.hbmColor)?;
        if width <= 0 || height <= 0 || width > 1024 || height > 2048 {
            return None;
        }
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let copied = with_screen_dc(|dc| {
            let mut header = dib_header(width, -height, 32);
            unsafe {
                GetDIBits(
                    dc,
                    info.hbmColor,
                    0,
                    height as u32,
                    pixels.as_mut_ptr() as *mut core::ffi::c_void,
                    &mut header,
                    DIB_RGB_COLORS,
                )
            }
        });
        if copied == 0 {
            return None;
        }
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        if !pixels.chunks_exact(4).any(|pixel| pixel[3] != 0) {
            let mask = read_mask(info.hbmMask, width, height)?;
            let stride = mask_stride(width as usize);
            for (y, row) in pixels.chunks_exact_mut(4 * width as usize).enumerate() {
                for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                    let set = mask
                        .get(y * stride + x / 8)
                        .is_some_and(|byte| byte & (0x80 >> (x % 8)) != 0);
                    pixel[3] = if set { 0 } else { 255 };
                }
            }
        }
        encode_png(&pixels, width, height)
    }

    fn bitmap_size(bitmap: HBITMAP) -> Option<(i32, i32)> {
        if bitmap.is_null() {
            return None;
        }
        let mut value: BITMAP = unsafe { std::mem::zeroed() };
        let result = unsafe {
            GetObjectW(
                bitmap,
                std::mem::size_of::<BITMAP>() as i32,
                &mut value as *mut BITMAP as *mut core::ffi::c_void,
            )
        };
        (result != 0).then_some((value.bmWidth, value.bmHeight))
    }

    fn mask_stride(width: usize) -> usize {
        (width + 31) / 32 * 4
    }

    fn read_mask(bitmap: HBITMAP, width: i32, height: i32) -> Option<Vec<u8>> {
        let mask_height = bitmap_size(bitmap)?.1;
        if mask_height < height {
            return None;
        }
        let stride = mask_stride(width as usize);
        let mut bits = vec![0u8; stride * mask_height as usize];
        let copied = with_screen_dc(|dc| {
            let mut header = dib_header(width, -mask_height, 1);
            unsafe {
                GetDIBits(
                    dc,
                    bitmap,
                    0,
                    mask_height as u32,
                    bits.as_mut_ptr() as *mut core::ffi::c_void,
                    &mut header,
                    DIB_RGB_COLORS,
                )
            }
        });
        (copied != 0).then_some(bits)
    }

    fn dib_header(width: i32, height: i32, bit_count: u16) -> BITMAPINFO {
        BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: height,
                biPlanes: 1,
                biBitCount: bit_count,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [unsafe { std::mem::zeroed() }],
        }
    }

    fn with_screen_dc<T>(callback: impl FnOnce(HDC) -> T) -> T {
        let dc = unsafe { GetDC(std::ptr::null_mut()) };
        let result = callback(dc);
        if !dc.is_null() {
            unsafe { ReleaseDC(std::ptr::null_mut(), dc) };
        }
        result
    }

    fn encode_png(pixels: &[u8], width: i32, height: i32) -> Option<Vec<u8>> {
        let mut output = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut output, width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(pixels).ok()?;
        }
        Some(output)
    }
}
