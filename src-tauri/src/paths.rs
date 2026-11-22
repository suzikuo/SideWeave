use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{Manager, Runtime};

static ATOMIC_WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

/// 应用数据根目录（%APPDATA%/<identifier>）
pub fn app_data_dir<R: Runtime>(app: &tauri::AppHandle<R>) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("sideweave_"))
}

pub fn state_file(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("state.json")
}

/// 原子写 JSON：先写同目录唯一临时文件再 rename 覆盖，避免并发写入冲突
pub fn save_json_atomic(path: &std::path::Path, value: &serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value)?;
    let seq = ATOMIC_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data.json");
    let tmp = path.with_file_name(format!("{file_name}.{}.{seq}.tmp", std::process::id()));
    std::fs::write(&tmp, text.as_bytes())?;
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

pub fn load_json(path: &std::path::Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}
