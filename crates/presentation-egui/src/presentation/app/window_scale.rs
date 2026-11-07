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
//! are left to winit. The minimum size stays the physical size egui-winit set at creation,
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
    imp::install(frame);
}

#[cfg(not(windows))]
pub(crate) fn install(_frame: &eframe::Frame) {}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

    use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::{
        CallWindowProcW, GetWindowLongPtrW, IsZoomed, SetWindowLongPtrW, GWLP_WNDPROC, GWL_STYLE, WINDOWPOS,
        WM_DPICHANGED, WM_WINDOWPOSCHANGING, WNDPROC, WS_CAPTION,
    };

    /// winit's window procedure, which ours forwards to.
    static ORIGINAL_PROC: AtomicIsize = AtomicIsize::new(0);
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    /// True while winit handles a `WM_DPICHANGED` for a window whose frame we keep.
    static FREEZE_FRAME: AtomicBool = AtomicBool::new(false);

    pub(super) fn install(frame: &eframe::Frame) {
        if INSTALLED.load(Ordering::Relaxed) {
            return;
        }
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = frame.window_handle() else { return };
        let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
        let hwnd = h.hwnd.get() as HWND;
        INSTALLED.store(true, Ordering::Relaxed);
        // SAFETY: `hwnd` is the live app window owned by this (the UI) thread. The previous
        // procedure is stored before ours can run, and ours forwards every message to it.
        let previous = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, fixed_size_proc as *const () as isize) };
        ORIGINAL_PROC.store(previous, Ordering::SeqCst);
        tracing::debug!("window: fixed pixel size across monitors enabled");
    }

    unsafe extern "system" fn fixed_size_proc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: ORIGINAL_PROC holds winit's WNDPROC (stored before this proc was installed).
        let original: WNDPROC = unsafe { std::mem::transmute(ORIGINAL_PROC.load(Ordering::SeqCst)) };
        match msg {
            WM_DPICHANGED => {
                // SAFETY: plain queries on the window this procedure belongs to.
                let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
                let keep = unsafe { IsZoomed(hwnd) } == 0 && style & WS_CAPTION == WS_CAPTION;
                let was = FREEZE_FRAME.swap(keep, Ordering::SeqCst);
                // winit updates its scale factor; its resize below is neutralised.
                let result = unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) };
                FREEZE_FRAME.store(was, Ordering::SeqCst);
                result
            }
            WM_WINDOWPOSCHANGING if FREEZE_FRAME.load(Ordering::SeqCst) => {
                // SAFETY: for WM_WINDOWPOSCHANGING, lparam points to a writable WINDOWPOS.
                if let Some(pos) = unsafe { (lparam as *mut WINDOWPOS).as_mut() } {
                    pos.flags = super::frozen_flags(pos.flags, true, true);
                }
                unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) }
            }
            _ => unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) },
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
