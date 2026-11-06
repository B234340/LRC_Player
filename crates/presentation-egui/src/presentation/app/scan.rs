//! Library folders and scanning: scan / cancel requests to the in-process scan server (IPC),
//! the native folder picker (run off the UI thread), and scan status updates.

use std::path::PathBuf;

use lrc_player_shared::ipc as ipc_mod;

use crate::presentation::app::{scan_status, GuiApp};

/// Sends a scan request without blocking the UI thread.
fn send_request(request: ipc_mod::ScanRequest) {
    let socket = ipc_mod::default_socket_path();
    std::thread::spawn(move || {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
            if let Err(err) = rt.block_on(async move { ipc_mod::send_scan_request(&socket, &request).await }) {
                tracing::warn!("sending the scan request failed: {err:#}");
                scan_status::publish(format!("Scan failed: {err}"), None, None, None);
            }
        }
    });
}

/// Kind of a scan status line (the server publishes English strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanPhase {
    Requested,
    Started,
    Progress,
    Completed,
    Cancelled,
    Failed,
    Other,
}

pub(crate) fn scan_phase(status: &str) -> ScanPhase {
    if status.starts_with("Scan progress") {
        ScanPhase::Progress
    } else if status == "Scan started" {
        ScanPhase::Started
    } else if status == "Scan completed" {
        ScanPhase::Completed
    } else if status.starts_with("Requested scan cancel") || status.starts_with("Scan cancellation") {
        ScanPhase::Cancelled
    } else if status.starts_with("Requested scan") {
        ScanPhase::Requested
    } else if status.starts_with("Scan failed") || status.starts_with("Cancel failed") {
        ScanPhase::Failed
    } else {
        ScanPhase::Other
    }
}

impl GuiApp {
    /// Asks the scan server to (re)scan all library and lyrics folders.
    pub(crate) fn request_scan(&mut self) {
        let tr = self.tr();
        if self.scan_targets.is_empty() && self.scan_lrc_targets.is_empty() {
            self.toast_warning(tr.t("toast.no_folders").to_string());
            return;
        }
        self.last_scan_status = Some("Requested scan".to_string());
        self.scanning = true;
        self.save_state();
        send_request(ipc_mod::ScanRequest {
            paths: self.scan_targets.clone(),
            lrc_paths: self.scan_lrc_targets.clone(),
            excludes: self.scan_excludes.clone(),
            cancel: false,
        });
    }

    pub(crate) fn request_scan_cancel(&mut self) {
        self.last_scan_status = Some("Requested scan cancel".to_string());
        self.save_state();
        send_request(ipc_mod::ScanRequest {
            paths: Vec::new(),
            lrc_paths: Vec::new(),
            excludes: self.scan_excludes.clone(),
            cancel: true,
        });
    }

    /// Opens the native folder dialog on a worker thread (`lyrics` = lyrics folder).
    pub(crate) fn pick_folder(&mut self, ctx: &eframe::egui::Context, lyrics: bool) {
        if self.ui.folder_picker.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let wake = ctx.clone();
        let title = self.tr().t(if lyrics { "folders.pick_lyrics" } else { "folders.pick_music" });
        let spawned = std::thread::Builder::new().name("folder-picker".into()).spawn(move || {
            // The async dialog is driven on this thread; on macOS rfd dispatches the panel to
            // the main thread itself, elsewhere it runs here (XDG portal / Win32).
            let picked = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()
                .and_then(|rt| rt.block_on(rfd::AsyncFileDialog::new().set_title(title).pick_folder()))
                .map(|h| h.path().to_path_buf());
            let _ = tx.send(picked);
            wake.request_repaint();
        });
        if spawned.is_ok() {
            self.ui.folder_picker = Some((lyrics, rx));
        }
    }

    pub(crate) fn poll_folder_picker(&mut self) {
        let Some((lyrics, rx)) = &self.ui.folder_picker else { return };
        let lyrics = *lyrics;
        let picked = match rx.try_recv() {
            Ok(p) => p,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => None,
        };
        self.ui.folder_picker = None;
        if let Some(dir) = picked {
            self.add_folder(dir, lyrics);
        }
    }

