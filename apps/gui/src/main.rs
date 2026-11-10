//! GUI application entrypoint.
//! Starts IPC scan server wiring and launches the egui frontend.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use anyhow::Result;
use lrc_player::config::Config;
use lrc_player::infrastructure::persistence::MetaCache;
use lrc_player::presentation::app::scan_status;
use lrc_player::infrastructure::scan::{ScanCoordinator, ScanOptions};
use lrc_player::infrastructure::scan::ScanHandle;
use lrc_player::shared::ipc as ipc_mod;
use std::process;
use std::thread;
use tokio::runtime::Builder;
use tokio::sync::mpsc;
use tracing::level_filters::LevelFilter;

fn update_scan_status(
    status: &str,
    scanned: Option<u64>,
    parsed: Option<u64>,
    registered: Option<u64>,
) -> anyhow::Result<()> {
    // Delivered in-process to the GUI (which persists the last status with its own state);
    // no more read-modify-write of state.json racing with the GUI's saves.
    scan_status::publish(status, scanned, parsed, registered);
    Ok(())
}

/// Log level: `RUST_LOG` (a plain level such as `debug`) wins, otherwise the configured one.
fn init_logging() {
    let configured = Config::load().log_level.as_filter_str().to_string();
    let requested = std::env::var("RUST_LOG").ok().unwrap_or(configured);
    let level = match requested.trim().to_ascii_lowercase().as_str() {
        "error" => LevelFilter::ERROR,
        "warn" => LevelFilter::WARN,
        "debug" => LevelFilter::DEBUG,
        "trace" => LevelFilter::TRACE,
        "off" => LevelFilter::OFF,
        _ => LevelFilter::INFO,
    };
    tracing_subscriber::fmt().with_max_level(level).init();
}

fn main() -> Result<()> {
    init_logging();

    let socket = ipc_mod::default_socket_path();
    // Run the IPC receiver on a dedicated thread so the GUI event loop stays responsive.
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
                    let mut all_scan_paths = req.paths;
                    all_scan_paths.extend(req.lrc_paths);
                    let paths: Vec<_> = all_scan_paths
                        .into_iter()
                        .map(std::path::PathBuf::from)
                        .collect();
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
                                // The channel closes when the job has finished and committed its
                                // results. A lagging receiver only missed intermediate counts.
                                loop {
                                    let progress = match progress_rx.recv().await {
                                        Ok(progress) => progress,
                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                                    };
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

    // Restore the last window size / position / maximized state (saved in state.json).
    // The minimum size keeps the sidebar's nav, playlists and bottom block from colliding
    // (see `ui::shell::MIN_WINDOW_SIZE`).
    let [min_w, min_h] = lrc_player::presentation::ui::shell::MIN_WINDOW_SIZE;
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("LRC Player")
        .with_app_id("lrc_player")
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([min_w, min_h])
        .with_drag_and_drop(true);
    if let Some(window) = lrc_player::presentation::app::initial_window() {
        viewport = viewport
            .with_inner_size([window.width.max(min_w), window.height.max(min_h)])
            .with_maximized(window.maximized);
        let on_screen = |v: f32| (-4_000.0..16_000.0).contains(&v);
        if let (Some(x), Some(y)) = (window.x.filter(|v| on_screen(*v)), window.y.filter(|v| on_screen(*v))) {
            viewport = viewport.with_position([x, y]);
        }
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    if let Err(err) = eframe::run_native(
        "LRC Player",
        options,
        Box::new(|cc| Ok(Box::new(lrc_player::presentation::app::GuiApp::new(cc)))),
    ) {
        tracing::error!("failed to start the GUI: {err}");
        eprintln!("failed to start the GUI: {err}");
        process::exit(1);
    }

    Ok(())
}
