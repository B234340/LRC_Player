//! Frame-rate cap for the main window when vsync is off (Windows).
//!
//! eframe draws every window on one thread, and with vsync each window's buffer swap waits for
//! the next display refresh. With the desktop-lyrics window animating, the two windows then
//! take turns on the refresh: the main window gets about half the frames and its input
//! handling stalls in `SwapBuffers`. On Windows the binary turns vsync off (the DWM compositor
//! never shows torn frames of a windowed app) and the main window is paced here instead: a
//! pass that comes sooner than [`FRAME`] after the previous one sleeps for the remainder. The
//! desktop-lyrics window paces itself with `request_repaint_after`.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// Minimum time between two main-window passes (~60 fps).
pub(crate) const FRAME: Duration = Duration::from_micros(16_667);

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Turns pacing on (the real app on Windows; tests never sleep).
pub(crate) fn enable() {
    ENABLED.set(true);
}

/// How long to wait before a pass starting at `now` (none for the first pass or after idle).
pub(crate) fn wait_before(last: Option<Instant>, now: Instant) -> Duration {
    last.map_or(Duration::ZERO, |l| FRAME.saturating_sub(now.saturating_duration_since(l)))
}

/// Call at the start of every main-window pass.
pub(crate) fn pace() {
    if !ENABLED.get() {
        return;
    }
    let wait = wait_before(LAST.get(), Instant::now());
    if !wait.is_zero() {
        std::thread::sleep(wait);
    }
    LAST.set(Some(Instant::now()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_only_for_the_rest_of_a_frame() {
        let t0 = Instant::now();
        assert_eq!(wait_before(None, t0), Duration::ZERO);
        assert_eq!(wait_before(Some(t0), t0 + FRAME * 2), Duration::ZERO, "after idle: no wait");
        let w = wait_before(Some(t0), t0 + Duration::from_millis(10));
        assert!(w > Duration::from_millis(6) && w <= Duration::from_millis(7), "{w:?}");
    }
}
