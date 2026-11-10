//! Source-format -> device-format conversion: channel mapping plus band-limited resampling
//! (rubato). The converter is stateful and is fed arbitrary-sized blocks; there are no
//! chunk-boundary artefacts, and the resampler's startup delay is trimmed so output frame `k`
//! corresponds exactly to source time `k / dst_rate`.

use anyhow::{anyhow, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Indexing, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

use super::channel_map::ChannelMatrix;

/// Which resampler to use when the source rate differs from the device rate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResamplerKind {
    /// Synchronous FFT resampler (rubato `Fft`): best quality/CPU for fixed integer rate pairs.
    #[default]
    Fft,
    /// Windowed-sinc resampler (rubato `Async::new_sinc`) with the given filter length
    /// (rounded up to a multiple of 8, clamped to 16..=512). Maps the "resampler taps" setting.
    Sinc { sinc_len: usize },
}

/// Input chunk size handed to rubato (frames).
const RESAMPLER_CHUNK: usize = 1024;

/// Format of decoded source audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceFormat {
    pub(crate) rate: u32,
    pub(crate) channels: usize,
    /// symphonia `Channels` bits, when known.
    pub(crate) layout: Option<u32>,
}

/// Stateful band-limited resampler working on interleaved f32.
pub(crate) struct StreamResampler {
    inner: Box<dyn Resampler<f32>>,
    channels: usize,
    src_rate: u64,
    dst_rate: u64,
    /// Interleaved input accumulator (exactly one rubato chunk when full).
    in_buf: Vec<f32>,
    in_frames: usize,
    out_buf: Vec<f32>,
    delay_left: usize,
    total_in: u64,
    total_out: u64,
}

impl StreamResampler {
    pub(crate) fn new(
        src_rate: u32,
        dst_rate: u32,
        channels: usize,
        kind: ResamplerKind,
    ) -> Result<Self> {
        let channels = channels.max(1);
        let inner: Box<dyn Resampler<f32>> = match kind {
            ResamplerKind::Fft => Box::new(
                Fft::<f32>::new(
                    src_rate as usize,
                    dst_rate as usize,
                    RESAMPLER_CHUNK,
                    channels,
                    FixedSync::Input,
                )
                .map_err(|e| anyhow!("resampler init failed: {e}"))?,
            ),
            ResamplerKind::Sinc { sinc_len } => {
                let len = sinc_len.clamp(16, 512).next_multiple_of(8);
                let params = SincInterpolationParameters::new(len, WindowFunction::BlackmanHarris2)
                    .oversampling_factor(256)
                    .interpolation(SincInterpolationType::Cubic);
                Box::new(
                    Async::<f32>::new_sinc(
                        dst_rate as f64 / src_rate as f64,
                        1.0,
                        &params,
                        RESAMPLER_CHUNK,
                        channels,
                        FixedAsync::Input,
                    )
                    .map_err(|e| anyhow!("resampler init failed: {e}"))?,
                )
            }
        };
        let in_cap = inner.input_frames_max() * channels;
        let out_cap = inner.output_frames_max() * channels;
        let delay = inner.output_delay();
        Ok(Self {
            inner,
            channels,
            src_rate: src_rate as u64,
            dst_rate: dst_rate as u64,
            in_buf: vec![0.0; in_cap],
            in_frames: 0,
            out_buf: vec![0.0; out_cap],
            delay_left: delay,
            total_in: 0,
            total_out: 0,
        })
    }

    /// Output frames that correspond to `in_frames` input frames.
    fn expected_out(&self, in_frames: u64) -> u64 {
        (in_frames * self.dst_rate + self.src_rate / 2) / self.src_rate
    }

