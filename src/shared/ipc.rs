// IPC implementation: Unix domain socket server/client for single-instance and ScanRequest delivery.
use serde::{Deserialize, Serialize};

#[cfg(unix)]
mod unix_ipc {
    use super::*;
    use anyhow::Result;

    use std::path::{Path, PathBuf};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::mpsc;

    #[derive(Serialize, Deserialize, Debug)]
    pub struct ScanRequest {
        pub paths: Vec<String>,
    }

    /// Try to bind to the socket path. If successful, start server later.
    pub async fn try_bind_socket(path: &Path) -> Result<bool> {
        // remove stale socket if exists
        let _ = std::fs::remove_file(path);
        match UnixListener::bind(path) {
            Ok(_) => Ok(true),
            Err(e) => {
                // bind failed — another instance may exist
                tracing::debug!("bind failed: {}", e);
                Ok(false)
            }
        }
    }

    /// Start an IPC server on the given socket path. Incoming JSON messages are forwarded to `scan_tx`.
    pub async fn start_server(path: &Path, scan_tx: mpsc::Sender<ScanRequest>) -> Result<()> {
        // Ensure old socket removed
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        tracing::info!("IPC server listening on {:?}", path);

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((mut stream, _addr)) => {
                        let mut buf = Vec::new();
                        // read all until EOF
                        match stream.read_to_end(&mut buf).await {
                            Ok(_) => {
                                if let Ok(req) = serde_json::from_slice::<ScanRequest>(&buf) {
                                    let _ = scan_tx.try_send(req);
                                } else {
                                    tracing::warn!("received invalid IPC message");
                                }
                            }
                            Err(e) => tracing::warn!("IPC read error: {}", e),
                        }
                    }
                    Err(e) => {
                        tracing::warn!("IPC accept error: {}", e);
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Send a ScanRequest to an existing server socket.
    pub async fn send_scan_request(path: &Path, req: &ScanRequest) -> Result<()> {
        let mut stream = UnixStream::connect(path).await?;
        let bytes = serde_json::to_vec(req)?;
        stream.write_all(&bytes).await?;
        stream.shutdown().await?;
        Ok(())
    }

    pub fn default_socket_path() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push("lrc_player.sock");
        p
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::Duration;
        use tempfile::tempdir;
        use tokio::runtime::Runtime;
        use tokio::sync::mpsc;

        #[test]
        fn server_receives_scan_requests() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("ipc_test.sock");
            let rt = Runtime::new().unwrap();

            rt.block_on(async {
                let (tx, mut rx) = mpsc::channel(1);
                start_server(&path, tx).await.unwrap();
                tokio::time::sleep(Duration::from_millis(10)).await;

                let request = ScanRequest {
                    paths: vec!["/tmp/scan1".into()],
                };
                send_scan_request(&path, &request).await.unwrap();

                let received = tokio::time::timeout(Duration::from_secs(1), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();

                assert_eq!(received.paths, request.paths);
            });
        }
    }
}

#[cfg(unix)]
pub use unix_ipc::*;

#[cfg(not(unix))]
mod fallback {
    use super::*;
    use anyhow::Result;
    use std::path::Path;

    #[derive(Serialize, Deserialize, Debug)]
    pub struct ScanRequest {
        pub paths: Vec<String>,
    }

    pub async fn try_bind_socket(_path: &Path) -> Result<bool> {
        Err(anyhow::anyhow!("IPC not implemented on this platform"))
    }
    pub async fn start_server(
        _path: &Path,
        _scan_tx: tokio::sync::mpsc::Sender<ScanRequest>,
    ) -> Result<()> {
        Err(anyhow::anyhow!("IPC not implemented on this platform"))
    }
    pub async fn send_scan_request(_path: &Path, _req: &ScanRequest) -> Result<()> {
        Err(anyhow::anyhow!("IPC not implemented on this platform"))
    }
    pub fn default_socket_path() -> std::path::PathBuf {
        std::env::temp_dir().join("lrc_player.sock")
    }
}

#[cfg(not(unix))]
pub use fallback::*;
