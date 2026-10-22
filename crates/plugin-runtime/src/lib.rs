//! Runtime registry and lifecycle orchestration for plugin adapters.

use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginEvent, PluginHandshake, PluginHandshakeResponse, PluginId,
    PluginInfo, PluginRequest, PluginRequestKind, PluginResponse, PluginStatus, PluginTrust,
    API_VERSION,
};
use sideweave_plugin_sdk::{lifecycle_request, HostServices, Plugin, PluginContext};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(windows)]
mod dylib;

#[cfg(windows)]
pub use dylib::DynamicPlugin;

pub struct PluginRegistry {
    entries: HashMap<PluginId, RegistryEntry>,
}

struct RegistryEntry {
    plugin: Option<Box<dyn Plugin>>,
    external: Option<Box<dyn ExternalPlugin>>,
    info: PluginInfo,
}

pub trait ExternalPlugin: Send {
    fn call(&mut self, request: PluginRequest) -> Result<PluginResponse, PluginError>;

    fn shutdown(&mut self) -> Result<(), PluginError> {
        Ok(())
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        plugin: Box<dyn Plugin>,
        source: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        let manifest = plugin.manifest();
        manifest.validate().map_err(RuntimeError::Protocol)?;
        let id = manifest.id.clone();
        if self.entries.contains_key(&id) {
            return Err(RuntimeError::AlreadyRegistered(id));
        }
        let mut info = sideweave_plugin_sdk::plugin_info(plugin.as_ref());
        info.source = source.into();
        info.status = PluginStatus::Discovered;
        info.effective_trust = PluginTrust::TrustedLocal;
        self.entries.insert(
            id,
            RegistryEntry {
                plugin: Some(plugin),
                external: None,
                info,
            },
        );
        Ok(())
    }

    pub fn register_manifest(
        &mut self,
        manifest: sideweave_plugin_api::PluginManifest,
        location: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.register_manifest_with_trust(manifest, location, source, PluginTrust::Sandboxed)
    }

    pub fn register_external(
        &mut self,
        manifest: sideweave_plugin_api::PluginManifest,
        external: Box<dyn ExternalPlugin>,
        location: impl Into<String>,
        source: impl Into<String>,
        trust: PluginTrust,
    ) -> Result<(), RuntimeError> {
        let id = manifest.id.clone();
        manifest.validate().map_err(RuntimeError::Protocol)?;
        if self.entries.contains_key(&id) {
            return Err(RuntimeError::AlreadyRegistered(id));
        }
        let runtime = manifest.runtime.clone();
        self.entries.insert(
            id,
            RegistryEntry {
                plugin: None,
                external: Some(external),
                info: PluginInfo {
                    manifest,
                    status: PluginStatus::Discovered,
                    runtime,
                    location: location.into(),
                    source: source.into(),
                    effective_trust: trust,
                    last_error: None,
                },
            },
        );
        Ok(())
    }

    #[cfg(windows)]
    pub fn register_dynamic(
        &mut self,
        plugin: Box<dyn Plugin>,
        location: impl Into<String>,
        source: impl Into<String>,
        trust: PluginTrust,
    ) -> Result<(), RuntimeError> {
        let manifest = plugin.manifest().clone();
        manifest.validate().map_err(RuntimeError::Protocol)?;
        let id = manifest.id.clone();
        if self.entries.contains_key(&id) {
            return Err(RuntimeError::AlreadyRegistered(id));
        }
        self.entries.insert(
            id,
            RegistryEntry {
                plugin: Some(plugin),
                external: None,
                info: PluginInfo {
                    runtime: manifest.runtime.clone(),
                    manifest,
                    status: PluginStatus::Discovered,
                    location: location.into(),
                    source: source.into(),
                    effective_trust: trust,
                    last_error: None,
                },
            },
        );
        Ok(())
    }

    pub fn register_trusted_local_manifest(
        &mut self,
        manifest: sideweave_plugin_api::PluginManifest,
        location: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.register_manifest_with_trust(manifest, location, source, PluginTrust::TrustedLocal)
    }

