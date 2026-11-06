//! OS media keys and the system "now playing" overlay via `souvlaki`:
//! Windows System Media Transport Controls (needs the window's HWND, taken from eframe via
//! `raw-window-handle`), macOS Now Playing / remote commands, and Linux/BSD MPRIS over
//! D-Bus (pure-Rust `zbus` backend, so no libdbus development package is needed).
//!
//! Controls are created lazily on the first frame (the HWND exists by then). Events arrive
//! on souvlaki's thread and are forwarded through a channel plus `request_repaint`, then
//! applied on the UI thread by [`GuiApp::poll_media_controls`]. Metadata and playback state
//! are pushed only when they change (or the position jumps, e.g. after a seek), never per
//! frame. Failure to register (no session bus, sandbox, ...) only logs a warning.

use std::path::Path;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui;
use lrc_player_domain::model::PlaybackState;
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};

use crate::presentation::app::{GuiApp, PlaybackView};

/// Step for "seek forward/backward" requests without an amount.
const SEEK_STEP: Duration = Duration::from_secs(10);
/// Re-send the position when it drifts this far from the extrapolated one (seek).
const POSITION_JUMP_MS: i64 = 1_500;

#[derive(Debug, Clone, PartialEq)]
struct MetaKey {
    title: String,
    artist: String,
    album: String,
    duration_ms: Option<u64>,
    cover: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PlayKind {
    Stopped,
    Paused,
    Playing,
}

pub(crate) struct MediaControlsBridge {
    enabled: bool,
    attempted: bool,
    controls: Option<MediaControls>,
    rx: Option<Receiver<MediaControlEvent>>,
    ctx: Option<egui::Context>,
    last_meta: Option<MetaKey>,
    last_play: Option<(PlayKind, u64, Instant)>,
}

/// `file://` URL for a local cover image.
fn file_url(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(target_os = "windows")]
fn hwnd_of(frame: &eframe::Frame) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn hwnd_of(_frame: &eframe::Frame) -> Option<*mut std::ffi::c_void> {
    None
}

impl MediaControlsBridge {
    pub(crate) fn new(enabled: bool) -> Self {
        Self { enabled, attempted: false, controls: None, rx: None, ctx: None, last_meta: None, last_play: None }
    }

    /// Creates and attaches the controls once (first frame).
    pub(crate) fn ensure_attached(&mut self, frame: &eframe::Frame, ctx: &egui::Context) {
        if !self.enabled || self.attempted {
            return;
        }
        self.attempted = true;
        self.ctx = Some(ctx.clone());
        let hwnd = hwnd_of(frame);
        if cfg!(target_os = "windows") && hwnd.is_none() {
            tracing::warn!("media controls: no HWND available");
            return;
        }
        let config = PlatformConfig { display_name: "LRC Player", dbus_name: "lrc_player", hwnd };
        let mut controls = match MediaControls::new(config) {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!("media controls unavailable: {err:?}");
                return;
            }
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let wake = ctx.clone();
        if let Err(err) = controls.attach(move |event| {
            let _ = tx.send(event);
            wake.request_repaint();
        }) {
            tracing::warn!("attaching media controls failed: {err:?}");
            return;
        }
        tracing::info!("media controls attached");
        self.controls = Some(controls);
        self.rx = Some(rx);
    }

    /// Controls are registered with the OS.
    pub(crate) fn is_active(&self) -> bool {
        self.controls.is_some()
    }

    fn events(&self) -> Vec<MediaControlEvent> {
        self.rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default()
    }

