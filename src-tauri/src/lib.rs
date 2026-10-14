//! SideWeave —— Rust + Tauri 桌面侧边栏宿主。
//! 双窗口：sidebar（贴边圆环轨道，鼠标离开自动收起）+ detail（详情卡片）。
//! 托盘、Alt+Space 全局热键、一次性旧数据迁移、内置 app_launcher 插件。

mod backup;
mod desktop_notifications;
mod extension_host;
mod external_process;
mod market;
mod migrate;
mod paths;
pub mod performance;
mod platform;
mod plugins;
mod preview_window;
mod secret_store;
mod settings;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::webview::PageLoadEvent;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::Shortcut;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use agile_tiles_plugin_api::{
    PluginHandshake, PluginHandshakeResponse, PluginInfo, PluginRequest, PluginResponse,
    PluginRuntime, PluginStatus,
};
use agile_tiles_plugin_runtime::PluginManager;
use plugins::PluginMeta;
use settings::Settings;

pub(crate) const SIDEBAR: &str = "sidebar";
const DETAIL: &str = "detail";
const DETAIL_W: i32 = 580;
const DETAIL_H: i32 = 860;
const DETAIL_COMPACT_H: f64 = 52.0;
const GAP: i32 = 8;
const SIDEBAR_COLLAPSE_DURATION: Duration = Duration::from_millis(340);
const SIDEBAR_COLLAPSE_FRAME: Duration = Duration::from_millis(16);

pub(crate) fn plugin_manifest<R: tauri::Runtime>(
    app: &AppHandle<R>,
    id: &str,
) -> Option<agile_tiles_plugin_api::PluginManifest> {
    let plugin_id = agile_tiles_plugin_api::PluginId::new(id).ok()?;
    let state = host(app);
    let manager = state.plugin_manager.lock().unwrap();
    manager.as_ref().and_then(|m| {
        m.registry()
            .get(&plugin_id)
            .map(|info| info.manifest.clone())
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MainSurface {
    Sidebar,
    Detail,
    Control,
}

/// 轨道插件清单来自运行时注册表，宿主不维护逐插件业务注册表。
fn rail_metas<R: tauri::Runtime>(app: &AppHandle<R>, s: &Settings) -> Vec<PluginMeta> {
    let runtime_plugins = host(app)
        .plugin_manager
        .lock()
        .unwrap()
        .as_ref()
        .map(|manager| {
            manager
                .registry()
                .list()
                .into_iter()
                .map(|info| PluginMeta {
                    id: info.manifest.id.to_string(),
                    name: info.manifest.name,
                    icon: plugin_icon_source(&info.manifest.id.to_string(), &info.manifest.icon),
                    description: info.manifest.description,
                    version: info.manifest.version,
                    enabled: true,
                    sidebar_visible: true,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    plugins::meta(&s.plugins, &runtime_plugins, &[])
}

/// 共享宿主状态
struct Host {
    settings: Mutex<Settings>,
    performance_manager: Arc<performance::PerformanceManager>,
    plugin_manager: Mutex<Option<PluginManager>>,
    plugin_services: Mutex<Option<Arc<dyn agile_tiles_plugin_sdk::HostServices>>>,
    /// 悬停令牌：hover_enter 自增，延迟收起前比对，防止误收
    hover_token: Mutex<u64>,
    /// 收起几何落地代数：延迟落地期间有 expand/新 collapse 则取消
    collapse_gen: AtomicU64,
    /// 详情卡尺寸只由 Rust 设定一次，其后尊重用户手动调整
    /// 详情窗口是否处于应用内紧凑态；紧凑态不等同于原生最小化。
    detail_compact: AtomicBool,
    /// 详情窗口销毁代数：关闭后延迟 30 秒销毁；若 30 秒内重新打开则递增取消
    detail_destroy_gen: AtomicU64,
    /// 侧边栏当前是否展开。
    expanded: AtomicBool,
    /// 详情卡当前展示的页面（供"再次点击同一插件 → 关闭详情页"判断）
    detail_page: Mutex<String>,
    main_surface: Mutex<MainSurface>,
    geometry_transition: AtomicBool,
    detail_opening: AtomicBool,
}

#[derive(Serialize)]
struct StateSnapshot {
    settings: Settings,
    plugins: Vec<PluginMeta>,
    autostart_enabled: bool,
    app_version: String,
    data_dir: String,
}

#[tauri::command]
fn host_notification_snapshot(
    window: tauri::WebviewWindow,
    app: AppHandle,
) -> Result<desktop_notifications::NotificationViewSnapshot, String> {
    let surface = desktop_notifications::Surface::from_label(window.label()).ok_or_else(|| {
        "notifications are only available in host notification windows".to_string()
    })?;
    Ok(desktop_notifications::view_snapshot(&app, surface))
}

#[tauri::command]
fn host_notification_dismiss(
    window: tauri::WebviewWindow,
    app: AppHandle,
    id: String,
) -> Result<(), String> {
    let surface = desktop_notifications::Surface::from_label(window.label()).ok_or_else(|| {
        "notifications are only available in host notification windows".to_string()
    })?;
    app.state::<desktop_notifications::DesktopNotificationManager>()
        .dismiss(app.clone(), surface, &id)
}

#[tauri::command]
fn host_notification_resize(
    window: tauri::WebviewWindow,
    app: AppHandle,
    height: f64,
) -> Result<(), String> {
    let surface = desktop_notifications::Surface::from_label(window.label())
        .ok_or_else(|| "only host notification windows can resize this surface".to_string())?;
    desktop_notifications::resize_surface_window(&window, &app, surface, height)
}

#[tauri::command]
fn host_notification_plugin_icon(
    window: tauri::WebviewWindow,
    app: AppHandle,
    plugin_id: String,
) -> Result<String, String> {
    if desktop_notifications::Surface::from_label(window.label()).is_none() {
        return Err("plugin icons are only available in host notification windows".to_string());
    }
    Ok(plugin_manifest(&app, &plugin_id)
        .map(|manifest| plugin_icon_source(&plugin_id, &manifest.icon))
        .unwrap_or_else(|| "puzzle".to_string()))
}

/// 命令之外的模块读取设置快照
pub(crate) fn host_settings<R: tauri::Runtime>(app: &AppHandle<R>) -> Settings {
    host(app).settings.lock().unwrap().clone()
}

pub(crate) fn performance_manager<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Arc<performance::PerformanceManager> {
    Arc::clone(&host(app).performance_manager)
}

pub(crate) fn plugin_icon_source(plugin_id: &str, icon_path: &str) -> String {
    use std::path::Component;

    let path = Path::new(icon_path);
    let supported_format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["svg", "png", "webp", "jpg", "jpeg", "ico"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        });
    if icon_path.trim().is_empty()
        || icon_path.contains('\\')
        || !path.is_relative()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        || !supported_format
    {
        return "puzzle".into();
    }

    let resource = icon_path
        .split('/')
        .map(|segment| {
            percent_encoding::utf8_percent_encode(segment, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("http://agile-plugin.localhost/{plugin_id}/{resource}")
}

impl Host {
    fn settings_clone(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
}

pub(crate) fn plugin_permission_granted<R: tauri::Runtime>(
    app: &AppHandle<R>,
    plugin_id: &agile_tiles_plugin_api::PluginId,
    permission: &str,
) -> bool {
    host(app)
        .settings_clone()
        .plugins
        .permission_grants
        .get(plugin_id.as_str())
        .and_then(|grants| grants.get(permission))
        .copied()
        .unwrap_or(false)
}

// ---------- 几何布局 ----------

/// 光标所在显示器的缩放系数（设置值是逻辑像素，窗口 API 用物理像素）
pub(crate) fn monitor_scale<R: tauri::Runtime>(app: &AppHandle<R>) -> f64 {
    let Ok(cursor) = app.cursor_position() else {
        return 1.0;
    };
    let Ok(monitors) = app.available_monitors() else {
        return 1.0;
    };
    for m in &monitors {
        let (mp, ms) = (m.position(), m.size());
        let (mx, my) = (mp.x as f64, mp.y as f64);
        if cursor.x >= mx
            && cursor.x < mx + ms.width as f64
            && cursor.y >= my
            && cursor.y < my + ms.height as f64
        {
            return m.scale_factor();
        }
    }
    monitors.first().map(|m| m.scale_factor()).unwrap_or(1.0)
}

pub(crate) fn interface_scale<R: tauri::Runtime>(app: &AppHandle<R>) -> f64 {
    host_settings(app)
        .appearance
        .interface_scale
        .clamp(0.7, 1.6)
}

pub(crate) fn apply_interface_scale_to_window<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    scale: f64,
) {
    let _ = window.set_zoom(scale.clamp(0.7, 1.6));
}

fn apply_interface_scale_to_windows<R: tauri::Runtime>(app: &AppHandle<R>, scale: f64) {
    for window in app.webview_windows().values() {
        apply_interface_scale_to_window(window, scale);
    }
}

/// 轨道内容尺寸（逻辑像素）：窗口收缩到正好包裹胶囊，避免透明死区挡点击
fn rail_content_size<R: tauri::Runtime>(
    app: &AppHandle<R>,
    s: &Settings,
    scale: f64,
) -> (i32, i32) {
    let enabled = rail_metas(app, s)
        .iter()
        .filter(|p| p.enabled && p.sidebar_visible)
        .count()
        + 1; // + gear
    let cw = s.appearance.collapsed_width as f64;
    let ring = (cw * 0.56).round();
    let gap = 14.0;
    let n = enabled as f64;
    let v_h = 24.0 + n * ring + (n - 1.0).max(0.0) * gap; // 纵向内容高（上下 padding 12）
    let h_w = 28.0 + n * ring + (n - 1.0).max(0.0) * gap; // 横向内容宽（左右 padding 14）
    ((h_w * scale).round() as i32, (v_h * scale).round() as i32)
}

/// 侧边栏窗口几何：窗口 = 轨道胶囊外框；收起 = 仅留屏幕边缘 peek 细缝
fn sidebar_geometry<R: tauri::Runtime>(
    app: &AppHandle<R>,
    area: &platform::WorkArea,
    s: &Settings,
    expanded: bool,
    scale: f64,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let a = &s.appearance;
    let cw = (a.collapsed_width as f64 * scale).round() as i32;
    // 收起时保证至少 1px 可悬停
    let peek = ((a.peek_width as f64 * scale).round() as i32).max(1);
    let y_off = (a.sidebar_y_offset as f64 * scale).round() as i32;
    let x_off = (a.sidebar_x_offset as f64 * scale).round() as i32;

    let (content_hw, content_vh) = rail_content_size(app, s, scale);
    // 余量必须 ≥ CSS 侧的 rail 外边距 + 边框，否则顶部模式最右侧图标会被裁掉
    let (win_w, win_h) = match a.sidebar_position.as_str() {
        "top" => (
            content_hw + (14.0 * scale).round() as i32 + 2,
            cw + (8.0 * scale).round() as i32,
        ),
        _ => (
            cw + (8.0 * scale).round() as i32,
            content_vh + (6.0 * scale).round() as i32,
        ),
    };
    let win_w = win_w.max(40);
    let win_h = win_h.max(40).min(area.h - 20);

    let (x, y) = match a.sidebar_position.as_str() {
        "left" => {
            let y = (area.y + (area.h - win_h) / 2 + y_off)
                .max(area.y)
                .min(area.y + area.h - win_h);
            let x = if expanded {
                area.x
            } else {
                area.x - win_w + peek
            };
            (x, y)
        }
        "top" => {
            let x = (area.x + (area.w - win_w) / 2 + x_off)
                .max(area.x)
                .min(area.x + area.w - win_w);
            let y = if expanded {
                area.y
            } else {
                area.y - win_h + peek
            };
            (x, y)
        }
        // right（默认）
        _ => {
            let y = (area.y + (area.h - win_h) / 2 + y_off)
                .max(area.y)
                .min(area.y + area.h - win_h);
            let x = if expanded {
                area.x + area.w - win_w
            } else {
                area.x + area.w - peek
            };
            (x, y)
        }
    };
    (
        PhysicalPosition::new(x, y),
        PhysicalSize::new(win_w as u32, win_h as u32),
    )
}

/// 详情卡几何：停靠在轨道内侧
fn detail_geometry(
    area: &platform::WorkArea,
    s: &Settings,
    scale: f64,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let a = &s.appearance;
    let cw = (a.collapsed_width as f64 * scale).round() as i32;
    let gap = (GAP as f64 * scale).round() as i32;
    let w = ((DETAIL_W as f64 * scale) as i32).min(area.w - gap * 2);
    let h = ((DETAIL_H as f64 * scale) as i32).min(area.h - gap * 2);
    let y_bias = (a.sidebar_y_offset as f64 * scale).round() as i32;
    let (x, y) = match a.sidebar_position.as_str() {
        "left" => (
            area.x + cw + gap,
            area.y + gap + y_bias.clamp(0, area.h - h),
        ),
        "top" => (area.x + (area.w - w) / 2, area.y + cw + gap),
        _ => (
            area.x + area.w - cw - gap - w,
            area.y + gap + y_bias.clamp(0, area.h - h),
        ),
    };
    (
        PhysicalPosition::new(x.max(area.x), y.max(area.y)),
        PhysicalSize::new(w as u32, h as u32),
    )
}

fn sidebar_window<R: tauri::Runtime>(app: &AppHandle<R>) -> Option<tauri::WebviewWindow<R>> {
    app.get_webview_window(SIDEBAR)
}

fn detail_window<R: tauri::Runtime>(app: &AppHandle<R>) -> Option<tauri::WebviewWindow<R>> {
    app.get_webview_window(DETAIL)
}

fn create_detail_window<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<tauri::WebviewWindow<R>, String> {
    let initial_page = host(app)
        .detail_page
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    let initial_script = if initial_page.is_empty() {
        None
    } else {
        let hash = serde_json::to_string(&format!("#/{initial_page}"))
            .map_err(|error| format!("详情页初始路由无效: {error}"))?;
        Some(format!("window.location.hash = {hash};"))
    };
    let route_app = app.clone();
    let mut builder = WebviewWindowBuilder::new(app, DETAIL, WebviewUrl::App("detail.html".into()))
        .title("SideWeave")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .visible(false);
    if let Some(script) = initial_script.as_deref() {
        builder = builder.initialization_script(script);
    }
    let detail = builder
        .on_page_load(move |window, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            let page = route_app
                .state::<Host>()
                .detail_page
                .lock()
                .map(|value| value.clone())
                .unwrap_or_default();
            if page.is_empty() {
                return;
            }
            let Ok(hash) = serde_json::to_string(&format!("#/{page}")) else {
                return;
            };
            let _ = window.eval(&format!("window.location.hash = {hash};"));
        })
        .build()
        .map_err(|error| format!("创建详情 WebView 失败: {error}"))?;
    apply_interface_scale_to_window(&detail, interface_scale(app));

    let h = app.clone();
    detail.on_window_event(move |event| match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            match current_surface(&h) {
                MainSurface::Detail => save_detail_geom(&h, false),
                MainSurface::Control => save_control_geom(&h, false),
                MainSurface::Sidebar => {}
            }
        }
        _ => {}
    });
    Ok(detail)
}

pub(crate) fn ensure_detail_window<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<Option<tauri::WebviewWindow<R>>, String> {
    if let Some(window) = detail_window(app) {
        return Ok(Some(window));
    }
    if host(app).detail_opening.swap(true, Ordering::SeqCst) {
        return Ok(None);
    }
    let result = create_detail_window(app);
    host(app).detail_opening.store(false, Ordering::SeqCst);
    result.map(Some)
}

pub fn prewarm_windows(app: &AppHandle) {
    let host_state = host(app);
    let user_delay = host_state.settings_clone().general.hover_preview_delay;
    let policy = host_state
        .performance_manager
        .effective_policy(Some(user_delay));

    if policy.detail_prewarm {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _ = ensure_detail_window(&handle);
        });
    }

    if policy.preview_prewarm {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _ = preview_window::ensure_preview_window(&handle);
        });
    }
}

fn persist_settings<R: tauri::Runtime>(
    app: &AppHandle<R>,
    settings: &Settings,
) -> Result<(), String> {
    let value = settings::to_persisted(settings)?;
    paths::save_json_atomic(&paths::state_file(&paths::app_data_dir(app)), &value)
        .map_err(|error| format!("保存设置失败: {error}"))
}

pub(crate) fn with_detail_not_always_on_top<R: tauri::Runtime, T>(
    app: &AppHandle<R>,
    action: impl FnOnce() -> T,
) -> T {
    let detail = detail_window(app);
    let restore_top = current_surface(app) != MainSurface::Control;
    let detail_visible = detail
        .as_ref()
        .is_some_and(|window| window.is_visible().unwrap_or(false));
    if detail_visible && restore_top {
        if let Some(window) = detail.as_ref() {
            let _ = window.set_always_on_top(false);
            let _ = window.set_focus();
        }
    }
    let result = action();
    if detail_visible && restore_top {
        if let Some(window) = detail.as_ref() {
            let _ = window.set_always_on_top(true);
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
    result
}

fn apply_sidebar<R: tauri::Runtime>(app: &AppHandle<R>) {
    let Some(win) = sidebar_window(app) else {
        return;
    };
    let s = host(app).settings_clone();
    let expanded = expanded_flag(app);
    let area = platform::work_area();
    let scale = monitor_scale(app) * s.appearance.interface_scale.clamp(0.7, 1.6);
    let (pos, size) = sidebar_geometry(app, &area, &s, expanded, scale);
    let _ = win.set_position(pos);
    let _ = win.set_size(size);
    // 侧边栏的尺寸完全由 sidebar_geometry 控制。禁用原生缩放边框，避免
    // 收起态的边缘 peek 被 Windows 命中为“拉伸窗口”，从而吞掉 mouseenter。
    let _ = win.set_resizable(false);
    let _ = win.set_always_on_top(true);
    // 悬停展开开启时必须保留收起态的边缘命中区域，否则 Windows 不会产生 mouseenter。
    let clickthrough = !expanded && !s.general.enable_mouse_hover;
    let _ = win.set_ignore_cursor_events(clickthrough);
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        let _ = platform::set_clickthrough(hwnd.0, clickthrough);
    }
}

pub(crate) fn expanded_flag<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    host(app).expanded.load(Ordering::SeqCst)
}

pub(crate) fn host<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::State<'_, Host> {
    app.state::<Host>()
}

fn current_surface<R: tauri::Runtime>(app: &AppHandle<R>) -> MainSurface {
    *host(app).main_surface.lock().unwrap()
}

fn surface_name(surface: MainSurface) -> &'static str {
    match surface {
        MainSurface::Sidebar => "sidebar",
        MainSurface::Detail => "detail",
        MainSurface::Control => "control",
    }
}

