// Accept raw decoded sample slices to avoid intermediate allocations.

/// Simple streaming converter that handles channel conversion and linear resampling.
pub struct StreamConverter {
    _src_rate: u32,
    _src_channels: usize,
    _dst_rate: u32,
    dst_channels: usize,
    taps: usize,
    window: Vec<f32>,
    // per-target-channel previous sample for continuity
    prev: Vec<Option<f32>>,
    // phase in input-sample units relative to working_input start
    phase: f64,
    // ratio increment per output sample
    incr: f64,
}
impl StreamConverter {
    pub fn new(
        src_rate: u32,
        src_channels: u16,
        dst_rate: u32,
        dst_channels: u16,
        taps: usize,
    ) -> Self {
        let incr = src_rate as f64 / dst_rate as f64;
        let taps = taps.max(4).min(256); // clamp to reasonable range
                                         // precompute Blackman window
        let mut window = Vec::with_capacity(taps);
        let n_max = (taps - 1) as f64;
        for n in 0..taps {
            let n_f = n as f64;
            let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * n_f / n_max).cos()
                + 0.08 * (4.0 * std::f64::consts::PI * n_f / n_max).cos();
            window.push(w as f32);
        }

        StreamConverter {
            _src_rate: src_rate,
            _src_channels: src_channels as usize,
            _dst_rate: dst_rate,
            dst_channels: dst_channels as usize,
            taps,
            window,
            prev: vec![None; dst_channels as usize],
            phase: 0.0,
            incr,
        }
    }

    /// Process an interleaved-decoded chunk and return interleaved samples at the target format.
    pub fn process(
        &mut self,
        samples: &[f32],
        src_channels: u16,
        _src_rate: u32,
        out: &mut Vec<f32>,
    ) {
        let frames_in = samples.len() / (src_channels as usize);
        if frames_in == 0 {
            return;
        }

        // deinterleave
        let mut src: Vec<Vec<f32>> = vec![Vec::with_capacity(frames_in); src_channels as usize];
        for (i, s) in samples.iter().enumerate() {
            let ch = i % (src_channels as usize);
            src[ch].push(*s);
        }

        // channel conversion -> produce dst_channels vectors of length frames_in
        let mut mid: Vec<Vec<f32>> = vec![Vec::with_capacity(frames_in); self.dst_channels];

        if src_channels as usize == self.dst_channels {
            mid = src;
        } else if src_channels == 1 && self.dst_channels == 2 {
            // mono -> stereo duplicate
            for i in 0..frames_in {
                let v = src[0][i];
                mid[0].push(v);
                mid[1].push(v);
            }
        } else if src_channels as usize > 1 && self.dst_channels == 1 {
            // downmix to mono by averaging
            for i in 0..frames_in {
                let mut sum = 0.0f32;
                for ch in 0..src_channels as usize {
                    sum += src[ch][i];
                }
                mid[0].push(sum / (src_channels as f32));
            }
        } else if self.dst_channels > src_channels as usize {
            // upmix by repeating channels round-robin
            for i in 0..frames_in {
                for dest in 0..self.dst_channels {
                    let src_ch = dest % (src_channels as usize);
                    mid[dest].push(src[src_ch][i]);
                }
            }
        } else {
            // other cases (e.g., src 6 -> dst 2): take first dst channels
            for dest in 0..self.dst_channels {
                let src_ch = dest.min(src_channels as usize - 1);
                mid[dest] = src[src_ch].clone();
            }
        }

        // resample per channel using linear interpolation, keeping continuity using `prev` and `phase`.
        let mut out_channels: Vec<Vec<f32>> = vec![Vec::new(); self.dst_channels];

        for ch in 0..self.dst_channels {
            let in_chan = &mid[ch];
            // construct working input: optional prev sample then current samples
            let mut work: Vec<f32> = Vec::with_capacity(in_chan.len() + 1);
            if let Some(p) = self.prev[ch] {
                work.push(p);
            }
            work.extend_from_slice(in_chan);

            let l = work.len();
            // generate output samples while we have idx+1 < l
            // Use windowed-sinc interpolation (higher-quality) with configurable taps
            while (self.phase.floor() as usize) + 1 < l {
                let pos = self.phase;
                let idx = pos.floor() as isize;
                let frac = pos - idx as f64;

                // accumulate using windowed sinc
                let mut sum = 0.0f32;
                let mut norm = 0.0f32;
                let taps_isize = self.taps as isize;
                let half = taps_isize / 2;
                for tap in -half..half {
                    let sample_idx = (idx + tap) as isize;
                    let in_val = if sample_idx < 0 {
                        work[0]
                    } else if (sample_idx as usize) < work.len() {
                        work[sample_idx as usize]
                    } else {
                        work[work.len() - 1]
                    };
                    let x = (tap as f64) - frac;
                    // sinc(pi*x)
                    let sinc = if x == 0.0 {
                        1.0
                    } else {
                        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                    };
                    // window from precomputed table
                    let n = (tap + half) as usize;
                    let w = self.window[n] as f64;
                    let coeff = (sinc * w) as f32;
                    sum += in_val * coeff;
                    norm += coeff.abs();
                }
                if norm > 0.0 {
                    sum /= norm;
                }
                out_channels[ch].push(sum);
                self.phase += self.incr;
            }

            // store last sample for continuity (last real sample in this chunk)
            if !in_chan.is_empty() {
                self.prev[ch] = Some(*in_chan.last().unwrap());
            }

            // adjust phase for next chunk: phase relative to next working input start
            // current working input had length L = prev? + frames_in, and its last index is L-1 which
            // corresponds to next chunk's prev sample. So subtract (L - 1) to rebase phase.
            self.phase -= l as f64 - 1.0;
        }
        // re-interleave outputs into provided `out` buffer
        let out_frames = out_channels.get(0).map(|c| c.len()).unwrap_or(0);
        out.clear();
        out.reserve(out_frames * self.dst_channels);
        for i in 0..out_frames {
            for ch in 0..self.dst_channels {
                out.push(out_channels[ch][i]);
            }
        }
    }
}
