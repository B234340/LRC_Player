//! Live spectrum overlay.
//!
//! Fed by the engine's lock-free spectrum tap (post-gain mono samples that were actually
//! played), so it is in sync with what is heard and needs no file decoding. One 4096-point
//! FFT per repaint at most, with a planner, window and buffers reused across frames; nothing
//! is computed when no new audio arrived (paused) or when the overlay is hidden (the caller
//! only calls [`FftAnalyzer::update`] while it is drawn).

use std::f32::consts::PI;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use lrc_player_application::infrastructure::audio::engine::SpectrumTap;
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

const FFT_MIN_FREQ_HZ: f32 = 50.0;
const FFT_MAX_FREQ_HZ: f32 = 20_000.0;
const FFT_REFERENCE_FREQ_HZ: f32 = 1_000.0;
const FFT_LOG_BAND_COUNT: usize = 40;
const FFT_WINDOW_SIZE: usize = 4096;
const MIN_DB: f32 = -60.0;
const MAX_DB: f32 = 0.0;
const PEAK_HOLD_SECS: f32 = 0.2;
const PEAK_FALL_DB_PER_SEC: f32 = 45.0;
/// Bars fall at most this fast (dB/s) so the display does not flicker at 30 fps.
const BAR_RELEASE_DB_PER_SEC: f32 = 90.0;

const DB_LABELS: [(i32, &str); 7] = [
    (-60, "-60 dB"),
    (-50, "-50 dB"),
    (-40, "-40 dB"),
    (-30, "-30 dB"),
    (-20, "-20 dB"),
    (-10, "-10 dB"),
    (0, "0 dB"),
];
const FREQ_TICKS: [(f32, &str); 9] = [
    (50.0, "50"),
    (100.0, "100"),
    (200.0, "200"),
    (500.0, "500"),
    (1_000.0, "1k"),
    (2_000.0, "2k"),
    (5_000.0, "5k"),
    (10_000.0, "10k"),
    (20_000.0, "20k"),
];

/// Reusable FFT state + latest band levels.
pub struct FftAnalyzer {
    planner: FftPlanner<f32>,
    fft: Option<Arc<dyn Fft<f32>>>,
    window: Vec<f32>,
    samples: Vec<f32>,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
    sample_rate: u32,
    max_freq_hz: f32,
    band_ranges: Vec<(usize, usize)>,
    band_edges_hz: Vec<(f32, f32)>,
    bands_db: Vec<f32>,
    peaks_db: Vec<f32>,
    peak_hold_left: Vec<f32>,
    last_written: u64,
    last_update: Option<Instant>,
    has_data: bool,
}

impl Default for FftAnalyzer {
    fn default() -> Self {
        let window = (0..FFT_WINDOW_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (FFT_WINDOW_SIZE - 1) as f32).cos())
            .collect();
        Self {
            planner: FftPlanner::new(),
            fft: None,
            window,
            samples: vec![0.0; FFT_WINDOW_SIZE],
            buffer: vec![Complex32::new(0.0, 0.0); FFT_WINDOW_SIZE],
            scratch: Vec::new(),
            sample_rate: 0,
            max_freq_hz: FFT_MAX_FREQ_HZ,
            band_ranges: Vec::new(),
            band_edges_hz: Vec::new(),
            bands_db: vec![MIN_DB; FFT_LOG_BAND_COUNT],
            peaks_db: vec![MIN_DB; FFT_LOG_BAND_COUNT],
            peak_hold_left: vec![0.0; FFT_LOG_BAND_COUNT],
            last_written: 0,
            last_update: None,
            has_data: false,
        }
    }
}

pub(crate) fn log_spaced_frequencies(min_hz: f32, max_hz: f32, count: usize, reference_hz: f32) -> Vec<f32> {
    if count == 0 || min_hz <= 0.0 || max_hz <= min_hz || reference_hz <= 0.0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![min_hz];
    }
    let log_min = (min_hz / reference_hz).log2();
    let log_max = (max_hz / reference_hz).log2();
    (0..count)
        .map(|i| {
            let t = i as f32 / (count - 1) as f32;
            reference_hz * 2.0_f32.powf(log_min + (log_max - log_min) * t)
        })
        .collect()
}

