//! In-process adapter API. The request/response types remain the wire contract.

use sideweave_plugin_api::{
    PluginEvent, PluginInfo, PluginManifest, PluginRequest, PluginRequestKind, PluginResponse,
    PluginTrust,
};

pub use sideweave_plugin_api as plugin_api;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[cfg(feature = "mock")]
pub mod mock;

pub mod dylib;

pub trait Plugin: Send {
    fn manifest(&self) -> &PluginManifest;

    fn handle(&mut self, request: PluginRequest, ctx: &PluginContext) -> PluginResponse;
}

pub trait StorageService: Send + Sync {
    fn get(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        key: &str,
    ) -> Result<Option<String>, HostError>;
    fn set(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        key: &str,
        value: &str,
    ) -> Result<(), HostError>;
    fn remove(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        key: &str,
    ) -> Result<(), HostError>;
}

/// Stores small plugin secrets outside the regular plugin state file.
///
/// The host owns encryption and user-profile binding. Plugins must never persist
/// credentials in `StorageService` themselves.
pub trait SecretService: Send + Sync {
    fn get(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        key: &str,
    ) -> Result<Option<String>, HostError>;
    fn set(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        key: &str,
        value: &str,
    ) -> Result<(), HostError>;
    fn remove(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        key: &str,
    ) -> Result<(), HostError>;
}

struct UnsupportedSecretService;

impl SecretService for UnsupportedSecretService {
    fn get(
        &self,
        _plugin_id: &sideweave_plugin_api::PluginId,
        _key: &str,
    ) -> Result<Option<String>, HostError> {
        Err(HostError::new("secret storage is not supported by this plugin runtime"))
    }

    fn set(
        &self,
        _plugin_id: &sideweave_plugin_api::PluginId,
        _key: &str,
        _value: &str,
    ) -> Result<(), HostError> {
        Err(HostError::new("secret storage is not supported by this plugin runtime"))
    }

    fn remove(
        &self,
        _plugin_id: &sideweave_plugin_api::PluginId,
        _key: &str,
    ) -> Result<(), HostError> {
        Err(HostError::new("secret storage is not supported by this plugin runtime"))
    }
}

static UNSUPPORTED_SECRETS: UnsupportedSecretService = UnsupportedSecretService;

/// Content and platform-supported options for an operating-system notification.
/// The operating system controls the notification's visual layout.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationOptions {
    pub title: String,
    pub body: String,
    /// An icon name or path understood by the host platform.
    pub icon: Option<String>,
    /// A sound name understood by the host platform.
    pub sound: Option<String>,
}

impl NotificationOptions {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            ..Self::default()
        }
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_sound(mut self, sound: impl Into<String>) -> Self {
        self.sound = Some(sound.into());
        self
    }
}

/// Visual style for a host-rendered desktop notification.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopNotificationKind {
    #[default]
    Info,
    Success,
    Warning,
    Alarm,
}

/// Content and presentation hints for a notification rendered by the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DesktopNotificationOptions {
    pub title: String,
    pub body: String,
    pub kind: DesktopNotificationKind,
    /// How long the notification remains visible, in milliseconds.
    pub duration_ms: u64,
}

impl Default for DesktopNotificationOptions {
    fn default() -> Self {
        Self {
            title: String::new(),
            body: String::new(),
            kind: DesktopNotificationKind::Info,
            duration_ms: 6_000,
        }
    }
}

impl DesktopNotificationOptions {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            ..Self::default()
        }
    }

    pub fn with_kind(mut self, kind: DesktopNotificationKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }
}

pub trait NotificationService: Send + Sync {
    fn notify(&self, title: &str, body: &str) -> Result<(), HostError>;

    /// Sends a notification with optional native icon and sound hints.
    /// Existing host implementations retain compatibility through this default.
    fn notify_with(&self, notification: &NotificationOptions) -> Result<(), HostError> {
        self.notify(&notification.title, &notification.body)
    }

    /// Shows a custom desktop notification using a host-managed window.
    fn show_desktop(&self, _notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        Err(HostError::new("host desktop notifications are not supported"))
    }

    /// Shows a desktop notification on behalf of a plugin so the host can use its icon.
    fn show_desktop_for_plugin(
        &self,
        _plugin_id: &sideweave_plugin_api::PluginId,
        notification: &DesktopNotificationOptions,
    ) -> Result<(), HostError> {
        self.show_desktop(notification)
    }

    /// Shows a custom notification beside the host sidebar.
    fn show_sidebar(&self, _notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        Err(HostError::new("host sidebar notifications are not supported"))
    }

    /// Shows a sidebar notification on behalf of a plugin so the host can use its icon.
    fn show_sidebar_for_plugin(
        &self,
        _plugin_id: &sideweave_plugin_api::PluginId,
        notification: &DesktopNotificationOptions,
    ) -> Result<(), HostError> {
        self.show_sidebar(notification)
    }
}

