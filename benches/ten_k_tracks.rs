use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use lrc_player::infrastructure::scan::file_scanner::FileScanner;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

fn generate_track_files(root: &Path, count: usize) {
    fs::create_dir_all(root).unwrap();
    let dirs = 10.min(count);
    let files_per_dir = count.div_ceil(dirs);

    for dir_index in 0..dirs {
        let dir = root.join(format!("batch_{:02}", dir_index));
        fs::create_dir_all(&dir).unwrap();

        for file_index in 0..files_per_dir {
            let global_index = dir_index * files_per_dir + file_index;
            if global_index >= count {
                break;
            }
            let file_name = format!("track_{:06}.mp3", global_index);
            let file_path = dir.join(file_name);
            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"dummy audio content\n").unwrap();
        }
    }
}

fn scan_benchmark(c: &mut Criterion) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("tracks");
    generate_track_files(&root, 10000);

    let scanner = FileScanner::new();
    c.bench_with_input(
        BenchmarkId::new("file_enumeration", 10000),
        &root,
        |b, root| {
            b.iter(|| {
                let paths = scanner.enumerate_paths(root);
                assert_eq!(paths.len(), 10000);
            });
        },
    );
}

criterion_group!(benches, scan_benchmark);
criterion_main!(benches);
