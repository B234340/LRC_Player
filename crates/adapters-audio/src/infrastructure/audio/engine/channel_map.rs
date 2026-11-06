//! Channel mapping (down/up-mix) between a decoded stream layout and the device layout.
//!
//! Source layouts come from symphonia's `Channels` bitmask (interleaved in bit order); device
//! layouts are assumed to follow the WAVEFORMATEXTENSIBLE / SMPTE default order for the channel
//! count (FL FR FC LFE BL BR SL SR), which is what WASAPI and CoreAudio use for their default
//! multichannel formats.
//!
//! Rules:
//! - identical layouts are copied verbatim (bit-exact);
//! - a channel present in both layouts maps 1:1;
//! - stereo -> 5.1/7.1 goes to FL/FR only (never full-band into C or LFE);
//! - mono -> stereo duplicates into FL and FR;
//! - downmix to stereo follows ITU-R BS.775: C and surrounds at -3 dB, LFE dropped.

const MINUS_3DB: f32 = std::f32::consts::FRAC_1_SQRT_2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Mono,
    FL,
    FR,
    FC,
    Lfe,
    BL,
    BR,
    BC,
    SL,
    SR,
    /// Other left-side channel (wide, high, top...).
    LeftOther,
    RightOther,
    CentreOther,
    Unused,
}

// symphonia::core::audio::Channels bit values.
const CH_FRONT_LEFT: u32 = 0x0000_0001;
const CH_FRONT_RIGHT: u32 = 0x0000_0002;
const CH_FRONT_CENTRE: u32 = 0x0000_0004;
const CH_LFE1: u32 = 0x0000_0008;
const CH_REAR_LEFT: u32 = 0x0000_0010;
const CH_REAR_RIGHT: u32 = 0x0000_0020;
const CH_FRONT_LEFT_CENTRE: u32 = 0x0000_0040;
const CH_FRONT_RIGHT_CENTRE: u32 = 0x0000_0080;
const CH_REAR_CENTRE: u32 = 0x0000_0100;
const CH_SIDE_LEFT: u32 = 0x0000_0200;
const CH_SIDE_RIGHT: u32 = 0x0000_0400;
const CH_LFE2: u32 = 0x0200_0000;
const LEFT_OTHERS: u32 = 0x0000_1000 | 0x0000_8000 | 0x0004_0000 | 0x0010_0000 | 0x0040_0000;
const RIGHT_OTHERS: u32 = 0x0000_4000 | 0x0002_0000 | 0x0008_0000 | 0x0020_0000 | 0x0100_0000;

fn role_from_bit(bit: u32) -> Role {
    match bit {
        CH_FRONT_LEFT => Role::FL,
        CH_FRONT_RIGHT => Role::FR,
        CH_FRONT_CENTRE => Role::FC,
        CH_LFE1 | CH_LFE2 => Role::Lfe,
        CH_REAR_LEFT => Role::BL,
        CH_REAR_RIGHT => Role::BR,
        CH_FRONT_LEFT_CENTRE => Role::LeftOther,
        CH_FRONT_RIGHT_CENTRE => Role::RightOther,
        CH_REAR_CENTRE => Role::BC,
        CH_SIDE_LEFT => Role::SL,
        CH_SIDE_RIGHT => Role::SR,
        b if b & LEFT_OTHERS != 0 => Role::LeftOther,
        b if b & RIGHT_OTHERS != 0 => Role::RightOther,
        _ => Role::CentreOther,
    }
}

/// Default (SMPTE / WAVEFORMATEXTENSIBLE) order for a channel count.
fn default_roles(count: usize) -> Vec<Role> {
    use Role::*;
    let base: &[Role] = match count {
        0 => &[],
        1 => &[Mono],
        2 => &[FL, FR],
        3 => &[FL, FR, FC],
        4 => &[FL, FR, BL, BR],
        5 => &[FL, FR, FC, BL, BR],
        6 => &[FL, FR, FC, Lfe, BL, BR],
        7 => &[FL, FR, FC, Lfe, BC, SL, SR],
        _ => &[FL, FR, FC, Lfe, BL, BR, SL, SR],
    };
    let mut v = base.to_vec();
    v.resize(count, Unused);
    v
}

fn source_roles(count: usize, layout_bits: Option<u32>) -> Vec<Role> {
    if count == 1 {
        return vec![Role::Mono];
    }
    if let Some(bits) = layout_bits {
        if bits.count_ones() as usize == count {
            let mut roles = Vec::with_capacity(count);
            for i in 0..32 {
                let bit = 1u32 << i;
                if bits & bit != 0 {
                    roles.push(role_from_bit(bit));
                }
            }
            return roles;
        }
    }
    default_roles(count)
}

