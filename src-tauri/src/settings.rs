//! 类型化设置模型：schema 集中在此，set_setting 按项校验并钳制范围。

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub run_on_startup: bool,
    pub enable_mouse_hover: bool,
    pub auto_hide_delay: u32,
    pub hover_preview_delay: u32,
    pub control_center_always_on_top: bool,
    pub performance_profile: String,
    /// Optional HTTPS prefix for GitHub proxy services. Empty means use GitHub directly.
    pub github_mirror: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            run_on_startup: false,
            enable_mouse_hover: true,
            auto_hide_delay: 1000,
            hover_preview_delay: 350,
            control_center_always_on_top: false,
            performance_profile: "balanced".into(),
            github_mirror: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme_mode: String,
    pub skin: String,
    pub accent_color: String,
    pub background_image_path: String,
    pub background_image_name: String,
    pub background_image_apply_to_sidebar: bool,
    pub background_image_fit: String,
    pub background_image_opacity: f64,
    pub background_image_blur: f64,
    pub interface_scale: f64,
    pub sidebar_position: String,
    pub sidebar_monitor_id: Option<String>,
    pub collapsed_width: u32,
    pub icon_size: u32,
    pub peek_width: u32,
    pub sidebar_panel_opacity: f64,
    pub detail_panel_opacity: f64,
    pub control_panel_opacity: f64,
    pub preview_panel_opacity: f64,
    pub preview_image_opacity: f64,
    pub preview_visible_rows: u32,
    pub preview_width: Option<u32>,
    pub preview_display_mode: String,
    pub sidebar_image_opacity: f64,
    pub detail_image_opacity: f64,
    pub component_opacity: f64,
    pub glass_effect_enabled: bool,
    pub sidebar_height_percent: f64,
    pub sidebar_y_offset: i32,
    pub sidebar_x_offset: i32,
    pub sidebar_border_color: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme_mode: "dark".into(),
            skin: "default".into(),
            accent_color: "#006874".into(),
            background_image_path: String::new(),
            background_image_name: String::new(),
            background_image_apply_to_sidebar: true,
            background_image_fit: "cover".into(),
            background_image_opacity: 0.7,
            background_image_blur: 1.0,
            interface_scale: 1.0,
            sidebar_position: "right".into(),
            sidebar_monitor_id: None,
            collapsed_width: 64,
            icon_size: 36,
            peek_width: 6,
            sidebar_panel_opacity: 0.0,
            detail_panel_opacity: 0.85,
            control_panel_opacity: 0.85,
            preview_panel_opacity: 0.97,
            preview_image_opacity: 0.7,
            preview_visible_rows: 3,
            preview_width: None,
            preview_display_mode: "default".into(),
            sidebar_image_opacity: 0.0,
            detail_image_opacity: 0.0,
            component_opacity: 1.0,
            glass_effect_enabled: false,
            sidebar_height_percent: 0.8,
            sidebar_y_offset: 0,
            sidebar_x_offset: 0,
            sidebar_border_color: "#5a5a5f".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginPrefs {
    pub disabled: Vec<String>,
    pub hidden_from_sidebar: Vec<String>,
    pub order: Vec<String>,
    /// Per-plugin overrides for manifest-declared host capabilities.
    pub permission_grants: HashMap<String, HashMap<String, bool>>,
    /// One-time migration marker for plugins installed before permission grants existed.
    pub permission_grants_initialized: bool,
}

impl Default for PluginPrefs {
    fn default() -> Self {
        Self {
            disabled: Vec::new(),
            hidden_from_sidebar: Vec::new(),
            order: Vec::new(),
            permission_grants: HashMap::new(),
            permission_grants_initialized: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub toggle_sidebar: String,
    /// 呼出控制中心（空串 = 不启用）
    pub control_center: String,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            toggle_sidebar: "alt+space".into(),
            control_center: String::new(),
        }
    }
}

pub struct LoadedSettings {
    pub settings: Settings,
}

