//! Windows 平台辅助：取鼠标所在显示器的工作区（扣除任务栏）。

#[derive(Debug, Clone, Copy)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[cfg(windows)]
pub fn work_area() -> WorkArea {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, SystemParametersInfoW, SPI_GETWORKAREA,
    };

    unsafe {
        let mut pt = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) != 0 {
            let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            if !hmon.is_null() {
                let mut info: MONITORINFO = std::mem::zeroed();
                info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                if GetMonitorInfoW(hmon, &mut info) != 0 {
                    let rc: &RECT = &info.rcWork;
                    return WorkArea {
                        x: rc.left,
                        y: rc.top,
                        w: rc.right - rc.left,
                        h: rc.bottom - rc.top,
                    };
                }
            }
        }
        // 兜底：主屏工作区
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rc as *mut RECT as _, 0) != 0 {
            return WorkArea {
                x: rc.left,
                y: rc.top,
                w: rc.right - rc.left,
                h: rc.bottom - rc.top,
            };
        }
        WorkArea {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        }
    }
}

#[cfg(not(windows))]
pub fn work_area() -> WorkArea {
    WorkArea {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    }
}

#[cfg(windows)]
mod clickthrough {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Shell::{
        DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, SUBCLASSPROC,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, HTTRANSPARENT, WM_NCHITTEST,
        WS_EX_TRANSPARENT,
    };

    const SUBCLASS_ID: usize = 0xA61C_0001;

    fn states() -> &'static Mutex<HashMap<isize, bool>> {
        static STATES: OnceLock<Mutex<HashMap<isize, bool>>> = OnceLock::new();
        STATES.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn enabled(hwnd: HWND) -> bool {
        states()
            .lock()
            .ok()
            .and_then(|items| items.get(&(hwnd as isize)).copied())
            .unwrap_or(false)
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _subclass_id: usize,
        _reference_data: usize,
    ) -> LRESULT {
        if message == WM_NCHITTEST && enabled(hwnd) {
            return HTTRANSPARENT as LRESULT;
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }

    pub(super) fn install(hwnd: HWND) -> Result<(), String> {
        let key = hwnd as isize;
        let mut items = states()
            .lock()
            .map_err(|_| "click-through window registry is poisoned".to_string())?;
        if !items.contains_key(&key) {
            let callback: SUBCLASSPROC = Some(subclass_proc);
            let installed = unsafe { SetWindowSubclass(hwnd, callback, SUBCLASS_ID, 0) };
            if installed == 0 {
                return Err("failed to install click-through window hook".into());
            }
            items.insert(key, false);
        }
        Ok(())
    }

    pub(super) fn set(hwnd: HWND, enabled: bool) -> Result<(), String> {
        install(hwnd)?;
        states()
            .lock()
            .map_err(|_| "click-through window registry is poisoned".to_string())?
            .insert(hwnd as isize, enabled);
        let styles = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        let styles = if enabled {
            styles | WS_EX_TRANSPARENT as isize
        } else {
            styles & !(WS_EX_TRANSPARENT as isize)
        };
        unsafe {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, styles);
        }
        Ok(())
    }

    pub(super) fn remove(hwnd: HWND) {
        if let Ok(mut items) = states().lock() {
            items.remove(&(hwnd as isize));
        }
        let callback: SUBCLASSPROC = Some(subclass_proc);
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, callback, SUBCLASS_ID);
        }
    }
}

#[cfg(windows)]
pub fn set_clickthrough(hwnd: *mut std::ffi::c_void, enabled: bool) -> Result<(), String> {
    clickthrough::set(hwnd, enabled)
}

#[cfg(windows)]
pub fn remove_clickthrough_hook(hwnd: *mut std::ffi::c_void) {
    clickthrough::remove(hwnd);
}

#[cfg(windows)]
pub fn show_window_no_activate(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
    unsafe {
        ShowWindow(hwnd as _, SW_SHOWNOACTIVATE);
    }
}

#[cfg(not(windows))]
pub fn set_clickthrough(_hwnd: *mut std::ffi::c_void, _enabled: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn remove_clickthrough_hook(_hwnd: *mut std::ffi::c_void) {}

#[cfg(not(windows))]
pub fn show_window_no_activate(_hwnd: *mut std::ffi::c_void) {}