    fn register_manifest_with_trust(
        &mut self,
        manifest: sideweave_plugin_api::PluginManifest,
        location: impl Into<String>,
        source: impl Into<String>,
        trust: PluginTrust,
    ) -> Result<(), RuntimeError> {
        manifest.validate().map_err(RuntimeError::Protocol)?;
        let id = manifest.id.clone();
        if self.entries.contains_key(&id) {
            return Err(RuntimeError::AlreadyRegistered(id));
        }
        let runtime = manifest.runtime.clone();
        self.entries.insert(
            id,
            RegistryEntry {
                plugin: None,
                external: None,
                info: PluginInfo {
                    manifest,
                    status: PluginStatus::Discovered,
                    runtime,
                    location: location.into(),
                    source: source.into(),
                    effective_trust: trust,
                    last_error: None,
                },
            },
        );
        Ok(())
    }

    pub fn get(&self, id: &PluginId) -> Option<&PluginInfo> {
        self.entries.get(id).map(|entry| &entry.info)
    }

    pub fn list(&self) -> Vec<PluginInfo> {
        let mut entries: Vec<_> = self
            .entries
            .values()
            .map(|entry| entry.info.clone())
            .collect();
        entries.sort_by(|left, right| left.manifest.id.as_str().cmp(right.manifest.id.as_str()));
        entries
    }

    fn get_mut(&mut self, id: &PluginId) -> Option<&mut RegistryEntry> {
        self.entries.get_mut(id)
    }

