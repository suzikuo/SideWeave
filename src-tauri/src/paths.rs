use std::path::PathBuf;

use tauri::{Manager, Runtime};

/// 应用数据根目录（%APPDATA%/<identifier>）
pub fn app_data_dir<R: Runtime>(app: &tauri::AppHandle<R>) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("sideweave_"))
}

pub fn state_file(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("state.json")
}

/// 原子写 JSON：先写同目录临时文件再 rename 覆盖
pub fn save_json_atomic(path: &std::path::Path, value: &serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text.as_bytes())?;
    std::fs::rename(&tmp, path)
}

pub fn load_json(path: &std::path::Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}
