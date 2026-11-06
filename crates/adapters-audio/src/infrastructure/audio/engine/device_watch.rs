//! When the engine thread may probe the audio system for device changes.
//!
//! Probing is never free: on Linux every failed attempt to open the ALSA `default` PCM
//! re-evaluates the ALSA configuration, scans `/dev/snd`/`/dev/aload*` and prints a burst of
//! `ALSA lib ...` lines to stderr; enumerating devices through cpal opens *every* PCM of the
//! system (each one possibly a new sound-server connection). So:
//!
//! - with no output device, retries back off exponentially (`base`, 2x, 4x, ... capped at
//!   [`MAX_RETRY_INTERVAL`]); a user action (load/play) may cut the wait short, but never below
//!   `base` since the previous attempt;
//! - with a device open, the only periodic checks are the ones that can detect something on
//!   this host (see [`PresentPolicy`]); stream errors and the stall watchdog trigger reopening
//!   independently of this schedule.

use std::time::{Duration, Instant};

/// Upper bound for the retry interval while no output device can be opened.
pub(crate) const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// Lower bound for the configured poll interval (`EngineConfig::device_poll_ms`).
pub(crate) const MIN_BASE_INTERVAL: Duration = Duration::from_millis(50);

/// How often an ALSA host looks for a missing preferred device to come back (name hints only,
/// no PCM is opened).
pub(crate) const ALSA_PREFERRED_RECHECK: Duration = Duration::from_secs(10);

/// What an open output needs to be checked for periodically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PresentPolicy {
    /// Watch for the system default device to change (only meaningful on hosts whose
    /// "default device" is a concrete device that can change, i.e. WASAPI/CoreAudio; on
    /// ALSA it is the constant `default` PCM and the sound server reroutes it by itself).
    pub follow_default: bool,
    /// The preferred device is missing and the engine fell back to the default: watch for
    /// it to reappear.
    pub preferred_missing: bool,
    /// Host where device probes are cheap enough to run at the base interval.
    pub cheap_probes: bool,
}