    fn remove(&mut self, id: &PluginId) -> Option<RegistryEntry> {
        self.entries.remove(id)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PluginManager {
    registry: PluginRegistry,
    services: Arc<dyn HostServices>,
}

impl PluginManager {
    pub fn new(services: Arc<dyn HostServices>) -> Self {
        Self {
            registry: PluginRegistry::new(),
            services,
        }
    }

    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    pub fn register(
        &mut self,
        plugin: Box<dyn Plugin>,
        source: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.registry.register(plugin, source)
    }

    pub fn discover_manifest(
        &mut self,
        manifest: sideweave_plugin_api::PluginManifest,
        location: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.registry.register_manifest(manifest, location, source)
    }

    pub fn discover_trusted_local_manifest(
        &mut self,
        manifest: sideweave_plugin_api::PluginManifest,
        location: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.registry
            .register_trusted_local_manifest(manifest, location, source)
    }

    pub fn register_external(
        &mut self,
        manifest: sideweave_plugin_api::PluginManifest,
        external: Box<dyn ExternalPlugin>,
        location: impl Into<String>,
        source: impl Into<String>,
        trust: PluginTrust,
    ) -> Result<(), RuntimeError> {
        self.registry
            .register_external(manifest, external, location, source, trust)
    }

    pub fn attach_external(
        &mut self,
        id: &PluginId,
        external: Box<dyn ExternalPlugin>,
    ) -> Result<(), RuntimeError> {
        let entry = self
            .registry
            .get_mut(id)
            .ok_or_else(|| RuntimeError::NotFound(id.clone()))?;
        if entry.info.runtime != sideweave_plugin_api::PluginRuntime::ExternalProcess
            || entry.info.effective_trust != PluginTrust::TrustedLocal
            || entry.external.is_some()
            || entry.plugin.is_some()
        {
            return Err(RuntimeError::Unavailable(id.clone()));
        }
        entry.external = Some(external);
        Ok(())
    }

    #[cfg(windows)]
    pub fn register_dynamic(
        &mut self,
        plugin: Box<dyn Plugin>,
        location: impl Into<String>,
        source: impl Into<String>,
        trust: PluginTrust,
    ) -> Result<(), RuntimeError> {
        self.registry
            .register_dynamic(plugin, location, source, trust)
    }

    #[cfg(windows)]
    pub fn attach_dynamic(&mut self, plugin: Box<dyn Plugin>) -> Result<(), RuntimeError> {
        let mut manifest = plugin.manifest().clone();
        manifest.validate().map_err(RuntimeError::Protocol)?;
        let id = manifest.id.clone();
        let entry = self
            .registry
            .get_mut(&id)
            .ok_or_else(|| RuntimeError::NotFound(id.clone()))?;
        if manifest.runtime != sideweave_plugin_api::PluginRuntime::DynamicLibrary
            || entry.info.runtime != sideweave_plugin_api::PluginRuntime::DynamicLibrary
            || entry.info.effective_trust != PluginTrust::TrustedLocal
            || entry.plugin.is_some()
            || entry.external.is_some()
        {
            return Err(RuntimeError::Unavailable(id));
        }
        // Required capabilities are host package metadata. Dynamic plugin SDKs
        // serialize their runtime manifest independently, so retain the
        // package-declared requirements when attaching the backend.
        manifest.required_permissions = entry.info.manifest.required_permissions.clone();
        manifest.validate().map_err(RuntimeError::Protocol)?;
        entry.info.manifest = manifest;
        entry.plugin = Some(plugin);
        Ok(())
    }

    pub fn backend_loaded(&self, id: &PluginId) -> bool {
        self.registry
            .entries
            .get(id)
            .is_some_and(|entry| entry.plugin.is_some() || entry.external.is_some())
    }

    pub fn unload(&mut self, id: &PluginId) -> Result<(), RuntimeError> {
        let should_unload = self
            .registry
            .get(id)
            .map(|info| {
                matches!(
                    info.runtime,
                    sideweave_plugin_api::PluginRuntime::DynamicLibrary
                        | sideweave_plugin_api::PluginRuntime::ExternalProcess
                )
            })
            .unwrap_or(false);
        if !should_unload {
            return Ok(());
        }
        let backend_loaded = self
            .registry
            .entries
            .get(id)
            .is_some_and(|entry| entry.plugin.is_some() || entry.external.is_some());
        if !backend_loaded {
            self.registry.remove(id);
            return Ok(());
        }
        self.services
            .windows()
            .close_plugin_windows(id)
            .map_err(|error| RuntimeError::Host(error.message))?;
        let _ = self.services.shortcuts().unregister(id);
        let result = if self
            .registry
            .get(id)
            .is_some_and(|info| {
                info.runtime == sideweave_plugin_api::PluginRuntime::ExternalProcess
            })
        {
            self.registry
                .get_mut(id)
                .and_then(|entry| entry.external.as_mut())
                .ok_or_else(|| RuntimeError::Unavailable(id.clone()))
                .and_then(|external| external.shutdown().map_err(RuntimeError::External))
        } else {
            self.shutdown(id).map(|_| ())
        };
        self.registry.remove(id);
        result
    }

    pub fn unload_installed(&mut self) -> Result<(), RuntimeError> {
        let ids = self
            .registry
            .list()
            .into_iter()
            .filter(|info| {
                matches!(
                    info.runtime,
                    sideweave_plugin_api::PluginRuntime::DynamicLibrary
                        | sideweave_plugin_api::PluginRuntime::ExternalProcess
                ) && info.source.starts_with("installed-")
            })
            .map(|info| info.manifest.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.unload(&id)?;
        }
        Ok(())
    }

    pub fn activate(&mut self, id: &PluginId) -> Result<PluginResponse, RuntimeError> {
        let request = lifecycle_request(id.clone(), PluginRequestKind::Activate);
        self.dispatch(id, request, PluginStatus::Active)
    }

    pub fn deactivate(&mut self, id: &PluginId) -> Result<PluginResponse, RuntimeError> {
        let request = lifecycle_request(id.clone(), PluginRequestKind::Deactivate);
        self.dispatch(id, request, PluginStatus::Disabled)
    }

    pub fn shutdown(&mut self, id: &PluginId) -> Result<PluginResponse, RuntimeError> {
        let request = lifecycle_request(id.clone(), PluginRequestKind::Shutdown);
        self.dispatch(id, request, PluginStatus::Stopped)
    }

    pub fn call(&mut self, request: PluginRequest) -> Result<PluginResponse, RuntimeError> {
        let id = request.plugin_id.clone();
        request.validate_for(&id).map_err(RuntimeError::Protocol)?;
        match request.kind.clone() {
            PluginRequestKind::Activate => self.dispatch(&id, request, PluginStatus::Active),
            PluginRequestKind::Deactivate => self.dispatch(&id, request, PluginStatus::Disabled),
            PluginRequestKind::Shutdown => self.dispatch(&id, request, PluginStatus::Stopped),
            PluginRequestKind::Call | PluginRequestKind::GetInfo => {
                self.dispatch_request(&id, request)
            }
        }
    }

    fn dispatch(
        &mut self,
        id: &PluginId,
        request: PluginRequest,
        status: PluginStatus,
    ) -> Result<PluginResponse, RuntimeError> {
        let response = self.dispatch_request(id, request)?;
        if let Some(entry) = self.registry.get_mut(id) {
            entry.info.status = if response.ok {
                status
            } else {
                PluginStatus::Failed
            };
            entry.info.last_error = response.error.clone();
        }
        Ok(response)
    }

    fn dispatch_request(
        &mut self,
        id: &PluginId,
        request: PluginRequest,
    ) -> Result<PluginResponse, RuntimeError> {
        let entry = self
            .registry
            .get_mut(id)
            .ok_or_else(|| RuntimeError::NotFound(id.clone()))?;
        let context = PluginContext::new(id.clone(), Arc::clone(&self.services));
        let response = if let Some(plugin) = entry.plugin.as_mut() {
            plugin.handle(request.clone(), &context)
        } else if let Some(external) = entry.external.as_mut() {
            external
                .call(request.clone())
                .map_err(RuntimeError::External)?
        } else {
            return Err(RuntimeError::Unavailable(id.clone()));
        };
        if response.request_id != request.request_id || response.plugin_id != request.plugin_id {
            return Err(RuntimeError::Protocol(
                sideweave_plugin_api::ProtocolError::InvalidRequest(
                    "plugin response does not match request".into(),
                ),
            ));
        }
        Ok(response)
    }

    pub fn handshake(&self, handshake: PluginHandshake) -> PluginHandshakeResponse {
        let error = if handshake.request_id.trim().is_empty() {
            Some(PluginError {
                code: ErrorCode::InvalidRequest,
                message: "handshake request_id cannot be empty".into(),
            })
        } else if handshake.api_version != API_VERSION {
            Some(PluginError {
                code: ErrorCode::UnsupportedApiVersion,
                message: format!("unsupported plugin api version: {}", handshake.api_version),
            })
        } else if self.registry.get(&handshake.plugin_id).is_none() {
            Some(PluginError {
                code: ErrorCode::NotFound,
                message: format!("plugin not found: {}", handshake.plugin_id),
            })
        } else {
            None
        };
        let capabilities = self
            .registry
            .get(&handshake.plugin_id)
            .map(|info| {
                let mut capabilities = vec!["plugin.call".to_string()];
                capabilities.extend(info.manifest.permissions.iter().cloned());
                capabilities.sort();
                capabilities.dedup();
                capabilities
            })
            .unwrap_or_default();
        PluginHandshakeResponse {
            request_id: handshake.request_id,
            plugin_id: handshake.plugin_id,
            accepted: error.is_none(),
            api_version: API_VERSION,
            capabilities,
            error,
        }
    }

    pub fn publish(&self, event: PluginEvent) -> Result<(), RuntimeError> {
        self.services
            .events()
            .publish(event)
            .map_err(|error| RuntimeError::Host(error.message))
    }
}

#[derive(Debug, Clone)]
pub enum RuntimeError {
    Protocol(sideweave_plugin_api::ProtocolError),
    AlreadyRegistered(PluginId),
    NotFound(PluginId),
    Unavailable(PluginId),
    Host(String),
    External(PluginError),
}

impl RuntimeError {
    pub fn as_plugin_error(&self) -> PluginError {
        match self {
            Self::Protocol(error) => PluginError {
                code: match error {
                    sideweave_plugin_api::ProtocolError::InvalidRequest(_) => {
                        ErrorCode::InvalidRequest
                    }
                    sideweave_plugin_api::ProtocolError::UnsupportedApiVersion(_) => {
                        ErrorCode::UnsupportedApiVersion
                    }
                    _ => ErrorCode::InvalidManifest,
                },
                message: error.to_string(),
            },
            Self::AlreadyRegistered(id) => PluginError {
                code: ErrorCode::InvalidRequest,
                message: format!("plugin already registered: {id}"),
            },
            Self::NotFound(id) => PluginError {
                code: ErrorCode::NotFound,
                message: format!("plugin not found: {id}"),
            },
            Self::Unavailable(id) => PluginError {
                code: ErrorCode::PluginUnavailable,
                message: format!("plugin backend is not loaded: {id}"),
            },
            Self::Host(message) => PluginError {
                code: ErrorCode::Internal,
                message: message.clone(),
            },
            Self::External(error) => error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sideweave_plugin_api::{
        PluginHandshake, PluginManifest, PluginRuntime, PluginUi, API_VERSION, MANIFEST_VERSION,
    };
    use sideweave_plugin_sdk::{
        ClipboardService, EventBus, FileService, GlobalShortcutService, HostError, HostServices,
        NotificationService, ShortcutAction, ShortcutHandler, StorageService, SystemService,
        WindowService,
    };
    use serde_json::{json, Value};

    struct TestServices;

    impl StorageService for TestServices {
        fn get(&self, _plugin_id: &PluginId, _key: &str) -> Result<Option<String>, HostError> {
            Ok(None)
        }

        fn set(&self, _plugin_id: &PluginId, _key: &str, _value: &str) -> Result<(), HostError> {
            Ok(())
        }

        fn remove(&self, _plugin_id: &PluginId, _key: &str) -> Result<(), HostError> {
            Ok(())
        }
    }

    impl NotificationService for TestServices {
        fn notify(&self, _title: &str, _body: &str) -> Result<(), HostError> {
            Ok(())
        }
    }

    impl EventBus for TestServices {
        fn publish(&self, _event: PluginEvent) -> Result<(), HostError> {
            Ok(())
        }
    }

    impl SystemService for TestServices {
        fn open_external(&self, _url: &str) -> Result<(), HostError> {
            Ok(())
        }

        fn spawn(&self, _program: &str, _args: &[String]) -> Result<(), HostError> {
            Ok(())
        }

        fn pick_file(&self, _extensions: &[String]) -> Result<Option<String>, HostError> {
            Ok(None)
        }
    }

    impl FileService for TestServices {
        fn read_private(
            &self,
            _plugin_id: &PluginId,
            _relative_path: &str,
        ) -> Result<Option<Vec<u8>>, HostError> {
            Ok(None)
        }

        fn write_private(
            &self,
            _plugin_id: &PluginId,
            _relative_path: &str,
            _bytes: &[u8],
        ) -> Result<(), HostError> {
            Ok(())
        }

        fn remove_private(
            &self,
            _plugin_id: &PluginId,
            _relative_path: &str,
        ) -> Result<(), HostError> {
            Ok(())
        }

        fn read_external(&self, _path: &str) -> Result<Vec<u8>, HostError> {
            Ok(Vec::new())
        }
    }

    impl ClipboardService for TestServices {
        fn read_text(&self) -> Result<Option<String>, HostError> {
            Ok(None)
        }

        fn write_text(&self, _text: &str) -> Result<(), HostError> {
            Ok(())
        }
    }

    impl GlobalShortcutService for TestServices {
        fn register(
            &self,
            _plugin_id: &PluginId,
            _bindings: &[(String, ShortcutAction)],
            _handler: ShortcutHandler,
        ) -> Result<(), HostError> {
            Ok(())
        }

        fn unregister(&self, _plugin_id: &PluginId) -> Result<(), HostError> {
            Ok(())
        }
    }

    impl WindowService for TestServices {
        fn create(
            &self,
            _spec: sideweave_plugin_sdk::WindowSpec,
            _handler: Option<std::sync::Arc<dyn sideweave_plugin_sdk::WindowEventHandler>>,
        ) -> Result<(), HostError> {
            Ok(())
        }

        fn show(&self, _id: &str) -> Result<(), HostError> {
            Ok(())
        }

        fn hide(&self, _id: &str) -> Result<(), HostError> {
            Ok(())
        }

        fn close(&self, _id: &str) -> Result<(), HostError> {
            Ok(())
        }

        fn emit(&self, _id: &str, _topic: &str, _payload: Value) -> Result<(), HostError> {
            Ok(())
        }

        fn set_clickthrough(&self, _id: &str, _enabled: bool) -> Result<(), HostError> {
            Ok(())
        }
    }

    impl HostServices for TestServices {
        fn storage(&self) -> &dyn StorageService {
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

    struct TestPlugin {
        manifest: PluginManifest,
    }

    impl Plugin for TestPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.manifest
        }

        fn handle(&mut self, request: PluginRequest, context: &PluginContext) -> PluginResponse {
            assert_eq!(context.plugin_id().as_str(), "demo");
            PluginResponse {
                request_id: request.request_id,
                plugin_id: request.plugin_id,
                ok: true,
                result: json!({ "method": request.method }),
                error: None,
            }
        }
    }

    fn test_plugin() -> TestPlugin {
        TestPlugin {
            manifest: PluginManifest {
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
                trust: sideweave_plugin_api::PluginTrust::Sandboxed,
                auto_start: false,
                permissions: vec![],
                required_permissions: vec![],
                ui: Some(PluginUi {
                    entry: "ui/index.html".into(),
                    ..Default::default()
                }),
                hover_preview: None,
                minimum_host_version: None,
            },
        }
    }

    #[test]
    fn lifecycle_and_call_share_registry_state() {
        let mut manager = PluginManager::new(Arc::new(TestServices));
        manager.register(Box::new(test_plugin()), "test").unwrap();

        let id = PluginId::new("demo").unwrap();
        assert!(matches!(
            manager.registry().get(&id).unwrap().status,
            PluginStatus::Discovered
        ));
        assert!(manager.activate(&id).unwrap().ok);
        assert!(matches!(
            manager.registry().get(&id).unwrap().status,
            PluginStatus::Active
        ));

        let response = manager
            .call(PluginRequest {
                request_id: "req-1".into(),
                plugin_id: id,
                kind: PluginRequestKind::Call,
                method: Some("ping".into()),
                payload: Value::Null,
                timeout_ms: None,
            })
            .unwrap();
        assert_eq!(response.result["method"], "ping");
    }

    #[test]
    fn manifest_cannot_self_grant_trusted_local_access() {
        let mut manager = PluginManager::new(Arc::new(TestServices));
        let mut manifest = test_plugin().manifest;
        manifest.id = PluginId::new("external-demo").unwrap();
        manifest.runtime = PluginRuntime::ExternalProcess;
        manifest.trust = PluginTrust::TrustedLocal;

        manager
            .discover_manifest(manifest, "plugins/external-demo", "installed")
            .unwrap();

        assert!(matches!(
            manager
                .registry()
                .get(&PluginId::new("external-demo").unwrap())
                .unwrap()
                .effective_trust,
            PluginTrust::Sandboxed
        ));
    }

    #[test]
    fn handshake_returns_capabilities_and_rejects_wrong_api() {
        let mut manager = PluginManager::new(Arc::new(TestServices));
        manager.register(Box::new(test_plugin()), "test").unwrap();
        let id = PluginId::new("demo").unwrap();

        let accepted = manager.handshake(PluginHandshake {
            request_id: "hello-1".into(),
            plugin_id: id.clone(),
            api_version: API_VERSION,
            host_version: "test".into(),
            capabilities: vec![],
        });
        assert!(accepted.accepted);
        assert!(accepted
            .capabilities
            .iter()
            .any(|item| item == "plugin.call"));

        let rejected = manager.handshake(PluginHandshake {
            request_id: "hello-2".into(),
            plugin_id: id,
            api_version: API_VERSION + 1,
            host_version: "test".into(),
            capabilities: vec![],
        });
        assert!(!rejected.accepted);
        assert!(matches!(
            rejected.error.map(|error| error.code),
            Some(ErrorCode::UnsupportedApiVersion)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn dynamic_library_adapter_loads_a_built_plugin() {
        let repository = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let library = repository
            .join("target")
            .join("debug")
            .join("sideweave_plugin_bookmarks.dll");
        assert!(library.is_file(), "build the bookmarks DLL before this test");
        let id = PluginId::new("bookmarks").unwrap();
        let manifest = PluginManifest {
            manifest_version: MANIFEST_VERSION,
            api_version: API_VERSION,
            id: id.clone(),
            name: "Bookmarks".into(),
            icon: "bookmark".into(),
            version: "1.0.2".into(),
            description: String::new(),
            author: String::new(),
            runtime: PluginRuntime::DynamicLibrary,
            backend: Some("plugin.dll".into()),
            trust: PluginTrust::TrustedLocal,
            auto_start: false,
            permissions: vec![],
            required_permissions: vec![],
            ui: None,
            hover_preview: None,
            minimum_host_version: None,
        };
        let services: Arc<dyn HostServices> = Arc::new(TestServices);
        let mut plugin = DynamicPlugin::load(&library, manifest, Arc::clone(&services)).unwrap();
        let request = PluginRequest {
            request_id: "dynamic-activate".into(),
            plugin_id: id.clone(),
            kind: PluginRequestKind::Activate,
            method: None,
            payload: Value::Null,
            timeout_ms: None,
        };
        let context = PluginContext::new(id, services);
        assert!(plugin.handle(request, &context).ok);
    }
}