    /// Pushes metadata / playback state when they changed.
    pub(crate) fn sync(&mut self, view: &PlaybackView, cover: Option<&Path>) {
        let Some(controls) = self.controls.as_mut() else { return };
        let meta = MetaKey {
            title: view.title.clone().unwrap_or_else(|| super::library_index::file_title(&view.path)),
            artist: view.artist.clone().unwrap_or_default(),
            album: view.album.clone().unwrap_or_default(),
            duration_ms: view.duration_ms,
            cover: cover.map(file_url),
        };
        if view.path.is_empty() {
            if !matches!(self.last_play, Some((PlayKind::Stopped, _, _))) {
                let _ = controls.set_playback(MediaPlayback::Stopped);
                self.last_play = Some((PlayKind::Stopped, 0, Instant::now()));
            }
            return;
        }
        if self.last_meta.as_ref() != Some(&meta) {
            let result = controls.set_metadata(MediaMetadata {
                title: Some(&meta.title),
                artist: (!meta.artist.is_empty()).then_some(meta.artist.as_str()),
                album: (!meta.album.is_empty()).then_some(meta.album.as_str()),
                cover_url: meta.cover.as_deref(),
                duration: meta.duration_ms.map(Duration::from_millis),
            });
            if let Err(err) = result {
                tracing::debug!("media controls metadata: {err:?}");
            }
            self.last_meta = Some(meta);
            self.last_play = None;
        }
        let kind = match view.state {
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => PlayKind::Playing,
            PlaybackState::Paused => PlayKind::Paused,
            PlaybackState::Stopped => PlayKind::Stopped,
        };
        let now = Instant::now();
        let resend = match self.last_play {
            None => true,
            Some((k, pos, at)) => {
                let expected = if k == PlayKind::Playing { pos + at.elapsed().as_millis() as u64 } else { pos };
                k != kind || (view.position_ms as i64 - expected as i64).abs() > POSITION_JUMP_MS
            }
        };
        if resend {
            let progress = Some(MediaPosition(Duration::from_millis(view.position_ms)));
            let playback = match kind {
                PlayKind::Playing => MediaPlayback::Playing { progress },
                PlayKind::Paused => MediaPlayback::Paused { progress },
                PlayKind::Stopped => MediaPlayback::Stopped,
            };
            if let Err(err) = controls.set_playback(playback) {
                tracing::debug!("media controls playback: {err:?}");
            }
            self.last_play = Some((kind, view.position_ms, now));
        }
    }
}

impl GuiApp {
    /// Applies media key / overlay commands.
    pub(crate) fn poll_media_controls(&mut self) {
        for event in self.media.events() {
            match event {
                MediaControlEvent::Play => self.play(),
                MediaControlEvent::Pause => self.pause(),
                MediaControlEvent::Toggle => self.toggle_play_pause(),
                MediaControlEvent::Next => self.next(),
                MediaControlEvent::Previous => self.previous(),
                MediaControlEvent::Stop => self.stop(),
                MediaControlEvent::Seek(dir) => self.seek_media(dir, SEEK_STEP),
                MediaControlEvent::SeekBy(dir, by) => self.seek_media(dir, by),
                MediaControlEvent::SetPosition(MediaPosition(pos)) => self.seek_to(pos.as_millis() as u64),
                MediaControlEvent::SetVolume(v) => self.set_volume((v as f32).clamp(0.0, 1.0)),
                MediaControlEvent::Raise => {
                    if let Some(ctx) = &self.media.ctx {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
                MediaControlEvent::Quit => {
                    if let Some(ctx) = &self.media.ctx {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                MediaControlEvent::OpenUri(uri) => {
                    let path = uri.strip_prefix("file://").unwrap_or(&uri).to_string();
                    if super::drop::is_audio(Path::new(&path)) {
                        self.play_tracks(vec![path], 0);
                    }
                }
                _ => {}
            }
        }
    }

    fn seek_media(&mut self, dir: SeekDirection, by: Duration) {
        let ms = by.as_millis() as i64;
        self.seek_relative(if dir == SeekDirection::Forward { ms } else { -ms });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_urls_are_escaped() {
        assert_eq!(file_url(Path::new("/m/a b/c.jpg")), "file:///m/a%20b/c.jpg");
        assert_eq!(file_url(Path::new("C:\\M\\x.png")), "file:///C:/M/x.png");
        assert!(file_url(Path::new("/m/夜.png")).starts_with("file:///m/%E5"));
    }
}