fn set_main_surface<R: tauri::Runtime>(app: &AppHandle<R>, surface: MainSurface) {
    *host(app).main_surface.lock().unwrap() = surface;
    let _ = app.emit_to(DETAIL, "main://surface", surface_name(surface));
}

fn close_main_surface<R: tauri::Runtime>(app: &AppHandle<R>) {
    let was_control_center = current_surface(app) == MainSurface::Control;
    match current_surface(app) {
        MainSurface::Detail => save_detail_geom(app, true),
        MainSurface::Control => save_control_geom(app, true),
        MainSurface::Sidebar => {}
    }
    host(app).detail_compact.store(false, Ordering::SeqCst);
    let _ = app.emit_to(DETAIL, "detail://compact", false);
    set_main_surface(app, MainSurface::Sidebar);
    if let Some(window) = detail_window(app) {
        let _ = window.hide();
    }
    apply_sidebar(app);
    if let Some(window) = sidebar_window(app) {
        let _ = window.set_resizable(false);
        let _ = window.set_always_on_top(true);
        let _ = window.show();
        let _ = window.unminimize();
    }
    if was_control_center {
        schedule_sidebar_collapse_after_control(app);
    }
    schedule_detail_delayed_destroy(app);
}

/// Closing the control center can leave the sidebar expanded without producing a
/// mouseleave event. Recheck the pointer after the configured auto-hide delay.
fn schedule_sidebar_collapse_after_control<R: tauri::Runtime>(app: &AppHandle<R>) {
    let settings = host(app).settings_clone();
    if !settings.general.enable_mouse_hover {
        return;
    }

    let delay = settings.general.auto_hide_delay as u64;
    let hover_token = *host(app).hover_token.lock().unwrap();
    let collapse_gen = host(app).collapse_gen.load(Ordering::SeqCst);
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(delay));
        let main_handle = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            let state = host(&main_handle);
            if current_surface(&main_handle) != MainSurface::Sidebar
                || !expanded_flag(&main_handle)
                || state.collapse_gen.load(Ordering::SeqCst) != collapse_gen
                || *state.hover_token.lock().unwrap() != hover_token
                || !state.settings_clone().general.enable_mouse_hover
            {
                return;
            }
            if let Some(coord) = main_handle.try_state::<Arc<preview_window::PreviewCoordinator>>()
            {
                if coord.is_active() {
                    return;
                }
            }

            let cursor_over_sidebar = sidebar_window(&main_handle)
                .and_then(|window| {
                    let cursor = window.cursor_position().ok()?;
                    let position = window.outer_position().ok()?;
                    let size = window.outer_size().ok()?;
                    Some(
                        cursor.x >= f64::from(position.x)
                            && cursor.y >= f64::from(position.y)
                            && cursor.x < f64::from(position.x) + f64::from(size.width)
                            && cursor.y < f64::from(position.y) + f64::from(size.height),
                    )
                })
                .unwrap_or(false);
            if !cursor_over_sidebar {
                collapse(&main_handle);
            }
        });
    });
}

