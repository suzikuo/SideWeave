//! Host-owned notification surfaces shared by native plugins.

use agile_tiles_plugin_sdk::{DesktopNotificationKind, DesktopNotificationOptions, HostError};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

#[cfg(windows)]
mod native;

pub(crate) const DESKTOP_WINDOW_LABEL: &str = "host-notifications";
pub(crate) const SIDEBAR_WINDOW_LABEL: &str = "host-sidebar-notifications";
const CHANGE_EVENT: &str = "host-notification://changed";
const DESKTOP_WIDTH: f64 = 420.0;
const DESKTOP_MAX_HEIGHT: f64 = 640.0;
const SIDEBAR_WIDTH: f64 = 400.0;
const SIDEBAR_MAX_HEIGHT: f64 = 420.0;
const MAX_ACTIVE_NOTIFICATIONS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Surface {
    Desktop,
    Sidebar,
}

impl Surface {
    fn label(self) -> &'static str {
        match self {
            Self::Desktop => DESKTOP_WINDOW_LABEL,
            Self::Sidebar => SIDEBAR_WINDOW_LABEL,
        }
    }

    pub(crate) fn width(self) -> f64 {
        match self {
            Self::Desktop => DESKTOP_WIDTH,
            Self::Sidebar => SIDEBAR_WIDTH,
        }
    }

    pub(super) fn max_height(self) -> f64 {
        match self {
            Self::Desktop => DESKTOP_MAX_HEIGHT,
            Self::Sidebar => SIDEBAR_MAX_HEIGHT,
        }
    }

    fn page(self) -> &'static str {
        match self {
            Self::Desktop => "notifications.html",
            Self::Sidebar => "sidebar-notifications.html",
        }
    }

    pub(crate) fn from_label(label: &str) -> Option<Self> {
        match label {
            DESKTOP_WINDOW_LABEL => Some(Self::Desktop),
            SIDEBAR_WINDOW_LABEL => Some(Self::Sidebar),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopNotification {
    pub id: String,
    pub title: String,
    pub body: String,
    pub kind: DesktopNotificationKind,
    pub source_plugin_id: Option<String>,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationViewSnapshot {
    pub notifications: Vec<DesktopNotification>,
    pub tail_side: String,
}

#[derive(Default)]
struct Queue {
    desktop: Vec<DesktopNotification>,
    sidebar: Vec<DesktopNotification>,
    worker_running: bool,
}

impl Queue {
    fn list(&self, surface: Surface) -> &Vec<DesktopNotification> {
        match surface {
            Surface::Desktop => &self.desktop,
            Surface::Sidebar => &self.sidebar,
        }
    }

    fn list_mut(&mut self, surface: Surface) -> &mut Vec<DesktopNotification> {
        match surface {
            Surface::Desktop => &mut self.desktop,
            Surface::Sidebar => &mut self.sidebar,
        }
    }

    fn is_empty(&self) -> bool {
        self.desktop.is_empty() && self.sidebar.is_empty()
    }
}

#[derive(Default)]
struct Inner {
    queue: Mutex<Queue>,
    changed: Condvar,
    pending_updates: Mutex<PendingWindowUpdates>,
}

#[derive(Default)]
struct PendingWindowUpdates {
    surfaces: HashSet<Surface>,
    scheduled: bool,
}

#[derive(Clone, Default)]
pub(crate) struct DesktopNotificationManager {
    inner: Arc<Inner>,
}

impl DesktopNotificationManager {
    pub(crate) fn refresh_all(&self, app: AppHandle) -> Result<(), String> {
        schedule_window_update(app.clone(), Surface::Desktop)?;
        schedule_window_update(app, Surface::Sidebar)
    }

    pub(crate) fn show_desktop(
        &self,
        app: AppHandle,
        options: &DesktopNotificationOptions,
    ) -> Result<(), HostError> {
        self.show_on(app, Surface::Desktop, options, None)
    }

    pub(crate) fn show_desktop_for_plugin(
        &self,
        app: AppHandle,
        options: &DesktopNotificationOptions,
        plugin_id: &str,
    ) -> Result<(), HostError> {
        self.show_on(app, Surface::Desktop, options, Some(plugin_id))
    }

    pub(crate) fn show_sidebar(
        &self,
        app: AppHandle,
        options: &DesktopNotificationOptions,
    ) -> Result<(), HostError> {
        self.show_on(app, Surface::Sidebar, options, None)
    }

    pub(crate) fn show_sidebar_for_plugin(
        &self,
        app: AppHandle,
        options: &DesktopNotificationOptions,
        plugin_id: &str,
    ) -> Result<(), HostError> {
        self.show_on(app, Surface::Sidebar, options, Some(plugin_id))
    }

    fn show_on(
        &self,
        app: AppHandle,
        surface: Surface,
        options: &DesktopNotificationOptions,
        source_plugin_id: Option<&str>,
    ) -> Result<(), HostError> {
        let title = options.title.trim();
        let body = options.body.trim();
        if title.is_empty() && body.is_empty() {
            return Err(HostError::new(
                "notification title and body cannot both be empty",
            ));
        }

        let now = now_ms();
        let duration_ms = options.duration_ms.clamp(1_000, 60_000);
        let notification = DesktopNotification {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.chars().take(200).collect(),
            body: body.chars().take(4_000).collect(),
            kind: options.kind,
            source_plugin_id: source_plugin_id.map(str::to_string),
            created_at_ms: now,
            expires_at_ms: now.saturating_add(duration_ms),
        };

        let start_worker = {
            let mut queue = self
                .inner
                .queue
                .lock()
                .map_err(|_| HostError::new("desktop notification queue is unavailable"))?;
            let list = queue.list_mut(surface);
            if list.len() >= MAX_ACTIVE_NOTIFICATIONS {
                list.remove(0);
            }
            list.push(notification);
            if queue.worker_running {
                self.inner.changed.notify_one();
                false
            } else {
                queue.worker_running = true;
                true
            }
        };

        if start_worker {
            let inner = Arc::clone(&self.inner);
            let worker_app = app.clone();
            thread::spawn(move || expire_notifications(inner, worker_app));
        }
        schedule_window_update(app, surface).map_err(HostError::new)
    }

    pub(crate) fn snapshot(&self, surface: Surface) -> Vec<DesktopNotification> {
        let now = now_ms();
        let queue = self
            .inner
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue
            .list(surface)
            .iter()
            .filter(|notification| notification.expires_at_ms > now)
            .cloned()
            .collect()
    }

    pub(crate) fn dismiss(&self, app: AppHandle, surface: Surface, id: &str) -> Result<(), String> {
        let mut queue = self
            .inner
            .queue
            .lock()
            .map_err(|_| "desktop notification queue is unavailable".to_string())?;
        queue
            .list_mut(surface)
            .retain(|notification| notification.id != id);
        self.inner.changed.notify_one();
        drop(queue);
        schedule_window_update(app, surface)
    }
}

fn expire_notifications(inner: Arc<Inner>, app: AppHandle) {
    loop {
        let mut queue = inner
            .queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        loop {
            let now = now_ms();
            let desktop_changed = prune_expired(&mut queue.desktop, now);
            let sidebar_changed = prune_expired(&mut queue.sidebar, now);
            if desktop_changed || sidebar_changed {
                drop(queue);
                if desktop_changed {
                    let _ = schedule_window_update(app.clone(), Surface::Desktop);
                }
                if sidebar_changed {
                    let _ = schedule_window_update(app.clone(), Surface::Sidebar);
                }
                break;
            }

            if queue.is_empty() {
                queue.worker_running = false;
                return;
            }

            let next_expiration = queue
                .desktop
                .iter()
                .chain(queue.sidebar.iter())
                .map(|notification| notification.expires_at_ms)
                .min()
                .unwrap_or_else(now_ms);
            let wait_ms = next_expiration.saturating_sub(now_ms()).max(1);
            let (next_queue, _) = inner
                .changed
                .wait_timeout(queue, Duration::from_millis(wait_ms))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue = next_queue;
        }
    }
}

fn prune_expired(notifications: &mut Vec<DesktopNotification>, now: u64) -> bool {
    let count_before = notifications.len();
    notifications.retain(|notification| notification.expires_at_ms > now);
    notifications.len() != count_before
}

fn schedule_window_update(app: AppHandle, surface: Surface) -> Result<(), String> {
    let inner = Arc::clone(&app.state::<DesktopNotificationManager>().inner);
    let should_schedule = {
        let mut pending = inner
            .pending_updates
            .lock()
            .map_err(|_| "notification update queue is unavailable".to_string())?;
        pending.surfaces.insert(surface);
        if pending.scheduled {
            false
        } else {
            pending.scheduled = true;
            true
        }
    };
    if !should_schedule {
        return Ok(());
    }

    let dispatch_inner = Arc::clone(&inner);
    thread::spawn(move || {
        // Let a short burst of plugin messages settle before touching the UI thread.
        thread::sleep(Duration::from_millis(16));
        let update_inner = Arc::clone(&dispatch_inner);
        let update_app = app.clone();
        if let Err(error) = app.run_on_main_thread(move || loop {
            let surfaces = {
                let mut pending = update_inner
                    .pending_updates
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if pending.surfaces.is_empty() {
                    pending.scheduled = false;
                    return;
                }
                std::mem::take(&mut pending.surfaces)
            };
            for surface in surfaces {
                update_window(&update_app, surface);
            }
        }) {
            let mut pending = dispatch_inner
                .pending_updates
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.scheduled = false;
            eprintln!("[agile-tiles] failed to schedule host notification update: {error}");
        }
    });
    Ok(())
}

fn update_window(app: &AppHandle, surface: Surface) {
    let notifications = app.state::<DesktopNotificationManager>().snapshot(surface);
    if notifications.is_empty() {
        if let Some(window) = app.get_webview_window(surface.label()) {
            let _ = window.destroy();
        }
        #[cfg(windows)]
        native::destroy(app, surface);
        return;
    }

    #[cfg(windows)]
    if crate::host_settings(app).notifications.render_mode == "native" {
        if let Some(window) = app.get_webview_window(surface.label()) {
            let _ = window.destroy();
        }
        match native::update(app, surface, &notifications, &sidebar_tail_side(app)) {
            Ok(()) => return,
            Err(error) => eprintln!("[agile-tiles] native notification renderer failed: {error}"),
        }
    }

    #[cfg(windows)]
    native::destroy(app, surface);

    let initial_height = (20.0 + notifications.len() as f64 * surface_item_height(surface))
        .min(surface.max_height());
    let window = if let Some(window) = app.get_webview_window(surface.label()) {
        window
    } else {
        match WebviewWindowBuilder::new(
            app,
            surface.label(),
            WebviewUrl::App(surface.page().into()),
        )
        .title("")
        .inner_size(surface.width(), initial_height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .closable(false)
        .focused(false)
        .visible(false)
        .build()
        {
            Ok(window) => window,
            Err(error) => {
                eprintln!("[agile-tiles] failed to create host notification window: {error}");
                return;
            }
        }
    };

    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        if !unsafe { native::enforce_borderless(hwnd.0 as _) } {
            eprintln!("[agile-tiles] failed to enforce borderless notification window styles");
        }
    }

    let area = crate::platform::work_area();
    let scale = window.scale_factor().unwrap_or(1.0);
    let screen_max_height = (area.h as f64 / scale - 24.0).clamp(80.0, surface.max_height());
    let height =
        (20.0 + notifications.len() as f64 * surface_item_height(surface)).min(screen_max_height);
    let current_height = window
        .inner_size()
        .map(|size| f64::from(size.height) / scale)
        .unwrap_or_default();
    if (current_height - height).abs() > 1.0 {
        let _ = window.set_size(LogicalSize::new(surface.width(), height));
    }

    let (x, y) = match surface {
        Surface::Desktop => desktop_position(area, scale, surface.width(), height),
        Surface::Sidebar => sidebar_position(app, area, scale, surface.width(), height),
    };
    if !matches!(window.outer_position(), Ok(position) if position.x == x && position.y == y) {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }

    let view = NotificationViewSnapshot {
        notifications,
        tail_side: sidebar_tail_side(app),
    };
    let _ = app.emit_to(surface.label(), CHANGE_EVENT, view);
    let _ = window.show();
    #[cfg(windows)]
    if let Err(error) = schedule_borderless_show(app.clone(), surface.label()) {
        eprintln!("[agile-tiles] failed to schedule borderless notification display: {error}");
    }
}

#[cfg(windows)]
pub(super) fn schedule_borderless_show(app: AppHandle, label: &'static str) -> Result<(), String> {
    let display_app = app.clone();
    app.run_on_main_thread(move || {
        let Some(window) = display_app.get_window(label) else {
            return;
        };
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        unsafe { native::strip_window_chrome(hwnd.0 as _) };
        crate::platform::show_window_no_activate(hwnd.0);
    })
    .map_err(|error| error.to_string())
}

fn surface_item_height(surface: Surface) -> f64 {
    match surface {
        Surface::Desktop => 124.0,
        Surface::Sidebar => 78.0,
    }
}

pub(crate) fn view_snapshot(app: &AppHandle, surface: Surface) -> NotificationViewSnapshot {
    NotificationViewSnapshot {
        notifications: app.state::<DesktopNotificationManager>().snapshot(surface),
        tail_side: sidebar_tail_side(app),
    }
}

fn sidebar_tail_side(app: &AppHandle) -> String {
    match crate::host_settings(app)
        .appearance
        .sidebar_position
        .as_str()
    {
        "right" => "right",
        "top" => "top",
        _ => "left",
    }
    .into()
}

fn desktop_position(
    area: crate::platform::WorkArea,
    scale: f64,
    width: f64,
    height: f64,
) -> (i32, i32) {
    let width = (width * scale).round() as i32;
    let height = (height * scale).round() as i32;
    clamp_position(
        area,
        area.x + area.w - width - (12.0 * scale).round() as i32,
        area.y + area.h - height - (12.0 * scale).round() as i32,
        width,
        height,
    )
}

fn sidebar_position(
    app: &AppHandle,
    area: crate::platform::WorkArea,
    scale: f64,
    width: f64,
    height: f64,
) -> (i32, i32) {
    let width_px = (width * scale).round() as i32;
    let height_px = (height * scale).round() as i32;
    let gap = (12.0 * scale).round() as i32;
    let settings = crate::host_settings(app);
    let sidebar = app.get_webview_window(crate::SIDEBAR);

    let (x, y) = if let Some(sidebar) = sidebar {
        match (sidebar.outer_position(), sidebar.outer_size()) {
            (Ok(position), Ok(size)) => match settings.appearance.sidebar_position.as_str() {
                "left" => (
                    position.x + size.width as i32 + gap,
                    position.y + (size.height as i32 - height_px) / 2,
                ),
                "top" => (
                    position.x + (size.width as i32 - width_px) / 2,
                    position.y + size.height as i32 + gap,
                ),
                _ => (
                    position.x - width_px - gap,
                    position.y + (size.height as i32 - height_px) / 2,
                ),
            },
            _ => fallback_sidebar_position(area, scale, width_px, gap),
        }
    } else {
        fallback_sidebar_position(area, scale, width_px, gap)
    };

    clamp_position(area, x, y, width_px, height_px)
}

fn fallback_sidebar_position(
    area: crate::platform::WorkArea,
    scale: f64,
    width_px: i32,
    gap: i32,
) -> (i32, i32) {
    (
        area.x + area.w - width_px - gap,
        area.y + (12.0 * scale).round() as i32,
    )
}

fn clamp_position(
    area: crate::platform::WorkArea,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> (i32, i32) {
    let min_x = area.x + 4;
    let max_x = (area.x + area.w - width - 4).max(min_x);
    let min_y = area.y + 4;
    let max_y = (area.y + area.h - height - 4).max(min_y);
    (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

pub(crate) fn max_height_for_window(window: &WebviewWindow, surface: Surface) -> f64 {
    let area = crate::platform::work_area();
    let scale = window.scale_factor().unwrap_or(1.0);
    (area.h as f64 / scale - 24.0).clamp(80.0, surface.max_height())
}

pub(crate) fn resize_surface_window(
    window: &WebviewWindow,
    app: &AppHandle,
    surface: Surface,
    requested_height: f64,
) -> Result<(), String> {
    let area = crate::platform::work_area();
    let scale = window.scale_factor().unwrap_or(1.0);
    let height = requested_height.clamp(80.0, max_height_for_window(window, surface));
    let current_height = window
        .inner_size()
        .map(|size| f64::from(size.height) / scale)
        .unwrap_or_default();
    if (current_height - height).abs() > 1.0 {
        window
            .set_size(LogicalSize::new(surface.width(), height))
            .map_err(|error| error.to_string())?;
    }
    let (x, y) = match surface {
        Surface::Desktop => desktop_position(area, scale, surface.width(), height),
        Surface::Sidebar => sidebar_position(app, area, scale, surface.width(), height),
    };
    if matches!(window.outer_position(), Ok(position) if position.x == x && position.y == y) {
        return Ok(());
    }
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| error.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