/// 读取磁盘状态并丢弃旧 OSS 凭据；SideWeave 不再使用或持久化云端配置。
pub fn load_persisted(value: JsonValue) -> LoadedSettings {
    let mut value = value;
    if let Some(object) = value.as_object_mut() {
        object.remove("cloud");
    }

    if let Some(appearance) = value
        .get_mut("appearance")
        .and_then(JsonValue::as_object_mut)
    {
        let legacy_sidebar = appearance.get("sidebar_bg_opacity").cloned();
        let legacy_detail = appearance.get("detail_bg_opacity").cloned();
        if let Some(value) = legacy_sidebar {
            appearance
                .entry("sidebar_panel_opacity")
                .or_insert_with(|| value.clone());
            appearance.entry("sidebar_image_opacity").or_insert(value);
        }
        if let Some(value) = legacy_detail {
            appearance
                .entry("detail_panel_opacity")
                .or_insert_with(|| value.clone());
            appearance
                .entry("control_panel_opacity")
                .or_insert_with(|| value.clone());
            appearance.entry("detail_image_opacity").or_insert(value);
        }
    }
    if let Some(notifications) = value
        .get_mut("notifications")
        .and_then(JsonValue::as_object_mut)
    {
        let legacy_opacity = notifications
            .get("background_opacity")
            .and_then(JsonValue::as_f64)
            .map(|opacity| JsonValue::from(opacity.clamp(0.0, 1.0)));
        if let Some(opacity) = legacy_opacity {
            notifications
                .entry("desktop_background_opacity")
                .or_insert_with(|| opacity.clone());
            notifications
                .entry("bubble_background_opacity")
                .or_insert(opacity);
        }
        // Upgrade untouched notification defaults while preserving custom colors and opacity.
        for (
            color_key,
            opacity_key,
            default_color,
            default_opacity,
        ) in [
            (
                "desktop_background_color",
                "desktop_background_opacity",
                "#18181b",
                0.94,
            ),
            (
                "bubble_background_color",
                "bubble_background_opacity",
                "#ffffff",
                0.9,
            ),
        ] {
            let opacity_is_old_default = notifications
                .get(opacity_key)
                .and_then(JsonValue::as_f64)
                .map(|opacity| (opacity - 0.92).abs() < 0.0001)
                .unwrap_or(false);
            let color_is_empty = notifications
                .get(color_key)
                .and_then(JsonValue::as_str)
                .map(|color| color.trim().is_empty())
                .unwrap_or(true);
            if opacity_is_old_default && color_is_empty {
                notifications.insert(opacity_key.into(), JsonValue::from(default_opacity));
                notifications.insert(color_key.into(), JsonValue::String(default_color.into()));
            }
        }
    }
    let mut settings: Settings = serde_json::from_value(value).unwrap_or_default();
    settings.general.github_mirror = validate_github_mirror(&settings.general.github_mirror)
        .unwrap_or_default();
    // Keep the old built-in skin selection mapped to its standalone package ID.
    // The package itself is installed and loaded like any other user skin.
    if settings.appearance.skin == "minecraft" {
        settings.appearance.skin = "user:minecraft".into();
    }
    LoadedSettings {
        settings,
    }
}

/// 序列化状态；旧 OSS 凭据字段不会进入新状态文件。
pub fn to_persisted(settings: &Settings) -> Result<JsonValue, String> {
    serde_json::to_value(settings).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub general: General,
    pub appearance: Appearance,
    pub notifications: NotificationSettings,
    pub plugins: PluginPrefs,
    pub shortcuts: Shortcuts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    /// `web` supports installed skin CSS; `native` uses the host's Windows drawing surface.
    pub render_mode: String,
    /// Legacy shared opacity. Read for migration, then omit from persisted settings.
    #[serde(skip_serializing)]
    pub background_opacity: f64,
    /// Background opacity from 0 (transparent) to 1 (opaque).
    pub desktop_background_opacity: f64,
    /// Background opacity from 0 (transparent) to 1 (opaque).
    pub bubble_background_opacity: f64,
    /// Empty values let the active skin choose the web background color.
    pub desktop_background_color: String,
    pub bubble_background_color: String,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            render_mode: "web".into(),
            background_opacity: 0.94,
            desktop_background_opacity: 0.94,
            bubble_background_opacity: 0.9,
            desktop_background_color: "#18181b".into(),
            bubble_background_color: "#ffffff".into(),
        }
    }
}

