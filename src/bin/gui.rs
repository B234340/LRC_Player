#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use anyhow::Result;
use lrc_player::application::dto::StateDto;
use lrc_player::infrastructure::persistence::{MetaCache, StateFile};
use lrc_player::infrastructure::scan::{ScanCoordinator, ScanOptions};
use lrc_player::infrastructure::scan::ScanHandle;
use lrc_player::shared::ipc as ipc_mod;
use std::process;
use std::thread;
use tokio::runtime::Builder;
use tokio::sync::mpsc;

fn update_scan_status(
    status: &str,
    scanned: Option<u64>,
    parsed: Option<u64>,
    registered: Option<u64>,
) -> anyhow::Result<()> {
    let mut state = StateFile::open()
        .and_then(|store| store.load())
        .unwrap_or(StateDto {
            last_tab: Some("Scan".into()),
            last_play_path: None,
            last_play_position_ms: None,
            last_scan_status: None,
            scan_scanned: None,
            scan_parsed: None,
            scan_registered: None,
            scan_targets: Vec::new(),
            scan_excludes: Vec::new(),
            queue_tracks: Vec::new(),
            selected_queue_index: None,
            queue_playback_mode: lrc_player::domain::model::PlaybackMode::Sequential,
            favorite_directories: Vec::new(),
        });
    state.last_scan_status = Some(status.to_string());
    state.scan_scanned = scanned;
    state.scan_parsed = parsed;
    state.scan_registered = registered;
    if let Ok(store) = StateFile::open() {
        store.save(&state)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let socket = ipc_mod::default_socket_path();
    let server_thread = thread::spawn(move || -> Result<()> {
        let rt = Builder::new_current_thread().enable_all().build()?;
        rt.block_on(async move {
            if ipc_mod::try_bind_socket(&socket).await? {
                let meta = MetaCache::open_default()?;
                let coordinator = ScanCoordinator::new(meta);
                let (tx, mut rx) = mpsc::channel::<ipc_mod::ScanRequest>(8);
                ipc_mod::start_server(&socket, tx).await?;

                let mut current_handle: Option<ScanHandle> = None;
                while let Some(req) = rx.recv().await {
                    if req.cancel {
                        tracing::info!("IPC scan cancel request received");
                        if let Some(handle) = current_handle.as_ref() {
                            match coordinator.cancel_scan(handle).await {
                                Ok(_) => {
                                    let _ = update_scan_status(
                                        "Scan cancellation requested",
                                        None,
                                        None,
                                        None,
                                    );
                                }
                                Err(e) => {
                                    tracing::error!("failed to cancel scan: {}", e);
                                    let _ = update_scan_status(
                                        &format!("Cancel failed: {}", e),
                                        None,
                                        None,
                                        None,
                                    );
                                }
                            }
                        } else {
                            let _ = update_scan_status(
                                "No active scan to cancel",
                                None,
                                None,
                                None,
                            );
                        }
                        continue;
                    }

                    tracing::info!("IPC scan request received: {:?}", req.paths);
                    let paths: Vec<_> = req.paths.into_iter().map(std::path::PathBuf::from).collect();
                    let excludes: Vec<_> = req.excludes.into_iter().map(std::path::PathBuf::from).collect();
                    let opts = ScanOptions {
                        fast: false,
                        excludes,
                    };
                    match coordinator.start_scan(paths, opts).await {
                        Ok(handle) => {
                            current_handle = Some(handle.clone());
                            let _ = update_scan_status("Scan started", Some(0), Some(0), Some(0));
                            let mut progress_rx = coordinator.subscribe_progress(&handle);
                            tokio::spawn(async move {
                                while let Ok(progress) = progress_rx.recv().await {
                                    let status = format!(
                                        "Scan progress: scanned={} parsed={} registered={}",
                                        progress.scanned, progress.parsed, progress.registered
                                    );
                                    let _ = update_scan_status(
                                        &status,
                                        Some(progress.scanned),
                                        Some(progress.parsed),
                                        Some(progress.registered),
                                    );
                                }
                                let _ = update_scan_status("Scan completed", None, None, None);
                            });
                        }
                        Err(e) => {
                            tracing::error!("failed to start scan: {}", e);
                            let _ = update_scan_status(
                                &format!("Scan failed: {}", e),
                                None,
                                None,
                                None,
                            );
                        }
                    }
                }
            } else {
                eprintln!("Another LRC Player instance is already running.");
                process::exit(0);
            }
            Ok::<(), anyhow::Error>(())
        })
    });

    // Detach the server thread; it will run for the lifetime of the GUI.
    let _ = server_thread;

    let options = eframe::NativeOptions::default();
    let _ = eframe::run_native(
        "LRC Player GUI",
        options,
        Box::new(|cc| Box::new(lrc_player::presentation::egui_app::GuiApp::new(cc))),
    );

    Ok(())
}
