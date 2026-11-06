//! Track decoding (symphonia): open/probe, packet filtering by track id, buffer-spec driven
//! format detection, fast accurate seeking, and metadata probing without decoding.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

use super::convert::SourceFormat;

/// Static information about a track, obtained without decoding audio.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackInfo {
    pub path: PathBuf,
    /// From the container's frame count / time base (no decode). `None` if unknown.
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// Probes `path` for duration, format and tags. Reads only headers/metadata (fast).
pub fn probe_track(path: impl AsRef<Path>) -> Result<TrackInfo> {
    let path = path.as_ref();
    let (format, mut probed_meta) = open_format(path)?;
    let track = select_track(format.as_ref())?;
    let mut info = info_from_track(path, track);
    if let Some(m) = probed_meta.as_mut() {
        if let Some(rev) = m.get().as_ref().and_then(|m| m.current().cloned()) {
            apply_tags(&mut info, &rev);
        }
    }
    let mut format = format;
    if let Some(rev) = format.metadata().skip_to_latest() {
        apply_tags(&mut info, rev);
    }
    Ok(info)
}

type ProbedMeta = Option<symphonia::core::probe::ProbedMetadata>;

fn open_format(path: &Path) -> Result<(Box<dyn FormatReader>, ProbedMeta)> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let format_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &MetadataOptions::default())
        .with_context(|| format!("unsupported or corrupt file {}", path.display()))?;
    Ok((probed.format, Some(probed.metadata)))
}

