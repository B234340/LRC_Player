use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait};
use lrc_player::infrastructure::io_worker::ScanOptions;
use lrc_player::infrastructure::persistence::MetaCache;
use lrc_player::infrastructure::scan::ScanCoordinator;
// AudioOutput import intentionally omitted; use fully-qualified paths where needed.
use lrc_player::shared::ipc as ipc_mod;
use std::path::PathBuf;
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
    /// Resampler taps (window size) for high-quality resampling
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
        let host = cpal::default_host();
        match host.output_devices() {
            Ok(devs) => {
                let needle = name.to_lowercase();
                let mut matches: Vec<String> = Vec::new();
                for d in devs {
                    if let Ok(n) = d.name() {
                        if n.to_lowercase().contains(&needle) {
                            matches.push(n);
                        }
                    }
                }

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
            }
            Err(e) => eprintln!("error enumerating devices: {}", e),
        }
        return Ok(());
    }

    // If user requested device listing, print available output devices and exit.
    if cli.list_devices {
        let host = cpal::default_host();
        match host.output_devices() {
            Ok(devs) => {
                println!("Available output devices:");
                for d in devs {
                    match d.name() {
                        Ok(n) => println!("- {}", n),
                        Err(_) => println!("- <unknown device name>"),
                    }
                }
            }
            Err(e) => eprintln!("error enumerating devices: {}", e),
        }
        return Ok(());
    }

    let socket = ipc_mod::default_socket_path();

    // Try to bind socket. If bind fails, send scan request to existing instance and exit.
    match ipc_mod::try_bind_socket(&socket).await {
        Ok(true) => {
            tracing::info!("acquired single-instance lock, starting server");

            // create MetaCache and ScanCoordinator
            let meta = MetaCache::new(":memory:")?;
            let coordinator = ScanCoordinator::new(meta);

            // channel for IPC -> scan coordinator
            let (tx, mut rx) = mpsc::channel::<ipc_mod::ScanRequest>(8);
            // start IPC server
            ipc_mod::start_server(&socket, tx).await?;

            // if CLI requested scan at startup, start a scan
            if !cli.scan.is_empty() {
                let paths = cli.scan.clone();
                let pb: Vec<_> = paths.into_iter().collect();
                let opts = ScanOptions { fast: false };
                let handle = coordinator.start_scan(pb, opts);
                tracing::info!("started initial scan: id={}", handle.id);
            }

            // demo play (runs in-process when server instance)
            if let Some(play_path) = cli.play.clone() {
                tracing::info!("play demo requested: {:?}", play_path);
                // Start output first so we can hand the producer sender and pool to the decoder
                let mut out = lrc_player::infrastructure::audio::CpalOutput::new();
                let req_fmt = lrc_player::ports::audio_output::AudioFormat {
                    sample_rate: 44100,
                    channels: 2,
                    sample_format: lrc_player::ports::audio_output::SampleFormat::F32,
                };

                let eff_fmt =
                    match out.start_with_device(req_fmt, cfg.preferred_output_device.clone()) {
                        Ok(f) => f,
                        Err(e) => {
                            tracing::error!("audio start failed: {}", e);
                            return Ok(());
                        }
                    };

                let producer_tx = out
                    .producer_sender()
                    .expect("producer sender should be available after start");
                let pool = out.pool_handle();

                // decide taps: CLI overrides saved config
                let taps = cli.resampler_taps.unwrap_or(cfg.resampler_taps);
                cfg.resampler_taps = taps;
                let _ = cfg.save();

                // persist selected device name from output
                if let Some(name) = out.selected_device_name() {
                    cfg.preferred_output_device = Some(name);
                    let _ = cfg.save();
                }

                // spawn decoder that decodes -> converts to eff_fmt -> sends Arc<Vec<f32>> to producer
                let _dec_handle =
                    lrc_player::infrastructure::audio::decoder::spawn_decode_to_producer(
                        play_path.clone(),
                        1024usize,
                        eff_fmt.sample_rate,
                        eff_fmt.channels,
                        producer_tx,
                        pool,
                        taps,
                    );

                // In this demo we leave playback running until user exits; decoder/writer handle lifecycle
                // is managed internally.
            }

            // main loop: process incoming IPC scan requests
            while let Some(req) = rx.recv().await {
                tracing::info!("received IPC ScanRequest: {:?}", req.paths);
                let paths = req.paths.into_iter().map(PathBuf::from).collect();
                let opts = ScanOptions { fast: false };
                let handle = coordinator.start_scan(paths, opts);
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
                let req = ipc_mod::ScanRequest { paths };
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
