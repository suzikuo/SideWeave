//! 旧 Python 版（AgileTiles）数据一次性导入。
//! 读取 %APPDATA%\AgileTiles\state.json，将可映射的应用设置写入新数据目录。

use std::path::{Path, PathBuf};

use super::paths;
use super::settings::{self, Settings};

fn legacy_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("AgileTiles"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|d| PathBuf::from(d).join(".config").join("AgileTiles"))
    }
}

/// 旧 → 新设置项映射（键名一致的部分直接沿用，由 apply_setting 做校验与钳制）
const MAPPED_SETTINGS: &[(&str, &str)] = &[
    ("general", "run_on_startup"),
    ("general", "enable_mouse_hover"),
    ("general", "auto_hide_delay"),
    ("appearance", "theme_mode"),
    ("appearance", "accent_color"),
    ("appearance", "sidebar_position"),
    ("appearance", "peek_width"),
    ("appearance", "sidebar_height_percent"),
    ("appearance", "sidebar_y_offset"),
    ("appearance", "sidebar_border_color"),
    ("shortcuts", "toggle_sidebar"),
];

pub fn migrate_legacy(new_data_dir: &Path, settings: &mut Settings) {
    let Some(legacy) = legacy_root() else {
        return;
    };
    if !legacy.exists() {
        return;
    }
    let marker = new_data_dir.join(".legacy-migrated-v1");
    if marker.exists() {
        return;
    }

    // 1. state.json 中的设置（损坏时保留重试机会，不写 marker）
    let legacy_state = legacy.join("state.json");
    if legacy_state.exists() {
        let Some(state) = paths::load_json(&legacy_state) else {
            return;
        };
        for (cat, key) in MAPPED_SETTINGS {
            if let Some(v) = state.pointer(&format!("/settings/{cat}/{key}")) {
                let _ = settings::apply_setting(settings, cat, key, v);
            }
        }
        if let Some(v) = state.pointer("/settings/appearance/sidebar_bg_opacity") {
            let _ = settings::apply_setting(settings, "appearance", "sidebar_panel_opacity", v);
            let _ = settings::apply_setting(settings, "appearance", "sidebar_image_opacity", v);
        }
        if let Some(v) = state.pointer("/settings/appearance/detail_bg_opacity") {
            let _ = settings::apply_setting(settings, "appearance", "detail_panel_opacity", v);
            let _ = settings::apply_setting(settings, "appearance", "control_panel_opacity", v);
            let _ = settings::apply_setting(settings, "appearance", "detail_image_opacity", v);
        }
        // 旧 collapsed_width 下限 32，新轨道下限 56，交给钳制逻辑
        if let Some(v) = state.pointer("/settings/appearance/collapsed_width") {
            let _ = settings::apply_setting(settings, "appearance", "collapsed_width", v);
        }
    }

    let _ = std::fs::write(&marker, b"ok");
}
