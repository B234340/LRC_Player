//! Keeps the window at a fixed size in physical pixels when it moves between monitors with
//! different scale factors (Windows).
//!
//! On `WM_DPICHANGED` winit resizes the window to keep its *logical* size (a 1000 px window
//! becomes 1500 px on a 150 % monitor). We want the opposite: the window keeps its pixel size
//! and only its content is drawn at the new scale.
//!
//! This must happen synchronously inside the message: while the window straddles a monitor
//! edge Windows can send several `WM_DPICHANGED` between two frames, so a per-frame correction
//! cannot keep up (the window grew while being dragged back and forth). The window procedure is
//! therefore subclassed: after winit has handled `WM_DPICHANGED` (it updates its scale factor
//! and emits `ScaleFactorChanged`), the client area is set back to its size from before the
//! message. Maximized, fullscreen and undecorated windows are left alone.
//!
//! The minimum size stays what egui-winit set when the window was created: a physical size,
//! which is consistent with a fixed pixel size (it never forces the window to grow, because
//! the window was at least that big on the monitor it came from).
//!
//! Other platforms are unchanged: macOS and Wayland keep the logical size natively.

/// Outer window size that gives the client area `client_before` again, given the current
/// outer (`window_now`) and client (`client_now`) sizes. `None` when nothing has to change.
#[cfg_attr(not(windows), allow(dead_code))] // used by the Windows window procedure
pub(crate) fn restore_outer_size(
    client_before: (i32, i32),
    window_now: (i32, i32),
    client_now: (i32, i32),
) -> Option<(i32, i32)> {
    if client_before == client_now || client_before.0 <= 0 || client_before.1 <= 0 {
        return None;
    }
    // Non-client frame (title bar, borders) at the new DPI.
    let frame = (window_now.0 - client_now.0, window_now.1 - client_now.1);
    Some((client_before.0 + frame.0, client_before.1 + frame.1))
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
    use winapi::shared::windef::{HWND, RECT};
    use winapi::um::winuser::{
        CallWindowProcW, GetClientRect, GetWindowLongPtrW, GetWindowRect, IsZoomed, SetWindowLongPtrW,
        SetWindowPos, GWLP_WNDPROC, GWL_STYLE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOZORDER,
        WM_DPICHANGED, WNDPROC, WS_CAPTION,
    };

    /// winit's window procedure, which ours forwards to.
    static ORIGINAL_PROC: AtomicIsize = AtomicIsize::new(0);
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    /// Set while we resize, so a nested `WM_DPICHANGED` is passed straight to winit.
    static ADJUSTING: AtomicBool = AtomicBool::new(false);

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

    fn client_size(hwnd: HWND) -> Option<(i32, i32)> {
        let mut r: RECT = unsafe { std::mem::zeroed() };
        // SAFETY: valid window handle and out pointer.
        (unsafe { GetClientRect(hwnd, &mut r) } != 0).then(|| (r.right - r.left, r.bottom - r.top))
    }

    fn window_size(hwnd: HWND) -> Option<(i32, i32)> {
        let mut r: RECT = unsafe { std::mem::zeroed() };
        // SAFETY: valid window handle and out pointer.
        (unsafe { GetWindowRect(hwnd, &mut r) } != 0).then(|| (r.right - r.left, r.bottom - r.top))
    }

    unsafe extern "system" fn fixed_size_proc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: ORIGINAL_PROC holds winit's WNDPROC (stored before this proc was installed).
        let original: WNDPROC = unsafe { std::mem::transmute(ORIGINAL_PROC.load(Ordering::SeqCst)) };
        if msg != WM_DPICHANGED || ADJUSTING.load(Ordering::SeqCst) {
            return unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) };
        }

        // SAFETY: plain queries on the window this procedure belongs to.
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        let keep = unsafe { IsZoomed(hwnd) } == 0 && style & WS_CAPTION == WS_CAPTION;
        let before = if keep { client_size(hwnd) } else { None };

        // Let winit update its scale factor (and resize to keep the logical size).
        let result = unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) };

        if let (Some(before), Some(window_now), Some(client_now)) = (before, window_size(hwnd), client_size(hwnd)) {
            if let Some((w, h)) = super::restore_outer_size(before, window_now, client_now) {
                ADJUSTING.store(true, Ordering::SeqCst);
                // SAFETY: resizing our own window from its window procedure; position and
                // z-order are kept.
                unsafe {
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        0,
                        0,
                        w,
                        h,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
                    );
                }
                ADJUSTING.store(false, Ordering::SeqCst);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::restore_outer_size;

    #[test]
    fn client_area_is_restored_with_the_new_frame_size() {
        // 1000×700 px client at 100 %; winit made it 1500×1050 at 150 %, where the frame
        // (borders + title bar) is 24×51 px.
        assert_eq!(restore_outer_size((1000, 700), (1524, 1101), (1500, 1050)), Some((1024, 751)));
    }

    #[test]
    fn going_to_a_lower_scale_shrinks_back_too() {
        assert_eq!(restore_outer_size((1500, 1050), (1016, 734), (1000, 700)), Some((1516, 1084)));
    }

    #[test]
    fn unchanged_or_unknown_sizes_do_nothing() {
        assert_eq!(restore_outer_size((1000, 700), (1016, 734), (1000, 700)), None);
        assert_eq!(restore_outer_size((0, 0), (1016, 734), (1000, 700)), None);
    }

    #[test]
    fn repeated_flips_keep_the_pixel_size() {
        // Each flip winit rescales the client; we always return to the original pixels.
        let original = (1200, 800);
        let mut client = original;
        for scale in [1.5_f64, 1.0 / 1.5, 1.25, 0.8, 1.5, 1.0 / 1.5] {
            let scaled = ((client.0 as f64 * scale).round() as i32, (client.1 as f64 * scale).round() as i32);
            let frame = (16, 39);
            let outer = restore_outer_size(client, (scaled.0 + frame.0, scaled.1 + frame.1), scaled);
            if let Some((w, h)) = outer {
                client = (w - frame.0, h - frame.1);
            }
            assert_eq!(client, original);
        }
    }
}
