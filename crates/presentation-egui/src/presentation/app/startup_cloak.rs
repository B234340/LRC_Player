//! Hides the window from the screen until its first frames are drawn (Windows).
//!
//! eframe shows the window after painting the first frame, but on Windows the OpenGL image is
//! not on screen yet at that moment, so DWM first shows the window's empty (white) client area,
//! and again when the window is resized right after showing (restored maximized state, DPI).
//! DWM *cloaking* keeps the window composed off-screen: it exists, is "visible" to the app and
//! receives frames, but nothing appears until it is uncloaked. The window is cloaked in
//! `GuiApp::new` (before eframe shows it) and uncloaked once a few frames have been painted
//! and the window size has settled, or after [`MAX_CLOAK`] in any case.

use std::time::Duration;

/// Frames painted before the window is revealed.
#[cfg_attr(not(windows), allow(dead_code))]
const FRAMES_BEFORE_REVEAL: u64 = 3;
/// Safety net: never keep the window hidden longer than this.
#[cfg_attr(not(windows), allow(dead_code))]
const MAX_CLOAK: Duration = Duration::from_millis(1500);

/// Whether the window can be revealed: enough frames painted with a stable size, or too long.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn ready(frames: u64, stable_frames: u64, elapsed: Duration) -> bool {
    (frames >= FRAMES_BEFORE_REVEAL && stable_frames >= 2) || elapsed >= MAX_CLOAK
}

#[cfg(windows)]
pub(crate) use imp::{cloak, expect_maximized, keep_opaque, reveal_when_ready};

/// No-op off Windows.
#[cfg(not(windows))]
pub(crate) fn keep_opaque(_cc: &eframe::CreationContext<'_>) {}

/// No-op off Windows (other platforms do not show an empty white window).
#[cfg(not(windows))]
pub(crate) fn cloak(_cc: &eframe::CreationContext<'_>) {}

#[cfg(not(windows))]
pub(crate) fn reveal_when_ready(_ctx: &eframe::egui::Context) {}

#[cfg(not(windows))]
#[allow(dead_code)]
pub(crate) fn expect_maximized() {}

#[cfg(windows)]
mod imp {
    use std::sync::Mutex;
    use std::time::Instant;

    use eframe::egui;
    use winapi::shared::minwindef::{BOOL, FALSE, TRUE};
    use winapi::shared::windef::HWND;
    use winapi::um::dwmapi::{DwmSetWindowAttribute, DWMWA_CLOAK};

    struct Cloaked {
        hwnd: isize,
        since: Instant,
        frames: u64,
        stable_frames: u64,
        last_size: Option<egui::Vec2>,
        /// A maximize was requested: stay hidden until the window reports it is maximized.
        expect_maximized: bool,
    }

    static STATE: Mutex<Option<Cloaked>> = Mutex::new(None);

    fn set_cloak(hwnd: isize, on: bool) -> bool {
        let value: BOOL = if on { TRUE } else { FALSE };
        // SAFETY: `hwnd` is the app's live top-level window; the attribute is a BOOL.
        let hr = unsafe {
            DwmSetWindowAttribute(
                hwnd as HWND,
                DWMWA_CLOAK,
                &value as *const BOOL as *const _,
                std::mem::size_of::<BOOL>() as u32,
            )
        };
        hr >= 0
    }

    /// The main window is created transparent only to get an OpenGL config with alpha (for
    /// the desktop-lyrics window). Turn DWM transparency (blur-behind, set by winit) off for
    /// it again: DWM then composes it opaque, as before, whatever alpha the frame has.
    pub(crate) fn keep_opaque(cc: &eframe::CreationContext<'_>) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use winapi::um::dwmapi::{DwmEnableBlurBehindWindow, DWM_BB_ENABLE, DWM_BLURBEHIND};
        let Ok(handle) = cc.window_handle() else { return };
        let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
        let bb = DWM_BLURBEHIND { dwFlags: DWM_BB_ENABLE, fEnable: FALSE, hRgnBlur: std::ptr::null_mut(), fTransitionOnMaximized: FALSE };
        // SAFETY: our live top-level window; `bb` is a valid DWM_BLURBEHIND.
        let hr = unsafe { DwmEnableBlurBehindWindow(h.hwnd.get() as HWND, &bb) };
        if hr < 0 {
            tracing::warn!("could not make the main window opaque: {hr:#x}");
        }
    }

    pub(crate) fn cloak(cc: &eframe::CreationContext<'_>) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = cc.window_handle() else { return };
        let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
        let hwnd = h.hwnd.get();
        if !set_cloak(hwnd, true) {
            // DWM unavailable (or cloaking refused): show the window normally.
            return;
        }
        let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
        *st = Some(Cloaked {
            hwnd,
            since: Instant::now(),
            frames: 0,
            stable_frames: 0,
            last_size: None,
            expect_maximized: false,
        });
    }

    /// The window is being maximized while cloaked; reveal only once it is.
    pub(crate) fn expect_maximized() {
        if let Some(c) = STATE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            c.expect_maximized = true;
        }
    }

    /// Call once per painted frame (from `App::ui`).
    pub(crate) fn reveal_when_ready(ctx: &egui::Context) {
        let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(c) = st.as_mut() else { return };
        let (size, maximized) = ctx.input(|i| {
            let vp = i.viewport();
            (vp.inner_rect.map(|r| r.size()), vp.maximized == Some(true))
        });
        let settled = size.is_some() && size == c.last_size && (maximized || !c.expect_maximized);
        c.stable_frames = if settled { c.stable_frames + 1 } else { 0 };
        c.last_size = size;
        c.frames += 1;
        if super::ready(c.frames, c.stable_frames, c.since.elapsed()) {
            set_cloak(c.hwnd, false);
            tracing::debug!("window revealed after {} frames, {:?}", c.frames, c.since.elapsed());
            *st = None;
        } else {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveals_after_settled_frames_or_timeout() {
        assert!(!ready(1, 1, Duration::ZERO));
        assert!(!ready(3, 1, Duration::from_millis(100)), "size still changing");
        assert!(ready(3, 2, Duration::from_millis(100)));
        assert!(ready(0, 0, MAX_CLOAK), "never stays hidden");
    }
}