/// 详情页关闭后延迟 30 秒销毁 WebView；30 秒内若重新打开则取消销毁，以兼顾快速复用与内存回收
fn schedule_detail_delayed_destroy<R: tauri::Runtime>(app: &AppHandle<R>) {
    let policy = host(app)
        .performance_manager
        .effective_policy(Some(host(app).settings_clone().general.hover_preview_delay));
    if policy.detail_keep_alive {
        return;
    }

    let gen = host(app).detail_destroy_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(30));
        if host(&handle).detail_destroy_gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let main_handle = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if host(&main_handle).detail_destroy_gen.load(Ordering::SeqCst) != gen {
                return;
            }
            if current_surface(&main_handle) != MainSurface::Sidebar {
                return;
            }
            let policy = host(&main_handle)
                .performance_manager
                .effective_policy(Some(
                    host(&main_handle)
                        .settings_clone()
                        .general
                        .hover_preview_delay,
                ));
            if policy.detail_keep_alive {
                return;
            }
            if let Some(window) = detail_window(&main_handle) {
                if !window.is_visible().unwrap_or(false) {
                    let _ = window.destroy();
                }
            }
        });
    });
}

fn expand<R: tauri::Runtime>(app: &AppHandle<R>, focus: bool) {
    host(app).collapse_gen.fetch_add(1, Ordering::SeqCst);
    host(app).expanded.store(true, Ordering::SeqCst);
    apply_sidebar(app);
    if let Some(win) = sidebar_window(app) {
        let _ = win.show();
        let _ = win.unminimize();
        if focus {
            let _ = win.set_focus();
        }
    }
    let _ = app.emit_to(SIDEBAR, "sidebar://expanded", true);
}

pub(crate) fn collapse<R: tauri::Runtime>(app: &AppHandle<R>) {
    host(app).collapse_gen.fetch_add(1, Ordering::SeqCst);
    host(app).expanded.store(false, Ordering::SeqCst);
    // 先让前端开始收起，再同步移动原生窗口，避免瞬移打断动画
    let _ = app.emit_to(SIDEBAR, "sidebar://expanded", false);
    let gen = host(app).collapse_gen.load(Ordering::SeqCst);
    animate_sidebar_collapse(app, gen);
}

/// Move the native window with the CSS collapse transition so the edge slit
/// arrives at the screen edge continuously instead of jumping there afterward.
fn animate_sidebar_collapse<R: tauri::Runtime>(app: &AppHandle<R>, generation: u64) {
    let Some(window) = sidebar_window(app) else {
        return;
    };
    let Ok(start) = window.outer_position() else {
        apply_sidebar(app);
        return;
    };

    let settings = host(app).settings_clone();
    let area = platform::work_area();
    let scale = monitor_scale(app) * settings.appearance.interface_scale.clamp(0.7, 1.6);
    let (target, _) = sidebar_geometry(app, &area, &settings, false, scale);
    let handle = app.clone();

    std::thread::spawn(move || {
        let started = Instant::now();
        loop {
            let progress = (started.elapsed().as_secs_f64()
                / SIDEBAR_COLLAPSE_DURATION.as_secs_f64())
            .clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - progress).powi(3);
            let position = PhysicalPosition::new(
                (f64::from(start.x) + f64::from(target.x - start.x) * eased).round() as i32,
                (f64::from(start.y) + f64::from(target.y - start.y) * eased).round() as i32,
            );
            let finished = progress >= 1.0;
            let main_handle = handle.clone();
            if handle
                .run_on_main_thread(move || {
                    if host(&main_handle).collapse_gen.load(Ordering::SeqCst) != generation
                        || expanded_flag(&main_handle)
                    {
                        return;
                    }
                    if finished {
                        apply_sidebar(&main_handle);
                    } else if let Some(window) = sidebar_window(&main_handle) {
                        let _ = window.set_position(position);
                    }
                })
                .is_err()
            {
                return;
            }
            if finished {
                break;
            }
            std::thread::sleep(SIDEBAR_COLLAPSE_FRAME);
        }
    });
}

fn toggle<R: tauri::Runtime>(app: &AppHandle<R>) {
    if current_surface(app) != MainSurface::Sidebar {
        close_main_surface(app);
    }
    if expanded_flag(app) {
        collapse(app);
    } else {
        expand(app, true);
    }
}

/// 详情卡位置/尺寸记忆（跨会话）：detail_geom.json（物理像素）。
/// v2 改名：旧 detail_window.json 会被启动期创建事件覆盖成默认值，弃用以免读到脏数据。
fn detail_geom_path<R: tauri::Runtime>(app: &AppHandle<R>) -> PathBuf {
    paths::app_data_dir(app).join("detail_geom.json")
}

fn load_detail_geom<R: tauri::Runtime>(app: &AppHandle<R>) -> Option<(i32, i32, u32, u32)> {
    let v = paths::load_json(&detail_geom_path(app))?;
    Some((
        v.get("x")?.as_i64()? as i32,
        v.get("y")?.as_i64()? as i32,
        v.get("w")?.as_i64()? as u32,
        v.get("h")?.as_i64()? as u32,
    ))
}

static DETAIL_GEOM_FLUSH: AtomicU64 = AtomicU64::new(0);

/// 保存详情卡当前位置/尺寸。拖拽/缩放事件高频触发，节流 500ms；force=true 跳过节流（隐藏前落盘）。
/// 仅在窗口可见时保存：启动创建窗口（隐藏）阶段也会触发 Moved/Resized，
/// 不拦截会把上次会话记忆的几何覆盖成创建默认值（表现为重启后详情卡大小重置）。
fn save_detail_geom<R: tauri::Runtime>(app: &AppHandle<R>, force: bool) {
    if current_surface(app) != MainSurface::Detail
        || host(app).geometry_transition.load(Ordering::SeqCst)
    {
        return;
    }
    let Some(win) = detail_window(app) else {
        return;
    };
    if host(app).detail_compact.load(Ordering::SeqCst)
        || !win.is_visible().unwrap_or(false)
        || win.is_minimized().unwrap_or(false)
    {
        return;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if !force {
        let last = DETAIL_GEOM_FLUSH.load(Ordering::SeqCst);
        if now_ms.saturating_sub(last) < 500 {
            return;
        }
        DETAIL_GEOM_FLUSH.store(now_ms, Ordering::SeqCst);
    }
    let Ok(pos) = win.outer_position() else {
        return;
    };
    let Ok(size) = win.outer_size() else { return };
    let v = serde_json::json!({ "x": pos.x, "y": pos.y, "w": size.width, "h": size.height });
    let _ = paths::save_json_atomic(&detail_geom_path(app), &v);
}

/// 在完整详情页和仅保留标题栏的紧凑态之间切换。
/// 紧凑态不会写入几何记忆，展开时恢复最近一次完整窗口的位置和尺寸。
fn set_detail_compact<R: tauri::Runtime>(
    app: &AppHandle<R>,
    compact: bool,
) -> Result<bool, String> {
    if current_surface(app) != MainSurface::Detail {
        return Err("详情页未打开".to_string());
    }
    let win = detail_window(app).ok_or_else(|| "详情窗口不存在".to_string())?;
    let current = host(app).detail_compact.load(Ordering::SeqCst);
    if current == compact {
        return Ok(compact);
    }

    if compact {
        save_detail_geom(app, true);
        host(app).detail_compact.store(true, Ordering::SeqCst);
        let current_size = win.outer_size().map_err(|error| error.to_string())?;
        let scale = monitor_scale(app) * interface_scale(app);
        let height = (DETAIL_COMPACT_H * scale).round().max(40.0) as u32;
        win.set_size(PhysicalSize::new(current_size.width, height))
            .map_err(|error| format!("详情窗口进入紧凑态失败: {error}"))?;
    } else {
        let geometry = load_detail_geom(app).or_else(|| {
            let settings = host(app).settings_clone();
            let area = platform::work_area();
            let scale = monitor_scale(app) * settings.appearance.interface_scale.clamp(0.7, 1.6);
            let (position, size) = detail_geometry(&area, &settings, scale);
            Some((position.x, position.y, size.width, size.height))
        });
        let Some((x, y, width, height)) = geometry else {
            return Err("详情窗口完整尺寸不可用".to_string());
        };
        win.set_position(PhysicalPosition::new(x, y))
            .map_err(|error| format!("详情窗口恢复位置失败: {error}"))?;
        win.set_size(PhysicalSize::new(width, height))
            .map_err(|error| format!("详情窗口恢复尺寸失败: {error}"))?;
        host(app).detail_compact.store(false, Ordering::SeqCst);
        save_detail_geom(app, true);
    }

    let _ = app.emit("detail://compact", compact);
    Ok(compact)
}

fn show_detail<R: tauri::Runtime>(app: &AppHandle<R>, page: &str) -> Result<(), String> {
    if current_surface(app) == MainSurface::Detail
        && detail_window(app)
            .as_ref()
            .is_some_and(|window| window.is_visible().unwrap_or(false))
        && host(app).detail_page.lock().unwrap().as_str() == page
    {
        close_main_surface(app);
        return Ok(());
    }

    host(app).detail_destroy_gen.fetch_add(1, Ordering::SeqCst);

    if current_surface(app) == MainSurface::Detail {
        save_detail_geom(app, true);
    } else if current_surface(app) == MainSurface::Control {
        save_control_geom(app, true);
    }
    expand(app, false);
    *host(app).detail_page.lock().unwrap() = page.to_string();
    set_main_surface(app, MainSurface::Detail);
    let window = match ensure_detail_window(app)? {
        Some(window) => window,
        None => return Ok(()),
    };
    let geometry = load_detail_geom(app).or_else(|| {
        let settings = host(app).settings_clone();
        let area = platform::work_area();
        let scale = monitor_scale(app) * settings.appearance.interface_scale.clamp(0.7, 1.6);
        let (position, size) = detail_geometry(&area, &settings, scale);
        Some((position.x, position.y, size.width, size.height))
    });
    let Some((x, y, width, height)) = geometry else {
        return Err("详情页尺寸不可用".to_string());
    };
    host(app).detail_compact.store(false, Ordering::SeqCst);
    let _ = window.set_resizable(true);
    let _ = window.set_ignore_cursor_events(false);
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        let _ = platform::set_clickthrough(hwnd.0, false);
    }
    host(app).geometry_transition.store(true, Ordering::SeqCst);
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| format!("详情页位置设置失败: {error}"))?;
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|error| format!("详情页尺寸设置失败: {error}"))?;
    host(app).geometry_transition.store(false, Ordering::SeqCst);
    save_detail_geom(app, true);
    let hash = serde_json::to_string(&format!("#/{page}"))
        .map_err(|error| format!("详情页路由无效: {error}"))?;
    let _ = window.eval(&format!("window.location.hash = {hash};"));
    let _ = window.set_always_on_top(true);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

