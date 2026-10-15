//! Low-resource Windows notification surfaces painted with GDI in a Tauri native window.

use super::{DesktopNotification, Surface};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WindowBuilder};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CombineRgn, CreateFontW, CreatePen, CreatePolygonRgn, CreateRectRgn,
    CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetDC,
    InvalidateRect, Polygon, ReleaseDC, RoundRect, SelectObject, SetBkMode, SetTextColor,
    SetWindowRgn, DT_CALCRECT, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_WORDBREAK,
    PS_SOLID, RGN_OR, TRANSPARENT, WINDING,
};
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, SUBCLASSPROC,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowLongPtrW, KillTimer, SetLayeredWindowAttributes, SetTimer,
    SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, LWA_ALPHA, MA_NOACTIVATE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_ERASEBKGND,
    WM_MOUSEACTIVATE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONUP, WM_SHOWWINDOW, WM_STYLECHANGED,
    WM_STYLECHANGING, WM_TIMER, WS_CAPTION, WS_EX_CLIENTEDGE, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_STATICEDGE, WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

const SUBCLASS_ID: usize = 0xA61C_0012;
const BORDERLESS_SUBCLASS_ID: usize = 0xA61C_0014;
const PROGRESS_TIMER_ID: usize = 0xA61C_0013;
const TIMER_INTERVAL_MS: u32 = 33;
const CARD_GAP: i32 = 9;
const EDGE_PADDING: i32 = 10;

fn empty_rect() -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }
}

#[derive(Clone)]
struct NativeSurfaceState {
    app: AppHandle,
    surface: Surface,
    notifications: Vec<DesktopNotification>,
    card_heights: Vec<i32>,
    tail_side: String,
    scale: f64,
    desktop_background: u32,
    bubble_background: u32,
    desktop_custom_background: bool,
    bubble_custom_background: bool,
    last_progress_widths: Vec<i32>,
}

#[derive(Clone, Copy)]
struct CardLayout {
    top: i32,
    bottom: i32,
    left: i32,
    right: i32,
    body_top: i32,
    body_bottom: i32,
    body_left: i32,
    body_right: i32,
}

fn states() -> &'static Mutex<HashMap<isize, NativeSurfaceState>> {
    static STATES: OnceLock<Mutex<HashMap<isize, NativeSurfaceState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn native_label(surface: Surface) -> &'static str {
    match surface {
        Surface::Desktop => "host-native-notifications",
        Surface::Sidebar => "host-native-sidebar-notifications",
    }
}

fn card_layouts(
    surface: Surface,
    width: i32,
    height: i32,
    card_heights: &[i32],
    scale: f64,
    tail_side: &str,
) -> Vec<CardLayout> {
    let margin = (EDGE_PADDING as f64 * scale).round() as i32;
    let gap = (CARD_GAP as f64 * scale).round() as i32;
    let tail = (12.0 * scale).round() as i32;
    let mut top = margin;
    card_heights
        .iter()
        .map(|card_height| {
            let bottom = (top + *card_height).min(height - margin);
            let (body_top, body_bottom, body_left, body_right) = match surface {
                Surface::Desktop => (top, bottom, margin, width - margin),
                Surface::Sidebar => match tail_side {
                    "left" => (top, bottom, margin + tail, width - margin),
                    "right" => (top, bottom, margin, width - margin - tail),
                    _ => (top + tail, bottom, margin, width - margin),
                },
            };
            let layout = CardLayout {
                top,
                bottom,
                left: body_left.min(margin),
                right: body_right.max(width - margin),
                body_top,
                body_bottom,
                body_left,
                body_right,
            };
            top = bottom + gap;
            layout
        })
        .collect()
}

