//! 插件悬浮预览卡片 (Plugin Hover Preview Card) 协调调度器与窗口管理。
//!
//! 职责划分：
//! 1. 负责窗口生命周期（惰性创建、按需隐藏）、物理屏幕坐标计算与停靠定位、无焦点显示（`SW_SHOWNOACTIVATE`）。
//! 2. 调度侧边栏与卡片两端的鼠标悬停（350ms 展开防抖，250ms 移出缓冲）。
//! 3. 严禁直接调用任何具体插件业务逻辑；所有预览数据获取必须经过通用的 `plugin_call_blocking`。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sideweave_plugin_api::{PluginId, PluginRequest, PluginRequestKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

pub const PREVIEW_WINDOW: &str = "hover_preview";
const DEFAULT_CARD_WIDTH: u32 = 280;
const DEFAULT_CARD_HEIGHT: u32 = 180;
const CARD_GAP: f64 = 8.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ButtonRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub struct PreviewCoordinator {
    show_gen: AtomicU64,
    hide_gen: AtomicU64,
    is_showing: AtomicBool,
    in_preview: AtomicBool,
    hover_target: Mutex<Option<(String, ButtonRect)>>,
    current_payload: Mutex<Option<Value>>,
}

impl Default for PreviewCoordinator {
    fn default() -> Self {
        Self {
            show_gen: AtomicU64::new(0),
            hide_gen: AtomicU64::new(0),
            is_showing: AtomicBool::new(false),
            in_preview: AtomicBool::new(false),
            hover_target: Mutex::new(None),
            current_payload: Mutex::new(None),
        }
    }
}

/// 计算预览卡片的屏幕物理像素坐标与物理尺寸，并进行多显示器工作区边界 Clamp
pub fn calculate_preview_geometry(
    sidebar_pos: PhysicalPosition<i32>,
    sidebar_size: PhysicalSize<u32>,
    scale: f64,
    sidebar_position: &str,
    btn_rect: &ButtonRect,
    card_logical_w: u32,
    card_logical_h: u32,
    work_area: &crate::platform::WorkArea,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let card_w_phys = (card_logical_w as f64 * scale).round() as i32;
    let card_h_phys = (card_logical_h as f64 * scale).round() as i32;
    let gap_phys = (CARD_GAP * scale).round() as i32;

    let btn_phys_x = sidebar_pos.x + (btn_rect.x * scale).round() as i32;
    let btn_phys_y = sidebar_pos.y + (btn_rect.y * scale).round() as i32;
    let btn_phys_w = (btn_rect.width * scale).round() as i32;
    let btn_phys_h = (btn_rect.height * scale).round() as i32;

    let (target_x, target_y) = match sidebar_position {
        "left" => {
            let x = sidebar_pos.x + sidebar_size.width as i32 + gap_phys;
            let y = btn_phys_y + (btn_phys_h - card_h_phys) / 2;
            (x, y)
        }
        "top" => {
            let x = btn_phys_x + (btn_phys_w - card_w_phys) / 2;
            let y = sidebar_pos.y + sidebar_size.height as i32 + gap_phys;
            (x, y)
        }
        _ => {
            // right (默认)
            let x = sidebar_pos.x - card_w_phys - gap_phys;
            let y = btn_phys_y + (btn_phys_h - card_h_phys) / 2;
            (x, y)
        }
    };

    let min_x = work_area.x + 4;
    let max_x = (work_area.x + work_area.w - card_w_phys - 4).max(min_x);
    let min_y = work_area.y + 4;
    let max_y = (work_area.y + work_area.h - card_h_phys - 4).max(min_y);

    let clamped_x = target_x.clamp(min_x, max_x);
    let clamped_y = target_y.clamp(min_y, max_y);

    (
        PhysicalPosition::new(clamped_x, clamped_y),
        PhysicalSize::new(card_w_phys as u32, card_h_phys as u32),
    )
}

