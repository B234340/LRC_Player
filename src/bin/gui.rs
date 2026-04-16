use anyhow::Result;
use lrc_player::shared::ipc as ipc_mod;
use std::process;
use tokio::runtime::Builder;
use tokio::sync::mpsc;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let rt = Builder::new_multi_thread().enable_all().build()?;
    let socket = ipc_mod::default_socket_path();

    rt.block_on(async {
        if ipc_mod::try_bind_socket(&socket).await? {
            let (tx, mut rx) = mpsc::channel::<ipc_mod::ScanRequest>(8);
            tokio::spawn(async move {
                while let Some(req) = rx.recv().await {
                    tracing::info!("IPC scan request received: {:?}", req.paths);
                }
            });
            ipc_mod::start_server(&socket, tx).await?;
        } else {
            eprintln!("Another LRC Player instance is already running.");
            process::exit(0);
        }
        Ok::<(), anyhow::Error>(())
    })?;

    let options = eframe::NativeOptions::default();
    let _ = eframe::run_native(
        "LRC Player GUI",
        options,
        Box::new(|cc| Box::new(lrc_player::presentation::egui_app::GuiApp::new(cc))),
    );

    Ok(())
}