fn measure_text_height(value: &str, width: i32, points: f64, single_line: bool, scale: f64) -> i32 {
    if value.trim().is_empty() || width <= 0 {
        return 0;
    }
    unsafe {
        let hdc = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            return (points * scale * 1.5).ceil() as i32;
        }
        let font_name: Vec<u16> = "Segoe UI".encode_utf16().chain(Some(0)).collect();
        let font = CreateFontW(
            (-(points * scale).round() as i32).min(-1),
            0,
            0,
            0,
            if single_line { 600 } else { 400 },
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            font_name.as_ptr(),
        );
        if font.is_null() {
            let _ = ReleaseDC(std::ptr::null_mut(), hdc);
            return (points * scale * 1.5).ceil() as i32;
        }
        let previous = SelectObject(hdc, font as _);
        let wide: Vec<u16> = value.encode_utf16().collect();
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: 0,
        };
        let mut flags = DT_CALCRECT | DT_LEFT | DT_NOPREFIX | DT_END_ELLIPSIS;
        if single_line {
            flags |= DT_SINGLELINE;
        } else {
            flags |= DT_WORDBREAK;
        }
        let _ = DrawTextW(hdc, wide.as_ptr(), wide.len() as i32, &mut rect, flags);
        let _ = SelectObject(hdc, previous);
        let _ = DeleteObject(font as _);
        let _ = ReleaseDC(std::ptr::null_mut(), hdc);
        (rect.bottom - rect.top).max((points * scale * 1.2).ceil() as i32)
    }
}

fn measured_card_height(
    surface: Surface,
    notification: &DesktopNotification,
    scale: f64,
    tail_side: &str,
    outer_width: i32,
    card_limit: i32,
) -> i32 {
    let margin = (EDGE_PADDING as f64 * scale).round() as i32;
    let tail = if surface == Surface::Sidebar && matches!(tail_side, "left" | "right") {
        (12.0 * scale).round() as i32
    } else {
        0
    };
    let horizontal_inset = match surface {
        Surface::Desktop => (16.0 * scale).round() as i32,
        Surface::Sidebar => (12.0 * scale).round() as i32,
    };
    let text_width = (outer_width - margin * 2 - tail - horizontal_inset * 2).max(1);
    let height = match surface {
        Surface::Sidebar => {
            let body = if notification.body.trim().is_empty() {
                notification.title.as_str()
            } else {
                notification.body.as_str()
            };
            let text_height = measure_text_height(body, text_width, 12.0, false, scale);
            let min_body_height = (40.0 * scale).round() as i32;
            let body_height = (text_height + (15.0 * scale).round() as i32).max(min_body_height);
            body_height
                + if tail_side == "top" {
                    (12.0 * scale).round() as i32
                } else {
                    0
                }
        }
        Surface::Desktop => {
            let has_title = !notification.title.trim().is_empty();
            let has_body = !notification.body.trim().is_empty();
            let title_height = if has_title {
                measure_text_height(&notification.title, text_width, 14.0, true, scale)
                    .max((22.0 * scale).round() as i32)
            } else {
                0
            };
            let body_height = if has_body {
                measure_text_height(&notification.body, text_width, 12.0, false, scale)
            } else {
                0
            };
            let vertical = (12.0 * scale).round() as i32;
            let title_top = (10.0 * scale).round() as i32;
            let content_height = match (has_title, has_body) {
                (true, true) => {
                    title_top + title_height + (4.0 * scale).round() as i32 + body_height + vertical
                }
                (true, false) => title_top + title_height + vertical,
                (false, true) => (17.0 * scale).round() as i32 + body_height + vertical,
                (false, false) => 0,
            };
            content_height.max((44.0 * scale).round() as i32)
        }
    };
    height.max(1).min(card_limit.max(1))
}