struct ContextNotificationService {
    plugin_id: sideweave_plugin_api::PluginId,
    services: Arc<dyn HostServices>,
}

impl NotificationService for ContextNotificationService {
    fn notify(&self, title: &str, body: &str) -> Result<(), HostError> {
        self.services.notification().notify(title, body)
    }

    fn notify_with(&self, notification: &NotificationOptions) -> Result<(), HostError> {
        self.services.notification().notify_with(notification)
    }

    fn show_desktop(&self, notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        self.services
            .notification()
            .show_desktop_for_plugin(&self.plugin_id, notification)
    }

    fn show_sidebar(&self, notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        self.services
            .notification()
            .show_sidebar_for_plugin(&self.plugin_id, notification)
    }

    fn show_desktop_for_plugin(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        notification: &DesktopNotificationOptions,
    ) -> Result<(), HostError> {
        self.services
            .notification()
            .show_desktop_for_plugin(plugin_id, notification)
    }

    fn show_sidebar_for_plugin(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        notification: &DesktopNotificationOptions,
    ) -> Result<(), HostError> {
        self.services
            .notification()
            .show_sidebar_for_plugin(plugin_id, notification)
    }
}

pub trait EventBus: Send + Sync {
    fn publish(&self, event: PluginEvent) -> Result<(), HostError>;
}

