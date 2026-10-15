use crate::{
    ClipboardService, EventBus, FileService, GlobalShortcutService, HostError, HostServices,
    DesktopNotificationOptions, NotificationService, SecretService, ShortcutAction, ShortcutHandler, StorageService, SystemService,
    WindowEventHandler, WindowService, WindowSpec,
};
use sideweave_plugin_api::{PluginEvent, PluginId};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct MockHostServices {
    storage: Arc<Mutex<HashMap<(String, String), String>>>,
    secrets: Arc<Mutex<HashMap<(String, String), String>>>,
    files: Arc<Mutex<HashMap<(String, String), Vec<u8>>>>,
    events: Arc<Mutex<Vec<PluginEvent>>>,
    notifications: Arc<Mutex<Vec<(String, String)>>>,
    clipboard: Arc<Mutex<Option<String>>>,
}

impl MockHostServices {
    pub fn storage_value(&self, plugin_id: &str, key: &str) -> Option<String> {
        self.storage
            .lock()
            .unwrap()
            .get(&(plugin_id.into(), key.into()))
            .cloned()
    }

    pub fn events(&self) -> Vec<PluginEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn notifications(&self) -> Vec<(String, String)> {
        self.notifications.lock().unwrap().clone()
    }
}

impl StorageService for MockHostServices {
    fn get(&self, plugin_id: &PluginId, key: &str) -> Result<Option<String>, HostError> {
        Ok(self.storage_value(plugin_id.as_str(), key))
    }

    fn set(&self, plugin_id: &PluginId, key: &str, value: &str) -> Result<(), HostError> {
        self.storage
            .lock()
            .unwrap()
            .insert((plugin_id.to_string(), key.into()), value.into());
        Ok(())
    }

    fn remove(&self, plugin_id: &PluginId, key: &str) -> Result<(), HostError> {
        self.storage
            .lock()
            .unwrap()
            .remove(&(plugin_id.to_string(), key.into()));
        Ok(())
    }
}

impl SecretService for MockHostServices {
    fn get(&self, plugin_id: &PluginId, key: &str) -> Result<Option<String>, HostError> {
        Ok(self
            .secrets
            .lock()
            .unwrap()
            .get(&(plugin_id.to_string(), key.into()))
            .cloned())
    }

    fn set(&self, plugin_id: &PluginId, key: &str, value: &str) -> Result<(), HostError> {
        self.secrets
            .lock()
            .unwrap()
            .insert((plugin_id.to_string(), key.into()), value.into());
        Ok(())
    }

    fn remove(&self, plugin_id: &PluginId, key: &str) -> Result<(), HostError> {
        self.secrets
            .lock()
            .unwrap()
            .remove(&(plugin_id.to_string(), key.into()));
        Ok(())
    }
}

impl NotificationService for MockHostServices {
    fn notify(&self, title: &str, body: &str) -> Result<(), HostError> {
        self.notifications
            .lock()
            .unwrap()
            .push((title.into(), body.into()));
        Ok(())
    }

    fn show_desktop(&self, notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        self.notifications
            .lock()
            .unwrap()
            .push((notification.title.clone(), notification.body.clone()));
        Ok(())
    }

    fn show_sidebar(&self, notification: &DesktopNotificationOptions) -> Result<(), HostError> {
        self.notifications
            .lock()
            .unwrap()
            .push((notification.title.clone(), notification.body.clone()));
        Ok(())
    }
}

impl EventBus for MockHostServices {
    fn publish(&self, event: PluginEvent) -> Result<(), HostError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

impl SystemService for MockHostServices {
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

impl FileService for MockHostServices {
    fn read_private(
        &self,
        plugin_id: &PluginId,
        relative_path: &str,
    ) -> Result<Option<Vec<u8>>, HostError> {
        Ok(self
            .files
            .lock()
            .unwrap()
            .get(&(plugin_id.to_string(), relative_path.into()))
            .cloned())
    }

    fn write_private(
        &self,
        plugin_id: &PluginId,
        relative_path: &str,
        bytes: &[u8],
    ) -> Result<(), HostError> {
        self.files.lock().unwrap().insert(
            (plugin_id.to_string(), relative_path.into()),
            bytes.to_vec(),
        );
        Ok(())
    }

    fn remove_private(&self, plugin_id: &PluginId, relative_path: &str) -> Result<(), HostError> {
        self.files
            .lock()
            .unwrap()
            .remove(&(plugin_id.to_string(), relative_path.into()));
        Ok(())
    }

    fn read_external(&self, _path: &str) -> Result<Vec<u8>, HostError> {
        Ok(Vec::new())
    }
}

impl ClipboardService for MockHostServices {
    fn read_text(&self) -> Result<Option<String>, HostError> {
        Ok(self.clipboard.lock().unwrap().clone())
    }

    fn write_text(&self, text: &str) -> Result<(), HostError> {
        *self.clipboard.lock().unwrap() = Some(text.into());
        Ok(())
    }
}

impl GlobalShortcutService for MockHostServices {
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

impl WindowService for MockHostServices {
    fn create(
        &self,
        _spec: WindowSpec,
        _handler: Option<Arc<dyn WindowEventHandler>>,
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

impl HostServices for MockHostServices {
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
