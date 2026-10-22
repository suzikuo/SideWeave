//! Serializable protocol shared by in-process and external SideWeave plugins.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::ffi::c_void;

pub const MANIFEST_VERSION: u32 = 1;
pub const API_VERSION: u32 = 1;
pub const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 8_000;
pub const MAX_REQUEST_TIMEOUT_MS: u64 = 60_000;
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;
pub const DYNAMIC_PLUGIN_ABI_VERSION: u32 = 1;
pub const DYNAMIC_PLUGIN_ENTRY_SYMBOL: &[u8] = b"sideweave_plugin_entry\0";
pub const LEGACY_DYNAMIC_PLUGIN_ENTRY_SYMBOL: &[u8] =
    concat!("agile", "_tiles_plugin_entry\0").as_bytes();

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(ProtocolError::InvalidPluginId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntime {
    InProcess,
    DynamicLibrary,
    ExternalProcess,
    WebSandbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginTrust {
    #[default]
    Sandboxed,
    TrustedLocal,
}

pub mod permissions {
    pub const STORAGE: &str = "storage";
    /// Legacy umbrella permission accepted by manifests written before notification types were split.
    pub const NOTIFICATION: &str = "notification";
    pub const NOTIFICATION_SYSTEM: &str = "notification.system";
    pub const NOTIFICATION_DESKTOP: &str = "notification.desktop";
    pub const NOTIFICATION_SIDEBAR: &str = "notification.sidebar";
    pub const EVENT_PUBLISH: &str = "event.publish";
    pub const FILESYSTEM_READ: &str = "filesystem.read";
    pub const FILESYSTEM_WRITE: &str = "filesystem.write";
    pub const FILESYSTEM_EXTERNAL_READ: &str = "filesystem.external_read";
    pub const CLIPBOARD: &str = "clipboard";
    pub const PROCESS_SPAWN: &str = "process.spawn";
    pub const NETWORK_LISTEN: &str = "network.listen";
    pub const NETWORK_CONNECT: &str = "network.connect";
    pub const WINDOW_CREATE: &str = "window.create";
    pub const WINDOW_MANAGE: &str = "window.manage";
    pub const SHORTCUT_GLOBAL: &str = "shortcut.global";
    pub const SYSTEM_OPEN_EXTERNAL: &str = "system.open_external";
    pub const SYSTEM_FILE_PICKER: &str = "system.file_picker";
    pub const SYSTEM_ICON: &str = "system.icon";

    pub const ALL: &[&str] = &[
        STORAGE,
        NOTIFICATION,
        NOTIFICATION_SYSTEM,
        NOTIFICATION_DESKTOP,
        NOTIFICATION_SIDEBAR,
        EVENT_PUBLISH,
        FILESYSTEM_READ,
        FILESYSTEM_WRITE,
        FILESYSTEM_EXTERNAL_READ,
        CLIPBOARD,
        PROCESS_SPAWN,
        NETWORK_LISTEN,
        NETWORK_CONNECT,
        WINDOW_CREATE,
        WINDOW_MANAGE,
        SHORTCUT_GLOBAL,
        SYSTEM_OPEN_EXTERNAL,
        SYSTEM_FILE_PICKER,
        SYSTEM_ICON,
    ];

    pub fn is_known(value: &str) -> bool {
        ALL.contains(&value)
    }

    /// Whether a manifest requested a permission. The old `notification`
    /// permission remains a compatibility alias for all three notification surfaces.
    pub fn is_declared(requested: &[String], permission: &str) -> bool {
        requested.iter().any(|item| item == permission)
            || (matches!(
                permission,
                NOTIFICATION_SYSTEM | NOTIFICATION_DESKTOP | NOTIFICATION_SIDEBAR
            ) && requested.iter().any(|item| item == NOTIFICATION))
    }

    /// Maps a host bridge method to the capability it requires.
    pub fn for_host_method(method: &str) -> Option<&'static str> {
        match method {
            "host.storage.get"
            | "host.storage.set"
            | "host.storage.remove"
            | "host.secrets.get"
            | "host.secrets.set"
            | "host.secrets.remove" => Some(STORAGE),
            "host.notification.notify" => Some(NOTIFICATION_SYSTEM),
            "host.notification.show_desktop" => Some(NOTIFICATION_DESKTOP),
            "host.notification.show_sidebar" => Some(NOTIFICATION_SIDEBAR),
            "host.events.publish" => Some(EVENT_PUBLISH),
            "host.system.open_external" => Some(SYSTEM_OPEN_EXTERNAL),
            "host.system.spawn" | "host.system.spawn_with_cwd" => Some(PROCESS_SPAWN),
            "host.system.pick_file" => Some(SYSTEM_FILE_PICKER),
            "host.system.extract_icon" => Some(SYSTEM_ICON),
            "host.files.read_private" | "host.files.private_path" => Some(FILESYSTEM_READ),
            "host.files.write_private" | "host.files.remove_private" => Some(FILESYSTEM_WRITE),
            "host.files.read_external" | "host.files.read_external_range" => {
                Some(FILESYSTEM_EXTERNAL_READ)
            }
            "host.clipboard.read" | "host.clipboard.read_text" | "host.clipboard.write"
            | "host.clipboard.write_text" => Some(CLIPBOARD),
            "host.shortcuts.register" | "host.shortcuts.unregister" => Some(SHORTCUT_GLOBAL),
            "host.windows.create" => Some(WINDOW_CREATE),
            "host.windows.show"
            | "host.windows.hide"
            | "host.windows.close"
            | "host.windows.set_clickthrough"
            | "host.windows.set_size"
            | "host.windows.set_position"
            | "host.windows.position"
            | "host.windows.focus"
            | "host.windows.emit" => Some(WINDOW_MANAGE),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginUiKind {
    Html,
    Vue,
    React,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginUiChrome {
    Host,
    Plugin,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct PluginUi {
    pub entry: String,
    pub kind: Option<PluginUiKind>,
    pub asset_root: String,
    pub chrome: Option<PluginUiChrome>,
    pub initial_width: Option<u32>,
    pub initial_height: Option<u32>,
    pub min_width: Option<u32>,
    pub min_height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct PluginHoverPreview {
    /// Preferred width for the host-rendered data preview; custom `entry` previews use it as their viewport width.
    pub width: Option<u32>,
    /// Viewport height for custom `entry` previews. Host-rendered previews size themselves from their content and host settings.
    pub height: Option<u32>,
    pub data_method: Option<String>,
    pub entry: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub manifest_version: u32,
    pub api_version: u32,
    pub id: PluginId,
    pub name: String,
    /// Relative icon asset path inside the plugin package; missing or unsupported assets use the host puzzle icon.
    #[serde(default)]
    pub icon: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub runtime: PluginRuntime,
    /// External-process executable or dynamic-library file relative to the package root.
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub trust: PluginTrust,
    /// Start this backend after the host UI is ready. Old manifests default to true for compatibility.
    #[serde(default = "default_auto_start")]
    pub auto_start: bool,
    #[serde(default, rename = "capabilities", alias = "permissions")]
    pub permissions: Vec<String>,
    /// Capabilities that the host must keep granted because the plugin cannot operate without them.
    #[serde(default, rename = "requiredCapabilities")]
    pub required_permissions: Vec<String>,
    #[serde(default)]
    pub ui: Option<PluginUi>,
    #[serde(default)]
    pub hover_preview: Option<PluginHoverPreview>,
    #[serde(default)]
    pub minimum_host_version: Option<String>,
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self {
            manifest_version: MANIFEST_VERSION,
            api_version: API_VERSION,
            id: PluginId::new("default").expect("default plugin id"),
            name: String::new(),
            icon: String::new(),
            version: String::new(),
            description: String::new(),
            author: String::new(),
            runtime: PluginRuntime::InProcess,
            backend: None,
            trust: PluginTrust::Sandboxed,
            auto_start: true,
            permissions: Vec::new(),
            required_permissions: Vec::new(),
            ui: None,
            hover_preview: None,
            minimum_host_version: None,
        }
    }
}

fn default_auto_start() -> bool {
    true
}

impl PluginManifest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(ProtocolError::UnsupportedManifestVersion(
                self.manifest_version,
            ));
        }
        if self.api_version != API_VERSION {
            return Err(ProtocolError::UnsupportedApiVersion(self.api_version));
        }
        if self.name.trim().is_empty() || self.version.trim().is_empty() {
            return Err(ProtocolError::InvalidManifest(
                "name/version cannot be empty".into(),
            ));
        }
        if let Some(ui) = &self.ui {
            if ui.entry.trim().is_empty() || !is_safe_relative_path(&ui.entry) {
                return Err(ProtocolError::InvalidManifest(
                    "ui.entry must be a safe relative path when ui is present".into(),
                ));
            }
            if !ui.asset_root.is_empty()
                && ui.asset_root != "."
                && !is_safe_relative_path(&ui.asset_root)
            {
                return Err(ProtocolError::InvalidManifest(
                    "ui.asset_root must be a safe relative path".into(),
                ));
            }
        }
        if let Some(backend) = &self.backend {
            if !matches!(
                self.runtime,
                PluginRuntime::ExternalProcess | PluginRuntime::DynamicLibrary
            ) {
                return Err(ProtocolError::InvalidManifest(
                    "backend is only valid for external_process or dynamic_library runtime".into(),
                ));
            }
            if backend.trim().is_empty() || !is_safe_relative_path(backend) {
                return Err(ProtocolError::InvalidManifest(
                    "backend must be a safe relative path".into(),
                ));
            }
        }
        if let Some(permission) = self
            .permissions
            .iter()
            .find(|permission| !permissions::is_known(permission))
        {
            return Err(ProtocolError::InvalidManifest(format!(
                "unknown permission: {permission}"
            )));
        }
        if let Some(permission) = self.required_permissions.iter().find(|permission| {
            !permissions::is_known(permission)
                || !permissions::is_declared(&self.permissions, permission)
        }) {
            return Err(ProtocolError::InvalidManifest(format!(
                "required capability must be a declared permission: {permission}"
            )));
        }
        if let Some(hover) = &self.hover_preview {
            if hover.data_method.is_some() && hover.entry.is_some() {
                return Err(ProtocolError::InvalidManifest(
                    "hover_preview cannot specify both data_method and entry".into(),
                ));
            }
            if let Some(entry) = &hover.entry {
                if entry.trim().is_empty() || !is_safe_relative_path(entry) {
                    return Err(ProtocolError::InvalidManifest(
                        "hover_preview.entry must be a safe relative path".into(),
                    ));
                }
            }
            if let Some(data_method) = &hover.data_method {
                if data_method.trim().is_empty() {
                    return Err(ProtocolError::InvalidManifest(
                        "hover_preview.data_method cannot be empty".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn is_safe_relative_path(value: &str) -> bool {
    let path = std::path::Path::new(value);
    !value.is_empty()
        && path.is_relative()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRequestKind {
    Activate,
    Deactivate,
    Shutdown,
    Call,
    GetInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRequest {
    #[serde(alias = "requestId")]
    pub request_id: String,
    #[serde(alias = "pluginId")]
    pub plugin_id: PluginId,
    pub kind: PluginRequestKind,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default, alias = "timeoutMs")]
    pub timeout_ms: Option<u64>,
}

impl PluginRequest {
    pub fn validate_for(&self, expected_plugin_id: &PluginId) -> Result<(), ProtocolError> {
        if self.request_id.trim().is_empty() || self.request_id.len() > 128 {
            return Err(ProtocolError::InvalidRequest(
                "request_id must be 1..=128 characters".into(),
            ));
        }
        if &self.plugin_id != expected_plugin_id {
            return Err(ProtocolError::InvalidRequest(
                "request plugin id does not match the target plugin".into(),
            ));
        }
        if let Some(timeout_ms) = self.timeout_ms {
            if timeout_ms == 0 || timeout_ms > MAX_REQUEST_TIMEOUT_MS {
                return Err(ProtocolError::InvalidRequest(format!(
                    "timeout_ms must be between 1 and {MAX_REQUEST_TIMEOUT_MS}"
                )));
            }
        }
        if matches!(self.kind, PluginRequestKind::Call)
            && self.method.as_deref().unwrap_or_default().trim().is_empty()
        {
            return Err(ProtocolError::InvalidRequest(
                "call requests require a method".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginResponse {
    pub request_id: String,
    pub plugin_id: PluginId,
    pub ok: bool,
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub error: Option<PluginError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginHandshake {
    pub request_id: String,
    pub plugin_id: PluginId,
    pub api_version: u32,
    #[serde(default)]
    pub host_version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginHandshakeResponse {
    pub request_id: String,
    pub plugin_id: PluginId,
    pub accepted: bool,
    pub api_version: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub error: Option<PluginError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum PluginWireMessage {
    Handshake(PluginHandshake),
    HandshakeResponse(PluginHandshakeResponse),
    Request(PluginRequest),
    Response(PluginResponse),
    Event(PluginEvent),
}

impl PluginWireMessage {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| ProtocolError::Serialization(error.to_string()))?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(ProtocolError::MessageTooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(ProtocolError::MessageTooLarge(bytes.len()));
        }
        serde_json::from_slice(bytes)
            .map_err(|error| ProtocolError::Serialization(error.to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEvent {
    pub plugin_id: PluginId,
    pub topic: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    InvalidManifest,
    UnsupportedApiVersion,
    PermissionDenied,
    NotFound,
    Timeout,
    PluginUnavailable,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginStatus {
    Discovered,
    Loaded,
    Active,
    Disabled,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub status: PluginStatus,
    pub runtime: PluginRuntime,
    pub location: String,
    pub source: String,
    /// Effective trust granted by the host; never taken directly from the manifest.
    #[serde(default)]
    pub effective_trust: PluginTrust,
    #[serde(default)]
    pub last_error: Option<PluginError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidPluginId,
    InvalidRequest(String),
    MessageTooLarge(usize),
    Serialization(String),
    UnsupportedManifestVersion(u32),
    UnsupportedApiVersion(u32),
    InvalidManifest(String),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPluginId => write!(f, "invalid plugin id"),
            Self::InvalidRequest(message) => write!(f, "invalid plugin request: {message}"),
            Self::MessageTooLarge(size) => write!(f, "plugin message is too large: {size} bytes"),
            Self::Serialization(message) => {
                write!(f, "plugin protocol serialization failed: {message}")
            }
            Self::UnsupportedManifestVersion(version) => {
                write!(f, "unsupported manifest version: {version}")
            }
            Self::UnsupportedApiVersion(version) => {
                write!(f, "unsupported plugin api version: {version}")
            }
            Self::InvalidManifest(message) => write!(f, "invalid plugin manifest: {message}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// A byte buffer allocated by the component that returns it.
///
/// The receiver must release it through the matching `free_buffer` callback.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PluginBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub capacity: usize,
}

impl PluginBuffer {
    pub const fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
            capacity: 0,
        }
    }
}

/// Host callback table supplied to a dynamic-library plugin for each call.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PluginHostApi {
    pub abi_version: u32,
    pub user_data: *mut c_void,
    pub call: Option<
        unsafe extern "C" fn(
            user_data: *mut c_void,
            method_ptr: *const u8,
            method_len: usize,
            payload_ptr: *const u8,
            payload_len: usize,
        ) -> PluginBuffer,
    >,
    pub free_buffer:
        Option<unsafe extern "C" fn(user_data: *mut c_void, buffer: PluginBuffer)>,
}

/// Versioned C ABI exposed by every dynamic-library plugin.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PluginDylibApi {
    pub abi_version: u32,
    pub manifest: Option<unsafe extern "C" fn() -> PluginBuffer>,
    pub create: Option<unsafe extern "C" fn() -> *mut c_void>,
    pub destroy: Option<unsafe extern "C" fn(state: *mut c_void)>,
    pub handle: Option<
        unsafe extern "C" fn(
            state: *mut c_void,
            request_ptr: *const u8,
            request_len: usize,
            host: PluginHostApi,
        ) -> PluginBuffer,
    >,
    pub handle_event: Option<
        unsafe extern "C" fn(
            state: *mut c_void,
            event_ptr: *const u8,
            event_len: usize,
            host: PluginHostApi,
        ) -> PluginBuffer,
    >,
    pub free_buffer: Option<unsafe extern "C" fn(buffer: PluginBuffer)>,
}

pub type PluginDylibEntry = unsafe extern "C" fn() -> *const PluginDylibApi;

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> PluginManifest {
        PluginManifest {
            manifest_version: MANIFEST_VERSION,
            api_version: API_VERSION,
            id: PluginId::new("demo").unwrap(),
            name: "Demo".into(),
            icon: "puzzle".into(),
            version: "0.1.0".into(),
            description: String::new(),
            author: String::new(),
            runtime: PluginRuntime::InProcess,
            backend: None,
            trust: PluginTrust::Sandboxed,
            auto_start: false,
            permissions: vec![],
            required_permissions: vec![],
            ui: Some(PluginUi {
                entry: "ui/index.html".into(),
                ..Default::default()
            }),
            hover_preview: None,
            minimum_host_version: None,
        }
    }

    #[test]
    fn validates_manifest_and_round_trips_json() {
        let mut value = manifest();
        value.hover_preview = Some(PluginHoverPreview {
            width: Some(300),
            height: Some(200),
            data_method: Some("get_preview".into()),
            entry: None,
        });
        value.validate().unwrap();
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(
            json["hoverPreview"]["dataMethod"],
            serde_json::json!("get_preview")
        );
        let decoded: PluginManifest = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.id.as_str(), "demo");
        assert_eq!(decoded.hover_preview, value.hover_preview);

        // Custom preview entry
        let mut custom_value = manifest();
        custom_value.hover_preview = Some(PluginHoverPreview {
            width: Some(320),
            height: Some(220),
            data_method: None,
            entry: Some("preview.html".into()),
        });
        custom_value.validate().unwrap();
        let custom_json = serde_json::to_value(&custom_value).unwrap();
        assert_eq!(
            custom_json["hoverPreview"]["entry"],
            serde_json::json!("preview.html")
        );
    }

    #[test]
    fn rejects_conflicting_or_unsafe_hover_preview() {
        let mut conflicting = manifest();
        conflicting.hover_preview = Some(PluginHoverPreview {
            width: Some(300),
            height: Some(200),
            data_method: Some("get_preview".into()),
            entry: Some("preview.html".into()),
        });
        assert!(matches!(
            conflicting.validate(),
            Err(ProtocolError::InvalidManifest(_))
        ));

        let mut traversal = manifest();
        traversal.hover_preview = Some(PluginHoverPreview {
            width: Some(300),
            height: Some(200),
            data_method: None,
            entry: Some("../escape.html".into()),
        });
        assert!(matches!(
            traversal.validate(),
            Err(ProtocolError::InvalidManifest(_))
        ));

        let mut absolute = manifest();
        absolute.hover_preview = Some(PluginHoverPreview {
            width: Some(300),
            height: Some(200),
            data_method: None,
            entry: Some("/root.html".into()),
        });
        assert!(matches!(
            absolute.validate(),
            Err(ProtocolError::InvalidManifest(_))
        ));
    }

    #[test]
    fn rejects_parent_traversal_in_ui_paths() {
        let mut value = manifest();
        value.ui.as_mut().unwrap().entry = "../index.html".into();
        assert!(matches!(
            value.validate(),
            Err(ProtocolError::InvalidManifest(_))
        ));
    }

    #[test]
    fn rejects_unknown_permissions() {
        let mut value = manifest();
        value.permissions = vec!["storage.typo".into()];
        assert!(matches!(
            value.validate(),
            Err(ProtocolError::InvalidManifest(_))
        ));
    }

    #[test]
    fn validates_request_identity_method_and_timeout() {
        let id = PluginId::new("demo").unwrap();
        let request = PluginRequest {
            request_id: "request-1".into(),
            plugin_id: id.clone(),
            kind: PluginRequestKind::Call,
            method: Some("ping".into()),
            payload: Value::Null,
            timeout_ms: Some(DEFAULT_REQUEST_TIMEOUT_MS),
        };
        request.validate_for(&id).unwrap();

        let mut invalid = request.clone();
        invalid.method = None;
        assert!(matches!(
            invalid.validate_for(&id),
            Err(ProtocolError::InvalidRequest(_))
        ));

        invalid = request;
        invalid.timeout_ms = Some(MAX_REQUEST_TIMEOUT_MS + 1);
        assert!(matches!(
            invalid.validate_for(&id),
            Err(ProtocolError::InvalidRequest(_))
        ));
    }
}
