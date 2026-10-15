//! 设置与数据备份：把应用数据目录的关键内容打包为 zip 导出，导入时原样恢复。
//! 覆盖：设置、插件数据、已安装的市场插件/皮肤、窗口记忆。
//! 不覆盖：日志与临时导入目录。导入完成后应用自动重启生效。

use std::io::Write as _;
use std::path::PathBuf;

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt as _;

use crate::paths;

/// 参与备份的顶层白名单
const TOP_ALLOWED: &[&str] = &[
    "state.json",
    "detail_geom.json",
    "detail_window.json",
    "control_window.json",
    "update_ignore.json",
    "plugins",
    "user_plugins",
    "user_skins",
];

fn app_data_root(app: &AppHandle) -> PathBuf {
    paths::app_data_dir(app)
}

fn collect_files(root: &PathBuf, out: &mut Vec<(String, PathBuf)>) {
    for f in TOP_ALLOWED {
        let p = root.join(f);
        if p.is_file() {
            out.push((f.to_string(), p));
        }
    }
    collect_dir(&root.join("plugins"), "plugins", out);
    collect_dir(&root.join("user_plugins"), "user_plugins", out);
    collect_dir(&root.join("user_skins"), "user_skins", out);
}

fn collect_dir(base: &PathBuf, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let child_rel = format!("{rel}/{}", e.file_name().to_string_lossy());
        if p.is_dir() {
            collect_dir(&p, &child_rel, out);
        } else {
            out.push((child_rel, p));
        }
    }
}

/// 导出备份：弹保存框 → 打包 zip → 返回保存路径（取消返回空串）
#[tauri::command]
pub async fn backup_export(app: AppHandle) -> Result<String, String> {
    let root = app_data_root(&app);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let file = crate::with_detail_not_always_on_top(&app, || {
        let mut dialog = app.dialog().file();
        if let Some(window) = crate::detail_window(&app) {
            dialog = dialog.set_parent(&window);
        }
        dialog
            .add_filter("备份文件", &["zip"])
            .set_file_name(&format!("sideweave_backup-{stamp}.zip"))
            .blocking_save_file()
    });
    let Some(file) = file else {
        return Ok(String::new());
    };
    let dest = file.into_path().map_err(|e| format!("路径无效: {e}"))?;

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    if files.is_empty() {
        return Err("没有可备份的数据".into());
    }

    let out_file = std::fs::File::create(&dest).map_err(|e| format!("创建备份文件失败: {e}"))?;
    let mut zip = zip::ZipWriter::new(out_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (rel, path) in &files {
        zip.start_file(rel.clone(), options)
            .map_err(|e| format!("写入备份失败: {e}"))?;
        let bytes = std::fs::read(path).map_err(|e| format!("读取 {rel} 失败: {e}"))?;
        zip.write_all(&bytes)
            .map_err(|e| format!("写入备份失败: {e}"))?;
    }
    zip.finish().map_err(|e| format!("完成备份失败: {e}"))?;
    Ok(dest.to_string_lossy().into_owned())
}

/// 导入备份：弹文件框选 zip → 校验 → 覆盖恢复 → 自动重启应用生效
#[tauri::command]
pub async fn backup_import(app: AppHandle) -> Result<(), String> {
    let file = crate::with_detail_not_always_on_top(&app, || {
        let mut dialog = app.dialog().file();
        if let Some(window) = crate::detail_window(&app) {
            dialog = dialog.set_parent(&window);
        }
        dialog.add_filter("备份文件", &["zip"]).blocking_pick_file()
    });
    let Some(file) = file else {
        return Ok(());
    };
    let path = file.into_path().map_err(|e| format!("路径无效: {e}"))?;
    let f = std::fs::File::open(&path).map_err(|e| format!("打开备份失败: {e}"))?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("备份文件无效: {e}"))?;
    let root = app_data_root(&app);

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // 防 zip slip：仅允许白名单顶层内的相对路径
        let Some(rel) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let top = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default();
        if !TOP_ALLOWED.contains(&top.as_str()) {
            continue;
        }
        let dest = root.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("写入 {} 失败: {e}", rel.to_string_lossy()))?;
    }

    // 全部数据已恢复：重启应用加载新状态
    app.restart();
    #[allow(unreachable_code)]
    Ok(())
}