fn as_bool(v: &JsonValue) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| "需要布尔值".into())
}

fn as_u32_clamped(v: &JsonValue, min: u32, max: u32) -> Result<u32, String> {
    let n = v
        .as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .ok_or_else(|| "需要数字".to_string())?;
    Ok(n.clamp(min as i64, max as i64) as u32)
}

fn as_f64_clamped(v: &JsonValue, min: f64, max: f64) -> Result<f64, String> {
    let n = v.as_f64().ok_or_else(|| "需要数字".to_string())?;
    Ok(n.clamp(min, max))
}

fn as_i32_clamped(v: &JsonValue, min: i32, max: i32) -> Result<i32, String> {
    let n = v
        .as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .ok_or_else(|| "需要数字".to_string())?
        .clamp(min as i64, max as i64);
    Ok(n as i32)
}

fn as_enum(v: &JsonValue, allowed: &[&str]) -> Result<String, String> {
    let s = v.as_str().ok_or_else(|| "需要字符串".to_string())?;
    if allowed.contains(&s) {
        Ok(s.into())
    } else {
        Err(format!("取值必须是 {}", allowed.join(" / ")))
    }
}

pub fn validate_github_mirror(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || matches!(
            value,
            "https://ghfast.top/https://"
                | "https://ghproxy.net/https://"
                | "https://gh-proxy.org/https://"
        )
    {
        return Ok(value.to_string());
    }
    if value.len() > 300 || !value.ends_with('/') {
        return Err("镜像前缀必须以 / 结尾，且不超过 300 个字符".into());
    }
    let url = url::Url::parse(value).map_err(|_| "镜像前缀必须是有效的 HTTPS URL".to_string())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("镜像前缀必须使用 HTTPS，且不能包含账号、密码、查询参数或片段".into());
    }
    Ok(value.to_string())
}

fn as_color(v: &JsonValue) -> Result<String, String> {
    let s = v.as_str().ok_or_else(|| "需要字符串".to_string())?;
    let ok = s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        Ok(s.to_lowercase())
    } else {
        Err("颜色格式应为 #RRGGBB".into())
    }
}

fn as_optional_color(v: &JsonValue) -> Result<String, String> {
    let s = v.as_str().ok_or_else(|| "需要字符串".to_string())?;
    if s.is_empty() {
        Ok(String::new())
    } else {
        as_color(v)
    }
}

/// 热键字符串基础校验（最终能否注册由全局快捷键插件判定）
fn as_shortcut_str(v: &JsonValue) -> Result<String, String> {
    let s = v
        .as_str()
        .ok_or_else(|| "需要字符串".to_string())?
        .trim()
        .to_lowercase();
    if s.is_empty() || s.len() > 64 {
        return Err("热键不能为空".into());
    }
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err("热键格式应为 ctrl+alt+key 形式".into());
    }
    Ok(s)
}

