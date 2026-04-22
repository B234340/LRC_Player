use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut root = PathBuf::from("./bench_tracks");
    let mut count = 10000;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                if let Some(val) = args.get(i + 1) {
                    root = PathBuf::from(val);
                }
                i += 2;
            }
            "--count" => {
                if let Some(val) = args.get(i + 1) {
                    count = val.parse().unwrap_or(count);
                }
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    println!("Generating {} track files under {}", count, root.display());
    fs::create_dir_all(&root).expect("create root directory");

    let dirs = 10.min(count);
    let files_per_dir = (count + dirs - 1) / dirs;

    for dir_index in 0..dirs {
        let dir = root.join(format!("batch_{:02}", dir_index));
        fs::create_dir_all(&dir).expect("create batch directory");

        for file_index in 0..files_per_dir {
            let global_index = dir_index * files_per_dir + file_index;
            if global_index >= count {
                break;
            }
            let file_name = format!("track_{:06}.mp3", global_index);
            let file_path = dir.join(file_name);
            let mut file = File::create(&file_path).expect("create file");
            let content = format!("Dummy audio file {}\n", global_index);
            file.write_all(content.as_bytes()).expect("write file");
        }
    }

    println!("Generated {} tracks", count);
}