impl FftAnalyzer {
    fn configure(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        if self.fft.is_none() {
            let fft = self.planner.plan_fft_forward(FFT_WINDOW_SIZE);
            self.scratch = vec![Complex32::new(0.0, 0.0); fft.get_inplace_scratch_len()];
            self.fft = Some(fft);
        }
        let nyquist = sample_rate as f32 * 0.5;
        self.max_freq_hz = FFT_MAX_FREQ_HZ.min(nyquist);
        let centers = log_spaced_frequencies(
            FFT_MIN_FREQ_HZ,
            self.max_freq_hz,
            FFT_LOG_BAND_COUNT,
            FFT_REFERENCE_FREQ_HZ,
        );
        let resolution = sample_rate as f32 / FFT_WINDOW_SIZE as f32;
        let max_idx = FFT_WINDOW_SIZE / 2;
        self.band_ranges.clear();
        self.band_edges_hz.clear();
        for i in 0..centers.len() {
            let center = centers[i];
            let left = if i == 0 {
                FFT_MIN_FREQ_HZ
            } else {
                (centers[i - 1] * center).sqrt()
            };
            let right = if i + 1 < centers.len() {
                (center * centers[i + 1]).sqrt()
            } else {
                self.max_freq_hz
            };
            let start = ((left / resolution).floor() as usize).clamp(1, max_idx);
            let mut end = ((right / resolution).ceil() as usize).clamp(1, max_idx + 1);
            if end <= start {
                end = (start + 1).min(max_idx + 1);
            }
            self.band_ranges.push((start, end));
            self.band_edges_hz.push((left, right));
        }
        self.bands_db = vec![MIN_DB; self.band_ranges.len()];
        self.peaks_db = vec![MIN_DB; self.band_ranges.len()];
        self.peak_hold_left = vec![0.0; self.band_ranges.len()];
    }

    /// Analyzes the newest audio from `tap`. Does nothing (and returns false) when no new
    /// samples were written since the last call, e.g. while paused.
    pub fn update(&mut self, tap: &SpectrumTap) -> bool {
        let written = tap.total_written();
        if written == self.last_written {
            return false;
        }
        self.last_written = written;
        let rate = tap.sample_rate();
        if rate == 0 {
            return false;
        }
        if rate != self.sample_rate || self.fft.is_none() {
            self.configure(rate);
        }
        let Some(fft) = self.fft.clone() else {
            return false;
        };

        tap.read_latest(&mut self.samples);
        for ((dst, s), w) in self.buffer.iter_mut().zip(&self.samples).zip(&self.window) {
            *dst = Complex32::new(s * w, 0.0);
        }
        fft.process_with_scratch(&mut self.buffer, &mut self.scratch);

        let now = Instant::now();
        let dt = self
            .last_update
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(0.0)
            .min(0.5);
        self.last_update = Some(now);

        // A full-scale sine through a Hann window peaks at N/4.
        let reference = FFT_WINDOW_SIZE as f32 / 4.0;
        for (i, &(start, end)) in self.band_ranges.iter().enumerate() {
            let bins = &self.buffer[start..end.min(self.buffer.len())];
            let mag = if bins.is_empty() {
                0.0
            } else {
                bins.iter().map(|c| c.norm()).fold(0.0_f32, f32::max)
            };
            let db = (20.0 * (mag / reference).max(1e-6).log10()).clamp(MIN_DB, MAX_DB);
            let shown = &mut self.bands_db[i];
            *shown = if db >= *shown {
                db
            } else {
                (*shown - BAR_RELEASE_DB_PER_SEC * dt).max(db)
            };

            let peak = &mut self.peaks_db[i];
            let hold = &mut self.peak_hold_left[i];
            if *shown >= *peak {
                *peak = *shown;
                *hold = PEAK_HOLD_SECS;
            } else if *hold > 0.0 {
                *hold -= dt;
            } else {
                *peak = (*peak - PEAK_FALL_DB_PER_SEC * dt).max(*shown);
            }
        }
        self.has_data = true;
        true
    }

    pub fn has_data(&self) -> bool {
        self.has_data
    }

    /// Current band levels in dB (after release smoothing).
    pub fn bands_db(&self) -> &[f32] {
        &self.bands_db
    }

    /// Forget levels (e.g. on track change while stopped).
    pub fn reset(&mut self) {
        self.bands_db.iter_mut().for_each(|v| *v = MIN_DB);
        self.peaks_db.iter_mut().for_each(|v| *v = MIN_DB);
        self.has_data = false;
    }
}

fn map_db_to_y(plot: egui::Rect, db: f32) -> f32 {
    let t = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
    plot.max.y - plot.height() * t
}

