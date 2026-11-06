//! Fast track metadata probe (no audio decoding).
//!
//! `SymphoniaDecoder::decode_file` decodes the entire file into memory, which
//! is far too expensive just to learn a track's duration and tags (hundreds of
//! MB and 0.5-3 s for a long track). [`probe_track_info`] only reads the
//! container headers and metadata:
//!
//! * duration from the track's `n_frames` (via `time_base`, or the sample
//!   rate), falling back to scanning packet timestamps **without decoding**
//!   when the container does not declare a frame count,
//! * sample rate, channels, bits per sample, codec name,
//! * title / artist / album / album artist / track & disc number / year /
//!   genre, read from both the container-level metadata found while probing
//!   (e.g. ID3v2 in MP3, which `decode_file` ignores) and the format's own
//!   metadata,
//! * whether embedded artwork (a visual) is present.
//!
//! It still does blocking file I/O; GUI code should call it from a worker
//! thread and post the result back (e.g. channel + `ctx.request_repaint()`).

use anyhow::{Context, Result};
use std::fs::File;
use std::path::Path;
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, Value};
use symphonia::core::probe::Hint;
use symphonia::core::units::TimeBase;
use symphonia::default::get_probe;

/// How [`TrackInfo::duration_ms`] was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DurationSource {
    /// Declared frame count in the container/codec header.
    Header,
    /// Sum of packet timestamps (container had no frame count). Reads the
    /// whole file but decodes nothing.
    PacketScan,
    #[default]
    Unknown,
}

/// Cheap-to-obtain description of an audio file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackInfo {
    pub duration_ms: Option<u64>,
    pub duration_source: DurationSource,
    /// Frames per channel, when known.
    pub n_frames: Option<u64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub bits_per_sample: Option<u32>,
    /// Short codec name (e.g. "flac", "mp3", "pcm_s16le").
    pub codec: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
    /// Four-digit year parsed from the date/year tag.
    pub year: Option<i32>,
    /// Raw date tag (e.g. "2021-04-01").
    pub date: Option<String>,
    pub genre: Option<String>,
    pub has_embedded_artwork: bool,
}

/// Probe `path` for duration, format and tags without decoding audio.
pub fn probe_track_info<P: AsRef<Path>>(path: P) -> Result<TrackInfo> {
    let path = path.as_ref();
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let format_opts = FormatOptions {
        // Don't build a seek index or gapless info: we only need headers.
        prebuild_seek_index: false,
        enable_gapless: false,
        ..Default::default()
    };
    let mut probed = get_probe()
        .format(&hint, mss, &format_opts, &MetadataOptions::default())
        .with_context(|| format!("probing {}", path.display()))?;

    let mut info = TrackInfo::default();

    // Container-level metadata found while probing (ID3v2 for MP3, etc.).
    if let Some(mut metadata) = probed.metadata.get() {
        if let Some(revision) = metadata.skip_to_latest() {
            apply_revision(&mut info, revision);
        }
    }
    let mut format = probed.format;
    if let Some(revision) = format.metadata().skip_to_latest() {
        apply_revision(&mut info, revision);
    }

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .or_else(|| format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL))
        .ok_or_else(|| anyhow::anyhow!("no audio track found in {}", path.display()))?;
    let params = track.codec_params.clone();
    let track_id = track.id;

    info.sample_rate = params.sample_rate;
    info.channels = params.channels.map(|c| c.count() as u16);
    info.bits_per_sample = params.bits_per_sample;
    info.n_frames = params.n_frames;
    info.codec = symphonia::default::get_codecs()
        .get_codec(params.codec)
        .map(|d| d.short_name.to_string());

    let time_base = params
        .time_base
        .or_else(|| params.sample_rate.map(|sr| TimeBase::new(1, sr)));

    if let (Some(n_frames), Some(tb)) = (params.n_frames, time_base) {
        info.duration_ms = Some(ts_to_ms(tb, n_frames));
        info.duration_source = DurationSource::Header;
    } else if let Some(tb) = time_base {
        if let Some(units) = scan_duration_from_packets(format.as_mut(), track_id, params.start_ts) {
            info.duration_ms = Some(ts_to_ms(tb, units));
            info.duration_source = DurationSource::PacketScan;
        }
    }

    Ok(info)
}

fn ts_to_ms(tb: TimeBase, ts: u64) -> u64 {
    let time = tb.calc_time(ts);
    time.seconds.saturating_mul(1000) + (time.frac * 1000.0).round() as u64
}