    fn run_chunk(
        &mut self,
        partial: Option<usize>,
        out: &mut Vec<f32>,
        limit: Option<u64>,
    ) -> Result<()> {
        let ch = self.channels;
        let in_frames_cap = self.in_buf.len() / ch;
        let out_frames_cap = self.out_buf.len() / ch;
        let input = InterleavedSlice::new(&self.in_buf[..], ch, in_frames_cap)
            .map_err(|e| anyhow!("resampler input: {e:?}"))?;
        let mut output = InterleavedSlice::new_mut(&mut self.out_buf[..], ch, out_frames_cap)
            .map_err(|e| anyhow!("resampler output: {e:?}"))?;
        let indexing = partial.map(|p| Indexing::new().partial_len(p));
        let (_nin, nout) = self
            .inner
            .process_into_buffer(&input, &mut output, indexing.as_ref())
            .map_err(|e| anyhow!("resampler: {e}"))?;
        let skip = self.delay_left.min(nout);
        self.delay_left -= skip;
        let mut emit = nout - skip;
        if let Some(limit) = limit {
            emit = emit.min(limit.saturating_sub(self.total_out) as usize);
        }
        out.extend_from_slice(&self.out_buf[skip * ch..(skip + emit) * ch]);
        self.total_out += emit as u64;
        Ok(())
    }

    /// Feeds interleaved frames; appends whatever output is ready.
    pub(crate) fn process(&mut self, mut input: &[f32], out: &mut Vec<f32>) -> Result<()> {
        let ch = self.channels;
        while !input.is_empty() {
            let need = self.inner.input_frames_next();
            let take = (need - self.in_frames).min(input.len() / ch);
            if take == 0 {
                break;
            }
            let start = self.in_frames * ch;
            self.in_buf[start..start + take * ch].copy_from_slice(&input[..take * ch]);
            self.in_frames += take;
            self.total_in += take as u64;
            input = &input[take * ch..];
            if self.in_frames == need {
                self.run_chunk(None, out, None)?;
                self.in_frames = 0;
            }
        }
        Ok(())
    }

    /// Drains the remaining delay so that the total output equals `round(total_in * ratio)`,
    /// then resets for reuse.
    pub(crate) fn flush(&mut self, out: &mut Vec<f32>) -> Result<()> {
        let target = self.expected_out(self.total_in);
        if self.in_frames > 0 {
            // Zero the tail of the chunk (the resampler reads the full buffer).
            let start = self.in_frames * self.channels;
            self.in_buf[start..].iter_mut().for_each(|s| *s = 0.0);
            let p = self.in_frames;
            self.run_chunk(Some(p), out, Some(target))?;
            self.in_frames = 0;
        }
        self.in_buf.iter_mut().for_each(|s| *s = 0.0);
        let mut guard = 0;
        while self.total_out < target && guard < 64 {
            self.run_chunk(Some(0), out, Some(target))?;
            guard += 1;
        }
        self.reset();
        Ok(())
    }

    pub(crate) fn reset(&mut self) {
        self.inner.reset();
        self.in_frames = 0;
        self.delay_left = self.inner.output_delay();
        self.total_in = 0;
        self.total_out = 0;
    }
}

/// Channel mapping + optional resampling from one source format to the device format.
pub(crate) struct Converter {
    src: SourceFormat,
    dst_rate: u32,
    dst_channels: usize,
    /// Mapping applied before resampling (when it reduces the channel count) or after.
    matrix: ChannelMatrix,
    map_first: bool,
    resampler: Option<StreamResampler>,
    scratch: Vec<f32>,
    total_in: u64,
}

impl Converter {
    pub(crate) fn new(
        src: SourceFormat,
        dst_rate: u32,
        dst_channels: usize,
        kind: ResamplerKind,
    ) -> Result<Self> {
        let matrix = ChannelMatrix::new(src.channels, src.layout, dst_channels);
        let map_first = dst_channels <= src.channels;
        let resampler = if src.rate != dst_rate {
            let ch = if map_first {
                dst_channels
            } else {
                src.channels
            };
            Some(StreamResampler::new(src.rate, dst_rate, ch, kind)?)
        } else {
            None
        };
        Ok(Self {
            src,
            dst_rate,
            dst_channels,
            matrix,
            map_first,
            resampler,
            scratch: Vec::new(),
            total_in: 0,
        })
    }

    pub(crate) fn source_format(&self) -> SourceFormat {
        self.src
    }

