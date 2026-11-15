//! Lyrics from LRCLIB for tracks that have none.
//!
//! Automatic when "Fetch lyrics online" is on (once per track and session, after the local
//! lyrics discovery found nothing), or on demand from the "Search online" button. The found
//! LRC is saved next to the audio file (`song.lrc`), or in the app's data folder when that is
//! not possible (then attached to the track), and loaded right away.
//!
//! The query building ([`track_query`]), saving ([`outcome_of`] / [`save_lrc`]) and installing
//! ([`GuiApp::install_fetched_lyrics`]) are shared with the bulk download (`lyric_batch.rs`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossbeam_channel::{Receiver, TryRecvError};
use eframe::egui;
use lrc_player_application::infrastructure::lyrics::{lrclib_fetch, Lyrics, LrclibQuery, LrclibResult};

use crate::presentation::app::GuiApp;

/// Outcome of a lookup (computed on the worker, including writing the file).
#[derive(Debug)]
pub(crate) enum FetchOutcome {
    Saved { path: PathBuf, word_sync: bool, attached: bool },
    Instrumental,
    PlainOnly,
    NotFound,
}

pub(crate) struct FetchJob {
    track: String,
    manual: bool,
    rx: Receiver<Result<FetchOutcome, String>>,
}

/// Folder for lyrics that cannot be written next to the audio.
pub(crate) fn data_lyrics_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("data").join("lyrics"))
}

/// File name for a downloaded LRC in the data folder (unique per audio path).
fn data_file_name(audio: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    audio.hash(&mut h);
    let stem: String = audio
        .file_stem()
        .map(|s| s.to_string_lossy().chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect())
        .unwrap_or_default();
    format!("{stem}.{:08x}.lrc", h.finish() as u32)
}