#[tauri::command]
fn detail_content_ready(app: AppHandle, plugin_id: String) {
    let _ = (app, plugin_id);
}

// ---------- 托盘 ----------

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let item_toggle = MenuItem::with_id(app, "toggle", "显示 / 隐藏", true, None::<&str>)?;
    let item_control = MenuItem::with_id(app, "control", "控制中心", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let item_restart = MenuItem::with_id(app, "restart", "重启", true, None::<&str>)?;
    let item_quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&item_toggle, &item_control, &sep, &item_restart, &item_quit],
    )?;

    let icon = tauri::include_image!("icons/icon.ico");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("SideWeave")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle(app),
            "control" => {
                let h = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = open_control_center(h).await;
                });
            }
            "restart" => restart_app_impl(app),
            "quit" => quit_app_impl(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击 = 切换侧边栏；去抖 260ms 让双击不会闪两下（快速两次点击只算一次切换）
            if let TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                static SEQ: AtomicU64 = AtomicU64::new(0);
                let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
                let handle = tray.app_handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(260));
                    if SEQ.load(Ordering::SeqCst) == seq {
                        toggle(&handle);
                    }
                });
            }
        })
        .build(app)?;
    Ok(())
}

// ---------- 命令 ----------

#[derive(Serialize)]
struct MonitorInfo {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    scale: f64,
}

#[derive(Serialize)]
struct SystemInfo {
    app_version: String,
    os_name: String,
    arch: String,
    uptime_secs: u64,
    work_area: MonitorInfo,
    monitors: Vec<MonitorInfo>,
    plugins_total: usize,
    plugins_enabled: usize,
}

static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

#[cfg(debug_assertions)]
fn startup_log(stage: impl std::fmt::Display) {
    let elapsed = STARTED
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64();
    eprintln!(
        "[startup +{elapsed:7.3}s {:?}] {stage}",
        std::thread::current().id()
    );
}

#[cfg(not(debug_assertions))]
fn startup_log(_stage: impl std::fmt::Display) {}

#[tauri::command]
fn get_system_info(app: AppHandle) -> SystemInfo {
    fn mon(x: i32, y: i32, w: u32, h: u32, scale: f64) -> MonitorInfo {
        MonitorInfo { x, y, w, h, scale }
    }
    let area = platform::work_area();
    let scale = monitor_scale(&app);
    let monitors = app
        .available_monitors()
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    mon(
                        m.position().x,
                        m.position().y,
                        m.size().width,
                        m.size().height,
                        m.scale_factor(),
                    )
                })
                .collect()
        })
        .unwrap_or_else(|_| vec![mon(area.x, area.y, area.w as u32, area.h as u32, scale)]);
    let s = host(&app).settings_clone();
    let meta = rail_metas(&app, &s);
    SystemInfo {
        app_version: app.package_info().version.to_string(),
        os_name: if cfg!(target_os = "windows") {
            "Microsoft Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        }
        .into(),
        arch: std::env::consts::ARCH.into(),
        uptime_secs: STARTED
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_secs(),
        work_area: mon(area.x, area.y, area.w as u32, area.h as u32, scale),
        monitors,
        plugins_total: meta.len(),
        plugins_enabled: meta.iter().filter(|p| p.enabled).count(),
    }
}

#[tauri::command]
fn get_state(app: AppHandle) -> StateSnapshot {
    startup_log("command get_state: begin");
    let s = host(&app).settings_clone();
    startup_log("command get_state: settings cloned");
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    startup_log("command get_state: autostart queried");
    let plugin_metas = rail_metas(&app, &s);
    startup_log(format!(
        "command get_state: plugin metadata collected ({})",
        plugin_metas.len()
    ));
    StateSnapshot {
        settings: s.clone(),
        plugins: plugin_metas,
        autostart_enabled: autostart,
        app_version: app.package_info().version.to_string(),
        data_dir: paths::app_data_dir(&app).to_string_lossy().into_owned(),
    }
}

#[tauri::command]
fn startup_frontend_log(stage: String) {
    let stage = stage
        .chars()
        .filter(|character| !character.is_control())
        .take(120)
        .collect::<String>();
    startup_log(format!("frontend: {stage}"));
}

#[tauri::command]
fn set_setting(
    app: AppHandle,
    category: String,
    key: String,
    value: serde_json::Value,
) -> Result<Settings, String> {
    // 热键变更：先校验新值可解析，避免"已保存但注册失败"的脏状态
    let is_shortcut_key =
        category == "shortcuts" && (key == "toggle_sidebar" || key == "control_center");
    if is_shortcut_key {
        let s_new = value.as_str().unwrap_or_default();
        if !s_new.is_empty() {
            let _: Shortcut = s_new
                .parse()
                .map_err(|e| format!("无效热键「{s_new}」: {e}"))?;
        }
    }

    let old_shortcut = if is_shortcut_key {
        Some(match key.as_str() {
            "control_center" => host(&app).settings_clone().shortcuts.control_center,
            _ => host(&app).settings_clone().shortcuts.toggle_sidebar,
        })
    } else {
        None
    };

    let changed = {
        let state = host(&app);
        let mut s = state.settings.lock().unwrap();
        settings::apply_setting(&mut s, &category, &key, &value)?;

        // 副作用：开机自启
        if category == "general" && key == "run_on_startup" {
            let auto = app.autolaunch();
            if s.general.run_on_startup {
                let _ = auto.enable();
            } else {
                let _ = auto.disable();
            }
        }
        // 持锁落盘，避免并发写盘乱序
        let snapshot = s.clone();
        persist_settings(&app, &snapshot)?;
        snapshot
    };

    // 热键重注册（新值已在上方预校验；空串 = 不启用）
    if let Some(old) = old_shortcut {
        let gs = app.global_shortcut();
        if let Ok(old_sc) = old.parse::<Shortcut>() {
            let _ = gs.unregister(old_sc);
        }
        let new_key = match key.as_str() {
            "control_center" => changed.shortcuts.control_center.clone(),
            _ => changed.shortcuts.toggle_sidebar.clone(),
        };
        if new_key.is_empty() {
            let snapshot = changed;
            let _ = app.emit("settings://changed", &snapshot);
            apply_sidebar(&app);
            return Ok(snapshot);
        }
        match new_key.parse::<Shortcut>() {
            Ok(sc) => {
                if let Err(e) = gs.register(sc) {
                    eprintln!("[agile-tiles] 注册新热键失败: {e}");
                    return Err(format!("热键注册失败（可能被其他程序占用）: {e}"));
                }
            }
            Err(e) => return Err(format!("无效热键: {e}")),
        }
    }

    // 界面缩放同步应用到所有已打开的 WebView。
    if category == "appearance" && key == "interface_scale" {
        apply_interface_scale_to_windows(&app, changed.appearance.interface_scale);
    }

    // 布局和收起态交互设置即时生效
    if category == "appearance" || (category == "general" && key == "enable_mouse_hover") {
        apply_sidebar(&app);
    }

    // 运行模式热切换
    if category == "general" && key == "performance_profile" {
        if let Some(prof) =
            performance::PerformanceProfile::parse(&changed.general.performance_profile)
        {
            host(&app).performance_manager.set_profile(prof);
            let user_delay = changed.general.hover_preview_delay;
            let policy = host(&app)
                .performance_manager
                .effective_policy(Some(user_delay));
            if policy.detail_prewarm || policy.preview_prewarm {
                prewarm_windows(&app);
            }
        }
    }

    if category == "notifications"
        && [
            "render_mode",
            "background_opacity",
            "desktop_background_color",
            "bubble_background_color",
        ]
        .contains(&key.as_str())
    {
        if let Err(error) = app
            .state::<desktop_notifications::DesktopNotificationManager>()
            .refresh_all(app.clone())
        {
            eprintln!("[agile-tiles] failed to refresh notification renderer: {error}");
        }
    }

    let _ = app.emit("settings://changed", &changed);
    Ok(changed)
}

#[tauri::command]
fn set_skin_appearance(
    app: AppHandle,
    skin_id: String,
    opacity_defaults: market::AppearanceOpacityDefaults,
) -> Result<Settings, String> {
    let opacity_values = [
        (
            "sidebar_panel_opacity",
            opacity_defaults.sidebar_panel_opacity,
        ),
        (
            "sidebar_image_opacity",
            opacity_defaults.sidebar_image_opacity,
        ),
        (
            "detail_panel_opacity",
            opacity_defaults.detail_panel_opacity,
        ),
        (
            "control_panel_opacity",
            opacity_defaults.control_panel_opacity,
        ),
        (
            "detail_image_opacity",
            opacity_defaults.detail_image_opacity,
        ),
        ("component_opacity", opacity_defaults.component_opacity),
    ];
    let changed = {
        let state = host(&app);
        let mut settings = state.settings.lock().unwrap();
        settings::apply_setting(
            &mut settings,
            "appearance",
            "skin",
            &serde_json::Value::String(skin_id),
        )?;
        for (key, value) in opacity_values {
            if let Some(value) = value {
                settings::apply_setting(
                    &mut settings,
                    "appearance",
                    key,
                    &serde_json::json!(value),
                )?;
            }
        }
        let snapshot = settings.clone();
        persist_settings(&app, &snapshot)?;
        snapshot
    };
    apply_sidebar(&app);
    let _ = app.emit("settings://changed", &changed);
    Ok(changed)
}

