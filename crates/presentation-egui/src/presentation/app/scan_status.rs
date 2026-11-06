//! In-process scan status bus.
//!
//! The scan server runs in the GUI process (see `apps/gui`). It used to persist progress by
//! read-modify-writing `state.json`, and the GUI re-read and parsed that file every frame on
//! the Scan tab — racing with the GUI's own state saves. Now the server calls [`publish`] and
//! the GUI drains updates with [`try_recv`] once per frame (a lock-free channel poll); a
//! repaint is requested so updates show up even when the UI is idle.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use eframe::egui;

/// One scan status change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanStatusUpdate {
    pub status: String,
    pub scanned: Option<u64>,
    pub parsed: Option<u64>,
    pub registered: Option<u64>,
}

struct Bus {
    /// Pending updates; consecutive progress updates are coalesced (latest wins) so a long
    /// scan cannot pile up messages while the window is not repainting.
    queue: Mutex<VecDeque<ScanStatusUpdate>>,
    ctx: Mutex<Option<egui::Context>>,
}

fn bus() -> &'static Bus {
    static BUS: OnceLock<Bus> = OnceLock::new();
    BUS.get_or_init(|| Bus {
        queue: Mutex::new(VecDeque::new()),
        ctx: Mutex::new(None),
    })
}

fn is_progress(update: &ScanStatusUpdate) -> bool {
    update.status.starts_with("Scan progress")
}

/// Publishes a scan status (callable from any thread).
pub fn publish(status: impl Into<String>, scanned: Option<u64>, parsed: Option<u64>, registered: Option<u64>) {
    let bus = bus();
    let update = ScanStatusUpdate {
        status: status.into(),
        scanned,
        parsed,
        registered,
    };
    if let Ok(mut queue) = bus.queue.lock() {
        match queue.back_mut() {
            Some(last) if is_progress(last) && is_progress(&update) => *last = update,
            _ => queue.push_back(update),
        }
    }
    let ctx = bus.ctx.lock().ok().and_then(|g| g.clone());
    if let Some(ctx) = ctx {
        // Coalesced: a burst of progress updates causes at most ~10 repaints per second.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

pub(crate) fn set_repaint_context(ctx: egui::Context) {
    if let Ok(mut slot) = bus().ctx.lock() {
        *slot = Some(ctx);
    }
}

pub(crate) fn try_recv() -> Option<ScanStatusUpdate> {
    bus().queue.lock().ok()?.pop_front()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_updates_are_coalesced_but_completion_is_kept() {
        while try_recv().is_some() {}
        publish("Scan started", Some(0), Some(0), Some(0));
        for i in 0..100 {
            publish(format!("Scan progress: scanned={i}"), Some(i), None, None);
        }
        publish("Scan completed", None, None, None);
        let got: Vec<_> = std::iter::from_fn(try_recv).collect();
        assert_eq!(got.len(), 3);
        assert_eq!(got[1].scanned, Some(99));
        assert_eq!(got[2].status, "Scan completed");
    }
}