/// Writes `lrc` next to `audio` (never replacing an existing file), else into `fallback_dir`.
/// Returns the path and whether it is outside the audio folder (must be attached).
pub(crate) fn save_lrc(audio: &Path, lrc: &str, fallback_dir: Option<&Path>) -> Result<(PathBuf, bool), String> {
    let beside = audio.with_extension("lrc");
    let write_new = |p: &Path| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(p)?;
        f.write_all(lrc.as_bytes())
    };
    if write_new(&beside).is_ok() {
        return Ok((beside, false));
    }
    let dir = fallback_dir.ok_or_else(|| format!("cannot write {}", beside.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(data_file_name(audio));
    std::fs::write(&path, lrc).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((path, true))
}

/// LRCLIB query for a track: its title (the file name when there is no title tag), artist,
/// album and duration.
pub(crate) fn track_query(path: &str, title: Option<&str>, artist: Option<&str>, album: Option<&str>, duration_ms: Option<u64>) -> LrclibQuery {
    let stem = || Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    LrclibQuery {
        title: title.filter(|t| !t.trim().is_empty()).map_or_else(stem, str::to_owned),
        artist: artist.unwrap_or_default().to_owned(),
        album: album.unwrap_or_default().to_owned(),
        duration_ms,
    }
}

/// Turns a lookup result into an outcome; found synced lyrics are written with [`save_lrc`].
pub(crate) fn outcome_of(result: LrclibResult, audio: &Path, fallback_dir: Option<&Path>) -> Result<FetchOutcome, String> {
    match result {
        LrclibResult::Synced { lrc, word_sync } => {
            save_lrc(audio, &lrc, fallback_dir).map(|(path, attached)| FetchOutcome::Saved { path, word_sync, attached })
        }
        LrclibResult::Instrumental => Ok(FetchOutcome::Instrumental),
        LrclibResult::PlainOnly => Ok(FetchOutcome::PlainOnly),
        LrclibResult::NotFound => Ok(FetchOutcome::NotFound),
    }
}

impl GuiApp {
    pub(crate) fn lyrics_fetch_running(&self) -> bool {
        self.ui.fetch_job.as_ref().is_some_and(|j| j.track == self.play_path)
    }

    /// The query for the playing track (file name when there is no title tag).
    fn lyrics_query(&self) -> LrclibQuery {
        track_query(
            &self.play_path,
            self.play_title.as_deref(),
            self.play_artist.as_deref(),
            self.play_album.as_deref(),
            self.play_duration_ms,
        )
    }

    /// Where downloaded lyrics go when they cannot be written beside the audio (none without
    /// persistence, e.g. in tests).
    pub(crate) fn lyrics_fallback_dir(&self) -> Option<PathBuf> {
        if self.persist { data_lyrics_dir() } else { None }
    }

    /// Records a downloaded LRC for `track`: attached when it lives outside the audio folder,
    /// and loaded right away when `track` is playing.
    pub(crate) fn install_fetched_lyrics(&mut self, track: &str, path: &Path, attached: bool) {
        if attached {
            self.lyrics_overrides.insert(track.to_string(), path.to_string_lossy().into_owned());
            self.save_state();
        }
        if track == self.play_path {
            self.set_lyrics_candidates(Lyrics::rank_candidates([path.to_path_buf()]));
        }
    }

    /// Looks the playing track up on LRCLIB.
    pub(crate) fn start_lyrics_fetch(&mut self, ctx: &egui::Context, manual: bool) {
        if self.play_path.is_empty() || self.ui.fetch_job.is_some() {
            return;
        }
        self.ui.fetch_tried.insert(self.play_path.clone());
        let query = self.lyrics_query();
        let audio = PathBuf::from(&self.play_path);
        let fallback = self.lyrics_fallback_dir();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name("lyrics-fetch".into()).spawn(move || {
            let outcome = match lrclib_fetch(&query) {
                Ok(result) => outcome_of(result, &audio, fallback.as_deref()),
                Err(e) => Err(format!("{e:#}")),
            };
            let _ = tx.send(outcome);
            wake.request_repaint();
        });
        if spawned.is_ok() {
            self.ui.fetch_job = Some(FetchJob { track: self.play_path.clone(), manual, rx });
        }
    }

    /// Starts automatic lookups and applies finished ones (called every frame).
    pub(crate) fn poll_lyrics_fetch(&mut self, ctx: &egui::Context) {
        if self.ui.fetch_job.is_none() {
            let auto = self.config.fetch_lyrics_online
                && !self.play_path.is_empty()
                && self.lyrics_discovered_for == self.play_path
                && self.lyrics_candidates.is_empty()
                && !self.ui.fetch_tried.contains(&self.play_path);
            if auto {
                self.start_lyrics_fetch(ctx, false);
            }
            return;
        }
        let Some(job) = &self.ui.fetch_job else { return };
        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(300));
                return;
            }
            Err(TryRecvError::Disconnected) => Err("worker stopped".to_string()),
        };
        let job = self.ui.fetch_job.take().expect("checked");
        self.apply_lyrics_fetch(&job.track, job.manual, result);
    }

    pub(crate) fn apply_lyrics_fetch(&mut self, track: &str, manual: bool, result: Result<FetchOutcome, String>) {
        let tr = self.tr();
        match result {
            Ok(FetchOutcome::Saved { path, word_sync, attached }) => {
                self.install_fetched_lyrics(track, &path, attached);
                let key = if word_sync { "toast.lyrics_fetched_words" } else { "toast.lyrics_fetched" };
                self.toast_success(tr.t(key).to_string());
            }
            Ok(other) => {
                tracing::info!("LRCLIB: {other:?} for {track}");
                if manual {
                    let key = match other {
                        FetchOutcome::Instrumental => "toast.lyrics_fetch_instrumental",
                        FetchOutcome::PlainOnly => "toast.lyrics_fetch_plain_only",
                        _ => "toast.lyrics_fetch_not_found",
                    };
                    self.toast_info(tr.t(key).to_string());
                }
            }
            Err(err) => {
                tracing::warn!("LRCLIB lookup failed: {err}");
                if manual {
                    self.toast_error(tr.f("toast.lyrics_fetch_failed", &[&err]));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_falls_back_to_the_file_name() {
        let q = track_query("/m/01 Song.flac", Some("  "), Some("Artist"), None, Some(200_000));
        assert_eq!(q, LrclibQuery { title: "01 Song".into(), artist: "Artist".into(), album: String::new(), duration_ms: Some(200_000) });
        assert_eq!(track_query("/m/x.flac", Some("Title"), None, Some("Album"), None).title, "Title");
    }

    #[test]
    fn saves_beside_the_audio_without_replacing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("song.flac");
        let (path, attached) = save_lrc(&audio, "[00:01.00]a\n", None).unwrap();
        assert_eq!(path, dir.path().join("song.lrc"));
        assert!(!attached);
        // An existing .lrc is never overwritten: the copy goes to the fallback folder.
        let fallback = dir.path().join("data");
        let (path2, attached2) = save_lrc(&audio, "[00:02.00]b\n", Some(&fallback)).unwrap();
        assert!(attached2 && path2.starts_with(&fallback));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[00:01.00]a\n");
        assert!(path2.file_name().unwrap().to_string_lossy().starts_with("song."));
        // No fallback: an error.
        assert!(save_lrc(&audio, "x", None).is_err());
    }

    #[test]
    fn fetched_lyrics_are_loaded_and_attached() {
        use crate::presentation::app::tests::test_app;
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        app.play_path = dir.path().join("song.flac").to_string_lossy().into_owned();
        let elsewhere = dir.path().join("data").join("song.1234.lrc");
        std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        std::fs::write(&elsewhere, "[00:01.00]<00:01.00>Hel<00:01.50>lo<00:02.00>\n").unwrap();
        let track = app.play_path.clone();
        app.apply_lyrics_fetch(&track, false, Ok(FetchOutcome::Saved { path: elsewhere.clone(), word_sync: true, attached: true }));
        assert_eq!(app.lyrics_overrides.get(&track).map(PathBuf::from), Some(elsewhere));
        let line = &app.loaded_timeline.as_ref().unwrap().lines()[0];
        assert!(line.is_karaoke() && !line.estimated);
        assert!(!app.ui.toasts.is_empty());
    }

    #[test]
    fn automatic_lookup_is_opt_in_and_once_per_track() {
        use crate::presentation::app::tests::test_app;
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        app.play_path = dir.path().join("song.flac").to_string_lossy().into_owned();
        app.lyrics_discovered_for = app.play_path.clone();
        assert!(!app.config.fetch_lyrics_online);
        app.poll_lyrics_fetch(&ctx);
        assert!(app.ui.fetch_job.is_none(), "off by default");
        // Mark as tried: no lookup even with the setting on (no network in this test).
        app.config.fetch_lyrics_online = true;
        app.ui.fetch_tried.insert(app.play_path.clone());
        app.poll_lyrics_fetch(&ctx);
        assert!(app.ui.fetch_job.is_none(), "once per track and session");
        // Discovery not finished yet: no lookup.
        app.ui.fetch_tried.clear();
        app.lyrics_discovered_for.clear();
        app.poll_lyrics_fetch(&ctx);
        assert!(app.ui.fetch_job.is_none());
    }
}