pub fn ensure_preview_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(win) = app.get_webview_window(PREVIEW_WINDOW) {
        return Ok(win);
    }
    let win =
        WebviewWindowBuilder::new(app, PREVIEW_WINDOW, WebviewUrl::App("preview.html".into()))
            .title("SideWeave Preview")
            .inner_size(DEFAULT_CARD_WIDTH as f64, DEFAULT_CARD_HEIGHT as f64)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .focused(false)
            .focusable(false)
            .visible(false)
            .build()
            .map_err(|e| format!("创建预览窗口失败: {e}"))?;
    crate::apply_interface_scale_to_window(&win, crate::interface_scale(app));

    Ok(win)
}

fn fallback_payload(plugin_id: &str, manifest: &sideweave_plugin_api::PluginManifest) -> Value {
    serde_json::json!({
        "mode": "static",
        "pluginId": plugin_id,
        "title": manifest.name,
        "status": "info",
        "statusText": format!("v{}", manifest.version),
        "subtitle": manifest.description,
        "metrics": [],
        "actions": []
    })
}

fn estimate_declarative_card_height(payload: &Value, max_height: u32) -> u32 {
    let mut content_height = 32_u32; // title and optional subtitle
    let mut sections = 1_u32; // header

    if let Some(progress) = payload.get("progress").filter(|value| value.is_object()) {
        content_height += if progress.get("label").and_then(Value::as_str).is_some() {
            24
        } else {
            7
        };
        sections += 1;
    }

    if let Some(items) = payload.get("items").and_then(Value::as_array) {
        if items.is_empty() {
            content_height += 92; // empty state, including its explanatory text
        } else {
            let rows = items.len().min(20) as u32;
            content_height += (rows * 26 + rows.saturating_sub(1) * 4).min(146);
        }
        sections += 1;
    } else if let Some(metrics) = payload
        .get("metrics")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    {
        let rows = ((metrics.len() + 1) / 2).min(10) as u32;
        content_height += rows * 40 + rows.saturating_sub(1) * 6;
        sections += 1;
    }

    let action_count = payload
        .get("actions")
        .and_then(Value::as_array)
        .map(|actions| {
            actions
                .iter()
                .filter(|action| {
                    action
                        .get("method")
                        .and_then(Value::as_str)
                        .map(|method| !method.is_empty())
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    if action_count > 0 {
        content_height += 26;
        sections += 1;
    }

    // The card has 12px of internal vertical padding and the transparent window adds 6px per side.
    let estimated = content_height + sections.saturating_sub(1) * 8 + 36;
    estimated.clamp(72.min(max_height), max_height.max(1))
}

fn fetch_preview_payload(app: &AppHandle, plugin_id: &str) -> Option<(Value, u32, u32)> {
    let manifest = crate::plugin_manifest(app, plugin_id)?;
    let hover_config = manifest.hover_preview.as_ref()?;
    let card_w = hover_config.width.unwrap_or(DEFAULT_CARD_WIDTH);
    let max_card_h = hover_config.height.unwrap_or(DEFAULT_CARD_HEIGHT);

    let payload = if let Some(ref entry) = hover_config.entry {
        let clean_entry = entry.trim_matches(|c| c == '/' || c == '\\').trim();
        if clean_entry.is_empty()
            || clean_entry.contains("..")
            || clean_entry.contains(':')
            || clean_entry.contains('\\')
        {
            fallback_payload(plugin_id, &manifest)
        } else {
            let clean_path = clean_entry
                .split('/')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            let custom_url = format!("http://agile-plugin.localhost/{}/{}", plugin_id, clean_path);
            serde_json::json!({
                "mode": "custom",
                "pluginId": plugin_id,
                "title": manifest.name,
                "customUrl": custom_url
            })
        }
    } else if let Some(ref method) = hover_config.data_method {
        let request = PluginRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            plugin_id: match PluginId::new(plugin_id) {
                Ok(id) => id,
                Err(_) => return None,
            },
            method: Some(method.clone()),
            payload: Value::Null,
            kind: PluginRequestKind::Call,
            timeout_ms: Some(3000),
        };
        match crate::plugin_call_blocking(app, request) {
            Ok(response) if response.ok => {
                let mut val = response.result;
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("mode".into(), Value::String("declarative".into()));
                    obj.insert("pluginId".into(), Value::String(plugin_id.to_string()));
                    if !obj.contains_key("title") {
                        obj.insert("title".into(), Value::String(manifest.name.clone()));
                    }
                }
                val
            }
            _ => fallback_payload(plugin_id, &manifest),
        }
    } else {
        fallback_payload(plugin_id, &manifest)
    };

    let card_h = if payload.get("mode").and_then(Value::as_str) == Some("custom") {
        max_card_h
    } else {
        estimate_declarative_card_height(&payload, max_card_h)
    };

    Some((payload, card_w, card_h))
}

impl PreviewCoordinator {
    fn show_preview_impl(
        &self,
        app: &AppHandle,
        rect: ButtonRect,
        payload_data: (Value, u32, u32),
    ) {
        let (payload, card_w, card_h) = payload_data;
        // 写入当前快照以供初次挂载主动读取
        *self.current_payload.lock().unwrap() = Some(payload.clone());

        // 广播给已挂载的预览卡片前端
        let _ = app.emit_to(PREVIEW_WINDOW, "preview://update", &payload);

        let Some(sidebar) = app.get_webview_window(crate::SIDEBAR) else {
            return;
        };
        let Ok(sidebar_pos) = sidebar.outer_position() else {
            return;
        };
        let Ok(sidebar_size) = sidebar.outer_size() else {
            return;
        };
        let s = crate::host_settings(app);
        let scale = crate::monitor_scale(app) * s.appearance.interface_scale.clamp(0.7, 1.6);
        let work_area = crate::platform::work_area();

        let (target_pos, target_size) = calculate_preview_geometry(
            sidebar_pos,
            sidebar_size,
            scale,
            &s.appearance.sidebar_position,
            &rect,
            card_w,
            card_h,
            &work_area,
        );

        let Ok(win) = ensure_preview_window(app) else {
            return;
        };

        let _ = win.set_size(target_size);
        let _ = win.set_position(target_pos);

        // 必须调用 win.show() 确保 Tauri/Wry 将 WebView2 控制器可见性设为 TRUE
        let _ = win.show();
        #[cfg(windows)]
        {
            if let Ok(hwnd) = win.hwnd() {
                crate::platform::show_window_no_activate(hwnd.0);
            }
        }

        self.is_showing.store(true, Ordering::SeqCst);
        let _ = app.emit_to(crate::SIDEBAR, "preview://opened", ());
    }

    pub fn hide_preview_impl(&self, app: &AppHandle) {
        if let Some(win) = app.get_webview_window(PREVIEW_WINDOW) {
            let _ = win.hide();
        }
        self.is_showing.store(false, Ordering::SeqCst);
        self.in_preview.store(false, Ordering::SeqCst);
        check_sidebar_collapse_after_preview(app);
    }

    pub fn sidebar_enter(self: &Arc<Self>, app: &AppHandle, plugin_id: String, rect: ButtonRect) {
        let has_hover = crate::plugin_manifest(app, &plugin_id)
            .and_then(|m| m.hover_preview)
            .is_some();
        if !has_hover {
            self.hide_now(app);
            return;
        }

        let policy = crate::performance_manager(app)
            .effective_policy(Some(crate::host_settings(app).general.hover_preview_delay));
        let base_delay = policy.hover_delay_ms as u64;
        let is_showing = self.is_showing.load(Ordering::SeqCst);
        let delay_ms = if is_showing {
            120.min(base_delay)
        } else {
            base_delay
        };

        let current_show = self.show_gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.hide_gen.fetch_add(1, Ordering::SeqCst); // 取消待决的隐藏

        *self.hover_target.lock().unwrap() = Some((plugin_id.clone(), rect));

        let coord = Arc::clone(self);
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if coord.show_gen.load(Ordering::SeqCst) == current_show {
                let target = coord.hover_target.lock().unwrap().clone();
                if let Some((target_id, target_rect)) = target {
                    if target_id == plugin_id {
                        let fetch_app = app_clone.clone();
                        let fetch_id = target_id.clone();
                        let fetched = tokio::task::spawn_blocking(move || {
                            fetch_preview_payload(&fetch_app, &fetch_id)
                        })
                        .await
                        .ok()
                        .flatten();
                        let still_current = coord.show_gen.load(Ordering::SeqCst) == current_show
                            && coord
                                .hover_target
                                .lock()
                                .unwrap()
                                .as_ref()
                                .is_some_and(|(id, _)| id == &target_id);
                        if still_current {
                            if let Some(payload_data) = fetched {
                                coord.show_preview_impl(&app_clone, target_rect, payload_data);
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn sidebar_leave(self: &Arc<Self>, app: &AppHandle, plugin_id: String) {
        {
            let mut target = self.hover_target.lock().unwrap();
            if let Some((id, _)) = target.as_ref() {
                if id == &plugin_id {
                    *target = None;
                }
            }
        }
        self.show_gen.fetch_add(1, Ordering::SeqCst); // 取消待决的弹出
        let current_hide = self.hide_gen.fetch_add(1, Ordering::SeqCst) + 1;

        let coord = Arc::clone(self);
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if coord.hide_gen.load(Ordering::SeqCst) == current_hide {
                if !coord.in_preview.load(Ordering::SeqCst)
                    && coord.hover_target.lock().unwrap().is_none()
                {
                    coord.hide_preview_impl(&app_clone);
                }
            }
        });
    }

    pub fn preview_window_enter(&self) {
        self.in_preview.store(true, Ordering::SeqCst);
        self.hide_gen.fetch_add(1, Ordering::SeqCst); // 鼠标进入卡片，取消隐藏
    }

    pub fn preview_window_leave(self: &Arc<Self>, app: &AppHandle) {
        self.in_preview.store(false, Ordering::SeqCst);
        let current_hide = self.hide_gen.fetch_add(1, Ordering::SeqCst) + 1;

        let coord = Arc::clone(self);
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if coord.hide_gen.load(Ordering::SeqCst) == current_hide {
                if !coord.in_preview.load(Ordering::SeqCst)
                    && coord.hover_target.lock().unwrap().is_none()
                {
                    coord.hide_preview_impl(&app_clone);
                }
            }
        });
    }

    pub fn hide_now(&self, app: &AppHandle) {
        self.show_gen.fetch_add(1, Ordering::SeqCst);
        self.hide_gen.fetch_add(1, Ordering::SeqCst);
        *self.hover_target.lock().unwrap() = None;
        self.hide_preview_impl(app);
    }

    pub fn get_current_preview(&self) -> Option<Value> {
        self.current_payload.lock().unwrap().clone()
    }

    pub fn refresh_preview(&self, app: &AppHandle) -> Option<Value> {
        let current_id = {
            let payload = self.current_payload.lock().unwrap();
            payload
                .as_ref()
                .and_then(|p| p.get("pluginId").and_then(Value::as_str).map(String::from))
        }?;
        let (new_payload, _, _) = fetch_preview_payload(app, &current_id)?;
        *self.current_payload.lock().unwrap() = Some(new_payload.clone());
        let _ = app.emit_to(PREVIEW_WINDOW, "preview://update", &new_payload);
        Some(new_payload)
    }

    #[allow(dead_code)]
    pub fn is_in_preview(&self) -> bool {
        self.in_preview.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.is_showing.load(Ordering::SeqCst) || self.in_preview.load(Ordering::SeqCst)
    }
}

pub fn point_in_rect(px: f64, py: f64, rx: i32, ry: i32, rw: u32, rh: u32) -> bool {
    px >= rx as f64
        && px < (rx + rw as i32) as f64
        && py >= ry as f64
        && py < (ry + rh as i32) as f64
}

pub fn is_cursor_in_sidebar<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    let Ok(cursor) = app.cursor_position() else {
        return false;
    };
    let Some(sidebar) = app.get_webview_window(crate::SIDEBAR) else {
        return false;
    };
    let Ok(pos) = sidebar.outer_position() else {
        return false;
    };
    let Ok(size) = sidebar.outer_size() else {
        return false;
    };
    point_in_rect(cursor.x, cursor.y, pos.x, pos.y, size.width, size.height)
}

pub fn check_sidebar_collapse_after_preview<R: tauri::Runtime>(app: &AppHandle<R>) {
    let settings = crate::host_settings(app);
    if !settings.general.enable_mouse_hover {
        return;
    }
    if !crate::expanded_flag(app) {
        return;
    }
    if is_cursor_in_sidebar(app) {
        return;
    }
    // 递增 hover_token，使残余的未决 hover 收起任务失效
    let host = crate::host(app);
    let mut token = host.hover_token.lock().unwrap();
    *token += 1;
    drop(token);

    crate::collapse(app);
}

// ---------- Tauri Commands ----------

#[tauri::command]
pub async fn preview_sidebar_enter(
    app: AppHandle,
    state: tauri::State<'_, Arc<PreviewCoordinator>>,
    plugin_id: String,
    rect: ButtonRect,
) -> Result<(), String> {
    state.sidebar_enter(&app, plugin_id, rect);
    Ok(())
}

#[tauri::command]
pub async fn preview_sidebar_leave(
    app: AppHandle,
    state: tauri::State<'_, Arc<PreviewCoordinator>>,
    plugin_id: String,
) -> Result<(), String> {
    state.sidebar_leave(&app, plugin_id);
    Ok(())
}

#[tauri::command]
pub fn preview_window_enter(state: tauri::State<'_, Arc<PreviewCoordinator>>) {
    state.preview_window_enter();
}

#[tauri::command]
pub async fn preview_window_leave(
    app: AppHandle,
    state: tauri::State<'_, Arc<PreviewCoordinator>>,
) -> Result<(), String> {
    state.preview_window_leave(&app);
    Ok(())
}

#[tauri::command]
pub fn preview_hide_now(app: AppHandle, state: tauri::State<'_, Arc<PreviewCoordinator>>) {
    state.hide_now(&app);
}

#[tauri::command]
pub fn get_current_preview(state: tauri::State<'_, Arc<PreviewCoordinator>>) -> Option<Value> {
    state.get_current_preview()
}

#[tauri::command]
pub async fn refresh_preview(
    app: AppHandle,
    state: tauri::State<'_, Arc<PreviewCoordinator>>,
) -> Result<Option<Value>, String> {
    let coordinator = Arc::clone(state.inner());
    tokio::task::spawn_blocking(move || coordinator.refresh_preview(&app))
        .await
        .map_err(|error| format!("preview refresh worker failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::WorkArea;

    #[test]
    fn test_calculate_preview_geometry_right() {
        let work_area = WorkArea {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let sidebar_pos = PhysicalPosition::new(1850, 200);
        let sidebar_size = PhysicalSize::new(70, 400);
        let btn_rect = ButtonRect {
            x: 10.0,
            y: 50.0,
            width: 40.0,
            height: 40.0,
        };

        let (pos, size) = calculate_preview_geometry(
            sidebar_pos,
            sidebar_size,
            1.0,
            "right",
            &btn_rect,
            280,
            180,
            &work_area,
        );

        assert_eq!(size.width, 280);
        assert_eq!(size.height, 180);
        // target_x = 1850 - 280 - 8 = 1562
        assert_eq!(pos.x, 1562);
        // btn_phys_y = 200 + 50 = 250. center = 250 + (40 - 180)/2 = 180
        assert_eq!(pos.y, 180);
    }

    #[test]
    fn test_calculate_preview_geometry_clamping() {
        let work_area = WorkArea {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let sidebar_pos = PhysicalPosition::new(1850, 10);
        let sidebar_size = PhysicalSize::new(70, 400);
        let btn_rect = ButtonRect {
            x: 10.0,
            y: 10.0,
            width: 40.0,
            height: 40.0,
        };

        // target_y would be 20 + (40-180)/2 = -50, clamped to min_y (4)
        let (pos, _) = calculate_preview_geometry(
            sidebar_pos,
            sidebar_size,
            1.0,
            "right",
            &btn_rect,
            280,
            180,
            &work_area,
        );

        assert_eq!(pos.y, 4);
    }

    #[test]
    fn test_point_in_rect() {
        assert!(point_in_rect(10.0, 20.0, 0, 0, 100, 100));
        assert!(point_in_rect(0.0, 0.0, 0, 0, 100, 100));
        assert!(!point_in_rect(100.0, 20.0, 0, 0, 100, 100));
        assert!(!point_in_rect(10.0, 100.0, 0, 0, 100, 100));
        assert!(!point_in_rect(-1.0, 20.0, 0, 0, 100, 100));
        // Negative coordinates on multi-monitor
        assert!(point_in_rect(-1500.0, 100.0, -1920, 0, 1920, 1080));
        assert!(!point_in_rect(-2000.0, 100.0, -1920, 0, 1920, 1080));
    }
}