impl PresentPolicy {
    /// Interval of the periodic check, or `None` when there is nothing to check.
    pub(crate) fn interval(&self, base: Duration) -> Option<Duration> {
        match (self.follow_default, self.preferred_missing, self.cheap_probes) {
            (_, _, true) if self.follow_default || self.preferred_missing => Some(base),
            (_, true, false) => Some(base.max(ALSA_PREFERRED_RECHECK)),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct DeviceWatch {
    base: Duration,
    /// Consecutive failed attempts to open an output.
    failures: u32,
    last_attempt: Option<Instant>,
    /// When the next probe is due; `None` = nothing scheduled.
    next_due: Option<Instant>,
}

impl DeviceWatch {
    pub(crate) fn new(poll_ms: u32) -> Self {
        Self {
            base: Duration::from_millis(poll_ms as u64).max(MIN_BASE_INTERVAL),
            failures: 0,
            last_attempt: None,
            next_due: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn base(&self) -> Duration {
        self.base
    }

    /// Delay before the next attempt after `failures` consecutive failed opens (>= 1).
    pub(crate) fn retry_interval(base: Duration, failures: u32) -> Duration {
        let shift = failures.saturating_sub(1).min(16);
        base.saturating_mul(1u32 << shift).min(MAX_RETRY_INTERVAL.max(base))
    }

    pub(crate) fn failures(&self) -> u32 {
        self.failures
    }

    #[cfg(test)]
    pub(crate) fn next_due(&self) -> Option<Instant> {
        self.next_due
    }

    pub(crate) fn is_due(&self, now: Instant) -> bool {
        self.next_due.is_some_and(|t| now >= t)
    }

    /// Time until the next probe (`None` when nothing is scheduled).
    pub(crate) fn until_due(&self, now: Instant) -> Option<Duration> {
        self.next_due.map(|t| t.saturating_duration_since(now))
    }

    /// An attempt to open an output failed: schedule the next one with backoff.
    pub(crate) fn open_failed(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        self.last_attempt = Some(now);
        self.next_due = Some(now + Self::retry_interval(self.base, self.failures));
    }

    /// An output was opened: reset the backoff and schedule the periodic check (if any).
    pub(crate) fn opened(&mut self, now: Instant, policy: PresentPolicy) {
        self.failures = 0;
        self.last_attempt = Some(now);
        self.rearm(now, policy);
    }

    /// A periodic check of an open output ran: schedule the next one.
    pub(crate) fn rearm(&mut self, now: Instant, policy: PresentPolicy) {
        self.next_due = policy.interval(self.base).map(|i| now + i);
    }

    /// The user asked for audio (load/play) while no output is open: retry soon, restarting
    /// the backoff, but not earlier than `base` after the previous attempt.
    pub(crate) fn expedite(&mut self, now: Instant) {
        if self.failures == 0 {
            return;
        }
        self.failures = 0;
        let earliest = self.last_attempt.map_or(now, |t| (t + self.base).max(now));
        self.next_due = Some(self.next_due.map_or(earliest, |d| d.min(earliest)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn absent() -> PresentPolicy {
        PresentPolicy {
            follow_default: false,
            preferred_missing: false,
            cheap_probes: true,
        }
    }

    #[test]
    fn retry_schedule_doubles_and_caps_at_30s() {
        let got: Vec<u64> = (1..=9)
            .map(|n| DeviceWatch::retry_interval(MS(1000), n).as_millis() as u64)
            .collect();
        assert_eq!(
            got,
            [1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000, 30000]
        );
        // Huge failure counts neither overflow nor exceed the cap.
        assert_eq!(DeviceWatch::retry_interval(MS(1000), u32::MAX), MAX_RETRY_INTERVAL);
        // A base above the cap is honoured as is (the user asked for slow polling).
        assert_eq!(
            DeviceWatch::retry_interval(MS(60_000), 3),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn base_is_clamped() {
        assert_eq!(DeviceWatch::new(0).base(), MIN_BASE_INTERVAL);
        assert_eq!(DeviceWatch::new(1000).base(), MS(1000));
    }

    /// Simulates an engine that keeps failing to open a device for 10 minutes and counts the
    /// probes: after the backoff ramp there is at most one probe per 30 s.
    #[test]
    fn no_device_probes_are_rate_limited() {
        let t0 = Instant::now();
        let mut w = DeviceWatch::new(1000);
        // The engine's initial open at start-up failed.
        w.open_failed(t0);
        let mut attempts = vec![Duration::ZERO];
        let mut now = t0;
        let end = t0 + Duration::from_secs(600);
        // Wake every 100 ms like a (busy) engine loop would at worst.
        while now < end {
            now += MS(100);
            if w.is_due(now) {
                attempts.push(now - t0);
                w.open_failed(now);
            }
        }
        let secs: Vec<u64> = attempts.iter().take(7).map(|d| d.as_secs()).collect();
        assert_eq!(secs, [0, 1, 3, 7, 15, 31, 61]);
        // 0,1,3,7,15,31 then every 30 s up to 600 s.
        assert_eq!(attempts.len(), 6 + (600 - 31) / 30);
        for pair in attempts.windows(2).skip(5) {
            assert!(pair[1] - pair[0] >= MAX_RETRY_INTERVAL);
        }
    }

    #[test]
    fn success_resets_backoff_and_stops_polling_when_nothing_to_watch() {
        let t0 = Instant::now();
        let mut w = DeviceWatch::new(1000);
        for i in 0..5 {
            w.open_failed(t0 + Duration::from_secs(i));
        }
        assert_eq!(w.failures(), 5);
        w.opened(t0 + Duration::from_secs(10), absent());
        assert_eq!(w.failures(), 0);
        assert_eq!(w.next_due(), None);
        assert!(!w.is_due(t0 + Duration::from_secs(3600)));
        // Losing the device again starts the ramp from the base interval.
        let t1 = t0 + Duration::from_secs(20);
        w.open_failed(t1);
        assert_eq!(w.until_due(t1), Some(MS(1000)));
    }

    #[test]
    fn present_policy_intervals() {
        let base = MS(1000);
        let p = |follow_default, preferred_missing, cheap_probes| PresentPolicy {
            follow_default,
            preferred_missing,
            cheap_probes,
        };
        // WASAPI/CoreAudio: follow the default device at the base interval.
        assert_eq!(p(true, false, true).interval(base), Some(base));
        assert_eq!(p(true, true, true).interval(base), Some(base));
        assert_eq!(p(false, true, true).interval(base), Some(base));
        // Using the preferred device: nothing to poll (errors trigger reopening).
        assert_eq!(p(false, false, true).interval(base), None);
        // ALSA: never poll the (constant) default; preferred-device return only slowly.
        assert_eq!(p(true, false, false).interval(base), None);
        assert_eq!(p(false, false, false).interval(base), None);
        assert_eq!(
            p(true, true, false).interval(base),
            Some(ALSA_PREFERRED_RECHECK)
        );
    }

    #[test]
    fn expedite_restarts_backoff_but_respects_base() {
        let t0 = Instant::now();
        let mut w = DeviceWatch::new(1000);
        for _ in 0..8 {
            w.open_failed(t0);
        }
        assert_eq!(w.until_due(t0), Some(MAX_RETRY_INTERVAL));
        // Right after the failed attempt: wait at least `base`.
        w.expedite(t0 + MS(200));
        assert_eq!(w.next_due(), Some(t0 + MS(1000)));
        assert_eq!(w.failures(), 0);
        // The next failure ramps from the start again.
        w.open_failed(t0 + MS(1000));
        assert_eq!(w.until_due(t0 + MS(1000)), Some(MS(1000)));
        // Long after the last attempt: due immediately.
        w.open_failed(t0 + MS(2000));
        let later = t0 + Duration::from_secs(5);
        w.expedite(later);
        assert!(w.is_due(later));
        // No-op while a device is open (no failures).
        let mut ok = DeviceWatch::new(1000);
        ok.opened(t0, absent());
        ok.expedite(t0);
        assert_eq!(ok.next_due(), None);
    }
}