fn layout_notifications(
    surface: Surface,
    notifications: &[DesktopNotification],
    scale: f64,
    tail_side: &str,
    area_height: i32,
) -> (Vec<DesktopNotification>, Vec<i32>, f64) {
    let edge = (EDGE_PADDING as f64 * scale).round() as i32;
    let gap = (CARD_GAP as f64 * scale).round() as i32;
    let max_height = ((surface.max_height() * scale).round() as i32)
        .min(area_height - (24.0 * scale).round() as i32)
        .max(edge * 2 + 1);
    let card_limit = (max_height - edge * 2).max(1);
    let outer_width = (surface.width() * scale).round() as i32;
    let heights: Vec<i32> = notifications
        .iter()
        .map(|notification| {
            measured_card_height(
                surface,
                notification,
                scale,
                tail_side,
                outer_width,
                card_limit,
            )
        })
        .collect();

    let mut first_visible = notifications.len();
    let mut used_height = 0;
    for index in (0..notifications.len()).rev() {
        let extra_gap = if first_visible == notifications.len() {
            0
        } else {
            gap
        };
        let next_height = used_height + extra_gap + heights[index];
        if first_visible < notifications.len() && next_height > card_limit {
            break;
        }
        first_visible = index;
        used_height = next_height.min(card_limit);
        if used_height >= card_limit {
            break;
        }
    }
    if first_visible == notifications.len() && !notifications.is_empty() {
        first_visible = notifications.len() - 1;
        used_height = heights[first_visible].min(card_limit);
    }
    let visible = notifications[first_visible..].to_vec();
    let visible_heights = heights[first_visible..].to_vec();
    let total_height = (edge * 2 + used_height).min(max_height);
    (visible, visible_heights, total_height as f64 / scale)
}

