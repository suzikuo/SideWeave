//! Runtime registry and lifecycle orchestration for plugin adapters.

use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginEvent, PluginHandshake, PluginHandshakeResponse, PluginId,
    PluginInfo, PluginRequest, PluginRequestKind, PluginResponse, PluginStatus, PluginTrust,
    API_VERSION,
};
use sideweave_plugin_sdk::{lifecycle_request, HostServices, Plugin, PluginContext};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
mod dylib;

#[cfg(windows)]
pub use dylib::DynamicPlugin;

pub struct PluginRegistry {
    entries: HashMap<PluginId, RegistryEntry>,
}

struct RegistryEntry {
    backend: Arc<Mutex<PluginBackendState>>,
    loaded: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
    info: PluginInfo,
}

enum PluginBackend {
    Unloaded,
    InProcess(Box<dyn Plugin>),
    External(Box<dyn ExternalPlugin>),
}

struct PluginBackendState {
    backend: PluginBackend,
    status: PluginStatus,
    last_error: Option<PluginError>,
}

impl PluginBackendState {
    fn new(backend: PluginBackend) -> Self {
        Self {
            backend,
            status: PluginStatus::Discovered,
            last_error: None,
        }
    }
}

#[derive(Clone)]
pub struct PluginExecutor {
    id: PluginId,
    backend: Arc<Mutex<PluginBackendState>>,
    closing: Arc<AtomicBool>,
    services: Arc<dyn HostServices>,
}

pub trait ExternalPlugin: Send {
    fn call(&mut self, request: PluginRequest) -> Result<PluginResponse, PluginError>;

    fn shutdown(&mut self) -> Result<(), PluginError> {
        Ok(())
    }
}

impl PluginExecutor {
    pub fn call(&self, request: PluginRequest) -> Result<PluginResponse, RuntimeError> {
        request
            .validate_for(&self.id)
            .map_err(RuntimeError::Protocol)?;
        let mut state = self
            .backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.closing.load(Ordering::Acquire) {
            return Err(RuntimeError::Unavailable(self.id.clone()));
        }
        let response = Self::invoke(&self.id, &self.services, &mut state, &request)?;
        Self::update_lifecycle_state(&mut state, &request, &response);
        Ok(response)
    }

    pub fn call_with_auto_activate(
        &self,
        request: PluginRequest,
    ) -> Result<PluginResponse, RuntimeError> {
        request
            .validate_for(&self.id)
            .map_err(RuntimeError::Protocol)?;
        let mut state = self
            .backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.closing.load(Ordering::Acquire) {
            return Err(RuntimeError::Unavailable(self.id.clone()));
        }
        if matches!(
            &request.kind,
            PluginRequestKind::Call | PluginRequestKind::GetInfo
        ) && matches!(&state.status, PluginStatus::Discovered)
            && !matches!(&state.backend, PluginBackend::Unloaded)
        {
            let activate = lifecycle_request(self.id.clone(), PluginRequestKind::Activate);
            let response = Self::invoke(&self.id, &self.services, &mut state, &activate)?;
            Self::update_lifecycle_state(&mut state, &activate, &response);
            if !response.ok {
                return Ok(response);
            }
        }
        let response = Self::invoke(&self.id, &self.services, &mut state, &request)?;
        Self::update_lifecycle_state(&mut state, &request, &response);
        Ok(response)
    }

    pub fn shutdown_for_unload(&self) -> Result<(), RuntimeError> {
        let mut state = self
            .backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.services
            .windows()
            .close_plugin_windows(&self.id)
            .map_err(|error| RuntimeError::Host(error.message))?;
        let _ = self.services.shortcuts().unregister(&self.id);
        if matches!(&state.backend, PluginBackend::External(_)) {
            if let PluginBackend::External(external) = &mut state.backend {
                external.shutdown().map_err(RuntimeError::External)
            } else {
                unreachable!()
            }
        } else if matches!(&state.backend, PluginBackend::InProcess(_)) {
            let request = lifecycle_request(self.id.clone(), PluginRequestKind::Shutdown);
            let response = Self::invoke(&self.id, &self.services, &mut state, &request)?;
            Self::update_lifecycle_state(&mut state, &request, &response);
            Ok(())
        } else {
            Ok(())
        }
    }

