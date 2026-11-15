//! Keeps the window at a fixed size in physical pixels when it moves between monitors with
//! different scale factors (Windows).
//!
//! On `WM_DPICHANGED` winit resizes (and re-positions) the window to keep its *logical* size:
//! a 1000 px window becomes 1500 px on a 150 % monitor. While the window straddles a monitor
//! edge, that resize moves its centre back over the edge, Windows flips the DPI again, and the
//! window grows and shrinks without end. Resizing it back afterwards does not help: the
//! intermediate size change alone is enough to move the centre.
//!
//! So the window frame must not change at all. The window procedure is subclassed, and while
//! winit handles `WM_DPICHANGED` (updating its scale factor and emitting `ScaleFactorChanged`),
//! the `WM_WINDOWPOSCHANGING` its `SetWindowPos` call produces is neutralised with
//! `SWP_NOSIZE | SWP_NOMOVE`. Only the scale used to draw the content changes; the window keeps
//! its position and pixel size, so its centre never moves and the DPI cannot flip back.
//!
//! Windows draws a window at a single DPI, so the part hanging over the other monitor is shown
//! with the same pixels (it cannot be scaled separately).
//!
//! Maximized windows (moved with Win+Shift+Arrow) and windows without a caption (fullscreen)
//! are left to winit, except the borderless desktop-lyrics window, which is kept too. The minimum size stays the physical size egui-winit set at creation,
//! consistent with a fixed pixel size. Other platforms are unchanged.

/// `SWP_*` flags winit's `SetWindowPos` should use during a DPI change.
#[cfg_attr(not(windows), allow(dead_code))] // used by the Windows window procedure
pub(crate) fn frozen_flags(flags: u32, in_dpi_change: bool, keep_frame: bool) -> u32 {
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    if in_dpi_change && keep_frame {
        flags | SWP_NOSIZE | SWP_NOMOVE
    } else {
        flags
    }
}

/// Installs the fixed-pixel-size behaviour on the app window (once; later calls do nothing).
#[cfg(windows)]
pub(crate) fn install(frame: &eframe::Frame) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = frame.window_handle() else { return };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
    imp::install(h.hwnd.get(), false);
}

#[cfg(not(windows))]
pub(crate) fn install(_frame: &eframe::Frame) {}

/// Same for another window of this app (the desktop-lyrics window). It has no caption, so
/// `borderless` keeps its frame too. Idempotent per window.
#[cfg(windows)]
pub(crate) fn install_hwnd(hwnd: isize, borderless: bool) {
    imp::install(hwnd, borderless);
}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::Mutex;

    use winapi::shared::basetsd::{DWORD_PTR, UINT_PTR};
    use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
    use winapi::shared::windef::HWND;
    use winapi::um::commctrl::{DefSubclassProc, SetWindowSubclass};
    use winapi::um::winuser::{GetWindowLongPtrW, IsZoomed, GWL_STYLE, WINDOWPOS, WM_DPICHANGED, WM_WINDOWPOSCHANGING, WS_CAPTION};

    /// Subclass id (any constant unique to this procedure).
    const SUBCLASS_ID: UINT_PTR = 0x4c52_4350; // "LRCP"
    /// Windows already subclassed.
    static INSTALLED: Mutex<Vec<isize>> = Mutex::new(Vec::new());
    /// The window whose `WM_DPICHANGED` winit is handling right now, if its frame is kept
    /// (0 = none). DPI changes are handled synchronously, one window at a time.
    static FROZEN: AtomicIsize = AtomicIsize::new(0);

    pub(super) fn install(hwnd: isize, borderless: bool) {
        let mut installed = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
        // Forget windows that no longer exist (the desktop-lyrics window is re-created).
        installed.retain(|&h| unsafe { winapi::um::winuser::IsWindow(h as HWND) } != 0);
        if installed.contains(&hwnd) {
            return;
        }
        // SAFETY: `hwnd` is a live window of this (the UI) thread; the subclass procedure
        // forwards every message with DefSubclassProc.
        let ok = unsafe { SetWindowSubclass(hwnd as HWND, Some(fixed_size_proc), SUBCLASS_ID, DWORD_PTR::from(borderless)) };
        if ok != 0 {
            installed.push(hwnd);
            tracing::debug!("window {hwnd:#x}: fixed pixel size across monitors enabled");
        }
    }

    unsafe extern "system" fn fixed_size_proc(
        hwnd: HWND,
        msg: UINT,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: UINT_PTR,
        borderless: DWORD_PTR,
    ) -> LRESULT {
        match msg {
            WM_DPICHANGED => {
                // SAFETY: plain queries on the window this procedure belongs to.
                let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
                let framed = style & WS_CAPTION == WS_CAPTION || borderless != 0;
                let keep = unsafe { IsZoomed(hwnd) } == 0 && framed;
                let was = FROZEN.swap(if keep { hwnd as isize } else { 0 }, Ordering::SeqCst);
                // winit updates its scale factor; its resize below is neutralised.
                let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
                FROZEN.store(was, Ordering::SeqCst);
                result
            }
            WM_WINDOWPOSCHANGING if FROZEN.load(Ordering::SeqCst) == hwnd as isize => {
                // SAFETY: for WM_WINDOWPOSCHANGING, lparam points to a writable WINDOWPOS.
                if let Some(pos) = unsafe { (lparam as *mut WINDOWPOS).as_mut() } {
                    pos.flags = super::frozen_flags(pos.flags, true, true);
                }
                unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
            }
            _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::frozen_flags;

    const NOSIZE: u32 = 0x0001;
    const NOMOVE: u32 = 0x0002;
    const NOZORDER: u32 = 0x0004;
    const NOACTIVATE: u32 = 0x0010;

    #[test]
    fn dpi_change_keeps_the_frame() {
        let winit_flags = NOZORDER | NOACTIVATE;
        let f = frozen_flags(winit_flags, true, true);
        assert_eq!(f & (NOSIZE | NOMOVE), NOSIZE | NOMOVE, "no resize, no move");
        assert_eq!(f & winit_flags, winit_flags, "other flags untouched");
    }

    #[test]
    fn other_moves_are_untouched() {
        // The user's own drag / resize, and maximized or fullscreen windows.
        assert_eq!(frozen_flags(NOZORDER, false, true), NOZORDER);
        assert_eq!(frozen_flags(NOZORDER, true, false), NOZORDER);
    }
}