/// Walk packets (no decoding) and return the end timestamp of the last packet
/// minus the stream start, in time-base units.
pub(crate) fn scan_duration_from_packets(
    format: &mut dyn FormatReader,
    track_id: u32,
    start_ts: u64,
) -> Option<u64> {
    let mut end: Option<u64> = None;
    loop {
        match format.next_packet() {
            Ok(packet) => {
                if packet.track_id() != track_id {
                    continue;
                }
                let packet_end = packet.ts().saturating_add(packet.dur());
                end = Some(end.map_or(packet_end, |e| e.max(packet_end)));
            }
            // End of stream, or a stream reset: stop counting.
            Err(SymphoniaError::IoError(_)) | Err(SymphoniaError::ResetRequired) => break,
            Err(_) => break,
        }
    }
    end.map(|e| e.saturating_sub(start_ts))
}

fn apply_revision(info: &mut TrackInfo, revision: &MetadataRevision) {
    if !revision.visuals().is_empty() {
        info.has_embedded_artwork = true;
    }
    for tag in revision.tags() {
        let text = value_text(&tag.value);
        // RIFF INFO strings keep their NUL terminator; strip it.
        let Some(text) = text
            .as_deref()
            .map(|t| t.trim_matches(|c: char| c == '\0' || c.is_whitespace()))
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let key = tag.key.to_ascii_lowercase();
        match tag.std_key {
            Some(StandardTagKey::TrackTitle) => set(&mut info.title, text),
            Some(StandardTagKey::Artist) => set(&mut info.artist, text),
            Some(StandardTagKey::Album) => set(&mut info.album, text),
            Some(StandardTagKey::AlbumArtist) => set(&mut info.album_artist, text),
            Some(StandardTagKey::Genre) => set(&mut info.genre, text),
            Some(StandardTagKey::TrackNumber) => {
                let (number, total) = parse_number_pair(text);
                info.track_number = info.track_number.or(number);
                info.track_total = info.track_total.or(total);
            }
            Some(StandardTagKey::TrackTotal) => {
                info.track_total = info.track_total.or_else(|| parse_number_pair(text).0)
            }
            Some(StandardTagKey::DiscNumber) => {
                info.disc_number = info.disc_number.or_else(|| parse_number_pair(text).0)
            }
            Some(StandardTagKey::Date) | Some(StandardTagKey::OriginalDate) => {
                set(&mut info.date, text);
                info.year = info.year.or_else(|| parse_year(text));
            }
            _ => match key.as_str() {
                "title" => set(&mut info.title, text),
                "artist" => set(&mut info.artist, text),
                "album" => set(&mut info.album, text),
                "albumartist" | "album_artist" | "album artist" => set(&mut info.album_artist, text),
                "tracknumber" | "track" | "itrk" => {
                    let (number, total) = parse_number_pair(text);
                    info.track_number = info.track_number.or(number);
                    info.track_total = info.track_total.or(total);
                }
                "year" | "date" | "tyer" | "tdrc" => {
                    set(&mut info.date, text);
                    info.year = info.year.or_else(|| parse_year(text));
                }
                "genre" => set(&mut info.genre, text),
                _ => {}
            },
        }
    }
}

fn set(slot: &mut Option<String>, value: &str) {
    if slot.is_none() {
        *slot = Some(value.to_string());
    }
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::UnsignedInt(n) => Some(n.to_string()),
        Value::SignedInt(n) => Some(n.to_string()),
        Value::Binary(_) | Value::Flag => None,
        other => Some(other.to_string()),
    }
}

/// "3", "03/12", "3 of 12" -> (Some(3), Some(12)).
fn parse_number_pair(text: &str) -> (Option<u32>, Option<u32>) {
    let mut numbers = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u32>().ok());
    let first = numbers.next().flatten();
    let second = numbers.next().flatten();
    (first, second)
}

