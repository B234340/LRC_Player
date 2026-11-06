//! Thread priority helpers.
//!
//! cpal 0.15.3's WASAPI backend tries to boost its render thread but passes a thread *id* where
//! a handle is required, so the boost silently fails. We therefore promote the callback thread
//! ourselves on its first callback:
//! - Windows: MMCSS "Pro Audio" task (AvSetMmThreadCharacteristicsW + AVRT_PRIORITY_HIGH) and
//!   THREAD_PRIORITY_TIME_CRITICAL on the pseudo-handle from GetCurrentThread().
//! - macOS: CoreAudio's IO thread is already realtime; nothing to do.
//! - Linux/other unix: best-effort SCHED_FIFO (succeeds only with rtkit/RLIMIT_RTPRIO; failure
//!   is ignored).
//!
//! We call `windows-sys` directly instead of using the `audio_thread_priority` crate: that crate
//! pulls `libdbus-sys` on Linux (needs system libdbus headers for the build) and adds nothing we
//! need beyond these three calls; `windows-sys` cross-compiles cleanly for
//! x86_64-pc-windows-gnu (import libraries come from `windows-link`/`windows_x86_64_gnu`).

/// Promotes the calling (audio callback) thread. Call once, from the callback thread.
pub(crate) fn promote_audio_thread() {
    #[cfg(windows)]
    win::promote_audio_thread();
    #[cfg(all(unix, not(target_os = "macos")))]
    unix::promote_audio_thread();
}

/// Raises the calling (decoder/engine) thread slightly above normal priority.
pub(crate) fn promote_decoder_thread() {
    #[cfg(windows)]
    win::promote_decoder_thread();
    #[cfg(all(unix, not(target_os = "macos")))]
    unix::promote_decoder_thread();
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::System::Threading::{
        AvSetMmThreadCharacteristicsW, AvSetMmThreadPriority, GetCurrentThread, SetThreadPriority,
        AVRT_PRIORITY_HIGH, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_TIME_CRITICAL,
    };

    pub(super) fn promote_audio_thread() {
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
        }
    }

    pub(super) fn promote_decoder_thread() {
        // SAFETY: plain Win32 call on the current thread's pseudo handle.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
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
}