fn map_freq_to_x(plot: egui::Rect, freq_hz: f32, max_freq_hz: f32) -> f32 {
    let min_f = FFT_MIN_FREQ_HZ;
    let max_f = max_freq_hz.max(min_f + 1.0);
    let f = freq_hz.clamp(min_f, max_f);
    let log_min = (min_f / FFT_REFERENCE_FREQ_HZ).log2();
    let log_max = (max_f / FFT_REFERENCE_FREQ_HZ).log2();
    let t = (((f / FFT_REFERENCE_FREQ_HZ).log2() - log_min) / (log_max - log_min)).clamp(0.0, 1.0);
    plot.min.x + plot.width() * t
}

/// Draws the latest spectrum of `analyzer` over `rect` (no analysis happens here).
pub fn render_fft_overlay(ui: &mut egui::Ui, rect: egui::Rect, analyzer: &FftAnalyzer) {
    if !analyzer.has_data() {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(100));

    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 44.0, rect.min.y + 12.0),
        egui::pos2(rect.max.x - 12.0, rect.max.y - 24.0),
    );
    if plot.width() <= 0.0 || plot.height() <= 0.0 {
        return;
    }

    let axis_color = egui::Color32::from_white_alpha(150);
    let grid_color = egui::Color32::from_white_alpha(32);
    let text_color = egui::Color32::from_white_alpha(170);
    let spectrum_color = egui::Color32::from_rgb(80, 220, 255);
    let peak_color = egui::Color32::from_rgba_unmultiplied(255, 70, 70, 180);
    let max_hz = analyzer.max_freq_hz;
    let label_font = egui::FontId::proportional(10.0);

    painter.rect_stroke(plot, 0.0, egui::Stroke::new(1.0, axis_color), egui::StrokeKind::Middle);
    for (db, label) in DB_LABELS {
        let y = map_db_to_y(plot, db as f32);
        painter.line_segment(
            [egui::pos2(plot.min.x, y), egui::pos2(plot.max.x, y)],
            egui::Stroke::new(1.0, grid_color),
        );
        painter.text(
            egui::pos2(rect.min.x + 4.0, y),
            egui::Align2::LEFT_CENTER,
            label,
            label_font.clone(),
            text_color,
        );
    }
    for (freq, label) in FREQ_TICKS {
        if freq > max_hz {
            continue;
        }
        let x = map_freq_to_x(plot, freq, max_hz);
        let stroke = if freq == 1_000.0 {
            egui::Stroke::new(1.5, egui::Color32::from_white_alpha(90))
        } else {
            egui::Stroke::new(1.0, grid_color)
        };
        painter.line_segment([egui::pos2(x, plot.min.y), egui::pos2(x, plot.max.y)], stroke);
        painter.text(
            egui::pos2(x, plot.max.y + 4.0),
            egui::Align2::CENTER_TOP,
            label,
            label_font.clone(),
            text_color,
        );
    }

    for (i, (db, peak_db)) in analyzer.bands_db.iter().zip(&analyzer.peaks_db).enumerate() {
        let Some(&(left_hz, right_hz)) = analyzer.band_edges_hz.get(i) else {
            break;
        };
        let left = map_freq_to_x(plot, left_hz, max_hz);
        let right = map_freq_to_x(plot, right_hz, max_hz);
        let width = (right - left).abs().max(1.0);
        let inset = (width * 0.05).min(0.75);
        let left = (left + inset).clamp(plot.min.x, plot.max.x);
        let right = (right - inset).clamp(plot.min.x, plot.max.x);
        if right <= left {
            continue;
        }
        let y = map_db_to_y(plot, *db);
        let peak_y = map_db_to_y(plot, *peak_db);
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(left, y), egui::pos2(right, plot.max.y)),
            0.0,
            spectrum_color.gamma_multiply(0.85),
        );
        if peak_y < y {
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(left, peak_y), egui::pos2(right, y)),
                0.0,
                peak_color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_bands_cover_range() {
        let f = log_spaced_frequencies(50.0, 20_000.0, 40, 1_000.0);
        assert_eq!(f.len(), 40);
        assert!((f[0] - 50.0).abs() < 1e-3);
        assert!((f[39] - 20_000.0).abs() < 1.0);
        assert!(f.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn configure_builds_monotonic_bands() {
        let mut a = FftAnalyzer::default();
        a.configure(48_000);
        assert_eq!(a.band_ranges.len(), FFT_LOG_BAND_COUNT);
        assert!(a.band_ranges.iter().all(|(s, e)| e > s));
        assert!(a.fft.is_some());
    }
}