fn select_track(format: &dyn FormatReader) -> Result<&symphonia::core::formats::Track> {
    format
        .default_track()
        .filter(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .or_else(|| {
            format.tracks().iter().find(|t| {
                t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some()
            })
        })
        .ok_or_else(|| anyhow!("no decodable audio track"))
}

fn info_from_track(path: &Path, track: &symphonia::core::formats::Track) -> TrackInfo {
    let p = &track.codec_params;
    let duration_ms = match (p.n_frames, p.time_base, p.sample_rate) {
        (Some(n), Some(tb), _) => Some(time_to_ms(tb.calc_time(n))),
        // u128: a corrupt header can claim an absurd frame count.
        (Some(n), None, Some(sr)) if sr > 0 => Some((n as u128 * 1000 / sr as u128) as u64),
        _ => None,
    };
    TrackInfo {
        path: path.to_path_buf(),
        duration_ms,
        sample_rate: p.sample_rate,
        channels: p.channels.map(|c| c.count() as u16),
        ..Default::default()
    }
}

fn time_to_ms(t: Time) -> u64 {
    t.seconds
        .saturating_mul(1000)
        .saturating_add((t.frac * 1000.0).round() as u64)
}

fn apply_tags(info: &mut TrackInfo, rev: &MetadataRevision) {
    for tag in rev.tags() {
        let slot = match tag.std_key {
            Some(StandardTagKey::TrackTitle) => &mut info.title,
            Some(StandardTagKey::Artist) => &mut info.artist,
            Some(StandardTagKey::Album) => &mut info.album,
            _ => continue,
        };
        if slot.is_none() {
            let v = tag.value.to_string();
            if !v.trim().is_empty() {
                *slot = Some(v);
            }
        }
    }
}

/// Corrupt packets in a row before a track is abandoned.
const MAX_CONSECUTIVE_ERRORS: u32 = 64;

/// An open, decodable track.
pub(crate) struct TrackSource {
    path: PathBuf,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    sample_rate: u32,
    info: TrackInfo,
    sample_buf: Option<SampleBuffer<f32>>,
    /// Range of `sample_buf.samples()` holding the current block.
    block: (usize, usize),
    /// After a seek: discard decoded audio before this timestamp (track time base units).
    discard_until_ts: Option<u64>,
    eof: bool,
    consecutive_errors: u32,
}

impl TrackSource {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let (format, mut probed_meta) = open_format(path)?;
        let track = select_track(format.as_ref())?;
        let track_id = track.id;
        let mut info = info_from_track(path, track);
        let time_base = track.codec_params.time_base;
        let sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .with_context(|| format!("no decoder for {}", path.display()))?;
        if let Some(m) = probed_meta.as_mut() {
            if let Some(rev) = m.get().as_ref().and_then(|m| m.current().cloned()) {
                apply_tags(&mut info, &rev);
            }
        }
        let mut format = format;
        if let Some(rev) = format.metadata().skip_to_latest() {
            apply_tags(&mut info, rev);
        }
        Ok(Self {
            path: path.to_path_buf(),
            format,
            decoder,
            track_id,
            time_base,
            sample_rate,
            info,
            sample_buf: None,
            block: (0, 0),
            discard_until_ts: None,
            eof: false,
            consecutive_errors: 0,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn info(&self) -> &TrackInfo {
        &self.info
    }

    fn ts_to_frames(&self, ts: u64) -> u64 {
        match self.time_base {
            Some(tb) if tb.numer == 1 && tb.denom == self.sample_rate => ts,
            Some(tb) => {
                (ts as u128 * tb.numer as u128 * self.sample_rate as u128 / tb.denom.max(1) as u128)
                    as u64
            }
            None => ts,
        }
    }

    fn ms_to_ts(&self, ms: u64) -> u64 {
        let time = Time::new(ms / 1000, (ms % 1000) as f64 / 1000.0);
        match self.time_base {
            Some(tb) => tb.calc_timestamp(time),
            None => ms * self.sample_rate as u64 / 1000,
        }
    }

    /// Seeks so that the next decoded frame is exactly at `ms` (sample accurate). Uses the
    /// container's seek index (no decode-from-start). Seeking past the end yields EOF.
    pub(crate) fn seek_ms(&mut self, ms: u64) -> Result<()> {
        self.eof = false;
        self.block = (0, 0);
        let required = self.ms_to_ts(ms);
        let res = self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time: Time::new(ms / 1000, (ms % 1000) as f64 / 1000.0),
                track_id: Some(self.track_id),
            },
        );
        self.decoder.reset();
        match res {
            Ok(seeked) => {
                self.discard_until_ts = Some(seeked.required_ts);
                Ok(())
            }
            Err(SymError::SeekError(symphonia::core::errors::SeekErrorKind::OutOfRange)) => {
                self.eof = true;
                Ok(())
            }
            Err(e) => {
                // Unseekable stream: reopen and skip by decoding (slow fallback, rare).
                tracing::warn!(
                    "seek failed on {} ({e}); falling back to decode-skip",
                    self.path.display()
                );
                let mut fresh = TrackSource::open(&self.path.clone())?;
                fresh.discard_until_ts = Some(required);
                *self = fresh;
                Ok(())
            }
        }
    }

    /// Decodes the next block. Returns its format, or `None` at end of stream. The samples
    /// are available through [`TrackSource::block`] until the next call.
    pub(crate) fn decode_next(&mut self) -> Result<Option<SourceFormat>> {
        if self.eof {
            return Ok(None);
        }
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.eof = true;
                    return Ok(None);
                }
                Err(SymError::ResetRequired) => {
                    // Chained stream with a new track list: treat as end of this track.
                    self.eof = true;
                    return Ok(None);
                }
                Err(SymError::IoError(e)) => {
                    self.eof = true;
                    return Err(anyhow!("read error: {e}"));
                }
                Err(SymError::DecodeError(e)) => {
                    // Corrupt container data (bad frame header, lost sync...): the reader
                    // resynchronises on the next call. Give up only on a long run of errors.
                    self.consecutive_errors += 1;
                    tracing::debug!("demux error in {}: {e}", self.path.display());
                    if self.consecutive_errors > MAX_CONSECUTIVE_ERRORS {
                        self.eof = true;
                        return Err(anyhow!("too many demux errors: {e}"));
                    }
                    continue;
                }
                Err(e) => {
                    self.eof = true;
                    return Err(anyhow!("demux error: {e}"));
                }
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let packet_ts = packet.ts();
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(e)) => {
                    self.consecutive_errors += 1;
                    tracing::debug!("decode error in {}: {e}", self.path.display());
                    if self.consecutive_errors > MAX_CONSECUTIVE_ERRORS {
                        self.eof = true;
                        return Err(anyhow!("too many decode errors: {e}"));
                    }
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.eof = true;
                    return Ok(None);
                }
                Err(e) => {
                    self.eof = true;
                    return Err(anyhow!("decode failed: {e}"));
                }
            };
            self.consecutive_errors = 0;
            let spec = *decoded.spec();
            let frames = decoded.frames();
            if frames == 0 || spec.channels.count() == 0 {
                continue;
            }
            let ch = spec.channels.count();
            let need_new = match &self.sample_buf {
                Some(sb) => sb.capacity() < frames * ch,
                None => true,
            };
            if need_new {
                self.sample_buf = Some(SampleBuffer::<f32>::new((frames as u64).max(4096), spec));
            }
            let sb = self.sample_buf.as_mut().expect("sample buffer");
            sb.copy_interleaved_ref(decoded);
            let len = sb.samples().len().min(frames * ch);

            let mut skip_frames = 0usize;
            if let Some(until) = self.discard_until_ts {
                if packet_ts >= until {
                    self.discard_until_ts = None;
                } else {
                    let delta = self.ts_to_frames(until - packet_ts) as usize;
                    if delta >= frames {
                        continue;
                    }
                    skip_frames = delta;
                    self.discard_until_ts = None;
                }
            }
            self.sample_rate = spec.rate;
            self.block = (skip_frames * ch, len);
            return Ok(Some(SourceFormat {
                rate: spec.rate,
                channels: ch,
                layout: Some(spec.channels.bits()),
            }));
        }
    }

    /// Samples of the block returned by the last successful [`TrackSource::decode_next`].
    pub(crate) fn block(&self) -> &[f32] {
        match &self.sample_buf {
            Some(sb) => &sb.samples()[self.block.0..self.block.1],
            None => &[],
        }
    }
}
