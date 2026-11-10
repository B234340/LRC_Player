//! CLI application entrypoint.
//! Supports playback demo, device inspection, and IPC-driven scan requests.

use clap::Parser;
use lrc_player::infrastructure::io_worker::ScanOptions;
use lrc_player::infrastructure::persistence::MetaCache;
use lrc_player::infrastructure::scan::ScanCoordinator;
// AudioOutput import intentionally omitted; use fully-qualified paths where needed.
use lrc_player::shared::ipc as ipc_mod;
use lrc_player_adapters_audio::audio::engine::{
    AudioEngine, EngineConfig, EngineEvent, ResamplerKind,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Paths to scan (if provided, will request scan)
    #[arg(long, value_name = "PATH", num_args=0..)]
    scan: Vec<PathBuf>,
    /// Play an audio file (demo)
    #[arg(long, value_name = "FILE")]
    play: Option<PathBuf>,
    /// Use a windowed-sinc resampler with N taps (default: FFT resampler)
    #[arg(long, value_name = "N")]
    resampler_taps: Option<usize>,
    /// List available output devices and exit
    #[arg(long)]
    list_devices: bool,
    /// Set preferred output device by name and persist to config
    #[arg(long, value_name = "NAME")]
    set_device: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // load persisted config; CLI flags override saved settings when provided
    let mut cfg = lrc_player::config::Config::load();
    let cli = Cli::parse();

    // If user requested to set preferred device, perform case-insensitive substring
    // matching against available output device names. If exactly one match is found,
    // persist it. If multiple matches are found, print candidates and exit.
    if let Some(name) = cli.set_device.as_ref() {
        let needle = name.to_lowercase();
        let mut matches: Vec<String> = AudioEngine::output_device_names()
            .into_iter()
            .filter(|n| n.to_lowercase().contains(&needle))
            .collect();
        match matches.len() {
            0 => eprintln!("no matching device found for '{}'", name),
            1 => {
                let chosen = matches.remove(0);
                cfg.preferred_output_device = Some(chosen.clone());
                cfg.save().ok();
                println!("preferred output device set to: {}", chosen);
            }
            _ => {
                eprintln!("multiple devices match '{}':", name);
                for m in matches.iter() {
                    eprintln!(" - {}", m);
                }
                eprintln!("Be more specific or use the GUI to pick a device.");
            }
        }
        return Ok(());
    }

    // If user requested device listing, print available output devices and exit.
    if cli.list_devices {
        let default = AudioEngine::default_output_device_name();
        println!("Available output devices:");
        for n in AudioEngine::output_device_names() {
            let mark = if Some(&n) == default.as_ref() {
                " (default)"
            } else {
                ""
            };
            println!("- {}{}", n, mark);
        }
        return Ok(());
    }

    let socket = ipc_mod::default_socket_path();

    // Try to bind socket. If bind fails, send scan request to existing instance and exit.
    match ipc_mod::try_bind_socket(&socket).await {
        Ok(true) => {
            tracing::info!("acquired single-instance lock, starting server");

            // create MetaCache and ScanCoordinator
            let meta = MetaCache::open_default()?;
            let coordinator = ScanCoordinator::new(meta);

            // channel for IPC -> scan coordinator
            let (tx, mut rx) = mpsc::channel::<ipc_mod::ScanRequest>(8);
            // start IPC server
            ipc_mod::start_server(&socket, tx).await?;

            // if CLI requested scan at startup, start a scan
            if !cli.scan.is_empty() {
                let paths = cli.scan.clone();
                let pb: Vec<_> = paths.into_iter().collect();
                let opts = ScanOptions {
                    fast: false,
                    excludes: Vec::new(),
                };
                let handle = coordinator.start_scan(pb, opts).await?;
                tracing::info!("started initial scan: id={}", handle.id);
            }

            // Keeps the engine (and with it playback) alive while the IPC loop runs.
            let mut _engine: Option<Arc<AudioEngine>> = None;
            if let Some(play_path) = cli.play.clone() {
                tracing::info!("play demo requested: {:?}", play_path);
                // Explicit --resampler-taps selects the sinc resampler; it is also persisted.
                let resampler = match cli.resampler_taps {
                    Some(taps) => {
                        cfg.resampler_taps = taps;
                        let _ = cfg.save();
                        ResamplerKind::Sinc { sinc_len: taps }
                    }
                    None => ResamplerKind::Fft,
                };
                let engine = Arc::new(AudioEngine::new(EngineConfig {
                    device: cfg.preferred_output_device.clone(),
                    resampler,
                    ..Default::default()
                })?);
                engine.load(play_path.clone(), 0, true);
                let ev_engine = engine.clone();
                tokio::spawn(async move {
                    let mut tick = tokio::time::interval(std::time::Duration::from_millis(200));
                    loop {
                        tick.tick().await;
                        while let Some(ev) = ev_engine.try_recv_event() {
                            match &ev {
                                EngineEvent::Error { path, message } => {
                                    tracing::error!("playback error ({:?}): {}", path, message)
                                }
                                EngineEvent::TrackFinished { path } => {
                                    tracing::info!("playback finished: {}", path.display());
                                    return;
                                }
                                other => tracing::info!("audio: {:?}", other),
                            }
                        }
                    }
                });
                _engine = Some(engine);
            }

            // main loop: process incoming IPC scan requests
            while let Some(req) = rx.recv().await {
                tracing::info!("received IPC ScanRequest: {:?}", req.paths);
                let mut paths: Vec<PathBuf> = req.paths.into_iter().map(PathBuf::from).collect();
                paths.extend(req.lrc_paths.into_iter().map(PathBuf::from));
                let excludes = req.excludes.into_iter().map(PathBuf::from).collect();
                let opts = ScanOptions {
                    fast: false,
                    excludes,
                };
                let handle = coordinator.start_scan(paths, opts).await?;
                tracing::info!("started scan from IPC: id={}", handle.id);
            }
        }
        Ok(false) => {
            tracing::info!("another instance is running; sending scan request to it");
            if !cli.scan.is_empty() {
                let paths: Vec<String> = cli
                    .scan
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                let req = ipc_mod::ScanRequest {
                    paths,
                    lrc_paths: Vec::new(),
                    excludes: Vec::new(),
                    cancel: false,
                };
                ipc_mod::send_scan_request(&socket, &req).await?;
                tracing::info!("scan request sent");
            } else {
                tracing::info!("no scan paths provided; exiting");
            }
        }
        Err(e) => {
            tracing::error!("ipc bind error: {}", e);
        }
    }

    Ok(())
}
