use lrc_player::application::usecases::play_track::play;
use lrc_player::infrastructure::decode::SymphoniaDecoder;
use lrc_player::infrastructure::lyrics::Lyrics;
use lrc_player::ports::audio_output::{AudioFormat, AudioOutput};
use std::fs::{read_dir, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

struct TestOutput {
    format: Option<AudioFormat>,
    written: Vec<f32>,
}

impl TestOutput {
    fn new() -> Self {
        Self {
            format: None,
            written: Vec::new(),
        }
    }
}

impl AudioOutput for TestOutput {
    fn start(&mut self, format: AudioFormat) -> anyhow::Result<AudioFormat> {
        self.format = Some(format.clone());
        Ok(format)
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn write_f32(&mut self, data: &[f32]) -> anyhow::Result<usize> {
        self.written.extend_from_slice(data);
        Ok(data.len())
    }
}

fn write_wav<P: AsRef<Path>>(path: P, samples: &[i16], sample_rate: u32, channels: u16) {
    let mut file = File::create(path).unwrap();
    let bits_per_sample = 16u16;
    let byte_rate = sample_rate * channels as u32 * (bits_per_sample / 8) as u32;
    let block_align = channels * (bits_per_sample / 8);
    let subchunk2_size = (samples.len() * 2) as u32;
    let chunk_size = 36 + subchunk2_size;

    file.write_all(b"RIFF").unwrap();
    file.write_all(&chunk_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&block_align.to_le_bytes()).unwrap();
    file.write_all(&bits_per_sample.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&subchunk2_size.to_le_bytes()).unwrap();

    for &sample in samples {
        file.write_all(&sample.to_le_bytes()).unwrap();
    }
}

#[test]
fn sample_data_directory_access_and_playback() {
    let sample_data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sample_data");
    assert!(sample_data_dir.exists(), "sample_data directory must exist");
    assert!(sample_data_dir.is_dir(), "sample_data path must be a directory");

    let lrc_files: Vec<PathBuf> = read_dir(&sample_data_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("lrc"))
                .unwrap_or(false)
        })
        .collect();

    assert!(lrc_files.len() >= 1, "sample_data directory must contain at least one .lrc file");

    let first_lrc = &lrc_files[0];
    let contents = std::fs::read_to_string(first_lrc).unwrap();
    assert!(!contents.is_empty(), "sample_data lyrics file must not be empty");

    let dir = tempdir().unwrap();
    let wav_path = dir.path().join("playback_test.wav");
    write_wav(&wav_path, &[0, 32767, -32768, 12345], 44100, 1);

    let decoder = SymphoniaDecoder::new();
    let mut output = TestOutput::new();
    play(&decoder, &mut output, wav_path.clone(), Some(0)).expect("playback should complete");

    assert!(output.format.is_some(), "output format must be negotiated");
    assert!(!output.written.is_empty(), "playback should write PCM samples");
}

#[test]
fn sample_data_lrc_metadata_and_playback() {
    let sample_data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sample_data");
    assert!(sample_data_dir.exists(), "sample_data directory must exist");
    assert!(sample_data_dir.is_dir(), "sample_data path must be a directory");

    let lrc_files: Vec<PathBuf> = read_dir(&sample_data_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("lrc"))
                .unwrap_or(false)
        })
        .collect();
    assert!(!lrc_files.is_empty(), "sample_data directory must contain at least one .lrc file");

    let source_lrc = &lrc_files[0];
    let stem = source_lrc.file_stem().unwrap().to_string_lossy();

    let dir = tempdir().unwrap();
    let track_path = dir.path().join(format!("{}.wav", stem));
    let lrc_path = dir.path().join(format!("{}.lrc", stem));

    let lrc_contents = std::fs::read_to_string(source_lrc).unwrap();
    std::fs::write(&lrc_path, &lrc_contents).unwrap();

    write_wav(&track_path, &[0, 32767, -32768, 12345, 0, 0], 44100, 1);

    let discovered = Lyrics::discover_for_track(&track_path);
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0], lrc_path.canonicalize().unwrap());

    let lyrics = Lyrics::from_lrc(&lrc_path).unwrap();
    assert!(!lyrics.lines.is_empty(), "lyrics file must parse at least one timed line");
    assert!(lyrics.lines.iter().all(|line| !line.text.is_empty()));

    let decoder = SymphoniaDecoder::new();
    let decoded = decoder.decode_file(&track_path).expect("decode wav file for metadata");
    assert_eq!(decoded.sample_rate, 44100);
    assert_eq!(decoded.channels, 1);
    assert_eq!(decoded.frames(), 6);

    let mut output = TestOutput::new();
    play(&decoder, &mut output, track_path.clone(), Some(0)).expect("playback should complete");
    assert!(output.format.is_some(), "output format must be negotiated");
    assert!(!output.written.is_empty(), "playback should write PCM samples");
}
