//! Thread priority helpers.
//!
//! cpal 0.15.3's WASAPI backend tries to boost its render thread but passes a thread *id* where
//! a handle is required, so the boost silently fails. We therefore promote the callback thread
//! ourselves on its first callback:
//! - Windows: MMCSS "Pro Audio" task (AvSetMmThreadCharacteristicsW + AVRT_PRIORITY_HIGH) and
//!   THREAD_PRIORITY_TIME_CRITICAL on the pseudo-handle from GetCurrentThread(); the MMCSS
//!   registration is reverted on that thread when the callback closure is dropped.
//! - macOS: CoreAudio's IO thread is already realtime; nothing to do.
//! - Linux/other unix: best-effort SCHED_FIFO (succeeds only with rtkit/RLIMIT_RTPRIO; failure
//!   is ignored).
//!
//! We call `windows-sys` directly instead of using the `audio_thread_priority` crate: that crate
//! pulls `libdbus-sys` on Linux (needs system libdbus headers for the build) and adds nothing we
//! need beyond these three calls; `windows-sys` cross-compiles cleanly for
//! x86_64-pc-windows-gnu (import libraries come from `windows-link`/`windows_x86_64_gnu`).

/// Undoes the audio-thread promotion when dropped *on the promoted thread* (Windows: reverts
/// the MMCSS registration with `AvRevertMmThreadCharacteristics`; elsewhere a no-op). Keep it
/// in the stream callback closure: cpal's WASAPI backend drops the closure on its render
/// thread when the stream ends.
#[derive(Debug, Default)]
pub(crate) struct AudioThreadGuard {
    #[cfg(windows)]
    mmcss: usize,
    #[cfg(windows)]
    thread_id: u32,
}

impl Drop for AudioThreadGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        win::revert(self.mmcss, self.thread_id);
    }
}

/// Promotes the calling (audio callback) thread. Call once, from the callback thread, and keep
/// the returned guard alive for as long as the thread renders audio.
#[must_use]
pub(crate) fn promote_audio_thread() -> AudioThreadGuard {
    #[cfg(windows)]
    {
        let (mmcss, thread_id) = win::promote_audio_thread();
        AudioThreadGuard { mmcss, thread_id }
    }
    #[cfg(not(windows))]
    {
        #[cfg(unix)]
        #[cfg(not(target_os = "macos"))]
        unix::promote_audio_thread();
        AudioThreadGuard::default()
    }
}

/// Raises the calling (decoder/engine) thread slightly above normal priority.
pub(crate) fn promote_decoder_thread() {
    #[cfg(windows)]
    win::promote_decoder_thread();
    #[cfg(all(unix, not(target_os = "macos")))]
    unix::promote_decoder_thread();
}

/// Lowers the calling (background analysis) thread below normal priority so it never competes
/// with decoding/rendering: Windows background mode (low CPU, I/O and memory priority; falls
/// back to `THREAD_PRIORITY_LOWEST`), nice +10 on Linux/BSD. Best effort.
pub(crate) fn demote_background_thread() {
    #[cfg(windows)]
    win::demote_background_thread();
    #[cfg(all(unix, not(target_os = "macos")))]
    unix::demote_background_thread();
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, AvSetMmThreadPriority,
        GetCurrentThread, GetCurrentThreadId, SetThreadPriority, AVRT_PRIORITY_HIGH,
        THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_TIME_CRITICAL,
    };

    /// Returns the MMCSS handle (0 if registration failed) and the current thread id.
    pub(super) fn promote_audio_thread() -> (usize, u32) {
        // "Pro Audio" as a NUL-terminated UTF-16 string.
        const TASK: [u16; 10] = [
            b'P' as u16,
            b'r' as u16,
            b'o' as u16,
            b' ' as u16,
            b'A' as u16,
            b'u' as u16,
            b'd' as u16,
            b'i' as u16,
            b'o' as u16,
            0,
        ];
        // SAFETY: plain Win32 calls on the current thread with valid arguments.
        unsafe {
            let mut task_index: u32 = 0;
            let h = AvSetMmThreadCharacteristicsW(TASK.as_ptr(), &mut task_index);
            if !h.is_null() {
                AvSetMmThreadPriority(h, AVRT_PRIORITY_HIGH);
            }
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
            (h as usize, GetCurrentThreadId())
        }
    }

    pub(super) fn revert(mmcss: usize, thread_id: u32) {
        // SAFETY: the handle came from AvSetMmThreadCharacteristicsW on `thread_id`; MMCSS
        // requires the revert to happen on that same thread, which is checked here.
        unsafe {
            if mmcss != 0 && GetCurrentThreadId() == thread_id {
                AvRevertMmThreadCharacteristics(mmcss as _);
            }
        }
    }

    pub(super) fn promote_decoder_thread() {
        // SAFETY: plain Win32 call on the current thread's pseudo handle.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        }
    }

    pub(super) fn demote_background_thread() {
        use windows_sys::Win32::System::Threading::{THREAD_MODE_BACKGROUND_BEGIN, THREAD_PRIORITY_LOWEST};
        // SAFETY: plain Win32 calls on the current thread's pseudo handle (background mode can
        // only be entered for the calling thread, which this is).
        unsafe {
            if SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) == 0 {
                SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
            }
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod unix {
    pub(super) fn promote_audio_thread() {
        // SAFETY: pthread_self() is always valid for the calling thread; param is initialised.
        unsafe {
            let min = libc::sched_get_priority_min(libc::SCHED_FIFO);
            let max = libc::sched_get_priority_max(libc::SCHED_FIFO);
            if min < 0 || max < 0 {
                return;
            }
            let param = libc::sched_param {
                sched_priority: (min + 10).min(max),
            };
            let _ = libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &param);
        }
    }

    pub(super) fn promote_decoder_thread() {
        // Lowering the nice value needs CAP_SYS_NICE; failure is harmless.
        // SAFETY: setpriority on the calling thread (tid 0 = caller for PRIO_PROCESS on Linux).
        unsafe {
            let _ = libc::setpriority(libc::PRIO_PROCESS, 0, -4);
        }
    }

    pub(super) fn demote_background_thread() {
        // Raising the nice value needs no privileges.
        // SAFETY: setpriority on the calling thread (tid 0 = caller for PRIO_PROCESS on Linux).
        unsafe {
            let _ = libc::setpriority(libc::PRIO_PROCESS, 0, 10);
        }
    }
}
