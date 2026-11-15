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
/// While the desktop-lyrics window animates, the main window yields: ~30 fps.
pub(crate) const FRAME_SHARED: Duration = Duration::from_micros(33_333);

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
    static SHARED: Cell<bool> = const { Cell::new(false) };
    /// Total time slept by [`pace`] (other windows' frames waited that long too; the overlay
    /// subtracts it from its lateness so it does not mistake our sleep for a busy thread).
    static SLEPT: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

/// The desktop-lyrics window is animating: cap the main window at [`FRAME_SHARED`].
pub(crate) fn set_shared(shared: bool) {
    SHARED.set(shared);
}

/// Total time slept so far on this thread.
pub(crate) fn slept_total() -> Duration {
    SLEPT.get()
}

/// Turns pacing on (the real app on Windows; tests never sleep).
pub(crate) fn enable() {
    ENABLED.set(true);
}

/// How long to wait before a pass starting at `now` (none for the first pass or after idle).
pub(crate) fn wait_before(last: Option<Instant>, now: Instant, frame: Duration) -> Duration {
    last.map_or(Duration::ZERO, |l| frame.saturating_sub(now.saturating_duration_since(l)))
}

/// Call at the start of every main-window pass.
pub(crate) fn pace() {
    if !ENABLED.get() {
        return;
    }
    let frame = if SHARED.get() { FRAME_SHARED } else { FRAME };
    let wait = wait_before(LAST.get(), Instant::now(), frame);
    if !wait.is_zero() {
        let t = Instant::now();
        std::thread::sleep(wait);
        SLEPT.set(SLEPT.get() + t.elapsed());
    }
    LAST.set(Some(Instant::now()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_only_for_the_rest_of_a_frame() {
        let t0 = Instant::now();
        assert_eq!(wait_before(None, t0, FRAME), Duration::ZERO);
        assert_eq!(wait_before(Some(t0), t0 + FRAME * 2, FRAME), Duration::ZERO, "after idle: no wait");
        let w = wait_before(Some(t0), t0 + Duration::from_millis(10), FRAME);
        assert!(w > Duration::from_millis(6) && w <= Duration::from_millis(7), "{w:?}");
        let w = wait_before(Some(t0), t0 + Duration::from_millis(10), FRAME_SHARED);
        assert!(w > Duration::from_millis(23) && w <= Duration::from_millis(24), "{w:?}");
    }
}
