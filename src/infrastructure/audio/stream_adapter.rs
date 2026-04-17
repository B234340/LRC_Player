// Accept raw decoded sample slices to avoid intermediate allocations.

/// Simple streaming converter that handles channel conversion and linear resampling.
pub struct StreamConverter {
    _src_rate: u32,
    _src_channels: usize,
    _dst_rate: u32,
    dst_channels: usize,
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
        _taps: usize,
    ) -> Self {
        let incr = src_rate as f64 / dst_rate as f64;

        StreamConverter {
            _src_rate: src_rate,
            _src_channels: src_channels as usize,
            _dst_rate: dst_rate,
            dst_channels: dst_channels as usize,
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
            for &v in src[0].iter().take(frames_in) {
                mid[0].push(v);
                mid[1].push(v);
            }
        } else if src_channels as usize > 1 && self.dst_channels == 1 {
            // downmix to mono by averaging
            for (i, _) in src[0].iter().enumerate().take(frames_in) {
                let sum: f32 = src.iter().map(|ch| ch[i]).sum();
                mid[0].push(sum / (src_channels as f32));
            }
        } else if self.dst_channels > src_channels as usize {
            // upmix by repeating channels round-robin
            for (i, _) in src[0].iter().enumerate().take(frames_in) {
                for (dest, slot) in mid.iter_mut().enumerate().take(self.dst_channels) {
                    let src_ch = dest % (src_channels as usize);
                    slot.push(src[src_ch][i]);
                }
            }
        } else {
            // other cases (e.g., src 6 -> dst 2): take first dst channels
            for (dest, slot) in mid.iter_mut().enumerate().take(self.dst_channels) {
                let src_ch = dest.min(src_channels as usize - 1);
                *slot = src[src_ch].clone();
            }
        }

        // Build work buffers with previous-sample continuity.
        let mut work_channels: Vec<Vec<f32>> = Vec::with_capacity(self.dst_channels);
        for (ch, in_chan) in mid.iter().enumerate() {
            let mut work: Vec<f32> = Vec::with_capacity(in_chan.len() + 2);
            if let Some(p) = self.prev[ch] {
                work.push(p);
            }
            work.extend_from_slice(in_chan);
            if let Some(last) = in_chan.last() {
                // Repeat the final sample for interpolation padding.
                work.push(*last);
            }
            work_channels.push(work);
        }

        let work_len = work_channels.first().map(|c| c.len()).unwrap_or(0);
        if work_len < 2 {
            for (ch, in_chan) in mid.iter().enumerate() {
                if let Some(last) = in_chan.last() {
                    self.prev[ch] = Some(*last);
                }
            }
            return;
        }

        // Compute output positions once and apply to all channels to keep channel sync.
        let mut positions: Vec<f64> = Vec::new();
        let mut pos = self.phase;
        while pos + 1.0 < work_len as f64 {
            positions.push(pos);
            pos += self.incr;
        }

        let mut out_channels: Vec<Vec<f32>> =
            vec![Vec::with_capacity(positions.len()); self.dst_channels];
        for (ch, work) in work_channels.iter().enumerate() {
            for &p in positions.iter() {
                let idx = p.floor() as usize;
                let frac = (p - idx as f64) as f32;
                let a = work[idx];
                let b = work[idx + 1];
                out_channels[ch].push(a + (b - a) * frac);
            }
        }

        self.phase = pos - (work_len as f64 - 1.0);

        // store last sample for continuity (last real sample in this chunk)
        for (ch, in_chan) in mid.iter().enumerate() {
            if let Some(last) = in_chan.last() {
                self.prev[ch] = Some(*last);
            }
        }

        // re-interleave outputs into provided `out` buffer
        let out_frames = out_channels.first().map(|c| c.len()).unwrap_or(0);
        out.clear();
        out.reserve(out_frames * self.dst_channels);
        for (i, _) in out_channels[0].iter().enumerate().take(out_frames) {
            for ch_samples in out_channels.iter() {
                out.push(ch_samples[i]);
            }
        }
    }
}