/// A dense `dst x src` mixing matrix, or a verbatim copy when layouts match.
#[derive(Clone, Debug)]
pub(crate) struct ChannelMatrix {
    src: usize,
    dst: usize,
    /// Row-major `dst` rows of `src` gains; empty for identity.
    gains: Vec<f32>,
}

impl ChannelMatrix {
    /// Builds the matrix for `src` channels (optional symphonia layout bits) to a device with
    /// `dst` channels in default order.
    pub(crate) fn new(src: usize, layout_bits: Option<u32>, dst: usize) -> Self {
        let src = src.max(1);
        let dst = dst.max(1);
        let s_roles = source_roles(src, layout_bits);
        let d_roles = default_roles(dst);
        if s_roles == d_roles {
            return Self {
                src,
                dst,
                gains: Vec::new(),
            };
        }
        let mut gains = vec![0.0f32; src * dst];
        let find = |r: Role| d_roles.iter().position(|&d| d == r);
        let has_lr = find(Role::FL).is_some() && find(Role::FR).is_some();
        let dst_mono = dst == 1;
        {
            let mut add = |d: usize, s: usize, g: f32| gains[d * src + s] += g;
            for (s, &role) in s_roles.iter().enumerate() {
                if dst_mono {
                    // Mono fold-down: equivalent to the stereo downmix averaged.
                    let g = match role {
                        Role::Mono => 1.0,
                        Role::FL | Role::FR | Role::LeftOther | Role::RightOther => 0.5,
                        Role::FC | Role::CentreOther => MINUS_3DB,
                        Role::BL | Role::BR | Role::SL | Role::SR => 0.5 * MINUS_3DB,
                        Role::BC => 0.5,
                        Role::Lfe | Role::Unused => 0.0,
                    };
                    add(0, s, g);
                    continue;
                }
                if role == Role::Mono {
                    if has_lr {
                        add(find(Role::FL).unwrap(), s, 1.0);
                        add(find(Role::FR).unwrap(), s, 1.0);
                    } else if let Some(c) = find(Role::FC) {
                        add(c, s, 1.0);
                    } else {
                        add(0, s, 1.0);
                    }
                    continue;
                }
                if role != Role::Unused {
                    if let Some(d) = find(role) {
                        add(d, s, 1.0);
                        continue;
                    }
                }
                match role {
                    Role::FC | Role::CentreOther => {
                        if role == Role::CentreOther {
                            if let Some(c) = find(Role::FC) {
                                add(c, s, MINUS_3DB);
                                continue;
                            }
                        }
                        if has_lr {
                            add(find(Role::FL).unwrap(), s, MINUS_3DB);
                            add(find(Role::FR).unwrap(), s, MINUS_3DB);
                        }
                    }
                    Role::BL | Role::SL | Role::LeftOther => {
                        let alt = match role {
                            Role::BL => find(Role::SL),
                            Role::SL => find(Role::BL),
                            _ => None,
                        };
                        if let Some(d) = alt {
                            add(d, s, 1.0);
                        } else if let Some(d) = find(Role::FL) {
                            add(d, s, MINUS_3DB);
                        }
                    }
                    Role::BR | Role::SR | Role::RightOther => {
                        let alt = match role {
                            Role::BR => find(Role::SR),
                            Role::SR => find(Role::BR),
                            _ => None,
                        };
                        if let Some(d) = alt {
                            add(d, s, 1.0);
                        } else if let Some(d) = find(Role::FR) {
                            add(d, s, MINUS_3DB);
                        }
                    }
                    Role::BC => {
                        if let (Some(l), Some(r)) = (find(Role::BL), find(Role::BR)) {
                            add(l, s, MINUS_3DB);
                            add(r, s, MINUS_3DB);
                        } else if let (Some(l), Some(r)) = (find(Role::SL), find(Role::SR)) {
                            add(l, s, MINUS_3DB);
                            add(r, s, MINUS_3DB);
                        } else if has_lr {
                            add(find(Role::FL).unwrap(), s, 0.5);
                            add(find(Role::FR).unwrap(), s, 0.5);
                        }
                    }
                    // FL/FR on a device without them (e.g. 1 ch handled above) and
                    // LFE on a device without LFE are dropped.
                    _ => {}
                }
            }
        }
        if dst < src {
            // A downmix sums several full-scale channels into one (5.1 -> stereo: FL + 0.71 FC
            // + 0.71 BL peaks at 2.4x). Without headroom that clips hard in the output stage
            // (crackle on loud passages), so normalise so that no output can exceed the
            // largest input.
            let max_row = (0..dst)
                .map(|d| {
                    gains[d * src..(d + 1) * src]
                        .iter()
                        .map(|g| g.abs())
                        .sum::<f32>()
                })
                .fold(0.0f32, f32::max);
            if max_row > 1.0 {
                let k = 1.0 / max_row;
                gains.iter_mut().for_each(|g| *g *= k);
            }
        }
        Self { src, dst, gains }
    }