fn parse_year(text: &str) -> Option<i32> {
    let digits: String = text.chars().skip_while(|c| !c.is_ascii_digit()).take(4).collect();
    if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_digit()) {
        digits.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::decode::SymphoniaDecoder;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::Instant;

    fn info_subchunk(id: &[u8; 4], text: &str) -> Vec<u8> {
        let mut data = text.as_bytes().to_vec();
        data.push(0);
        let mut out = id.to_vec();
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// 16-bit PCM WAV with a LIST/INFO chunk before the data chunk.
    fn write_wav(path: &Path, seconds: u32, sample_rate: u32, channels: u16) {
        let frames = seconds * sample_rate;
        let data_len = frames * channels as u32 * 2;

        let mut info = b"INFO".to_vec();
        info.extend(info_subchunk(b"INAM", "Probe Title"));
        info.extend(info_subchunk(b"IART", "Probe Artist"));
        info.extend(info_subchunk(b"IPRD", "Probe Album"));
        info.extend(info_subchunk(b"ICRD", "2019-05-04"));
        info.extend(info_subchunk(b"ITRK", "7/12"));

        let riff_len = 4 + (8 + 16) + (8 + info.len() as u32) + (8 + data_len);
        let mut f = std::io::BufWriter::new(File::create(path).unwrap());
        f.write_all(b"RIFF").unwrap();
        f.write_all(&riff_len.to_le_bytes()).unwrap();
        f.write_all(b"WAVE").unwrap();
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
        f.write_all(&channels.to_le_bytes()).unwrap();
        f.write_all(&sample_rate.to_le_bytes()).unwrap();
        f.write_all(&(sample_rate * channels as u32 * 2).to_le_bytes()).unwrap();
        f.write_all(&(channels * 2).to_le_bytes()).unwrap();
        f.write_all(&16u16.to_le_bytes()).unwrap();
        f.write_all(b"LIST").unwrap();
        f.write_all(&(info.len() as u32).to_le_bytes()).unwrap();
        f.write_all(&info).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_len.to_le_bytes()).unwrap();
        let mut frame = Vec::with_capacity(channels as usize * 2);
        for i in 0..frames {
            frame.clear();
            let s = (((i % 200) as i32 - 100) * 300) as i16;
            for _ in 0..channels {
                frame.extend_from_slice(&s.to_le_bytes());
            }
            f.write_all(&frame).unwrap();
        }
        f.flush().unwrap();
    }

    fn temp_wav(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lrc_probe_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("probe.wav")
    }

    #[test]
    fn probe_reads_duration_format_and_tags_without_decoding() {
        let path = temp_wav("tags");
        write_wav(&path, 3, 48_000, 2);
        let info = probe_track_info(&path).unwrap();
        assert_eq!(info.duration_ms, Some(3_000));
        assert_eq!(info.duration_source, DurationSource::Header);
        assert_eq!(info.n_frames, Some(144_000));
        assert_eq!(info.sample_rate, Some(48_000));
        assert_eq!(info.channels, Some(2));
        assert_eq!(info.bits_per_sample, Some(16));
        assert_eq!(info.title.as_deref(), Some("Probe Title"));
        assert_eq!(info.artist.as_deref(), Some("Probe Artist"));
        assert_eq!(info.album.as_deref(), Some("Probe Album"));
        assert_eq!(info.year, Some(2019));
        assert_eq!(info.track_number, Some(7));
        assert_eq!(info.track_total, Some(12));
        assert!(!info.has_embedded_artwork);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn packet_scan_fallback_matches_header_duration() {
        let path = temp_wav("scan");
        write_wav(&path, 2, 44_100, 1);
        let mss = MediaSourceStream::new(Box::new(File::open(&path).unwrap()), Default::default());
        let probed = get_probe()
            .format(&Hint::new(), mss, &FormatOptions::default(), &MetadataOptions::default())
            .unwrap();
        let mut format = probed.format;
        let track = format.default_track().unwrap().clone();
        let units = scan_duration_from_packets(format.as_mut(), track.id, track.codec_params.start_ts).unwrap();
        assert_eq!(units, 88_200);
        assert_eq!(ts_to_ms(track.codec_params.time_base.unwrap(), units), 2_000);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn probe_is_much_faster_than_full_decode() {
        let path = temp_wav("speed");
        write_wav(&path, 30, 44_100, 2);

        let start = Instant::now();
        let info = probe_track_info(&path).unwrap();
        let probe_time = start.elapsed();

        let start = Instant::now();
        let decoded = SymphoniaDecoder::new().decode_file(&path).unwrap();
        let decode_time = start.elapsed();

        eprintln!("probe: {probe_time:?}, full decode: {decode_time:?}");
        let decoded_ms = decoded.frames() as u64 * 1000 / decoded.sample_rate as u64;
        assert_eq!(info.duration_ms, Some(30_000));
        assert_eq!(info.duration_ms, Some(decoded_ms));
        // (decode_file reports no title for this file at all; the probe does.)
        assert_eq!(info.title.as_deref(), Some("Probe Title"));
        assert!(
            probe_time * 20 < decode_time,
            "probe {probe_time:?} should be >20x faster than decode {decode_time:?}"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn number_and_year_parsing() {
        assert_eq!(parse_number_pair("03/12"), (Some(3), Some(12)));
        assert_eq!(parse_number_pair("5"), (Some(5), None));
        assert_eq!(parse_number_pair(""), (None, None));
        assert_eq!(parse_year("2021-04-01"), Some(2021));
        assert_eq!(parse_year("1999"), Some(1999));
        assert_eq!(parse_year("99"), None);
    }

    #[test]
    fn probe_errors_on_missing_or_non_audio_file() {
        assert!(probe_track_info("/definitely/not/here.flac").is_err());
        let path = temp_wav("bad").with_extension("txt");
        std::fs::write(&path, b"not audio at all").unwrap();
        assert!(probe_track_info(&path).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
