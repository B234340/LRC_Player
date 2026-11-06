//! Files dropped onto the window: audio files and folders are played (when idle) or queued
//! (while something is loaded); a `.lrc` attaches to the current track. Folders are expanded
//! on a worker thread so a large drop never stalls the UI.

use std::path::{Path, PathBuf};

use eframe::egui;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::GuiApp;

/// Extensions the decoder handles.
pub(crate) const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "wave", "ogg", "oga", "opus", "m4a", "mp4", "aac", "alac", "aif", "aiff", "caf", "mka", "webm",
];

pub(crate) fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

pub(crate) fn is_lrc(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("lrc"))
}

/// Audio files in `paths` (folders recursively, sorted by path within each folder).
pub(crate) fn expand_audio(paths: &[PathBuf]) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>, depth: usize) {
        if depth > 32 {
            return;
        }
        let Ok(read) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out, depth + 1);
            } else if is_audio(&p) {
                out.push(p.to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(p, &mut out, 0);
        } else if is_audio(p) {
            out.push(p.to_string_lossy().into_owned());
        }
    }
    out
}

impl GuiApp {
    pub(crate) fn handle_dropped_files(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let (lrcs, rest): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|p| is_lrc(p));
        if let Some(lrc) = lrcs.first() {
            self.attach_lrc(lrc);
        }
        if rest.is_empty() {
            return;
        }
        let idle = self.play_path.is_empty() || self.playback_state == PlaybackState::Stopped;
        let (tx, rx) = crossbeam_channel::bounded(1);
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name("drop-expand".into()).spawn(move || {
            let _ = tx.send(expand_audio(&rest));
            wake.request_repaint();
        });
        if spawned.is_ok() {
            self.ui.drop_rx = Some((idle, rx));
        }
    }

    pub(crate) fn poll_dropped(&mut self) {
        let Some((play_now, rx)) = &self.ui.drop_rx else { return };
        let play_now = *play_now;
        let files = match rx.try_recv() {
            Ok(files) => files,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => Vec::new(),
        };
        self.ui.drop_rx = None;
        let tr = self.tr();
        if files.is_empty() {
            self.toast_warning(tr.t("toast.drop_no_audio").to_string());
            return;
        }
        self.library.ensure_probed(files.iter());
        if play_now {
            let n = files.len();
            self.play_tracks(files, 0);
            self.toast_success(tr.f("toast.drop_playing", &[&n]));
        } else {
            self.enqueue_with_toast(files);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_folders_and_filters_audio() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("b")).unwrap();
        for f in ["a.flac", "b/c.MP3", "b/cover.jpg", "notes.txt", "x.lrc"] {
            std::fs::write(dir.path().join(f), b"").unwrap();
        }
        let got = expand_audio(&[dir.path().to_path_buf()]);
        assert_eq!(got.len(), 2);
        assert!(got[0].ends_with("a.flac"));
        assert!(got[1].ends_with("c.MP3"));
        assert!(is_lrc(Path::new("/x/Y.LRC")));
        assert!(!is_audio(Path::new("/x/y.lrc")));
    }
}