    fn invoke(
        id: &PluginId,
        services: &Arc<dyn HostServices>,
        state: &mut PluginBackendState,
        request: &PluginRequest,
    ) -> Result<PluginResponse, RuntimeError> {
        let context = PluginContext::new(id.clone(), Arc::clone(services));
        let response = match &mut state.backend {
            PluginBackend::InProcess(plugin) => plugin.handle(request.clone(), &context),
            PluginBackend::External(external) => external
                .call(request.clone())
                .map_err(RuntimeError::External)?,
            PluginBackend::Unloaded => return Err(RuntimeError::Unavailable(id.clone())),
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

    fn update_lifecycle_state(
        state: &mut PluginBackendState,
        request: &PluginRequest,
        response: &PluginResponse,
    ) {
        let next_status = match &request.kind {
            PluginRequestKind::Activate => Some(PluginStatus::Active),
            PluginRequestKind::Deactivate => Some(PluginStatus::Disabled),
            PluginRequestKind::Shutdown => Some(PluginStatus::Stopped),
            PluginRequestKind::Call | PluginRequestKind::GetInfo => None,
        };
        if let Some(status) = next_status {
            state.status = if response.ok {
                status
            } else {
                PluginStatus::Failed
            };
            state.last_error = response.error.clone();
        }
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
                backend: Arc::new(Mutex::new(PluginBackendState::new(
                    PluginBackend::InProcess(plugin),
                ))),
                loaded: Arc::new(AtomicBool::new(true)),
                closing: Arc::new(AtomicBool::new(false)),
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
                backend: Arc::new(Mutex::new(PluginBackendState::new(
                    PluginBackend::External(external),
                ))),
                loaded: Arc::new(AtomicBool::new(true)),
                closing: Arc::new(AtomicBool::new(false)),
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
                backend: Arc::new(Mutex::new(PluginBackendState::new(
                    PluginBackend::InProcess(plugin),
                ))),
                loaded: Arc::new(AtomicBool::new(true)),
                closing: Arc::new(AtomicBool::new(false)),
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
                backend: Arc::new(Mutex::new(PluginBackendState::new(PluginBackend::Unloaded))),
                loaded: Arc::new(AtomicBool::new(false)),
                closing: Arc::new(AtomicBool::new(false)),
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
        let mut backend = entry
            .backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entry.info.runtime != sideweave_plugin_api::PluginRuntime::ExternalProcess
            || entry.info.effective_trust != PluginTrust::TrustedLocal
            || !matches!(backend.backend, PluginBackend::Unloaded)
        {
            return Err(RuntimeError::Unavailable(id.clone()));
        }
        backend.backend = PluginBackend::External(external);
        entry.loaded.store(true, Ordering::Release);
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
        let mut backend = entry
            .backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if manifest.runtime != sideweave_plugin_api::PluginRuntime::DynamicLibrary
            || entry.info.runtime != sideweave_plugin_api::PluginRuntime::DynamicLibrary
            || entry.info.effective_trust != PluginTrust::TrustedLocal
            || !matches!(backend.backend, PluginBackend::Unloaded)
        {
            return Err(RuntimeError::Unavailable(id));
        }
        // Required capabilities are host package metadata. Dynamic plugin SDKs
        // serialize their runtime manifest independently, so retain the
        // package-declared requirements when attaching the backend.
        manifest.required_permissions = entry.info.manifest.required_permissions.clone();
        manifest.validate().map_err(RuntimeError::Protocol)?;
        entry.info.manifest = manifest;
        backend.backend = PluginBackend::InProcess(plugin);
        entry.loaded.store(true, Ordering::Release);
        Ok(())
    }

    pub fn backend_loaded(&self, id: &PluginId) -> bool {
        self.registry
            .entries
            .get(id)
            .is_some_and(|entry| entry.loaded.load(Ordering::Acquire))
    }

    pub fn executor(&self, id: &PluginId) -> Result<PluginExecutor, RuntimeError> {
        let entry = self
            .registry
            .entries
            .get(id)
            .ok_or_else(|| RuntimeError::NotFound(id.clone()))?;
        Ok(PluginExecutor {
            id: id.clone(),
            backend: Arc::clone(&entry.backend),
            closing: Arc::clone(&entry.closing),
            services: Arc::clone(&self.services),
        })
    }

    pub fn sync_plugin_status(&mut self, id: &PluginId) {
        let Some(entry) = self.registry.entries.get_mut(id) else {
            return;
        };
        let Ok(backend) = entry.backend.try_lock() else {
            return;
        };
        entry.info.status = backend.status.clone();
        entry.info.last_error = backend.last_error.clone();
    }

    pub fn unload(&mut self, id: &PluginId) -> Result<(), RuntimeError> {
        let Some(executor) = self.begin_unload(id)? else {
            return Ok(());
        };
        let result = executor.shutdown_for_unload();
        if matches!(&result, Err(RuntimeError::Host(_))) {
            self.cancel_unload(id);
        } else {
            self.finish_unload(id);
        }
        result
    }

    pub fn begin_unload(&mut self, id: &PluginId) -> Result<Option<PluginExecutor>, RuntimeError> {
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
            return Ok(None);
        }
        let executor = self.executor(id)?;
        if let Some(entry) = self.registry.entries.get(id) {
            if entry.closing.swap(true, Ordering::AcqRel) {
                return Err(RuntimeError::Unavailable(id.clone()));
            }
        }
        Ok(Some(executor))
    }

    pub fn finish_unload(&mut self, id: &PluginId) {
        self.registry.remove(id);
    }

    pub fn cancel_unload(&mut self, id: &PluginId) {
        if let Some(entry) = self.registry.entries.get(id) {
            entry.closing.store(false, Ordering::Release);
        }
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
        self.dispatch(id, request)
    }

    pub fn deactivate(&mut self, id: &PluginId) -> Result<PluginResponse, RuntimeError> {
        let request = lifecycle_request(id.clone(), PluginRequestKind::Deactivate);
        self.dispatch(id, request)
    }

    pub fn shutdown(&mut self, id: &PluginId) -> Result<PluginResponse, RuntimeError> {
        let request = lifecycle_request(id.clone(), PluginRequestKind::Shutdown);
        self.dispatch(id, request)
    }

    pub fn call(&mut self, request: PluginRequest) -> Result<PluginResponse, RuntimeError> {
        let id = request.plugin_id.clone();
        request.validate_for(&id).map_err(RuntimeError::Protocol)?;
        let executor = self.executor(&id)?;
        let response = executor.call(request)?;
        self.sync_plugin_status(&id);
        Ok(response)
    }

    fn dispatch(
        &mut self,
        id: &PluginId,
        request: PluginRequest,
    ) -> Result<PluginResponse, RuntimeError> {
        let response = self.executor(id)?.call(request)?;
        self.sync_plugin_status(id);
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
    use serde_json::{json, Value};
    use sideweave_plugin_api::{
        PluginHandshake, PluginManifest, PluginRuntime, PluginUi, API_VERSION, MANIFEST_VERSION,
    };
    use sideweave_plugin_sdk::{
        ClipboardService, EventBus, FileService, GlobalShortcutService, HostError, HostServices,
        NotificationService, ShortcutAction, ShortcutHandler, StorageService, SystemService,
        WindowService,
    };
    use std::time::Duration;

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

    struct DelayPlugin {
        manifest: PluginManifest,
        delay: Duration,
        started: Option<std::sync::mpsc::Sender<()>>,
        release: Option<std::sync::mpsc::Receiver<()>>,
        active_calls: Arc<std::sync::atomic::AtomicUsize>,
        max_active_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Plugin for DelayPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.manifest
        }

        fn handle(&mut self, request: PluginRequest, _context: &PluginContext) -> PluginResponse {
            if let Some(started) = self.started.take() {
                let _ = started.send(());
            }
            let active = self.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active_calls.fetch_max(active, Ordering::SeqCst);
            if let Some(release) = self.release.take() {
                let _ = release.recv();
            } else {
                std::thread::sleep(self.delay);
            }
            self.active_calls.fetch_sub(1, Ordering::SeqCst);
            PluginResponse {
                request_id: request.request_id,
                plugin_id: request.plugin_id,
                ok: true,
                result: Value::Null,
                error: None,
            }
        }
    }

    fn delay_plugin(
        id: &str,
        delay: Duration,
        started: Option<std::sync::mpsc::Sender<()>>,
        release: Option<std::sync::mpsc::Receiver<()>>,
        active_calls: Arc<std::sync::atomic::AtomicUsize>,
        max_active_calls: Arc<std::sync::atomic::AtomicUsize>,
    ) -> DelayPlugin {
        let mut manifest = test_plugin().manifest;
        manifest.id = PluginId::new(id).unwrap();
        DelayPlugin {
            manifest,
            delay,
            started,
            release,
            active_calls,
            max_active_calls,
        }
    }

    fn call_request(id: &PluginId, request_id: &str) -> PluginRequest {
        PluginRequest {
            request_id: request_id.into(),
            plugin_id: id.clone(),
            kind: PluginRequestKind::Call,
            method: Some("ping".into()),
            payload: Value::Null,
            timeout_ms: None,
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
    fn plugin_executors_allow_parallel_plugins_and_serialize_each_plugin() {
        let mut manager = PluginManager::new(Arc::new(TestServices));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let slow_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let slow_max = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fast_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fast_max = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let slow_id = PluginId::new("slow-plugin").unwrap();
        let fast_id = PluginId::new("fast-plugin").unwrap();
        manager
            .register(
                Box::new(delay_plugin(
                    "slow-plugin",
                    Duration::ZERO,
                    Some(started_tx),
                    Some(release_rx),
                    Arc::clone(&slow_active),
                    Arc::clone(&slow_max),
                )),
                "test",
            )
            .unwrap();
        manager
            .register(
                Box::new(delay_plugin(
                    "fast-plugin",
                    Duration::ZERO,
                    None,
                    None,
                    Arc::clone(&fast_active),
                    Arc::clone(&fast_max),
                )),
                "test",
            )
            .unwrap();
        let slow = manager.executor(&slow_id).unwrap();
        let fast = manager.executor(&fast_id).unwrap();
        let slow_thread =
            std::thread::spawn(move || slow.call(call_request(&slow_id, "slow-call")).unwrap());
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("slow plugin should start");

        let (fast_done_tx, fast_done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = fast.call(call_request(&fast_id, "fast-call"));
            let _ = fast_done_tx.send(result);
        });
        assert!(
            fast_done_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("fast plugin should not wait for the slow plugin")
                .unwrap()
                .ok
        );
        release_tx.send(()).unwrap();
        assert!(slow_thread.join().unwrap().ok);

        let same_id = PluginId::new("same-plugin").unwrap();
        let same_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let same_max = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        manager
            .register(
                Box::new(delay_plugin(
                    "same-plugin",
                    Duration::from_millis(40),
                    None,
                    None,
                    Arc::clone(&same_active),
                    Arc::clone(&same_max),
                )),
                "test",
            )
            .unwrap();
        let same = manager.executor(&same_id).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for request_id in ["same-a", "same-b"] {
            let executor = same.clone();
            let barrier = Arc::clone(&barrier);
            let id = same_id.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                executor.call(call_request(&id, request_id)).unwrap()
            }));
        }
        barrier.wait();
        for worker in workers {
            assert!(worker.join().unwrap().ok);
        }
        assert_eq!(same_max.load(Ordering::SeqCst), 1);
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
        assert!(
            library.is_file(),
            "build the bookmarks DLL before this test"
        );
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