/// 应用单个设置项；返回 Err 表示 key 未知或值非法。
pub fn apply_setting(
    s: &mut Settings,
    category: &str,
    key: &str,
    v: &JsonValue,
) -> Result<(), String> {
    match (category, key) {
        ("general", "run_on_startup") => s.general.run_on_startup = as_bool(v)?,
        ("general", "enable_mouse_hover") => s.general.enable_mouse_hover = as_bool(v)?,
        ("general", "auto_hide_delay") => {
            s.general.auto_hide_delay = as_u32_clamped(v, 0, 10000)?;
        }
        ("general", "hover_preview_delay") => {
            s.general.hover_preview_delay = as_u32_clamped(v, 50, 5000)?;
        }
        ("general", "control_center_always_on_top") => {
            s.general.control_center_always_on_top = as_bool(v)?;
        }
        ("general", "performance_profile") => {
            s.general.performance_profile =
                as_enum(v, &["eco", "balanced", "performance", "auto"])?;
        }
        ("general", "github_mirror") => {
            let value = v.as_str().ok_or_else(|| "镜像前缀需要是字符串".to_string())?;
            s.general.github_mirror = validate_github_mirror(value)?;
        }
        ("notifications", "render_mode") => {
            s.notifications.render_mode = as_enum(v, &["web", "native"])?;
        }
        ("notifications", "background_opacity") => {
            let opacity = as_f64_clamped(v, 0.0, 1.0)?;
            s.notifications.background_opacity = opacity;
            s.notifications.desktop_background_opacity = opacity;
            s.notifications.bubble_background_opacity = opacity;
        }
        ("notifications", "desktop_background_opacity") => {
            s.notifications.desktop_background_opacity = as_f64_clamped(v, 0.0, 1.0)?;
        }
        ("notifications", "bubble_background_opacity") => {
            s.notifications.bubble_background_opacity = as_f64_clamped(v, 0.0, 1.0)?;
        }
        ("notifications", "desktop_background_color") => {
            s.notifications.desktop_background_color = as_optional_color(v)?;
        }
        ("notifications", "bubble_background_color") => {
            s.notifications.bubble_background_color = as_optional_color(v)?;
        }
        ("appearance", "theme_mode") => {
            s.appearance.theme_mode = as_enum(v, &["light", "dark", "system"])?
        }
        ("appearance", "skin") => {
            let s_val = v.as_str().ok_or_else(|| "需要字符串".to_string())?;
            if let Some(uid) = s_val.strip_prefix("user:") {
                let ok = !uid.is_empty()
                    && uid.len() <= 64
                    && uid.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
                    });
                if !ok {
                    return Err("非法的用户皮肤 id".into());
                }
                s.appearance.skin = s_val.into();
            } else {
                s.appearance.skin = as_enum(v, &["default"])?;
            }
        }
        ("appearance", "accent_color") => s.appearance.accent_color = as_color(v)?,
        ("appearance", "background_image") => {
            let image = v.as_object().ok_or_else(|| "背景图片设置无效".to_string())?;
            let path = image.get("path").and_then(JsonValue::as_str)
                .ok_or_else(|| "背景图片路径无效".to_string())?;
            let name = image.get("name").and_then(JsonValue::as_str)
                .ok_or_else(|| "背景图片名称无效".to_string())?;
            if path.len() > 4096 || name.len() > 256
                || path.chars().any(char::is_control)
                || name.chars().any(char::is_control)
                || path.is_empty() != name.is_empty()
            {
                return Err("背景图片设置无效".into());
            }
            s.appearance.background_image_path = path.to_string();
            s.appearance.background_image_name = name.to_string();
        }
        ("appearance", "background_image_apply_to_sidebar") => {
            s.appearance.background_image_apply_to_sidebar = as_bool(v)?;
        }
        ("appearance", "background_image_fit") => {
            s.appearance.background_image_fit = as_enum(v, &["cover", "fill", "contain"])?;
        }
        ("appearance", "background_image_opacity") => {
            s.appearance.background_image_opacity = as_f64_clamped(v, 0.0, 1.0)?;
        }
        ("appearance", "background_image_blur") => {
            s.appearance.background_image_blur = as_f64_clamped(v, 0.0, 24.0)?;
        }
        ("appearance", "interface_scale") => {
            s.appearance.interface_scale = as_f64_clamped(v, 0.7, 1.6)?
        }
        ("appearance", "sidebar_position") => {
            s.appearance.sidebar_position = as_enum(v, &["left", "right", "top"])?
        }
        ("appearance", "sidebar_monitor_id") => {
            s.appearance.sidebar_monitor_id = match v {
                JsonValue::Null => None,
                JsonValue::String(id) if !id.trim().is_empty() && id.len() <= 1024 => {
                    Some(id.clone())
                }
                JsonValue::String(_) => return Err("显示器标识不能为空或过长".into()),
                _ => return Err("显示器标识格式无效".into()),
            }
        }
        ("appearance", "collapsed_width") => {
            s.appearance.collapsed_width = as_u32_clamped(v, 56, 96)?
        }
        ("appearance", "icon_size") => s.appearance.icon_size = as_u32_clamped(v, 20, 72)?,
        ("appearance", "peek_width") => s.appearance.peek_width = as_u32_clamped(v, 0, 10)?,
        ("appearance", "sidebar_panel_opacity") => {
            s.appearance.sidebar_panel_opacity = as_f64_clamped(v, 0.0, 1.0)?
        }
        ("appearance", "detail_panel_opacity") => {
            let opacity = as_f64_clamped(v, 0.0, 1.0)?;
            s.appearance.detail_panel_opacity = opacity;
            if s.appearance.skin == "default" {
                s.appearance.control_panel_opacity = opacity;
            }
        }
        ("appearance", "control_panel_opacity") => {
            let opacity = as_f64_clamped(v, 0.0, 1.0)?;
            s.appearance.control_panel_opacity = opacity;
            if s.appearance.skin == "default" {
                s.appearance.detail_panel_opacity = opacity;
            }
        }
        ("appearance", "sidebar_image_opacity") => {
            s.appearance.sidebar_image_opacity = as_f64_clamped(v, 0.0, 1.0)?
        }
        ("appearance", "detail_image_opacity") => {
            s.appearance.detail_image_opacity = as_f64_clamped(v, 0.0, 1.0)?
        }
        ("appearance", "preview_panel_opacity") => {
            s.appearance.preview_panel_opacity = as_f64_clamped(v, 0.0, 1.0)?
        }
        ("appearance", "preview_image_opacity") => {
            s.appearance.preview_image_opacity = as_f64_clamped(v, 0.0, 1.0)?
        }
        ("appearance", "preview_visible_rows") => {
            s.appearance.preview_visible_rows = as_u32_clamped(v, 1, 8)?
        }
        ("appearance", "preview_width") => {
            s.appearance.preview_width = if v.is_null() {
                None
            } else {
                Some(as_u32_clamped(v, 240, 480)?)
            }
        }
        ("appearance", "preview_display_mode") => {
            let mode = v
                .as_str()
                .filter(|mode| matches!(*mode, "default" | "compact"))
                .ok_or_else(|| "预览显示模式无效".to_string())?;
            s.appearance.preview_display_mode = mode.to_string();
        }
        ("appearance", "component_opacity") => {
            s.appearance.component_opacity = as_f64_clamped(v, 0.0, 1.0)?
        }
        ("appearance", "glass_effect_enabled") => {
            s.appearance.glass_effect_enabled = as_bool(v)?
        }
        ("appearance", "sidebar_height_percent") => {
            s.appearance.sidebar_height_percent = as_f64_clamped(v, 0.2, 1.0)?
        }
        ("appearance", "sidebar_y_offset") => {
            s.appearance.sidebar_y_offset = as_i32_clamped(v, -2000, 2000)?
        }
        ("appearance", "sidebar_x_offset") => {
            s.appearance.sidebar_x_offset = as_i32_clamped(v, -2000, 2000)?
        }
        ("appearance", "sidebar_border_color") => s.appearance.sidebar_border_color = as_color(v)?,
        ("shortcuts", "toggle_sidebar") => s.shortcuts.toggle_sidebar = as_shortcut_str(v)?,
        ("shortcuts", "control_center") => s.shortcuts.control_center = as_shortcut_str(v)?,
        _ => return Err(format!("未知设置项 {category}.{key}")),
    }
    Ok(())
}