pub trait SystemService: Send + Sync {
    fn open_external(&self, url: &str) -> Result<(), HostError>;
    fn spawn(&self, program: &str, args: &[String]) -> Result<(), HostError>;
    fn spawn_with_cwd(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<(), HostError> {
        let _ = cwd;
        self.spawn(program, args)
    }
    fn pick_file(&self, extensions: &[String]) -> Result<Option<String>, HostError>;
    fn extract_icon(&self, path: &str) -> Result<Option<String>, HostError> {
        let _ = path;
        Ok(None)
    }
}

pub trait FileService: Send + Sync {
    fn read_private(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        relative_path: &str,
    ) -> Result<Option<Vec<u8>>, HostError>;
    fn write_private(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        relative_path: &str,
        bytes: &[u8],
    ) -> Result<(), HostError>;
    fn remove_private(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        relative_path: &str,
    ) -> Result<(), HostError>;
    fn read_external(&self, path: &str) -> Result<Vec<u8>, HostError>;
    fn read_external_range(
        &self,
        path: &str,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Vec<u8>, HostError> {
        let bytes = self.read_external(path)?;
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let Some(remaining) = bytes.get(start..) else {
            return Ok(Vec::new());
        };
        Ok(remaining.iter().take(max_bytes).copied().collect())
    }
    fn private_path(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        relative_path: &str,
    ) -> Result<String, HostError> {
        let _ = (plugin_id, relative_path);
        Err(HostError::new(
            "private file paths are not exposed by this host",
        ))
    }
}

pub trait ClipboardService: Send + Sync {
    fn read_text(&self) -> Result<Option<String>, HostError>;
    fn write_text(&self, text: &str) -> Result<(), HostError>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    Toggle,
    Previous,
    Next,
}

pub type ShortcutHandler = Arc<dyn Fn(ShortcutAction) + Send + Sync>;

pub trait GlobalShortcutService: Send + Sync {
    fn register(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
        bindings: &[(String, ShortcutAction)],
        handler: ShortcutHandler,
    ) -> Result<(), HostError>;
    fn unregister(&self, plugin_id: &sideweave_plugin_api::PluginId) -> Result<(), HostError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSpec {
    pub id: String,
    pub title: String,
    pub url: String,
    pub width: f64,
    pub height: f64,
    pub position_px: Option<(i32, i32)>,
    pub always_on_top: bool,
    pub skip_taskbar: bool,
    pub resizable: bool,
    pub transparent: bool,
    pub decorations: bool,
    pub maximizable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum WindowEvent {
    Moved { x: i32, y: i32 },
    Resized { width: f64, height: f64 },
    Closed,
    NativeHandleAvailable { handle: u64 },
    VisibilityChanged { visible: bool },
}

pub trait WindowEventHandler: Send + Sync {
    fn on_event(&self, event: WindowEvent);
}

pub trait WindowService: Send + Sync {
    fn create(
        &self,
        spec: WindowSpec,
        handler: Option<Arc<dyn WindowEventHandler>>,
    ) -> Result<(), HostError>;
    fn show(&self, id: &str) -> Result<(), HostError>;
    fn hide(&self, id: &str) -> Result<(), HostError>;
    fn close(&self, id: &str) -> Result<(), HostError>;
    fn emit(&self, id: &str, topic: &str, payload: Value) -> Result<(), HostError>;
    fn set_clickthrough(&self, id: &str, enabled: bool) -> Result<(), HostError>;
    fn set_size(&self, id: &str, width: f64, height: f64) -> Result<(), HostError> {
        let _ = (id, width, height);
        Err(HostError::new("window resizing is not supported"))
    }
    fn set_position(&self, id: &str, x: i32, y: i32) -> Result<(), HostError> {
        let _ = (id, x, y);
        Err(HostError::new("window positioning is not supported"))
    }
    fn position(&self, id: &str) -> Result<Option<(i32, i32)>, HostError> {
        let _ = id;
        Err(HostError::new("window positioning is not supported"))
    }
    fn focus(&self, id: &str) -> Result<(), HostError> {
        let _ = id;
        Err(HostError::new("window focus is not supported"))
    }
    fn close_plugin_windows(
        &self,
        plugin_id: &sideweave_plugin_api::PluginId,
    ) -> Result<(), HostError> {
        let _ = plugin_id;
        Ok(())
    }
}

pub trait HostServices: Send + Sync {
    /// Checks the user's grant for a manifest-declared host capability.
    /// Non-Tauri hosts may keep the compatibility default and allow requests.
    fn permission_granted(
        &self,
        _plugin_id: &sideweave_plugin_api::PluginId,
        _permission: &str,
    ) -> Result<bool, HostError> {
        Ok(true)
    }
    fn storage(&self) -> &dyn StorageService;
    fn secrets(&self) -> &dyn SecretService {
        &UNSUPPORTED_SECRETS
    }
    fn notification(&self) -> &dyn NotificationService;
    fn events(&self) -> &dyn EventBus;
    fn system(&self) -> &dyn SystemService;
    fn files(&self) -> &dyn FileService;
    fn clipboard(&self) -> &dyn ClipboardService;
    fn shortcuts(&self) -> &dyn GlobalShortcutService;
    fn windows(&self) -> &dyn WindowService;
}

pub struct PluginContext {
    plugin_id: sideweave_plugin_api::PluginId,
    services: Arc<dyn HostServices>,
    notification_service: ContextNotificationService,
}

impl PluginContext {
    pub fn new(
        plugin_id: sideweave_plugin_api::PluginId,
        services: Arc<dyn HostServices>,
    ) -> Self {
        let notification_service = ContextNotificationService {
            plugin_id: plugin_id.clone(),
            services: Arc::clone(&services),
        };
        Self {
            plugin_id,
            services,
            notification_service,
        }
    }

    pub fn plugin_id(&self) -> &sideweave_plugin_api::PluginId {
        &self.plugin_id
    }

    pub fn services(&self) -> &dyn HostServices {
        self.services.as_ref()
    }

    pub fn services_arc(&self) -> Arc<dyn HostServices> {
        Arc::clone(&self.services)
    }

    pub fn storage(&self) -> &dyn StorageService {
        self.services.storage()
    }

    pub fn secrets(&self) -> &dyn SecretService {
        self.services.secrets()
    }

    pub fn notification(&self) -> &dyn NotificationService {
        &self.notification_service
    }

    pub fn events(&self) -> &dyn EventBus {
        self.services.events()
    }

    pub fn system(&self) -> &dyn SystemService {
        self.services.system()
    }

    pub fn files(&self) -> &dyn FileService {
        self.services.files()
    }

    pub fn clipboard(&self) -> &dyn ClipboardService {
        self.services.clipboard()
    }

    pub fn shortcuts(&self) -> &dyn GlobalShortcutService {
        self.services.shortcuts()
    }

    pub fn windows(&self) -> &dyn WindowService {
        self.services.windows()
    }
}

pub fn lifecycle_request(
    plugin_id: sideweave_plugin_api::PluginId,
    kind: PluginRequestKind,
) -> PluginRequest {
    PluginRequest {
        request_id: uuid_like_id(),
        plugin_id,
        kind,
        method: None,
        payload: Value::Null,
        timeout_ms: None,
    }
}

pub fn success_response(request: &PluginRequest, result: Value) -> PluginResponse {
    PluginResponse {
        request_id: request.request_id.clone(),
        plugin_id: request.plugin_id.clone(),
        ok: true,
        result,
        error: None,
    }
}

pub fn plugin_info(plugin: &dyn Plugin) -> PluginInfo {
    PluginInfo {
        manifest: plugin.manifest().clone(),
        status: sideweave_plugin_api::PluginStatus::Loaded,
        runtime: sideweave_plugin_api::PluginRuntime::InProcess,
        location: "in-process".into(),
        source: "workspace".into(),
        effective_trust: PluginTrust::Sandboxed,
        last_error: None,
    }
}

#[derive(Debug, Clone)]
pub struct HostError {
    pub message: String,
}

impl HostError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

fn uuid_like_id() -> String {
    // The SDK deliberately avoids making UUID a wire-level requirement.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("req-{nanos:x}")
}
