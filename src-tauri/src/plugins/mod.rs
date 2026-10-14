//! Runtime plugin metadata and the web-plugin bridge.

pub mod user_bridge;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginMeta {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub version: String,
    pub enabled: bool,
    pub sidebar_visible: bool,
}

pub fn meta(
    settings_plugins: &crate::settings::PluginPrefs,
    runtime_plugins: &[PluginMeta],
    extra: &[PluginMeta],
) -> Vec<PluginMeta> {
    let enabled_of = |id: &str| {
        !settings_plugins
            .disabled
            .iter()
            .any(|disabled| disabled == id)
    };
    let sidebar_visible_of = |id: &str| {
        !settings_plugins
            .hidden_from_sidebar
            .iter()
            .any(|hidden| hidden == id)
    };
    let mut known = runtime_plugins.to_vec();
    for item in &mut known {
        item.enabled = enabled_of(&item.id);
        item.sidebar_visible = sidebar_visible_of(&item.id);
    }
    for item in extra {
        if !known.iter().any(|known| known.id == item.id) {
            let mut item = item.clone();
            item.enabled = enabled_of(&item.id);
            item.sidebar_visible = sidebar_visible_of(&item.id);
            known.push(item);
        }
    }
    known.sort_by_key(|item| {
        settings_plugins
            .order
            .iter()
            .position(|ordered| ordered == &item.id)
            .unwrap_or(usize::MAX)
    });
    known
}