    /// Adds a folder and starts a scan.
    pub(crate) fn add_folder(&mut self, dir: PathBuf, lyrics: bool) {
        let path = dir.to_string_lossy().into_owned();
        let tr = self.tr();
        let list = if lyrics { &mut self.scan_lrc_targets } else { &mut self.scan_targets };
        if list.contains(&path) {
            self.toast_info(tr.f("toast.folder_exists", &[&path]));
            return;
        }
        list.push(path.clone());
        self.save_state();
        self.toast_success(tr.f("toast.folder_added", &[&path]));
        self.request_scan();
    }

    pub(crate) fn remove_folder(&mut self, dir: &str) {
        let before = self.scan_targets.len() + self.scan_lrc_targets.len();
        self.scan_targets.retain(|p| p != dir);
        self.scan_lrc_targets.retain(|p| p != dir);
        if self.scan_targets.len() + self.scan_lrc_targets.len() != before {
            self.save_state();
            let tr = self.tr();
            self.toast_info(tr.f("toast.folder_removed", &[&dir]));
        }
    }

    /// Applies scan status updates published in-process by the scan server.
    pub(crate) fn poll_scan_status(&mut self) {
        let mut completed = false;
        while let Some(update) = scan_status::try_recv() {
            let phase = scan_phase(&update.status);
            let tr = self.tr();
            match phase {
                ScanPhase::Started | ScanPhase::Progress | ScanPhase::Requested => self.scanning = true,
                ScanPhase::Completed => {
                    self.scanning = false;
                    completed = true;
                }
                ScanPhase::Failed => {
                    self.scanning = false;
                    self.toast_error(tr.f("toast.scan_failed", &[&update.status]));
                }
                ScanPhase::Cancelled => {}
                ScanPhase::Other => {
                    if update.status.starts_with("No active scan") {
                        self.scanning = false;
                    }
                }
            }
            if update.scanned.is_some() || update.parsed.is_some() || update.registered.is_some() {
                self.scan_counts = (update.scanned, update.parsed, update.registered);
            }
            self.last_scan_status = Some(update.status);
            self.save_state();
        }
        if completed {
            let tr = self.tr();
            let n = self.scan_counts.2.or(self.scan_counts.0).unwrap_or(0);
            self.toast_success(tr.f("toast.scan_completed", &[&n]));
            self.on_library_changed();
            self.request_track_info();
        }
    }

    /// Reloads library-derived data after a scan or a library clear.
    pub(crate) fn on_library_changed(&mut self) {
        self.artwork.clear();
        if let Some(meta) = &self.meta {
            let path = meta.path().to_path_buf();
            self.library.reload(&path);
        }
    }

    pub(crate) fn poll_library(&mut self) {
        if self.library.poll() && self.play_title.is_none() && !self.play_path.is_empty() {
            if let Some(i) = self.library.row_index(std::path::Path::new(&self.play_path)) {
                let row = &self.library.rows[i];
                self.play_title = Some(row.title.clone());
                self.play_artist = Some(row.artist.clone()).filter(|a| !a.is_empty());
                self.play_album = Some(row.album.clone()).filter(|a| !a.is_empty());
            }
        }
    }

    /// Removes every track from the library database.
    pub(crate) fn clear_library(&mut self) {
        let tr = self.tr();
        let result = match self.meta.as_ref() {
            Some(meta) => meta.with(|cache| cache.clear_library()),
            None => return,
        };
        match result {
            Ok(()) => {
                self.toast_info(tr.t("toast.library_cleared").to_string());
                self.on_library_changed();
            }
            Err(err) => self.toast_error(tr.f("toast.library_clear_failed", &[&format!("{err:#}")])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_status_strings_map_to_phases() {
        assert_eq!(scan_phase("Scan progress: scanned=1 parsed=0 registered=0"), ScanPhase::Progress);
        assert_eq!(scan_phase("Scan started"), ScanPhase::Started);
        assert_eq!(scan_phase("Scan completed"), ScanPhase::Completed);
        assert_eq!(scan_phase("Requested scan"), ScanPhase::Requested);
        assert_eq!(scan_phase("Requested scan cancel"), ScanPhase::Cancelled);
        assert_eq!(scan_phase("Scan failed: boom"), ScanPhase::Failed);
        assert_eq!(scan_phase("whatever"), ScanPhase::Other);
    }
}