    /// Whether audio of `fmt` can continue through this converter without a reset
    /// (enables seamless gapless joins through a running resampler).
    pub(crate) fn accepts(&self, fmt: &SourceFormat) -> bool {
        self.src.rate == fmt.rate
            && self.src.channels == fmt.channels
            && self.src.layout == fmt.layout
    }

    /// Source frames fed since creation/reset.
    pub(crate) fn total_in(&self) -> u64 {
        self.total_in
    }

    /// Output frames corresponding to `in_frames` source frames.
    pub(crate) fn out_frames_for(&self, in_frames: u64) -> u64 {
        if self.src.rate == self.dst_rate {
            in_frames
        } else {
            (in_frames * self.dst_rate as u64 + self.src.rate as u64 / 2) / self.src.rate as u64
        }
    }

    /// Converts interleaved source samples, appending device-format samples to `out`.
    pub(crate) fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<()> {
        let frames = input.len() / self.src.channels;
        let input = &input[..frames * self.src.channels];
        self.total_in += frames as u64;
        match self.resampler.as_mut() {
            None => {
                self.matrix.apply(input, out);
                Ok(())
            }
            Some(rs) => {
                if self.map_first {
                    if self.matrix.is_identity() {
                        rs.process(input, out)
                    } else {
                        self.scratch.clear();
                        self.matrix.apply(input, &mut self.scratch);
                        rs.process(&self.scratch, out)
                    }
                } else {
                    self.scratch.clear();
                    rs.process(input, &mut self.scratch)?;
                    self.matrix.apply(&self.scratch, out);
                    Ok(())
                }
            }
        }
    }

