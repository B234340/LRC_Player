//! Per-track auxiliary data (duration / tags probe + lyrics discovery) loaded off the UI
//! thread.
//!
//! Previously every track change decoded the whole file on the UI thread to learn its
//! duration (`refresh_play_metadata`) and parsed each candidate `.lrc` inside a sort
//! comparator. Now one worker thread runs `probe_track_info` (headers only) and
//! `Lyrics::discover_ranked` (each file parsed once) for the latest requested track; stale
//! requests are dropped ("latest wins") and the result is posted back with a repaint.

use std::sync::{Arc, Condvar, Mutex};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use lrc_player_application::infrastructure::decode::{probe_track_info, TrackInfo};
use lrc_player_application::infrastructure::lyrics::{Lyrics, LyricsCandidate};

use crate::presentation::app::GuiApp;

pub(crate) struct TrackInfoResult {
    pub(crate) path: String,
    pub(crate) info: Option<TrackInfo>,
    pub(crate) candidates: Vec<LyricsCandidate>,
}

struct Request {
    path: String,
    lrc_targets: Vec<String>,
}

#[derive(Default)]
struct Slot {
    request: Option<Request>,
    shutdown: bool,
}

pub(crate) struct TrackInfoLoader {
    slot: Arc<(Mutex<Slot>, Condvar)>,
    rx: Receiver<TrackInfoResult>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl TrackInfoLoader {
    pub(crate) fn new(ctx: egui::Context) -> Self {
        let slot: Arc<(Mutex<Slot>, Condvar)> = Arc::default();
        let (tx, rx) = crossbeam_channel::unbounded();
        let worker_slot = Arc::clone(&slot);
        let worker = std::thread::Builder::new()
            .name("track-info".into())
            .spawn(move || worker_loop(worker_slot, tx, ctx))
            .ok();
        Self { slot, rx, worker }
    }

    /// Replaces any not-yet-started request with this one.
    pub(crate) fn request(&self, path: String, lrc_targets: Vec<String>) {
        let (lock, cvar) = &*self.slot;
        if let Ok(mut slot) = lock.lock() {
            slot.request = Some(Request { path, lrc_targets });
            cvar.notify_one();
        }
    }

    pub(crate) fn try_recv(&self) -> Option<TrackInfoResult> {
        self.rx.try_recv().ok()
    }
}

impl Drop for TrackInfoLoader {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.slot;
        if let Ok(mut slot) = lock.lock() {
            slot.shutdown = true;
            slot.request = None;
            cvar.notify_one();
        }
        // Not joined: a long probe/decode must not delay exit; the worker stops at its next
        // queue check (or when its result channel is gone).
        drop(self.worker.take());
    }
}

fn worker_loop(slot: Arc<(Mutex<Slot>, Condvar)>, tx: Sender<TrackInfoResult>, ctx: egui::Context) {
    let (lock, cvar) = &*slot;
    loop {
        let request = {
            let Ok(mut guard) = lock.lock() else {
                return;
            };
            loop {
                if guard.shutdown {
                    return;
                }
                if let Some(req) = guard.request.take() {
                    break req;
                }
                guard = match cvar.wait(guard) {
                    Ok(g) => g,
                    Err(_) => return,
                };
            }
        };
        let info = probe_track_info(&request.path).ok();
        let candidates = Lyrics::discover_ranked(&request.path, &request.lrc_targets);
        if tx
            .send(TrackInfoResult {
                path: request.path,
                info,
                candidates,
            })
            .is_err()
        {
            return;
        }
        ctx.request_repaint();
    }
}

impl GuiApp {
    /// Asks the worker for duration/tags and lyrics of the current track.
    pub(crate) fn request_track_info(&mut self) {
        if self.play_path.is_empty() {
            self.lyrics_files.clear();
            self.lyrics_candidates.clear();
            self.selected_lyrics_index = None;
            self.loaded_lyrics = None;
            self.loaded_timeline = None;
            return;
        }
        self.track_info
            .request(self.play_path.clone(), self.scan_lrc_targets.clone());
    }

    /// Applies finished probe/lyrics results for the current track.
    pub(crate) fn poll_track_info(&mut self) {
        while let Some(result) = self.track_info.try_recv() {
            if result.path != self.play_path {
                continue;
            }
            if let Some(info) = result.info {
                if self.play_duration_ms.is_none() {
                    self.play_duration_ms = info.duration_ms;
                }
                if info.title.is_some() {
                    self.play_title = info.title;
                }
                if info.artist.is_some() {
                    self.play_artist = info.artist;
                }
                if info.album.is_some() {
                    self.play_album = info.album;
                }
                if let Some(d) = self.play_duration_ms {
                    if self.playback_state == lrc_player_domain::model::PlaybackState::Stopped {
                        self.play_position_ms = self.play_position_ms.min(d);
                    }
                }
            }
            self.set_lyrics_candidates(result.candidates);
        }
    }
}
