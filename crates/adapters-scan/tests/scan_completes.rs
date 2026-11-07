//! A scan must report cumulative counts, commit its results and then close the progress
//! channel (that is how the GUI learns the scan finished).

use std::time::Duration;

use lrc_player_adapters_persistence::persistence::MetaCache;
use lrc_player_adapters_scan::infrastructure::scan::{Progress, ScanOptions, ScanWorker};
use tokio::sync::broadcast::error::RecvError;

async fn run_scan(root: &std::path::Path, db: &std::path::Path) -> Vec<Progress> {
    let worker = ScanWorker::new_with_db_path(Some(db.to_path_buf()));
    let handle = worker
        .start_scan(vec![root.to_path_buf()], ScanOptions { fast: false, excludes: vec![] })
        .await
        .unwrap();
    let mut rx = worker.subscribe_progress(&handle);
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match rx.recv().await {
                Ok(p) => seen.push(p),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    })
    .await
    .expect("scan never finished (progress channel stayed open)");
    seen
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn small_scan_finishes_with_cumulative_counts_and_commits() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(music.join("album")).unwrap();
    for name in ["a.mp3", "b.flac", "c.m4a"] {
        std::fs::write(music.join("album").join(name), b"not really audio, but hashable").unwrap();
    }
    std::fs::write(music.join("album").join("a.lrc"), "[00:01.00]hello\n[00:02.00]world\n").unwrap();
    std::fs::write(music.join("album").join("notes.txt"), "ignored").unwrap();
    let db = dir.path().join("meta.db");

    let seen = run_scan(&music, &db).await;
    let last = seen.last().expect("at least the final progress update");
    assert_eq!(last.scanned, 4, "3 audio + 1 lyrics file found");
    assert_eq!(last.parsed, 4);
    assert_eq!(last.registered, 4);
    assert!(seen.windows(2).all(|w| w[0].scanned <= w[1].scanned && w[0].registered <= w[1].registered));

    // Everything is committed by the time the channel closes.
    let cache = MetaCache::new(&db).unwrap();
    assert_eq!(cache.library_counts().unwrap().1, 3);
    let lyrics = cache.lyrics_for_track_path(&music.join("album").join("a.mp3")).unwrap();
    assert_eq!(lyrics.len(), 1, "lyrics associated with the track scanned in the same run");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_scan_finishes_even_when_the_receiver_lags() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    for d in 0..6 {
        let sub = music.join(format!("d{d}"));
        std::fs::create_dir_all(&sub).unwrap();
        for i in 0..100 {
            std::fs::write(sub.join(format!("t{i:03}.mp3")), format!("track {d}-{i}")).unwrap();
        }
    }
    let db = dir.path().join("meta.db");

    let seen = run_scan(&music, &db).await;
    let last = seen.last().unwrap();
    assert_eq!((last.scanned, last.registered), (600, 600));
    assert_eq!(MetaCache::new(&db).unwrap().library_counts().unwrap().1, 600);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_path_list_finishes_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let worker = ScanWorker::new_with_db_path(Some(dir.path().join("meta.db")));
    let handle = worker.start_scan(vec![], ScanOptions { fast: false, excludes: vec![] }).await.unwrap();
    let mut rx = worker.subscribe_progress(&handle);
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Err(RecvError::Closed) = rx.recv().await {
                break;
            }
        }
    })
    .await;
    assert!(closed.is_ok());
}