#[tauri::command]
fn get_performance_status(app: AppHandle) -> performance::PerformanceStatusPayload {
    let user_delay = host(&app).settings_clone().general.hover_preview_delay;
    host(&app)
        .performance_manager
        .status_payload(Some(user_delay))
}

#[tauri::command]
fn set_plugin_enabled(
    app: AppHandle,
    plugin_id: String,
    enabled: bool,
) -> Result<Vec<PluginMeta>, String> {
    if !host_plugin_registered(&app, &plugin_id) {
        return Err(format!("未知插件: {plugin_id}"));
    }
    let s = {
        let state = host(&app);
        let mut s = state.settings.lock().unwrap();
        s.plugins.disabled.retain(|d| d != &plugin_id);
        if !enabled {
            s.plugins.disabled.push(plugin_id);
            s.plugins.disabled.sort();
        }
        let snapshot = s.clone();
        persist_settings(&app, &snapshot)?;
        snapshot
    };
    let _ = app.emit("settings://changed", &s);
    apply_sidebar(&app);
    Ok(rail_metas(&app, &s))
}

#[tauri::command]
fn set_plugin_sidebar_visible(
    app: AppHandle,
    plugin_id: String,
    visible: bool,
) -> Result<Vec<PluginMeta>, String> {
    if !host_plugin_registered(&app, &plugin_id) {
        return Err(format!("Unknown plugin: {plugin_id}"));
    }
    let s = {
        let state = host(&app);
        let mut s = state.settings.lock().unwrap();
        s.plugins
            .hidden_from_sidebar
            .retain(|hidden| hidden != &plugin_id);
        if !visible {
            s.plugins.hidden_from_sidebar.push(plugin_id);
            s.plugins.hidden_from_sidebar.sort();
        }
        let snapshot = s.clone();
        persist_settings(&app, &snapshot)?;
        snapshot
    };
    let _ = app.emit("settings://changed", &s);
    apply_sidebar(&app);
    Ok(rail_metas(&app, &s))
}

#[tauri::command]
fn set_plugin_order(app: AppHandle, order: Vec<String>) -> Result<Vec<PluginMeta>, String> {
    for id in &order {
        if !host_plugin_registered(&app, id) {
            return Err(format!("未知插件: {id}"));
        }
    }
    let s = {
        let state = host(&app);
        let mut s = state.settings.lock().unwrap();
        s.plugins.order = order;
        let snapshot = s.clone();
        persist_settings(&app, &snapshot)?;
        snapshot
    };
    let _ = app.emit("settings://changed", &s);
    apply_sidebar(&app);
    Ok(rail_metas(&app, &s))
}

#[tauri::command]
async fn open_control_center(app: AppHandle) -> Result<(), String> {
    open_control_center_sync(&app)
}

/// 同步版：供全局热键 handler（非 async 上下文）调用
fn open_control_center_sync<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    host(app).detail_destroy_gen.fetch_add(1, Ordering::SeqCst);
    if current_surface(app) == MainSurface::Control {
        if let Some(window) = detail_window(app) {
            let _ = window.show();
            let _ = window.set_focus();
        }
        return Ok(());
    }
    if current_surface(app) == MainSurface::Detail {
        save_detail_geom(app, true);
        host(app).detail_compact.store(false, Ordering::SeqCst);
        let _ = app.emit_to(DETAIL, "detail://compact", false);
    }
    expand(app, false);
    set_main_surface(app, MainSurface::Control);
    let window = match ensure_detail_window(app)? {
        Some(window) => window,
        None => return Ok(()),
    };
    let geometry = load_control_geom(app).or_else(|| {
        let area = platform::work_area();
        let scale = monitor_scale(app) * interface_scale(app);
        let width = (1040.0 * scale).round() as u32;
        let height = (780.0 * scale).round() as u32;
        let width = width.min(area.w.max(32) as u32 - 16);
        let height = height.min(area.h.max(32) as u32 - 16);
        Some((
            area.x + (area.w - width as i32) / 2,
            area.y + (area.h - height as i32) / 2,
            width,
            height,
        ))
    });
    let Some((x, y, width, height)) = geometry else {
        return Err("控制中心尺寸不可用".to_string());
    };
    set_main_surface(app, MainSurface::Control);
    let _ = window.set_resizable(true);
    let _ = window.set_ignore_cursor_events(false);
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        let _ = platform::set_clickthrough(hwnd.0, false);
    }
    let _ = window.set_always_on_top(false);
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| format!("控制中心位置设置失败: {error}"))?;
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|error| format!("控制中心尺寸设置失败: {error}"))?;
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

#[cfg(any())]
fn open_control_center_sync_legacy<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("control") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }
    // 独立宽窗口：原生标题栏（拖拽/缩放/关闭齐全），非置顶，进任务栏；
    // 位置/尺寸跨会话记忆（control_window.json，无记忆回落默认居中）
    let builder = WebviewWindowBuilder::new(app, "control", WebviewUrl::App("control.html".into()))
        .title("SideWeave 控制中心")
        .inner_size(1040.0, 780.0)
        .min_inner_size(880.0, 640.0)
        .resizable(true)
        .visible(false);
    let win = match load_control_geom(app) {
        Some((x, y, w, h)) => {
            let scale = crate::monitor_scale(app);
            builder
                // 记忆的是物理像素，而 builder.position/inner_size 吃逻辑像素：
                // 不除缩放的话，高 DPI 屏上每次重启都会放大 scale 倍（表现为窗口越来越大）
                .position(f64::from(x) / scale, f64::from(y) / scale)
                .inner_size(f64::from(w) / scale, f64::from(h) / scale)
                .build()
                .map_err(|e| e.to_string())?
        }
        None => builder.center().build().map_err(|e| e.to_string())?,
    };
    {
        let h = app.clone();
        win.on_window_event(move |e| match e {
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                save_control_geom(&h);
            }
            _ => {}
        });
    }
    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

fn control_geom_path<R: tauri::Runtime>(app: &AppHandle<R>) -> PathBuf {
    paths::app_data_dir(app).join("control_window.json")
}

fn load_control_geom<R: tauri::Runtime>(app: &AppHandle<R>) -> Option<(i32, i32, u32, u32)> {
    let v = paths::load_json(&control_geom_path(app))?;
    Some((
        v.get("x")?.as_i64()? as i32,
        v.get("y")?.as_i64()? as i32,
        v.get("w")?.as_i64()? as u32,
        v.get("h")?.as_i64()? as u32,
    ))
}

static CONTROL_GEOM_FLUSH: AtomicU64 = AtomicU64::new(0);

fn save_control_geom<R: tauri::Runtime>(app: &AppHandle<R>, force: bool) {
    if current_surface(app) != MainSurface::Control
        || host(app).geometry_transition.load(Ordering::SeqCst)
    {
        return;
    }
    let Some(win) = detail_window(app) else {
        return;
    };
    // 最大化或创建期（不可见）不写记忆：避免把整屏尺寸/初始默认值当成用户设置
    if win.is_maximized().unwrap_or(false) || !win.is_visible().unwrap_or(false) {
        return;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let last = CONTROL_GEOM_FLUSH.load(Ordering::SeqCst);
    if !force && now_ms.saturating_sub(last) < 500 {
        return;
    }
    CONTROL_GEOM_FLUSH.store(now_ms, Ordering::SeqCst);
    let Ok(pos) = win.outer_position() else {
        return;
    };
    let Ok(size) = win.outer_size() else { return };
    let scale = monitor_scale(app);
    if size.width < (880.0 * scale).round() as u32 || size.height < (640.0 * scale).round() as u32 {
        return;
    }
    let v = serde_json::json!({ "x": pos.x, "y": pos.y, "w": size.width, "h": size.height });
    let _ = paths::save_json_atomic(&control_geom_path(app), &v);
}

#[tauri::command]
fn get_main_surface(app: AppHandle) -> &'static str {
    surface_name(current_surface(&app))
}

#[tauri::command]
async fn open_detail(app: AppHandle, page: String) -> Result<(), String> {
    // 点击当前已打开的插件 → 关闭详情页；点击其他插件 → 正常切换
    show_detail(&app, &page)
}

#[tauri::command]
fn close_detail(app: AppHandle) {
    close_main_surface(&app);
    // 隐藏前强制落盘最后一次几何（绕过 500ms 节流，避免末次调整丢失）
}

#[tauri::command]
fn minimize_detail(app: AppHandle) -> Result<bool, String> {
    let compact = !host(&app).detail_compact.load(Ordering::SeqCst);
    set_detail_compact(&app, compact)
}

#[tauri::command]
fn toggle_sidebar(app: AppHandle) {
    toggle(&app);
}

#[tauri::command]
fn sidebar_hover_enter(app: AppHandle) -> u64 {
    let state = host(&app);
    let mut token = state.hover_token.lock().unwrap();
    *token += 1;
    let t = *token;
    drop(token);
    expand(&app, false);
    t
}

#[tauri::command]
fn sidebar_collapse_if_unhovered(app: AppHandle, token: u64) {
    let current = *host(&app).hover_token.lock().unwrap();
    if current == token {
        let delay = host(&app).settings_clone().general.auto_hide_delay as u64;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(delay));
            // 双检：延迟期间令牌可能已被新的 hover_enter 递增。
            // 详情卡与侧边栏已解耦，即使详情卡开着也正常收起轨道。
            if *host(&app).hover_token.lock().unwrap() == token {
                if let Some(coord) = app.try_state::<Arc<preview_window::PreviewCoordinator>>() {
                    if coord.is_active() {
                        return;
                    }
                }
                collapse(&app);
            }
        });
    }
}

#[tauri::command]
fn open_data_dir(app: AppHandle) {
    let dir = paths::app_data_dir(&app);
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&dir).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
}

#[tauri::command]
fn restart_app(app: AppHandle) {
    restart_app_impl(&app)
}

/// 重启应用。开发模式下 app.restart() 会直接拉起裸 exe，而 devUrl（Vite）随
/// `tauri dev` 主进程一起结束，重启后所有窗口都会"无法访问此页面"——
/// 因此开发模式改为整体刷新前端（Rust 代码变更由 tauri dev 的文件监听自动重启）。
fn restart_app_impl<R: tauri::Runtime>(app: &AppHandle<R>) {
    #[cfg(debug_assertions)]
    {
        for win in app.webview_windows().values() {
            let _ = win.eval("window.location.reload()");
        }
    }
    #[cfg(not(debug_assertions))]
    app.restart();
}

