use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use lrc_player::infrastructure::scan::file_scanner::FileScanner;
use std::path::Path;


fn scan_benchmark(c: &mut Criterion) {
    let windows_root = Path::new("/mnt/c/Users/OU/Music/Music");
    let (root, expected_count) = if windows_root.exists() {
        println!("Using real Windows path for benchmark: {}", windows_root.display());
        (windows_root.to_path_buf(), None)
    } else {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("tracks");
        FileScanner::generate_dummy_track_files(&root, 10000);
        println!("Windows path not found; using generated temp data: {}", root.display());
        (root, Some(10000))
    };

    let scanner = FileScanner::new();
    c.bench_with_input(
        BenchmarkId::new("file_enumeration", expected_count.unwrap_or(0)),
        &root,
        |b, root| {
            b.iter(|| {
                let paths = scanner.enumerate_paths(root, &[]);
                assert!(!paths.is_empty());
                if let Some(expected) = expected_count {
                    assert_eq!(paths.len(), expected);
                }
            });
        },
    );
}

criterion_group!(benches, scan_benchmark);
criterion_main!(benches);