    pub(crate) fn is_identity(&self) -> bool {
        self.gains.is_empty()
    }

    #[allow(dead_code)]
    pub(crate) fn src_channels(&self) -> usize {
        self.src
    }

    pub(crate) fn dst_channels(&self) -> usize {
        self.dst
    }

    /// Gain from source channel `s` to destination channel `d`.
    #[cfg(test)]
    pub(crate) fn gain(&self, d: usize, s: usize) -> f32 {
        if self.is_identity() {
            if d == s {
                1.0
            } else {
                0.0
            }
        } else {
            self.gains[d * self.src + s]
        }
    }

    /// Maps interleaved `input` (src channels) and appends interleaved dst-channel frames.
    pub(crate) fn apply(&self, input: &[f32], out: &mut Vec<f32>) {
        if self.is_identity() {
            out.extend_from_slice(input);
            return;
        }
        let frames = input.len() / self.src;
        out.reserve(frames * self.dst);
        for frame in input.chunks_exact(self.src) {
            for d in 0..self.dst {
                let row = &self.gains[d * self.src..(d + 1) * self.src];
                let mut acc = 0.0f32;
                for (g, x) in row.iter().zip(frame) {
                    acc += g * x;
                }
                out.push(acc);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_identity_is_verbatim() {
        let m = ChannelMatrix::new(2, Some(0b11), 2);
        assert!(m.is_identity());
        let mut out = Vec::new();
        m.apply(&[0.1, -0.2, 0.3, -0.4], &mut out);
        assert_eq!(out, vec![0.1, -0.2, 0.3, -0.4]);
    }

    #[test]
    fn mono_to_stereo_duplicates() {
        let m = ChannelMatrix::new(1, None, 2);
        let mut out = Vec::new();
        m.apply(&[0.5, -0.25], &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.25, -0.25]);
    }

    #[test]
    fn five_one_to_stereo_keeps_centre_and_drops_lfe() {
        // FL FR FC LFE BL BR
        let bits = CH_FRONT_LEFT
            | CH_FRONT_RIGHT
            | CH_FRONT_CENTRE
            | CH_LFE1
            | CH_REAR_LEFT
            | CH_REAR_RIGHT;
        let m = ChannelMatrix::new(6, Some(bits), 2);
        // BS.775 proportions, normalised so the downmix cannot clip.
        let k = m.gain(0, 0);
        assert!((k - 1.0 / (1.0 + 2.0 * MINUS_3DB)).abs() < 1e-6);
        assert_eq!(m.gain(1, 1), k);
        assert!((m.gain(0, 2) - MINUS_3DB * k).abs() < 1e-6);
        assert!((m.gain(1, 2) - MINUS_3DB * k).abs() < 1e-6);
        assert_eq!(m.gain(0, 3), 0.0);
        assert_eq!(m.gain(1, 3), 0.0);
        assert!((m.gain(0, 4) - MINUS_3DB * k).abs() < 1e-6);
        assert_eq!(m.gain(1, 4), 0.0);
        assert!((m.gain(1, 5) - MINUS_3DB * k).abs() < 1e-6);
    }

    #[test]
    fn full_scale_downmix_never_exceeds_full_scale() {
        let mut out = Vec::new();
        for (src, dst) in [(6, 2), (8, 2), (6, 1), (8, 6), (2, 1), (3, 2)] {
            let m = ChannelMatrix::new(src, None, dst);
            out.clear();
            m.apply(&vec![1.0; src], &mut out);
            m.apply(&vec![-1.0; src], &mut out);
            assert!(
                out.iter().all(|v| v.abs() <= 1.0 + 1e-6),
                "{src}->{dst}: {out:?}"
            );
        }
    }

    #[test]
    fn stereo_to_seven_one_goes_to_front_pair_only() {
        let m = ChannelMatrix::new(2, None, 8);
        let mut out = Vec::new();
        m.apply(&[0.3, -0.6], &mut out);
        assert_eq!(out, vec![0.3, -0.6, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let m = ChannelMatrix::new(2, None, 1);
        let mut out = Vec::new();
        m.apply(&[0.5, 0.25], &mut out);
        assert_eq!(out, vec![0.375]);
    }
}
