use anyhow::Result;
use blake3::Hasher;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug)]
pub struct DedupService {
    tx: mpsc::Sender<(PathBuf, oneshot::Sender<Result<String>>)>,
}

impl DedupService {
    /// Spawn a background dedup service. The worker computes full hashes on demand.
    pub fn spawn() -> Self {
        let (tx, mut rx) = mpsc::channel::<(PathBuf, oneshot::Sender<Result<String>>)>(64);

        tokio::spawn(async move {
            while let Some((path, resp_tx)) = rx.recv().await {
                let res = tokio::task::spawn_blocking(move || compute_full_hash(&path)).await;
                match res {
                    Ok(Ok(hash)) => {
                        let _ = resp_tx.send(Ok(hash));
                    }
                    Ok(Err(e)) => {
                        let _ = resp_tx.send(Err(e));
                    }
                    Err(e) => {
                        let _ = resp_tx.send(Err(anyhow::anyhow!(e.to_string())));
                    }
                }
            }
        });

        DedupService { tx }
    }

    /// Request a full content hash asynchronously. Returns a oneshot receiver for the result.
    pub async fn request_full_hash(&self, path: PathBuf) -> oneshot::Receiver<Result<String>> {
        let (resp_tx, resp_rx) = oneshot::channel();
        // best-effort send
        let _ = self.tx.send((path, resp_tx)).await;
        resp_rx
    }
}

/// Compute a partial hash by reading the first and last `n` bytes and hashing them.
pub fn compute_partial_hash(path: &PathBuf, n: usize) -> Result<String> {
    let mut f = File::open(path)?;
    let meta = f.metadata()?;
    let len = meta.len() as usize;
    let mut hasher = Hasher::new();

    let first_n = std::cmp::min(n, len);
    let mut buf = vec![0u8; first_n];
    f.read_exact(&mut buf)?;
    hasher.update(&buf);

    if len > first_n {
        let last_n = std::cmp::min(n, len - first_n);
        f.seek(SeekFrom::End(-(last_n as i64)))?;
        let mut buf2 = vec![0u8; last_n];
        f.read_exact(&mut buf2)?;
        hasher.update(&buf2);
    }

    Ok(hex::encode(hasher.finalize().as_bytes()))
}

/// Compute a full content blake3 hash by streaming the file.
pub fn compute_full_hash(path: &PathBuf) -> Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize().as_bytes()))
}