pub(super) fn update(
    app: &AppHandle,
    surface: Surface,
    notifications: &[DesktopNotification],
    tail_side: &str,
) -> Result<(), String> {
    if notifications.is_empty() {
        destroy(app, surface);
        return Ok(());
    }

    let area = crate::platform::work_area();
    let scale = crate::monitor_scale(app).max(0.5);
    let notification_settings = crate::host_settings(app).notifications;
    let opacity = match surface {
        Surface::Desktop => notification_settings.desktop_background_opacity,
        Surface::Sidebar => notification_settings.bubble_background_opacity,
    }
    .clamp(0.0, 1.0);
    let desktop_background = parse_hex_color(&notification_settings.desktop_background_color)
        .unwrap_or_else(|| rgb(35, 37, 44));
    let bubble_background = parse_hex_color(&notification_settings.bubble_background_color)
        .unwrap_or_else(|| rgb(255, 253, 247));
    let desktop_custom_background = !notification_settings
        .desktop_background_color
        .trim()
        .is_empty();
    let bubble_custom_background = !notification_settings
        .bubble_background_color
        .trim()
        .is_empty();
    let (_, _, initial_height) =
        layout_notifications(surface, notifications, scale, tail_side, area.h);
    let width = surface.width();
    let label = native_label(surface);

    let window = if let Some(window) = app.get_window(label) {
        window
    } else {
        WindowBuilder::new(app, label)
            .title("")
            .decorations(false)
            .transparent(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .minimizable(false)
            .maximizable(false)
            .closable(false)
            .focused(false)
            .visible(false)
            .inner_size(width, initial_height)
            .build()
            .map_err(|error| format!("failed to create native notification window: {error}"))?
    };
    let hwnd = window
        .hwnd()
        .map_err(|error| format!("failed to access native notification window: {error}"))?
        .0 as HWND;
    if !unsafe { enforce_borderless(hwnd) } {
        return Err("failed to enforce borderless notification window styles".into());
    }
    unsafe { set_window_opacity(hwnd, opacity) };
    let actual_scale = window.scale_factor().unwrap_or(scale).max(0.5);
    let (visible, card_heights, height) =
        layout_notifications(surface, notifications, actual_scale, tail_side, area.h);
    let target_size = PhysicalSize::new(
        (width * actual_scale).round() as u32,
        (height * actual_scale).round() as u32,
    );
    if !matches!(window.inner_size(), Ok(size) if size.width == target_size.width && size.height == target_size.height)
    {
        window
            .set_size(target_size)
            .map_err(|error| error.to_string())?;
    }

    let (x, y) = match surface {
        Surface::Desktop => super::desktop_position(area, actual_scale, width, height),
        Surface::Sidebar => super::sidebar_position(app, area, actual_scale, width, height),
    };
    if !matches!(window.outer_position(), Ok(position) if position.x == x && position.y == y) {
        window
            .set_position(PhysicalPosition::new(x, y))
            .map_err(|error| error.to_string())?;
    }

    let key = hwnd as isize;
    let visible_count = visible.len();
    states()
        .lock()
        .map_err(|_| "native notification registry is poisoned".to_string())?
        .insert(
            key,
            NativeSurfaceState {
                app: app.clone(),
                surface,
                notifications: visible,
                card_heights,
                tail_side: tail_side.to_string(),
                scale: actual_scale,
                desktop_background,
                bubble_background,
                desktop_custom_background,
                bubble_custom_background,
                last_progress_widths: vec![-1; visible_count],
            },
        );

    let callback: SUBCLASSPROC = Some(subclass_proc);
    if unsafe { SetWindowSubclass(hwnd, callback, SUBCLASS_ID, 0) } == 0 {
        states().lock().ok().map(|mut items| items.remove(&key));
        let _ = window.destroy();
        return Err("failed to attach native notification drawing handler".into());
    }
    unsafe {
        let _ = SetTimer(hwnd, PROGRESS_TIMER_ID, TIMER_INTERVAL_MS, None);
        apply_window_region(hwnd, surface, actual_scale, tail_side);
        let _ = InvalidateRect(hwnd, std::ptr::null(), 1);
    }
    window.show().map_err(|error| error.to_string())?;
    crate::platform::show_window_no_activate(hwnd);
    Ok(())
}

pub(super) unsafe fn strip_window_chrome(hwnd: HWND) {
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    let style_mask =
        (WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as isize;
    let clean_style = style & !style_mask;
    if clean_style != style {
        SetWindowLongPtrW(hwnd, GWL_STYLE, clean_style);
    }

    let extended_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    let edge_mask = (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_STATICEDGE) as isize;
    let clean_extended_style = (extended_style & !edge_mask) | WS_EX_NOACTIVATE as isize;
    if clean_extended_style != extended_style {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, clean_extended_style);
    }

    let _ = SetWindowPos(
        hwnd,
        std::ptr::null_mut(),
        0,
        0,
        0,
        0,
        SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

pub(super) unsafe fn enforce_borderless(hwnd: HWND) -> bool {
    let callback: SUBCLASSPROC = Some(borderless_subclass_proc);
    if SetWindowSubclass(hwnd, callback, BORDERLESS_SUBCLASS_ID, 0) == 0 {
        return false;
    }
    strip_window_chrome(hwnd);
    true
}

unsafe fn set_window_opacity(hwnd: HWND, opacity: f64) {
    let extended_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    if extended_style & WS_EX_LAYERED as isize == 0 {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended_style | WS_EX_LAYERED as isize);
    }
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let _ = SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
}

#[repr(C)]
struct WindowStyleChange {
    style_old: u32,
    style_new: u32,
}

unsafe extern "system" fn borderless_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _reference_data: usize,
) -> LRESULT {
    match message {
        WM_STYLECHANGING if lparam != 0 => {
            let requested_style = &mut *(lparam as *mut WindowStyleChange);
            let mask = if wparam as i32 == GWL_STYLE {
                (WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as u32
            } else if wparam as i32 == GWL_EXSTYLE {
                (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_STATICEDGE) as u32
            } else {
                0
            };
            requested_style.style_new &= !mask;
            if wparam as i32 == GWL_EXSTYLE {
                requested_style.style_new |= WS_EX_NOACTIVATE as u32;
                if requested_style.style_old & WS_EX_LAYERED != 0 {
                    requested_style.style_new |= WS_EX_LAYERED;
                }
            }
        }
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_STYLECHANGED | WM_SHOWWINDOW => strip_window_chrome(hwnd),
        WM_NCDESTROY => {
            let callback: SUBCLASSPROC = Some(borderless_subclass_proc);
            let _ = RemoveWindowSubclass(hwnd, callback, BORDERLESS_SUBCLASS_ID);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

pub(super) fn destroy(app: &AppHandle, surface: Surface) {
    if let Some(window) = app.get_window(native_label(surface)) {
        let _ = window.destroy();
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _reference_data: usize,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            if let Some(state) = states()
                .lock()
                .ok()
                .and_then(|items| items.get(&(hwnd as isize)).cloned())
            {
                let drawn_progress = paint(hwnd, &state);
                if let Ok(mut items) = states().lock() {
                    if let Some(current) = items.get_mut(&(hwnd as isize)) {
                        if current.notifications.len() == drawn_progress.len() {
                            current.last_progress_widths = drawn_progress;
                        }
                    }
                }
            } else {
                let mut paint = std::mem::zeroed();
                let _ = BeginPaint(hwnd, &mut paint);
                let _ = EndPaint(hwnd, &paint);
            }
            return 0;
        }
        WM_TIMER if wparam == PROGRESS_TIMER_ID => {
            update_progress(hwnd);
            return 0;
        }
        WM_SHOWWINDOW => {
            strip_window_chrome(hwnd);
            if let Some(state) = states()
                .lock()
                .ok()
                .and_then(|items| items.get(&(hwnd as isize)).cloned())
            {
                apply_window_region(hwnd, state.surface, state.scale, &state.tail_side);
                let _ = InvalidateRect(hwnd, std::ptr::null(), 1);
            }
        }
        WM_RBUTTONUP => {
            let point = POINT {
                x: (lparam as u32 as u16 as i16) as i32,
                y: ((lparam as u32 >> 16) as u16 as i16) as i32,
            };
            let action = states()
                .lock()
                .ok()
                .and_then(|items| items.get(&(hwnd as isize)).cloned())
                .and_then(|state| {
                    let mut client = empty_rect();
                    let _ = GetClientRect(hwnd, &mut client);
                    let layouts = card_layouts(
                        state.surface,
                        client.right - client.left,
                        client.bottom - client.top,
                        &state.card_heights,
                        state.scale,
                        &state.tail_side,
                    );
                    layouts
                        .iter()
                        .position(|card| {
                            point.x >= card.left
                                && point.x <= card.right
                                && point.y >= card.top
                                && point.y <= card.bottom
                        })
                        .map(|index| {
                            (
                                state.app,
                                state.surface,
                                state.notifications[index].id.clone(),
                            )
                        })
                });
            if let Some((app, surface, id)) = action {
                let _ = app.state::<super::DesktopNotificationManager>().dismiss(
                    app.clone(),
                    surface,
                    &id,
                );
            }
            return 0;
        }
        WM_NCDESTROY => {
            let _ = KillTimer(hwnd, PROGRESS_TIMER_ID);
            if let Ok(mut items) = states().lock() {
                items.remove(&(hwnd as isize));
            }
            let callback: SUBCLASSPROC = Some(subclass_proc);
            let _ = RemoveWindowSubclass(hwnd, callback, SUBCLASS_ID);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

unsafe fn apply_window_region(hwnd: HWND, surface: Surface, scale: f64, tail_side: &str) {
    let mut client = empty_rect();
    if GetClientRect(hwnd, &mut client) == 0 {
        return;
    }
    let width = client.right - client.left;
    let height = client.bottom - client.top;
    let card_heights = states()
        .lock()
        .ok()
        .and_then(|items| {
            items
                .get(&(hwnd as isize))
                .map(|state| state.card_heights.clone())
        })
        .unwrap_or_default();
    let layouts = card_layouts(surface, width, height, &card_heights, scale, tail_side);
    let combined = CreateRectRgn(0, 0, 0, 0);
    if combined.is_null() {
        return;
    }
    for layout in layouts {
        let card_region = CreateRoundRectRgn(
            layout.body_left,
            layout.body_top,
            layout.body_right,
            layout.body_bottom,
            (18.0 * scale).round() as i32,
            (18.0 * scale).round() as i32,
        );
        if !card_region.is_null() {
            let _ = CombineRgn(combined, combined, card_region, RGN_OR);
            let _ = DeleteObject(card_region as _);
        }
        if surface == Surface::Sidebar {
            let points = tail_points(layout, tail_side, scale);
            if points.len() == 3 {
                let tail_region = CreatePolygonRgn(points.as_ptr(), points.len() as i32, WINDING);
                if !tail_region.is_null() {
                    let _ = CombineRgn(combined, combined, tail_region, RGN_OR);
                    let _ = DeleteObject(tail_region as _);
                }
            }
        }
    }
    if SetWindowRgn(hwnd, combined, 1) == 0 {
        let _ = DeleteObject(combined as _);
    }
}

fn tail_points(layout: CardLayout, tail_side: &str, scale: f64) -> Vec<POINT> {
    let tail = (12.0 * scale).round() as i32;
    let middle_y = (layout.body_top + layout.body_bottom) / 2;
    let middle_x = (layout.body_left + layout.body_right) / 2;
    match tail_side {
        "left" => vec![
            POINT {
                x: layout.body_left + 1,
                y: middle_y - tail,
            },
            POINT {
                x: layout.left,
                y: middle_y,
            },
            POINT {
                x: layout.body_left + 1,
                y: middle_y + tail,
            },
        ],
        "right" => vec![
            POINT {
                x: layout.body_right - 1,
                y: middle_y - tail,
            },
            POINT {
                x: layout.right,
                y: middle_y,
            },
            POINT {
                x: layout.body_right - 1,
                y: middle_y + tail,
            },
        ],
        "top" => vec![
            POINT {
                x: middle_x - tail,
                y: layout.body_top + 1,
            },
            POINT {
                x: middle_x,
                y: layout.top,
            },
            POINT {
                x: middle_x + tail,
                y: layout.body_top + 1,
            },
        ],
        _ => Vec::new(),
    }
}

fn progress_rect(layout: CardLayout, surface: Surface, scale: f64) -> RECT {
    let horizontal_inset = match surface {
        Surface::Desktop => 14.0,
        Surface::Sidebar => 12.0,
    };
    let height = (2.0 * scale).round().max(1.0) as i32;
    let inset = (horizontal_inset * scale).round() as i32;
    let bottom_gap = (5.0 * scale).round() as i32;
    let top = layout.body_bottom - bottom_gap - height;
    RECT {
        left: layout.body_left + inset,
        top,
        right: layout.body_right - inset,
        bottom: top + height,
    }
}

fn progress_width(notification: &DesktopNotification, rect: RECT) -> i32 {
    let duration = notification
        .expires_at_ms
        .saturating_sub(notification.created_at_ms)
        .max(1) as f64;
    let remaining = notification.expires_at_ms.saturating_sub(now_ms()) as f64;
    (((rect.right - rect.left) as f64) * (remaining / duration).clamp(0.0, 1.0)).round() as i32
}

fn progress_track_color(surface: Surface) -> u32 {
    match surface {
        Surface::Desktop => rgb(72, 75, 85),
        Surface::Sidebar => rgb(224, 224, 218),
    }
}

unsafe fn update_progress(hwnd: HWND) {
    let hdc = GetDC(hwnd);
    if hdc.is_null() {
        return;
    }
    let mut client = empty_rect();
    let _ = GetClientRect(hwnd, &mut client);
    let mut needs_full_paint = false;
    if let Ok(mut items) = states().lock() {
        if let Some(state) = items.get_mut(&(hwnd as isize)) {
            let layouts = card_layouts(
                state.surface,
                client.right - client.left,
                client.bottom - client.top,
                &state.card_heights,
                state.scale,
                &state.tail_side,
            );
            for (index, (notification, layout)) in
                state.notifications.iter().zip(layouts.iter()).enumerate()
            {
                let area = progress_rect(*layout, state.surface, state.scale);
                let next = progress_width(notification, area);
                let previous = state.last_progress_widths.get(index).copied().unwrap_or(-1);
                if previous < 0 || next > previous {
                    needs_full_paint = true;
                    continue;
                }
                if next < previous {
                    let changed = RECT {
                        left: area.left + next,
                        top: area.top,
                        right: area.left + previous.min(area.right - area.left),
                        bottom: area.bottom,
                    };
                    let track = CreateSolidBrush(progress_track_color(state.surface));
                    if !track.is_null() {
                        let _ = FillRect(hdc, &changed, track);
                        let _ = DeleteObject(track as _);
                    }
                    if let Some(last) = state.last_progress_widths.get_mut(index) {
                        *last = next;
                    }
                }
            }
        }
    }
    let _ = ReleaseDC(hwnd, hdc);
    if needs_full_paint {
        let _ = InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

unsafe fn paint(hwnd: HWND, state: &NativeSurfaceState) -> Vec<i32> {
    let mut paint = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut paint);
    if hdc.is_null() {
        return Vec::new();
    }
    let mut client = empty_rect();
    let _ = GetClientRect(hwnd, &mut client);
    let width = client.right - client.left;
    let height = client.bottom - client.top;
    let layouts = card_layouts(
        state.surface,
        width,
        height,
        &state.card_heights,
        state.scale,
        &state.tail_side,
    );
    SetBkMode(hdc, TRANSPARENT as i32);

    let mut drawn_progress = Vec::with_capacity(state.notifications.len());
    for (notification, layout) in state.notifications.iter().zip(layouts.iter()) {
        let (background, foreground, body_color, tone, border) = colors(notification, state);
        let brush = CreateSolidBrush(background);
        let pen = CreatePen(
            PS_SOLID,
            if state.surface == Surface::Sidebar {
                (2.0 * state.scale).round() as i32
            } else {
                (1.0 * state.scale).round() as i32
            },
            border,
        );
        let old_brush = SelectObject(hdc, brush as _);
        let old_pen = SelectObject(hdc, pen as _);

        if state.surface == Surface::Sidebar {
            let points = tail_points(*layout, &state.tail_side, state.scale);
            if points.len() == 3 {
                Polygon(hdc, points.as_ptr(), points.len() as i32);
            }
            RoundRect(
                hdc,
                layout.body_left,
                layout.body_top,
                layout.body_right,
                layout.body_bottom,
                (18.0 * state.scale).round() as i32,
                (18.0 * state.scale).round() as i32,
            );
            let inset = (12.0 * state.scale).round() as i32;
            let body = if notification.body.trim().is_empty() {
                notification.title.as_str()
            } else {
                notification.body.as_str()
            };
            if !body.trim().is_empty() {
                draw_text(
                    hdc,
                    body,
                    RECT {
                        left: layout.body_left + inset,
                        top: layout.body_top + (5.0 * state.scale).round() as i32,
                        right: layout.body_right - inset,
                        bottom: layout.body_bottom - (10.0 * state.scale).round() as i32,
                    },
                    body_color,
                    12.0,
                    false,
                    state.scale,
                );
            }
        } else {
            RoundRect(
                hdc,
                layout.body_left,
                layout.body_top,
                layout.body_right,
                layout.body_bottom,
                (16.0 * state.scale).round() as i32,
                (16.0 * state.scale).round() as i32,
            );
            let stripe = RECT {
                left: layout.body_left + (1.0 * state.scale).round() as i32,
                top: layout.body_top + (12.0 * state.scale).round() as i32,
                right: layout.body_left + (4.0 * state.scale).round() as i32,
                bottom: layout.body_bottom - (12.0 * state.scale).round() as i32,
            };
            let tone_brush = CreateSolidBrush(tone);
            FillRect(hdc, &stripe, tone_brush);
            let _ = DeleteObject(tone_brush as _);
            let has_title = !notification.title.trim().is_empty();
            let has_body = !notification.body.trim().is_empty();
            let text_left = layout.body_left + (16.0 * state.scale).round() as i32;
            let text_right = layout.body_right - (16.0 * state.scale).round() as i32;
            if has_title {
                let title_top = if has_body {
                    layout.body_top + (10.0 * state.scale).round() as i32
                } else {
                    layout.body_top
                        + ((layout.body_bottom
                            - layout.body_top
                            - (22.0 * state.scale).round() as i32)
                            / 2)
                };
                draw_text(
                    hdc,
                    &notification.title,
                    RECT {
                        left: text_left,
                        top: title_top,
                        right: text_right,
                        bottom: title_top + (22.0 * state.scale).round() as i32,
                    },
                    foreground,
                    14.0,
                    true,
                    state.scale,
                );
            }
            if has_body {
                let body_top = if has_title {
                    layout.body_top + (34.0 * state.scale).round() as i32
                } else {
                    layout.body_top + (17.0 * state.scale).round() as i32
                };
                draw_text(
                    hdc,
                    &notification.body,
                    RECT {
                        left: text_left,
                        top: body_top,
                        right: text_right,
                        bottom: layout.body_bottom - (12.0 * state.scale).round() as i32,
                    },
                    body_color,
                    12.0,
                    false,
                    state.scale,
                );
            }
        }

        let mut progress = progress_rect(*layout, state.surface, state.scale);
        let progress_width = progress_width(notification, progress);
        drawn_progress.push(progress_width);
        let track = CreateSolidBrush(progress_track_color(state.surface));
        FillRect(hdc, &progress, track);
        let _ = DeleteObject(track as _);
        progress.right = progress.left + progress_width;
        let progress_brush = CreateSolidBrush(tone);
        FillRect(hdc, &progress, progress_brush);
        let _ = DeleteObject(progress_brush as _);

        let _ = SelectObject(hdc, old_pen);
        let _ = SelectObject(hdc, old_brush);
        let _ = DeleteObject(pen as _);
        let _ = DeleteObject(brush as _);
    }
    let _ = EndPaint(hwnd, &paint);
    drawn_progress
}

unsafe fn draw_text(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    value: &str,
    mut rect: RECT,
    color: u32,
    points: f64,
    single_line: bool,
    scale: f64,
) {
    if value.is_empty() || rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }
    let font_name: Vec<u16> = "Segoe UI".encode_utf16().chain(Some(0)).collect();
    let font = CreateFontW(
        (-(points * scale).round() as i32).min(-1),
        0,
        0,
        0,
        if single_line { 600 } else { 400 },
        0,
        0,
        0,
        1,
        0,
        0,
        5,
        0,
        font_name.as_ptr(),
    );
    let previous = SelectObject(hdc, font as _);
    SetTextColor(hdc, color);
    let wide: Vec<u16> = value.encode_utf16().collect();
    let mut flags = DT_LEFT | DT_NOPREFIX | DT_END_ELLIPSIS;
    if single_line {
        flags |= DT_SINGLELINE;
    } else {
        flags |= DT_WORDBREAK;
    }
    let _ = DrawTextW(hdc, wide.as_ptr(), wide.len() as i32, &mut rect, flags);
    let _ = SelectObject(hdc, previous);
    let _ = DeleteObject(font as _);
}

unsafe fn colors(
    notification: &DesktopNotification,
    state: &NativeSurfaceState,
) -> (u32, u32, u32, u32, u32) {
    let tone = match notification.kind {
        sideweave_plugin_sdk::DesktopNotificationKind::Success => rgb(71, 207, 166),
        sideweave_plugin_sdk::DesktopNotificationKind::Warning => rgb(232, 177, 79),
        sideweave_plugin_sdk::DesktopNotificationKind::Alarm => rgb(255, 106, 120),
        sideweave_plugin_sdk::DesktopNotificationKind::Info => rgb(110, 157, 255),
    };
    let (background, custom_background, default_foreground, default_body, border) =
        match state.surface {
            Surface::Desktop => (
                state.desktop_background,
                state.desktop_custom_background,
                rgb(244, 245, 248),
                rgb(203, 205, 213),
                rgb(71, 74, 84),
            ),
            Surface::Sidebar => (
                state.bubble_background,
                state.bubble_custom_background,
                rgb(32, 32, 32),
                rgb(68, 68, 68),
                rgb(28, 28, 28),
            ),
        };
    let (foreground, body_color) = if custom_background {
        let (foreground, body) = contrast_colors(background);
        (foreground, body)
    } else {
        (default_foreground, default_body)
    };
    (background, foreground, body_color, tone, border)
}

const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
}

fn contrast_colors(background: u32) -> (u32, u32) {
    let red = (background & 0xff) as u8;
    let green = ((background >> 8) & 0xff) as u8;
    let blue = ((background >> 16) & 0xff) as u8;
    let luminance = red as f64 * 0.299 + green as f64 * 0.587 + blue as f64 * 0.114;
    if luminance > 150.0 {
        (rgb(32, 33, 36), rgb(75, 77, 82))
    } else {
        (rgb(243, 244, 247), rgb(200, 202, 209))
    }
}

fn parse_hex_color(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(rgb(red, green, blue))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
