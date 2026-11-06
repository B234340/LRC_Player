//! Legacy streaming converter (channel mapping + linear-interpolation resampling) used by the
//! old `spawn_decode_to_producer` path. The new `engine` module uses a band-limited rubato
//! resampler instead; this type is kept for API compatibility.
//!
//! Fixed defects:
//! - DSP-1: the previous implementation appended a duplicate of the last sample as padding and
//!   then rebased the phase on it, which rewound one input sample at every chunk boundary
//!   (audible ~21 Hz tick, 0.05 % drift). The phase is now tracked relative to the last *real*
//!   sample, so chunked processing equals one-shot processing.
//! - Equal rates are a pure copy/channel-map (bit-exact).
//! - Channel mapping uses a proper matrix (5.1 -> stereo keeps the centre, stereo -> 5.1 goes
//!   to FL/FR only).
//! - No per-chunk allocations after warm-up.

use super::engine::channel_map::ChannelMatrix;

/// Streaming converter that handles channel conversion and linear resampling.
pub struct StreamConverter {
    src_rate: u32,
    src_channels: usize,
    dst_rate: u32,
    dst_channels: usize,
    matrix: ChannelMatrix,
    /// Last mapped frame of the previous chunk (dst channels), if any.
    prev: Vec<f32>,
    has_prev: bool,
    /// Next output position, in input frames, relative to `prev` (index 0 of the work buffer).
    phase: f64,
    /// Input frames per output frame.
    incr: f64,
    mapped: Vec<f32>,
}

impl StreamConverter {
    pub fn new(
        src_rate: u32,
        src_channels: u16,
        dst_rate: u32,
        dst_channels: u16,
        _taps: usize,
    ) -> Self {
        let src_channels = (src_channels as usize).max(1);
        let dst_channels = (dst_channels as usize).max(1);
        StreamConverter {
            src_rate,
            src_channels,
            dst_rate,
            dst_channels,
            matrix: ChannelMatrix::new(src_channels, None, dst_channels),
            prev: vec![0.0; dst_channels],
            has_prev: false,
            phase: 0.0,
            incr: src_rate as f64 / dst_rate.max(1) as f64,
            mapped: Vec::new(),
        }
    }

    fn sample(&self, frame: usize, ch: usize) -> f32 {
        // Frame 0 is `prev` when present; the rest come from `mapped`.
        if self.has_prev {
            if frame == 0 {
                self.prev[ch]
            } else {
                self.mapped[(frame - 1) * self.dst_channels + ch]
            }
        } else {
            self.mapped[frame * self.dst_channels + ch]
        }
    }

    /// Process an interleaved-decoded chunk and write interleaved samples at the target
    /// format into `out` (which is cleared first).
    pub fn process(
        &mut self,
        samples: &[f32],
        src_channels: u16,
        _src_rate: u32,
        out: &mut Vec<f32>,
    ) {
        out.clear();
        let src_ch = (src_channels as usize).max(1);
        if src_ch != self.src_channels {
            self.src_channels = src_ch;
            self.matrix = ChannelMatrix::new(src_ch, None, self.dst_channels);
        }
        let frames_in = samples.len() / src_ch;
        if frames_in == 0 {
            return;
        }
        let samples = &samples[..frames_in * src_ch];

        if self.src_rate == self.dst_rate {
            self.matrix.apply(samples, out);
            return;
        }

        self.mapped.clear();
        self.matrix.apply(samples, &mut self.mapped);
        let dst = self.dst_channels;
        let work_len = frames_in + usize::from(self.has_prev);
        let estimate = ((work_len as f64 - self.phase) / self.incr).ceil().max(0.0) as usize + 1;
        out.reserve(estimate * dst);

        let mut pos = self.phase;
        while pos + 1.0 < work_len as f64 {
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            for ch in 0..dst {
                let a = self.sample(idx, ch);
                let b = self.sample(idx + 1, ch);
                out.push(a + (b - a) * frac);
            }
            pos += self.incr;
        }
        // The last real sample becomes index 0 of the next call.
        self.phase = pos - (work_len as f64 - 1.0);
        let last = &self.mapped[(frames_in - 1) * dst..frames_in * dst];
        self.prev.copy_from_slice(last);
        self.has_prev = true;
    }

    /// Emits the output frames that fall between the last input sample and the end of the
    /// stream (holding the last sample), then resets. Call once at end of stream.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        out.clear();
        if self.src_rate != self.dst_rate && self.has_prev {
            let mut pos = self.phase;
            while pos < 1.0 {
                out.extend_from_slice(&self.prev);
                pos += self.incr;
            }
        }
        self.has_prev = false;
        self.phase = 0.0;
    }

    pub fn src_rate(&self) -> u32 {
        self.src_rate
    }

    pub fn dst_rate(&self) -> u32 {
        self.dst_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(conv: &mut StreamConverter, input: &[f32], ch: u16, sizes: &[usize]) -> Vec<f32> {
        let mut all = Vec::new();
        let mut out = Vec::new();
        let mut pos = 0;
        let mut i = 0;
        while pos < input.len() {
            let n = (sizes[i % sizes.len()] * ch as usize).min(input.len() - pos);
            conv.process(&input[pos..pos + n], ch, conv.src_rate(), &mut out);
            all.extend_from_slice(&out);
            pos += n;
            i += 1;
        }
        conv.flush(&mut out);
        all.extend_from_slice(&out);
        all
    }

    /// DSP-1 regression: 50 chunks of 2048 frames at equal rates must give exactly the input
    /// (the old code emitted 102449 frames with 49 duplicated samples).
    #[test]
    fn equal_rate_multi_chunk_is_exact_without_duplicates() {
        for rate in [44_100, 48_000] {
            let mut conv = StreamConverter::new(rate, 2, rate, 2, 32);
            let input: Vec<f32> = (0..2048 * 50 * 2).map(|i| i as f32).collect();
            let out = run(&mut conv, &input, 2, &[2048]);
            assert_eq!(out.len(), input.len());
            assert_eq!(out, input);
        }
    }

    /// Ramp through 44.1k -> 48k in many odd-sized chunks: constant step, no repeats, matches
    /// one-shot processing, and the length is within one frame of the ideal.
    #[test]
    fn resampled_ramp_is_identical_for_any_chunking() {
        let frames = 2048 * 20;
        let input: Vec<f32> = (0..frames).map(|i| i as f32).collect();
        let mut one = StreamConverter::new(44_100, 1, 48_000, 1, 32);
        let mut many = StreamConverter::new(44_100, 1, 48_000, 1, 32);
        let a = run(&mut one, &input, 1, &[frames]);
        let b = run(&mut many, &input, 1, &[2048, 1, 333, 7, 4096]);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() < 1e-3, "{x} vs {y}");
        }
        let step = 44_100.0 / 48_000.0;
        for w in b.windows(2).take(b.len() - 2) {
            let d = w[1] - w[0];
            assert!((d - step as f32).abs() < 1e-2, "non-uniform step {d}");
        }
        let expected = frames as f64 * 48_000.0 / 44_100.0;
        assert!(
            (b.len() as f64 - expected).abs() <= 1.0,
            "{} vs {expected}",
            b.len()
        );
    }

    #[test]
    fn five_one_downmix_keeps_centre() {
        let mut conv = StreamConverter::new(48_000, 6, 48_000, 2, 32);
        let mut out = Vec::new();
        conv.process(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0], 6, 48_000, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out[0] > 0.7 && out[1] > 0.7);
    }
}