fn quit_app_impl<R: tauri::Runtime>(app: &AppHandle<R>) {
    app.exit(0);
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1000));
        std::process::exit(0);
    });
}

#[tauri::command]
fn plugin_registry_list(app: AppHandle) -> Vec<PluginInfo> {
    host(&app)
        .plugin_manager
        .lock()
        .unwrap()
        .as_ref()
        .map(|manager| {
            manager
                .registry()
                .list()
                .into_iter()
                .map(|mut info| {
                    let id = info.manifest.id.to_string();
                    info.manifest.icon = plugin_icon_source(&id, &info.manifest.icon);
                    info
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
fn set_plugin_permission(
    app: AppHandle,
    window: tauri::WebviewWindow,
    plugin_id: String,
    permission: String,
    granted: bool,
) -> Result<Settings, String> {
    if window.label() != DETAIL {
        return Err("插件权限只能从宿主设置页修改".into());
    }
    let parsed_id = agile_tiles_plugin_api::PluginId::new(plugin_id.clone())
        .map_err(|error| error.to_string())?;
    if !agile_tiles_plugin_api::permissions::is_known(&permission) {
        return Err(format!("未知插件权限：{permission}"));
    }
    let requested = {
        let state = host(&app);
        let managers = state.plugin_manager.lock().unwrap();
        let manager = managers
            .as_ref()
            .ok_or_else(|| "插件运行时尚未初始化".to_string())?;
        manager
            .registry()
            .get(&parsed_id)
            .map(|info| info.manifest.permissions.clone())
            .ok_or_else(|| format!("插件不存在：{plugin_id}"))?
    };
    if !agile_tiles_plugin_api::permissions::is_declared(&requested, &permission) {
        return Err(format!("插件未声明此权限：{permission}"));
    }

    let changed = {
        let state = host(&app);
        let mut settings = state.settings.lock().unwrap();
        settings
            .plugins
            .permission_grants
            .entry(plugin_id)
            .or_default()
            .insert(permission, granted);
        let snapshot = settings.clone();
        persist_settings(&app, &snapshot)?;
        snapshot
    };
    let _ = app.emit("settings://changed", &changed);
    Ok(changed)
}

#[tauri::command]
fn plugin_handshake(app: AppHandle, handshake: PluginHandshake) -> PluginHandshakeResponse {
    let fallback_request_id = handshake.request_id.clone();
    let fallback_plugin_id = handshake.plugin_id.clone();
    host(&app)
        .plugin_manager
        .lock()
        .unwrap()
        .as_ref()
        .map(|manager| manager.handshake(handshake))
        .unwrap_or_else(|| PluginHandshakeResponse {
            request_id: fallback_request_id,
            plugin_id: fallback_plugin_id,
            accepted: false,
            api_version: agile_tiles_plugin_api::API_VERSION,
            capabilities: Vec::new(),
            error: Some(agile_tiles_plugin_api::PluginError {
                code: agile_tiles_plugin_api::ErrorCode::PluginUnavailable,
                message: "plugin runtime is not initialized".into(),
            }),
        })
}

pub(crate) fn plugin_call_blocking(
    app: &AppHandle,
    request: PluginRequest,
) -> Result<PluginResponse, String> {
    if market::is_web_plugin(&paths::app_data_dir(app), request.plugin_id.as_str()) {
        return market::web_plugin_call(app, request);
    }
    let state = host(app);
    let mut managers = state.plugin_manager.lock().unwrap();
    let manager = managers
        .as_mut()
        .ok_or_else(|| "plugin runtime is not initialized".to_string())?;
    ensure_native_plugin_backend(app, manager, &request.plugin_id)?;
    let should_activate = matches!(
        &request.kind,
        agile_tiles_plugin_api::PluginRequestKind::Call
            | agile_tiles_plugin_api::PluginRequestKind::GetInfo
    ) && manager
        .registry()
        .get(&request.plugin_id)
        .is_some_and(|info| matches!(&info.status, PluginStatus::Discovered))
        && manager.backend_loaded(&request.plugin_id);
    if should_activate {
        let response = manager
            .activate(&request.plugin_id)
            .map_err(|error| error.as_plugin_error().message)?;
        if !response.ok {
            return Ok(response);
        }
    }
    manager
        .call(request)
        .map_err(|error| error.as_plugin_error().message)
}

fn ensure_native_plugin_backend(
    app: &AppHandle,
    manager: &mut PluginManager,
    plugin_id: &agile_tiles_plugin_api::PluginId,
) -> Result<(), String> {
    let Some(info) = manager.registry().get(plugin_id).cloned() else {
        return Ok(());
    };
    if manager.backend_loaded(plugin_id)
        || !matches!(
            info.runtime,
            PluginRuntime::DynamicLibrary | PluginRuntime::ExternalProcess
        )
    {
        return Ok(());
    }

    let location = PathBuf::from(&info.location);
    if !market::is_trusted_local_plugin(&location) {
        return Err(format!("plugin backend is not trusted: {plugin_id}"));
    }
    let backend = market::plugin_backend_path(&location, &info.manifest)
        .ok_or_else(|| format!("plugin backend is missing or invalid: {plugin_id}"))?;
    let services = host(app)
        .plugin_services
        .lock()
        .unwrap()
        .as_ref()
        .cloned()
        .ok_or_else(|| "plugin host services are not initialized".to_string())?;

    match info.runtime {
        PluginRuntime::DynamicLibrary => {
            let plugin =
                agile_tiles_plugin_runtime::DynamicPlugin::load(&backend, info.manifest, services)
                    .map_err(|error| format!("failed to load plugin {plugin_id}: {error}"))?;
            manager
                .attach_dynamic(Box::new(plugin))
                .map_err(|error| error.as_plugin_error().message)?;
        }
        PluginRuntime::ExternalProcess => {
            let process = external_process::ExternalPluginProcess::start_with_working_dir(
                &info.manifest,
                &backend,
                &[],
                services,
                Some(&location),
            )
            .map_err(|error| error.message)?;
            manager
                .attach_external(plugin_id, Box::new(process))
                .map_err(|error| error.as_plugin_error().message)?;
        }
        _ => {}
    }
    Ok(())
}

fn unload_plugin_for_package_change<R: tauri::Runtime>(
    app: &AppHandle<R>,
    id: &str,
) -> Result<(), String> {
    let plugin_id = agile_tiles_plugin_api::PluginId::new(id).map_err(|error| error.to_string())?;
    let host_state = host(app);
    let mut manager = host_state.plugin_manager.lock().unwrap();
    if let Some(manager) = manager.as_mut() {
        manager
            .unload(&plugin_id)
            .map_err(|error| error.as_plugin_error().message)?;
    }
    Ok(())
}

fn load_runtime_plugins(plugin_manager: &mut PluginManager, data_dir: &Path) -> Result<(), String> {
    for (manifest, location, source) in market::runtime_plugin_manifests(data_dir) {
        if plugin_manager.registry().get(&manifest.id).is_some() {
            continue;
        }
        let location_text = location.display().to_string();
        let plugin_id = manifest.id.clone();
        match manifest.runtime.clone() {
            agile_tiles_plugin_api::PluginRuntime::DynamicLibrary => {
                if !market::is_trusted_local_plugin(&location) {
                    eprintln!("[agile-tiles] skip untrusted dynamic plugin: {}", plugin_id);
                    continue;
                }
                let Some(_library) = market::plugin_backend_path(&location, &manifest) else {
                    eprintln!(
                        "[agile-tiles] dynamic plugin library is missing: {}",
                        plugin_id
                    );
                    continue;
                };
                plugin_manager
                    .discover_trusted_local_manifest(manifest, location_text, source)
                    .map_err(|error| error.as_plugin_error().message)?;
            }
            agile_tiles_plugin_api::PluginRuntime::ExternalProcess => {
                if !market::is_trusted_local_plugin(&location) {
                    eprintln!(
                        "[agile-tiles] skip untrusted external plugin: {}",
                        plugin_id
                    );
                    continue;
                }
                let Some(_executable) = market::plugin_backend_path(&location, &manifest) else {
                    eprintln!(
                        "[agile-tiles] external plugin backend is missing: {}",
                        plugin_id
                    );
                    continue;
                };
                plugin_manager
                    .discover_trusted_local_manifest(manifest, location_text, source)
                    .map_err(|error| error.as_plugin_error().message)?;
            }
            agile_tiles_plugin_api::PluginRuntime::WebSandbox => {
                plugin_manager
                    .discover_manifest(manifest, location_text, source)
                    .map_err(|error| error.as_plugin_error().message)?;
            }
            agile_tiles_plugin_api::PluginRuntime::InProcess => {
                eprintln!(
                    "[agile-tiles] reject legacy in-process plugin package: {}",
                    plugin_id
                );
            }
        }
    }
    Ok(())
}

fn activate_startup_plugins(app: &AppHandle) -> Result<(), String> {
    startup_log("app_ready worker: locking plugin manager");
    let state = host(app);
    let disabled_plugins = state.settings_clone().plugins.disabled;
    let mut managers = state.plugin_manager.lock().unwrap();
    let manager = managers
        .as_mut()
        .ok_or_else(|| "plugin runtime is not initialized".to_string())?;
    let ids = manager
        .registry()
        .list()
        .into_iter()
        .filter(|info| {
            let disabled = disabled_plugins
                .iter()
                .any(|disabled_id| disabled_id == info.manifest.id.as_str());
            if disabled && info.manifest.auto_start {
                startup_log(format!(
                    "app_ready worker: skipping disabled plugin {}",
                    info.manifest.id
                ));
            }
            info.manifest.auto_start
                && !disabled
                && matches!(
                    info.runtime,
                    PluginRuntime::DynamicLibrary | PluginRuntime::ExternalProcess
                )
                && matches!(&info.status, PluginStatus::Discovered)
        })
        .map(|info| info.manifest.id)
        .collect::<Vec<_>>();
    for id in ids {
        startup_log(format!("app_ready worker: loading backend {id}"));
        if let Err(error) = ensure_native_plugin_backend(app, manager, &id) {
            eprintln!("[agile-tiles] startup plugin load failed ({id}): {error}");
            continue;
        }
        startup_log(format!("app_ready worker: activating {id}"));
        match manager.activate(&id) {
            Ok(response) if response.ok => {
                startup_log(format!("app_ready worker: activated {id}"));
            }
            Ok(_) => eprintln!("[agile-tiles] startup plugin activation failed: {id}"),
            Err(error) => eprintln!(
                "[agile-tiles] startup plugin activation failed ({}): {}",
                id,
                error.as_plugin_error().message
            ),
        }
    }
    startup_log("app_ready worker: all startup plugins finished");
    Ok(())
}

fn reload_installed_plugins(app: &AppHandle) -> Result<(), String> {
    let data_dir = paths::app_data_dir(app);
    let host_state = host(app);
    let mut managers = host_state.plugin_manager.lock().unwrap();
    let manager = managers
        .as_mut()
        .ok_or_else(|| "plugin runtime is not initialized".to_string())?;
    load_runtime_plugins(manager, &data_dir)
}

#[tauri::command]
async fn app_ready(app: AppHandle) -> Result<(), String> {
    startup_log("command app_ready: begin");
    let result = activate_startup_plugins_async(app).await;
    startup_log(format!(
        "command app_ready: finished (ok={})",
        result.is_ok()
    ));
    result
}

async fn activate_startup_plugins_async(app: AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || activate_startup_plugins(&app))
        .await
        .map_err(|error| format!("plugin startup worker failed: {error}"))?
}

#[tauri::command]
async fn plugin_call(app: AppHandle, request: PluginRequest) -> Result<PluginResponse, String> {
    tokio::task::spawn_blocking(move || plugin_call_blocking(&app, request))
        .await
        .map_err(|error| format!("plugin call worker failed: {error}"))?
}

pub(crate) fn host_plugin_registered<R: tauri::Runtime>(app: &AppHandle<R>, id: &str) -> bool {
    if let Some(user_id) = id.strip_prefix("user:") {
        return market::valid_id(user_id);
    }
    let Ok(plugin_id) = agile_tiles_plugin_api::PluginId::new(id) else {
        return false;
    };
    host(app)
        .plugin_manager
        .lock()
        .unwrap()
        .as_ref()
        .map(|manager| manager.registry().get(&plugin_id).is_some())
        .unwrap_or(false)
}

#[tauri::command]
async fn market_refresh(app: AppHandle) -> Result<market::MarketIndex, String> {
    market::refresh_market_cache(&app).await
}

#[tauri::command]
fn market_snapshot(app: AppHandle) -> Result<market::MarketIndex, String> {
    market::market_snapshot(&app)
}

#[tauri::command]
fn market_install_info(app: AppHandle, kind: String, id: String) -> Result<market::MarketInstallInfo, String> {
    market::install_info(&app, &kind, &id)
}

#[tauri::command]
async fn market_installed(app: AppHandle) -> market::InstalledLists {
    market::installed_lists(&app).await
}

/// 安装/卸载后同步：广播设置变化（各窗口刷新插件清单）并重算轨道几何
fn refresh_rail<R: tauri::Runtime>(app: &AppHandle<R>) {
    let _ = app.emit("settings://changed", &host(app).settings_clone());
    apply_sidebar(app);
}

#[tauri::command]
async fn market_install(
    app: AppHandle,
    kind: String,
    id: String,
    trust_approval: Option<market::MarketTrustApproval>,
) -> Result<(), String> {
    let should_reload_plugin = kind == "plugin";
    let unload_app = app.clone();
    market::install(&app, &kind, &id, trust_approval, move |manifest| {
        if should_reload_plugin {
            unload_plugin_for_package_change(&unload_app, &manifest.id)
        } else {
            Ok(())
        }
    })
    .await?;
    if should_reload_plugin {
        reload_installed_plugins(&app)?;
        activate_startup_plugins_async(app.clone()).await?;
    }
    refresh_rail(&app);
    Ok(())
}

#[tauri::command]
async fn market_uninstall(app: AppHandle, kind: String, id: String) -> Result<(), String> {
    if kind == "plugin" {
        unload_plugin_for_package_change(&app, &id)?;
    }
    market::uninstall(&app, &kind, &id).await?;
    refresh_rail(&app);
    Ok(())
}

#[tauri::command]
async fn plugin_import_local(app: AppHandle, kind: String) -> Result<(), String> {
    let unload_app = app.clone();
    market::plugin_import_local(app.clone(), kind.clone(), move |id| {
        unload_plugin_for_package_change(&unload_app, id)
    })
    .await?;
    if kind == "plugin" {
        reload_installed_plugins(&app)?;
        activate_startup_plugins_async(app.clone()).await?;
    }
    refresh_rail(&app);
    Ok(())
}

#[tauri::command]
fn skin_user_css(app: AppHandle, id: String) -> Result<String, String> {
    startup_log(format!("command skin_user_css: begin ({id})"));
    let result = market::user_skin_css(&app, &id);
    startup_log(format!(
        "command skin_user_css: finished (ok={})",
        result.is_ok()
    ));
    result
}

#[tauri::command]
fn skin_user_ui(app: AppHandle, id: String) -> Result<Option<market::SkinUi>, String> {
    startup_log(format!("command skin_user_ui: begin ({id})"));
    let result = market::user_skin_ui(&app, &id);
    startup_log(format!(
        "command skin_user_ui: finished (ok={})",
        result.is_ok()
    ));
    result
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    market::open_external_url(&url)
}

// ---------- 在线更新（GitHub 固定 update-manifest Release，Tauri 签名校验） ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub version: String,
    pub notes: String,
}

fn update_ignore_path<R: tauri::Runtime>(app: &AppHandle<R>) -> PathBuf {
    paths::app_data_dir(app).join("update_ignore.json")
}

fn update_ignore_version<R: tauri::Runtime>(app: &AppHandle<R>) -> Option<String> {
    paths::load_json(&update_ignore_path(app)).and_then(|v| {
        v.get("version")
            .and_then(|x| x.as_str().map(str::to_string))
    })
}

#[tauri::command]
fn update_ignore_set(app: AppHandle, version: String) -> Result<(), String> {
    let path = update_ignore_path(&app);
    let version = version.trim().to_string();
    if version.is_empty() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    paths::save_json_atomic(&path, &serde_json::json!({ "version": version }))
        .map_err(|e| e.to_string())
}

fn build_updater<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    const UPDATE_MANIFEST_URL: &str =
        "https://github.com/suzikuo/SideWeave/releases/download/update-manifest/latest.json";
    let mirror_prefix = host_settings(app).general.github_mirror;
    let endpoint = market::mirrored_github_url(&mirror_prefix, UPDATE_MANIFEST_URL)?
        .parse::<url::Url>()
        .map_err(|error| format!("更新清单 URL 无效: {error}"))?;
    app.updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_check(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let updater = build_updater(&app)?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("检查更新失败：{e}"))?;
    let ignored = update_ignore_version(&app);
    Ok(match update {
        Some(u) if Some(&u.version) == ignored.as_ref() => {
            // 用户忽略过此版本：视为最新，但带出被忽略的版本号供界面显示撤销入口
            UpdateInfo {
                available: false,
                current_version: current,
                version: u.version.clone(),
                notes: String::new(),
            }
        }
        Some(u) => UpdateInfo {
            available: true,
            current_version: u.current_version.clone(),
            version: u.version.clone(),
            notes: u.body.clone().unwrap_or_default(),
        },
        None => UpdateInfo {
            available: false,
            current_version: current.clone(),
            version: current,
            notes: String::new(),
        },
    })
}

#[tauri::command]
async fn update_install(app: AppHandle) -> Result<String, String> {
    let updater = build_updater(&app)?;
    let mut update = updater
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?
        .ok_or_else(|| "当前已是最新版本".to_string())?;
    let new_version = update.version.clone();
    let mirror_prefix = host_settings(&app).general.github_mirror;
    update.download_url = market::mirrored_github_url(&mirror_prefix, update.download_url.as_str())?
        .parse::<url::Url>()
        .map_err(|error| format!("更新包 URL 无效: {error}"))?;
    let progress_app = app.clone();
    let finish_app = app.clone();
    let downloaded = std::sync::Arc::new(AtomicU64::new(0));
    let chunk_counter = std::sync::Arc::clone(&downloaded);
    let finish_counter = std::sync::Arc::clone(&downloaded);
    update
        .download_and_install(
            &mut move |chunk: usize, total: Option<u64>| {
                let current = chunk_counter.fetch_add(chunk as u64, Ordering::Relaxed) + chunk as u64;
                let percent = total.map(|size| {
                    if size == 0 { 0 } else { ((current * 100) / size).min(100) }
                });
                let _ = progress_app.emit(
                    "update://progress",
                    serde_json::json!({
                        "downloaded": current,
                        "total": total,
                        "percent": percent,
                        "phase": "download"
                    }),
                );
            },
            move || {
                let _ = finish_app.emit(
                    "update://progress",
                    serde_json::json!({
                        "downloaded": finish_counter.load(Ordering::Relaxed),
                        "total": null,
                        "percent": 100,
                        "phase": "install"
                    }),
                );
            },
        )
        .await
        .map_err(|e| format!("下载/安装更新失败: {e}"))?;
    // NSIS 安装器已拉起：退出当前实例，安装器接管
    eprintln!("[update] 安装器已启动（目标版本 {new_version}），当前实例退出");
    let _ = app.exit(0);
    #[allow(unreachable_code)]
    Ok(new_version)
}

// ---------- 入口 ----------

fn is_market_install_deep_link(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    if url.scheme() != "sideweave"
        || url.host_str() != Some("install")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let path = url.path().strip_prefix('/').unwrap_or(url.path());
    let path = path.strip_suffix('/').unwrap_or(path);
    let mut segments = path.split('/');
    let Some(kind) = segments.next() else {
        return false;
    };
    let Some(id) = segments.next() else {
        return false;
    };
    matches!(kind, "plugin" | "skin")
        && !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
        && segments.next().is_none()
}

pub fn run() {
    startup_log("run(): begin Tauri initialization");
    let autostart =
        tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None);
    startup_log("run(): autostart plugin configured");

    let mut builder = tauri::Builder::default();
    #[cfg(windows)]
    {
        // This must run before deep-link so a second process forwards its URL to the first one.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if argv.iter().any(|argument| is_market_install_deep_link(argument)) {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = open_control_center(handle).await;
                });
            }
        }));
    }

    builder
        .register_uri_scheme_protocol("agile-plugin", |context, request| {
            market::plugin_resource_response(context.app_handle(), request)
        })
        .plugin(autostart)
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        // 全局 handler 会收到所有已注册热键的事件：按热键串分发
                        let settings = host(app).settings_clone();
                        // 1) 侧边栏热键：切换侧边栏（隐藏时连带关闭详情卡）
                        if let Ok(expected) = settings.shortcuts.toggle_sidebar.parse::<Shortcut>()
                        {
                            if shortcut == &expected {
                                if expanded_flag(app) {
                                    close_main_surface(app);
                                    collapse(app);
                                } else {
                                    expand(app, true);
                                }
                                return;
                            }
                        }
                        // 2) 控制中心热键（空串 = 未启用）
                        if !settings.shortcuts.control_center.trim().is_empty() {
                            if let Ok(expected) =
                                settings.shortcuts.control_center.parse::<Shortcut>()
                            {
                                if shortcut == &expected {
                                    let handle = app.clone();
                                    tauri::async_runtime::spawn(async move {
                                        let _ = open_control_center(handle).await;
                                    });
                                }
                            }
                        }
                    }
                })
                .build(),
        )
        .manage(Host {
            settings: Mutex::new(Settings::default()),
            performance_manager: Arc::new(performance::PerformanceManager::new(
                performance::PerformanceProfile::default(),
            )),
            plugin_manager: Mutex::new(None),
            plugin_services: Mutex::new(None),
            hover_token: Mutex::new(0),
            collapse_gen: AtomicU64::new(0),
            detail_compact: AtomicBool::new(false),
            detail_destroy_gen: AtomicU64::new(0),
            expanded: AtomicBool::new(true),
            detail_page: Mutex::new(String::new()),
            main_surface: Mutex::new(MainSurface::Sidebar),
            geometry_transition: AtomicBool::new(false),
            detail_opening: AtomicBool::new(false),
        })
        .manage(desktop_notifications::DesktopNotificationManager::default())
        .manage(Arc::new(preview_window::PreviewCoordinator::default()))
        .setup(|app| {
            startup_log("setup: entered");
            let handle = app.handle().clone();
            use tauri_plugin_deep_link::DeepLinkExt;
            let has_initial_market_link = app
                .deep_link()
                .get_current()?
                .is_some_and(|urls| urls.iter().any(|url| is_market_install_deep_link(url.as_str())));

            #[cfg(all(debug_assertions, windows))]
            {
                app.deep_link().register_all()?;
            }

            // 单实例：已有实例在跑则直接退出（命名互斥体，随进程释放）
            #[cfg(windows)]
            {
                use std::os::windows::ffi::OsStrExt;
                let name: Vec<u16> = std::ffi::OsStr::new("Global\\AgileTilesSingleInstance")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let guard = unsafe {
                    windows_sys::Win32::System::Threading::CreateMutexW(
                        std::ptr::null(),
                        0,
                        name.as_ptr(),
                    )
                };
                if !guard.is_null()
                    && unsafe { windows_sys::Win32::Foundation::GetLastError() } == 183
                {
                    // ERROR_ALREADY_EXISTS
                    handle.exit(0);
                    return Ok(());
                }
                let _ = guard; // 句柄由进程持有到退出，互斥体随进程释放
            }

            // 状态装载 + 旧数据一次性迁移
            startup_log("setup: app data initialization begin");
            let data_dir = paths::app_data_dir(&handle);
            let _ = std::fs::create_dir_all(&data_dir);
            let marketplace_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = market::refresh_market_cache(&marketplace_handle).await {
                    eprintln!("[marketplace] background refresh failed: {error}");
                }
            });
            market::cleanup_staging_dirs(&data_dir);
            startup_log("setup: staging cleanup finished");
            let services = extension_host::services(handle.clone(), data_dir.clone());
            *host(&handle).plugin_services.lock().unwrap() = Some(Arc::clone(&services));
            let mut plugin_manager = PluginManager::new(Arc::clone(&services));
            // 原生后端按首次请求加载，启动时只登记清单与包路径。
            load_runtime_plugins(&mut plugin_manager, &data_dir)?;
            startup_log("setup: runtime plugin manifests loaded");
            *host(&handle).plugin_manager.lock().unwrap() = Some(plugin_manager);
            let loaded_settings = paths::load_json(&paths::state_file(&data_dir))
                .map(settings::load_persisted)
                .unwrap_or_else(|| settings::LoadedSettings {
                    settings: Settings::default(),
                });
            let settings::LoadedSettings {
                settings: mut s,
            } = loaded_settings;
            startup_log("setup: settings loaded");
            if s.appearance.skin == "user:minecraft" {
                if let Err(error) = market::ensure_pixel_skin_package(&data_dir) {
                    eprintln!("[agile-tiles] Pixel skin migration failed: {error}");
                }
            }
            migrate::migrate_legacy(&data_dir, &mut s);
            if !s.plugins.permission_grants_initialized {
                let installed_permissions = host(&handle)
                    .plugin_manager
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|manager| {
                        manager
                            .registry()
                            .list()
                            .into_iter()
                            .map(|info| {
                                (
                                    info.manifest.id.to_string(),
                                    info.manifest.permissions,
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for (plugin_id, requested) in installed_permissions {
                    let grants = s
                        .plugins
                        .permission_grants
                        .entry(plugin_id)
                        .or_default();
                    for permission in requested {
                        let expanded = match permission.as_str() {
                            agile_tiles_plugin_api::permissions::NOTIFICATION => vec![
                                agile_tiles_plugin_api::permissions::NOTIFICATION_SYSTEM,
                                agile_tiles_plugin_api::permissions::NOTIFICATION_DESKTOP,
                                agile_tiles_plugin_api::permissions::NOTIFICATION_SIDEBAR,
                            ],
                            _ => vec![permission.as_str()],
                        };
                        for permission in expanded {
                            grants.entry(permission.to_string()).or_insert(true);
                        }
                    }
                }
                s.plugins.permission_grants_initialized = true;
            }
            *host(&handle).settings.lock().unwrap() = s.clone();
            host(&handle).performance_manager.set_profile(
                performance::PerformanceProfile::parse(&s.general.performance_profile)
                    .unwrap_or_default(),
            );
            // 首启即写入当前设置结构，旧 OSS 凭据不会再保存。
            if let Err(error) = persist_settings(&handle, &s) {
                eprintln!("[agile-tiles] {error}");
            }
            startup_log("setup: settings persistence finished");

            // 开机自启状态与设置对齐
            startup_log("setup: autostart configuration begin");
            if s.general.run_on_startup {
                let _ = handle.autolaunch().enable();
            } else {
                let _ = handle.autolaunch().disable();
            }
            startup_log("setup: autostart configuration finished");

            // 窗口：侧边栏 + 详情卡
            startup_log("setup: sidebar webview build begin");
            let sidebar =
                WebviewWindowBuilder::new(app, SIDEBAR, WebviewUrl::App("sidebar.html".into()))
                    .title("SideWeave")
                    .decorations(false)
                    .transparent(true)
                    .shadow(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .resizable(true)
                    .visible(false)
                    .on_page_load(|_, payload| match payload.event() {
                        PageLoadEvent::Started => startup_log("sidebar page load started"),
                        PageLoadEvent::Finished => startup_log("sidebar page load finished"),
                    })
                    .build()?;
            startup_log("setup: sidebar webview build finished");

            apply_interface_scale_to_window(&sidebar, s.appearance.interface_scale);
            apply_sidebar(&handle);
            startup_log("setup: sidebar geometry applied");
            let _ = sidebar.show();
            startup_log("setup: sidebar shown");

            // 全局热键：从用户设置注册（失败不阻断启动，托盘仍可切换）
            let hotkey = s.shortcuts.toggle_sidebar.clone();
            match hotkey.parse::<Shortcut>() {
                Ok(shortcut) => {
                    if let Err(e) = app.global_shortcut().register(shortcut) {
                        eprintln!("[agile-tiles] 注册全局热键 {hotkey} 失败: {e}");
                    }
                }
                Err(e) => eprintln!("[agile-tiles] 热键配置无效 {hotkey}: {e}"),
            }

            // 控制中心热键（空串 = 不启用）
            let cc_hotkey = s.shortcuts.control_center.clone();
            if !cc_hotkey.trim().is_empty() {
                match cc_hotkey.parse::<Shortcut>() {
                    Ok(shortcut) => {
                        if let Err(e) = app.global_shortcut().register(shortcut) {
                            eprintln!("[agile-tiles] 控制中心热键 {cc_hotkey} 注册失败: {e}");
                        }
                    }
                    Err(e) => eprintln!("[agile-tiles] 控制中心热键配置无效 {cc_hotkey}: {e}"),
                }
            }

            setup_tray(&handle)?;
            startup_log("setup: tray created");

            let prewarm_handle = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1500));
                startup_log("prewarm: begin");
                prewarm_windows(&prewarm_handle);
                startup_log("prewarm: scheduled");
            });

            if has_initial_market_link {
                let link_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    if let Err(error) = open_control_center(link_handle).await {
                        startup_log(format!("deep link control center failed: {error}"));
                    }
                });
            }

            startup_log("setup: finished");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            host_notification_snapshot,
            host_notification_dismiss,
            host_notification_resize,
            host_notification_plugin_icon,
            startup_frontend_log,
            app_ready,
            get_system_info,
            get_performance_status,
            get_main_surface,
            set_setting,
            set_skin_appearance,
            set_plugin_enabled,
            set_plugin_sidebar_visible,
            set_plugin_order,
            open_detail,
            detail_content_ready,
            close_detail,
            minimize_detail,
            toggle_sidebar,
            sidebar_hover_enter,
            sidebar_collapse_if_unhovered,
            open_data_dir,
            restart_app,
            plugin_registry_list,
            set_plugin_permission,
            plugin_handshake,
            plugin_call,
            plugin_import_local,
            update_ignore_set,
            backup::backup_export,
            backup::backup_import,
            market_refresh,
            market_snapshot,
            market_install_info,
            market_installed,
            market_install,
            market_uninstall,
            skin_user_css,
            skin_user_ui,
            open_external_url,
            open_control_center,
            update_check,
            update_install,
            preview_window::preview_sidebar_enter,
            preview_window::preview_sidebar_leave,
            preview_window::preview_window_enter,
            preview_window::preview_window_leave,
            preview_window::preview_hide_now,
            preview_window::get_current_preview,
            preview_window::refresh_preview
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