    /// Emits the resampler tail (end of stream / before switching format). Resets state.
    pub(crate) fn flush(&mut self, out: &mut Vec<f32>) -> Result<()> {
        self.total_in = 0;
        if let Some(rs) = self.resampler.as_mut() {
            if self.map_first {
                rs.flush(out)?;
            } else {
                self.scratch.clear();
                rs.flush(&mut self.scratch)?;
                self.matrix.apply(&self.scratch, out);
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn dst_channels(&self) -> usize {
        debug_assert_eq!(self.dst_channels, self.matrix.dst_channels());
        self.dst_channels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f32, frames: usize, ch: usize, amp: f32) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames * ch);
        for n in 0..frames {
            let x = amp
                * (2.0 * std::f64::consts::PI * freq as f64 * n as f64 / rate as f64).sin() as f32;
            for _ in 0..ch {
                v.push(x);
            }
        }
        v
    }

    fn run_chunked(
        conv: &mut Converter,
        input: &[f32],
        ch: usize,
        chunk_frames: &[usize],
    ) -> Vec<f32> {
        let mut out = Vec::new();
        let mut pos = 0;
        let mut i = 0;
        while pos < input.len() {
            let n = (chunk_frames[i % chunk_frames.len()] * ch).min(input.len() - pos);
            conv.process(&input[pos..pos + n], &mut out).unwrap();
            pos += n;
            i += 1;
        }
        conv.flush(&mut out).unwrap();
        out
    }

    #[test]
    fn equal_rate_is_bit_exact_across_many_chunks() {
        let src = SourceFormat {
            rate: 44_100,
            channels: 2,
            layout: None,
        };
        let mut conv = Converter::new(src, 44_100, 2, ResamplerKind::Fft).unwrap();
        let input: Vec<f32> = (0..200_000)
            .map(|i| ((i as f32) * 0.000_01).sin())
            .collect();
        let out = run_chunked(&mut conv, &input, 2, &[2048, 1, 333, 4096, 7]);
        assert_eq!(out.len(), input.len());
        assert!(out
            .iter()
            .zip(&input)
            .all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    fn check_upsample(kind: ResamplerKind) {
        let frames = 44_100 * 2;
        let src = SourceFormat {
            rate: 44_100,
            channels: 2,
            layout: None,
        };
        let input = sine(44_100, 1000.0, frames, 2, 0.5);
        let mut conv = Converter::new(src, 48_000, 2, kind).unwrap();
        let out = run_chunked(&mut conv, &input, 2, &[1152, 4096, 17, 2048]);
        let out_frames = out.len() / 2;
        let expected = (frames as u64 * 48_000 + 22_050) / 44_100;
        assert!(
            (out_frames as i64 - expected as i64).abs() <= 1,
            "{out_frames} vs {expected}"
        );
        // Compare against the analytic 1 kHz sine at 48 kHz (skip the edges where the
        // band-limited filter sees the implicit zeros before/after the clip). The sinc
        // resampler reports an integer delay, so allow a sub-sample alignment there.
        let shifts: Vec<f64> = match kind {
            ResamplerKind::Fft => vec![0.0],
            ResamplerKind::Sinc { .. } => (-100..=100).map(|i| i as f64 / 100.0).collect(),
        };
        let mut max_err = f32::MAX;
        for shift in shifts {
            let mut e = 0.0f32;
            for n in 2000..out_frames - 2000 {
                let t = (n as f64 + shift) / 48_000.0;
                let ideal = 0.5 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f32;
                e = e.max((out[n * 2] - ideal).abs());
            }
            max_err = max_err.min(e);
        }
        let mut max_delta = 0.0f32;
        for n in 2000..out_frames - 2000 {
            max_delta = max_delta.max((out[n * 2] - out[(n - 1) * 2]).abs());
            assert_eq!(out[n * 2], out[n * 2 + 1]);
        }
        let bound = 0.5 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0;
        assert!(max_delta <= bound * 1.02, "max delta {max_delta} > {bound}");
        assert!(max_err < 2e-3, "max error vs ideal {max_err}");
    }

    #[test]
    fn upsample_44k1_to_48k_fft_is_continuous_and_correct_length() {
        check_upsample(ResamplerKind::Fft);
    }

    #[test]
    fn upsample_44k1_to_48k_sinc_is_continuous_and_correct_length() {
        check_upsample(ResamplerKind::Sinc { sinc_len: 128 });
    }

    #[test]
    fn chunking_does_not_change_resampled_output() {
        let src = SourceFormat {
            rate: 48_000,
            channels: 1,
            layout: None,
        };
        let input = sine(48_000, 440.0, 30_000, 1, 0.3);
        let mut a = Converter::new(src, 44_100, 2, ResamplerKind::Fft).unwrap();
        let mut b = Converter::new(src, 44_100, 2, ResamplerKind::Fft).unwrap();
        let oa = run_chunked(&mut a, &input, 1, &[30_000]);
        let ob = run_chunked(&mut b, &input, 1, &[1, 2, 3, 511, 1024, 7]);
        assert_eq!(oa.len(), ob.len());
        let max = oa
            .iter()
            .zip(&ob)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max);
        assert!(max < 1e-6, "chunked differs by {max}");
    }

    /// Final packets shorter than the resampler's chunk (and whole tracks shorter than its
    /// delay) must still come out with the right length.
    #[test]
    fn tiny_inputs_resample_to_the_right_length() {
        for kind in [ResamplerKind::Fft, ResamplerKind::Sinc { sinc_len: 64 }] {
            for (src_rate, dst_rate) in [(44_100, 48_000), (192_000, 48_000), (8_000, 48_000)] {
                for frames in [1usize, 7, 50, 1023, 1025] {
                    let src = SourceFormat {
                        rate: src_rate,
                        channels: 2,
                        layout: None,
                    };
                    let mut conv = Converter::new(src, dst_rate, 2, kind).unwrap();
                    let input = sine(src_rate, 100.0, frames, 2, 0.5);
                    let out = run_chunked(&mut conv, &input, 2, &[3]);
                    let expected =
                        (frames as u64 * dst_rate as u64 + src_rate as u64 / 2) / src_rate as u64;
                    assert_eq!(
                        (out.len() / 2) as u64,
                        expected,
                        "{kind:?} {src_rate}->{dst_rate} {frames} frames"
                    );
                    assert!(out.iter().all(|v| v.is_finite() && v.abs() < 1.0));
                }
            }
        }
    }
}
