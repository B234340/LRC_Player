use std::f32::consts::PI;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use eframe::egui;
use lrc_player_application::infrastructure::decode::SymphoniaDecoder;
use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

const FFT_MIN_FREQ_HZ: usize = 50;
const FFT_MAX_FREQ_HZ: usize = 20_000;
const FFT_REFERENCE_FREQ_HZ: f32 = 1_000.0;
const FFT_LOG_BAND_COUNT: usize = 40;
const ANALYSIS_FPS: f32 = 15.0;
const DISPLAY_FPS: f32 = 60.0;
const FFT_WINDOW_SIZE: usize = 4096;
const MIN_DB: f32 = -60.0;
const MAX_DB: f32 = 0.0;
const PEAK_HOLD_MS: u64 = 200;
const PEAK_FALL_DB_PER_SEC: f32 = 45.0;

#[derive(Clone)]
struct FftOverlayData {
	track_path: String,
	bin_freqs_hz: Vec<f32>,
	min_freq_hz: f32,
	max_freq_hz: f32,
	frames_db: Vec<Vec<f32>>,
	analysis_frame_ms: f32,
}

impl FftOverlayData {
	fn interpolated_db_frame(&self, current_time_ms: u64) -> Option<Vec<f32>> {
		if self.frames_db.is_empty() {
			return None;
		}

		let last_index = self.frames_db.len().saturating_sub(1);
		let frame_pos = current_time_ms as f32 / self.analysis_frame_ms;
		let lower = (frame_pos.floor().max(0.0) as usize).min(last_index);
		let upper = (lower + 1).min(last_index);
		let t = (frame_pos - lower as f32).clamp(0.0, 1.0);

		if lower == upper {
			return Some(self.frames_db[lower].clone());
		}

		let mut interpolated = Vec::with_capacity(self.bin_freqs_hz.len());
		for i in 0..self.bin_freqs_hz.len() {
			let a = self.frames_db[lower][i];
			let b = self.frames_db[upper][i];
			interpolated.push(a + (b - a) * t);
		}
		Some(interpolated)
	}
}

	#[cfg(test)]
	mod tests {
		use super::FftOverlayData;

		#[test]
		fn interpolated_db_frame_clamps_after_last_frame() {
			let data = FftOverlayData {
				track_path: "/tmp/test.mp3".to_string(),
				bin_freqs_hz: vec![100.0, 200.0],
				min_freq_hz: 50.0,
				max_freq_hz: 20_000.0,
				frames_db: vec![vec![-40.0, -30.0], vec![-10.0, -5.0]],
				analysis_frame_ms: 100.0,
			};

			let frame = data
				.interpolated_db_frame(10_000)
				.expect("frame should be returned");

			assert_eq!(frame, vec![-10.0, -5.0]);
		}
	}

#[derive(Default)]
struct FftOverlayCache {
	current: Option<Arc<FftOverlayData>>,
	last_failed_path: Option<String>,
}

static FFT_OVERLAY_CACHE: OnceLock<Mutex<FftOverlayCache>> = OnceLock::new();

#[derive(Clone)]
struct PeakHoldState {
	track_path: String,
	held_db: Vec<f32>,
	hold_until_ms: Vec<u64>,
	last_time_ms: u64,
}

#[derive(Default)]
struct PeakHoldCache {
	state: Option<PeakHoldState>,
}

static PEAK_HOLD_CACHE: OnceLock<Mutex<PeakHoldCache>> = OnceLock::new();

fn fft_cache() -> &'static Mutex<FftOverlayCache> {
	FFT_OVERLAY_CACHE.get_or_init(|| Mutex::new(FftOverlayCache::default()))
}

fn peak_hold_cache() -> &'static Mutex<PeakHoldCache> {
	PEAK_HOLD_CACHE.get_or_init(|| Mutex::new(PeakHoldCache::default()))
}

fn update_peak_hold_db(track_path: &str, current_db: &[f32], current_time_ms: u64) -> Vec<f32> {
	let cache = peak_hold_cache();
	let mut cache = if let Ok(cache) = cache.lock() {
		cache
	} else {
		return current_db.to_vec();
	};

	let reset_required = match &cache.state {
		Some(state) => {
			state.track_path != track_path
				|| state.held_db.len() != current_db.len()
				|| current_time_ms < state.last_time_ms
		}
		None => true,
	};

	if reset_required {
		cache.state = Some(PeakHoldState {
			track_path: track_path.to_owned(),
			held_db: current_db.to_vec(),
			hold_until_ms: vec![current_time_ms.saturating_add(PEAK_HOLD_MS); current_db.len()],
			last_time_ms: current_time_ms,
		});
		return current_db.to_vec();
	}

	let state = if let Some(state) = cache.state.as_mut() {
		state
	} else {
		return current_db.to_vec();
	};
	let delta_ms = current_time_ms.saturating_sub(state.last_time_ms).min(1000);
	let fall_step_db = PEAK_FALL_DB_PER_SEC * (delta_ms as f32 / 1000.0);

	let mut out = Vec::with_capacity(current_db.len());
	for (idx, db) in current_db.iter().enumerate() {
		if *db >= state.held_db[idx] {
			state.held_db[idx] = *db;
			state.hold_until_ms[idx] = current_time_ms.saturating_add(PEAK_HOLD_MS);
		} else if current_time_ms >= state.hold_until_ms[idx] {
			state.held_db[idx] = (state.held_db[idx] - fall_step_db).max(*db);
		}
		out.push(state.held_db[idx]);
	}
	state.last_time_ms = current_time_ms;
	out
}

fn build_fft_overlay_data(track_path: &str) -> Option<FftOverlayData> {
	let decoded = SymphoniaDecoder::new().decode_file(Path::new(track_path)).ok()?;
	if decoded.samples.is_empty() || decoded.sample_rate == 0 {
		return None;
	}

	let channels = decoded.channels.max(1) as usize;
	let frame_count = decoded.samples.len() / channels;
	if frame_count == 0 {
		return None;
	}

	let mut mono = Vec::with_capacity(frame_count);
	for frame in 0..frame_count {
		let mut sum = 0.0_f32;
		for ch in 0..channels {
			sum += decoded.samples[frame * channels + ch];
		}
		mono.push(sum / channels as f32);
	}

	let nyquist = decoded.sample_rate as f32 * 0.5;
	let max_freq_hz = (FFT_MAX_FREQ_HZ as f32).min(nyquist);
	if max_freq_hz <= FFT_MIN_FREQ_HZ as f32 {
		return None;
	}

	let bin_freqs_hz = log_spaced_frequencies_from_reference(
		FFT_MIN_FREQ_HZ as f32,
		max_freq_hz,
		FFT_LOG_BAND_COUNT,
		FFT_REFERENCE_FREQ_HZ,
	);
	if bin_freqs_hz.is_empty() {
		return None;
	}

	let freq_resolution = decoded.sample_rate as f32 / FFT_WINDOW_SIZE as f32;
	let max_fft_idx = FFT_WINDOW_SIZE / 2;
	let mut band_ranges = Vec::with_capacity(bin_freqs_hz.len());
	for i in 0..bin_freqs_hz.len() {
		let center = bin_freqs_hz[i];
		let left_edge = if i == 0 {
			FFT_MIN_FREQ_HZ as f32
		} else {
			(bin_freqs_hz[i - 1] * center).sqrt()
		};
		let right_edge = if i + 1 < bin_freqs_hz.len() {
			(center * bin_freqs_hz[i + 1]).sqrt()
		} else {
			max_freq_hz
		};

		let start = ((left_edge / freq_resolution).floor() as usize).clamp(1, max_fft_idx);
		let mut end = ((right_edge / freq_resolution).ceil() as usize).clamp(1, max_fft_idx + 1);
		if end <= start {
			end = (start + 1).min(max_fft_idx + 1);
		}
		band_ranges.push((start, end));
	}

	let hop_size = ((decoded.sample_rate as f32 / ANALYSIS_FPS).round() as usize).max(1);
	let analysis_frame_ms = 1000.0 / ANALYSIS_FPS;
	let frame_total = mono.len().saturating_add(hop_size - 1) / hop_size;

	let window: Vec<f32> = (0..FFT_WINDOW_SIZE)
		.map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (FFT_WINDOW_SIZE.saturating_sub(1)) as f32).cos())
		.collect();

	let mut planner = FftPlanner::<f32>::new();
	let fft = planner.plan_fft_forward(FFT_WINDOW_SIZE);

	let mut linear_frames: Vec<Vec<f32>> = Vec::with_capacity(frame_total.max(1));
	let mut global_peak = 0.0_f32;

	for frame_idx in 0..frame_total.max(1) {
		let start = frame_idx * hop_size;
		let mut buffer = vec![Complex32::new(0.0, 0.0); FFT_WINDOW_SIZE];

		for i in 0..FFT_WINDOW_SIZE {
			let sample = mono.get(start + i).copied().unwrap_or(0.0);
			buffer[i] = Complex32::new(sample * window[i], 0.0);
		}

		fft.process(&mut buffer);

		let mut frame_bins = Vec::with_capacity(band_ranges.len());
		for (start, end) in &band_ranges {
			let mut sum = 0.0_f32;
			let mut count = 0usize;
			for fft_idx in *start..*end {
				sum += buffer[fft_idx].norm();
				count += 1;
			}
			let mag = if count > 0 { sum / count as f32 } else { 0.0 };
			if mag > global_peak {
				global_peak = mag;
			}
			frame_bins.push(mag);
		}
		linear_frames.push(frame_bins);
	}

	let norm = global_peak.max(1e-9);
	let mut frames_db = Vec::with_capacity(linear_frames.len());
	for frame in linear_frames {
		let mut db_frame = Vec::with_capacity(frame.len());
		for magnitude in frame {
			let db = 20.0 * (magnitude / norm).max(1e-6).log10();
			db_frame.push(db.clamp(MIN_DB, MAX_DB));
		}
		frames_db.push(db_frame);
	}

	Some(FftOverlayData {
		track_path: track_path.to_owned(),
		bin_freqs_hz,
		min_freq_hz: FFT_MIN_FREQ_HZ as f32,
		max_freq_hz,
		frames_db,
		analysis_frame_ms,
	})
}

fn get_fft_overlay_data(track_path: &str) -> Option<Arc<FftOverlayData>> {
	let cache = fft_cache();
	let mut cache = cache.lock().ok()?;

	if let Some(current) = &cache.current {
		if current.track_path == track_path {
			return Some(current.clone());
		}
	}

	if cache.last_failed_path.as_deref() == Some(track_path) {
		return None;
	}

	match build_fft_overlay_data(track_path) {
		Some(data) => {
			let data = Arc::new(data);
			cache.current = Some(data.clone());
			cache.last_failed_path = None;
			Some(data)
		}
		None => {
			cache.last_failed_path = Some(track_path.to_owned());
			None
		}
	}
}

fn map_db_to_y(plot: egui::Rect, db: f32) -> f32 {
	let t = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
	plot.max.y - plot.height() * t
}

fn log_spaced_frequencies_from_reference(
	min_hz: f32,
	max_hz: f32,
	count: usize,
	reference_hz: f32,
) -> Vec<f32> {
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

fn map_freq_to_x(plot: egui::Rect, freq_hz: f32, max_freq_hz: f32) -> f32 {
	let min_f = FFT_MIN_FREQ_HZ as f32;
	let max_f = max_freq_hz.max(min_f + 1.0);
	let f = freq_hz.clamp(min_f, max_f);

	let log_min = (min_f / FFT_REFERENCE_FREQ_HZ).log2();
	let log_max = (max_f / FFT_REFERENCE_FREQ_HZ).log2();
	let t = (((f / FFT_REFERENCE_FREQ_HZ).log2() - log_min) / (log_max - log_min)).clamp(0.0, 1.0);
	plot.min.x + plot.width() * t
}

/// アートワーク領域へ FFT スペクトラムを重ね描画します。
///
/// - 縦軸: dB
/// - 横軸: 周波数（50Hz刻み、20kHzまで）
/// - 解析サンプリング: 30fps
/// - 実表示: 60fps 相当に線形補間
pub fn render_fft_overlay(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	play_path: &str,
	current_time_ms: u64,
) {
	if play_path.trim().is_empty() {
		return;
	}

	let Some(data) = get_fft_overlay_data(play_path) else {
		return;
	};
	let Some(interpolated_db) = data.interpolated_db_frame(current_time_ms) else {
		return;
	};
    let peak_hold_db = update_peak_hold_db(play_path, &interpolated_db, current_time_ms);

	let painter = ui.painter().with_clip_rect(rect);
	painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(100));

	let left_pad = 44.0;
	let right_pad = 12.0;
	let top_pad = 12.0;
	let bottom_pad = 24.0;
	let plot = egui::Rect::from_min_max(
		egui::pos2(rect.min.x + left_pad, rect.min.y + top_pad),
		egui::pos2(rect.max.x - right_pad, rect.max.y - bottom_pad),
	);
	if plot.width() <= 0.0 || plot.height() <= 0.0 {
		return;
	}

	let axis_color = egui::Color32::from_white_alpha(150);
	let grid_color = egui::Color32::from_white_alpha(32);
	let text_color = egui::Color32::from_white_alpha(170);
	let spectrum_color = egui::Color32::from_rgb(80, 220, 255);
	let peak_color = egui::Color32::from_rgba_unmultiplied(255, 70, 70, 180);
	let display_max_hz = data.max_freq_hz.min(FFT_MAX_FREQ_HZ as f32);

	painter.rect_stroke(plot, 0.0, egui::Stroke::new(1.0, axis_color));

	for db in (-60..=0).step_by(10) {
		let y = map_db_to_y(plot, db as f32);
		painter.line_segment(
			[egui::pos2(plot.min.x, y), egui::pos2(plot.max.x, y)],
			egui::Stroke::new(1.0, grid_color),
		);
		painter.text(
			egui::pos2(rect.min.x + 4.0, y),
			egui::Align2::LEFT_CENTER,
			format!("{} dB", db),
			egui::FontId::proportional(10.0),
			text_color,
		);
	}

	let freq_ticks_hz = [50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 20_000];
	for freq_hz in freq_ticks_hz {
		if (freq_hz as f32) > display_max_hz {
			continue;
		}
		let x = map_freq_to_x(plot, freq_hz as f32, display_max_hz);
		let stroke = if freq_hz == 1_000 {
			egui::Stroke::new(1.5, egui::Color32::from_white_alpha(90))
		} else {
			egui::Stroke::new(1.0, grid_color)
		};
		painter.line_segment(
			[egui::pos2(x, plot.min.y), egui::pos2(x, plot.max.y)],
			stroke,
		);
		let label = if freq_hz >= 1_000 {
			format!("{}k", freq_hz / 1_000)
		} else {
			freq_hz.to_string()
		};
		painter.text(
			egui::pos2(x, plot.max.y + 4.0),
			egui::Align2::CENTER_TOP,
			label,
			egui::FontId::proportional(10.0),
			text_color,
		);
	}

	let band_boundaries_hz: Vec<(f32, f32)> = data
		.bin_freqs_hz
		.iter()
		.enumerate()
		.map(|(idx, &center)| {
			let left = if idx == 0 {
				data.min_freq_hz
			} else {
				(data.bin_freqs_hz[idx - 1] * center).sqrt()
			};
			let right = if idx + 1 < data.bin_freqs_hz.len() {
				(center * data.bin_freqs_hz[idx + 1]).sqrt()
			} else {
				data.max_freq_hz
			};
			(left, right)
		})
		.collect();

	for (idx, db) in interpolated_db.iter().enumerate() {
		let (left_hz, right_hz) = band_boundaries_hz[idx];
		let left = map_freq_to_x(plot, left_hz, display_max_hz);
		let right = map_freq_to_x(plot, right_hz, display_max_hz);
		let y = map_db_to_y(plot, *db);
		let peak_y = map_db_to_y(plot, peak_hold_db[idx]);

		let width = (right - left).abs().max(1.0);
		let inset = (width * 0.05).min(0.75);
		let left = (left + inset).clamp(plot.min.x, plot.max.x);
		let right = (right - inset).clamp(plot.min.x, plot.max.x);

		if right <= left {
			continue;
		}

		let bar_rect = egui::Rect::from_min_max(egui::pos2(left, y), egui::pos2(right, plot.max.y));
		painter.rect_filled(bar_rect, 0.0, spectrum_color.gamma_multiply(0.85));

		// Dynamic range visualization: draw red only above the blue bar and use the same horizontal width.
		if peak_y < y {
			let peak_rect = egui::Rect::from_min_max(egui::pos2(left, peak_y), egui::pos2(right, y));
			painter.rect_filled(peak_rect, 0.0, peak_color);
		}
	}

	// 解析は30fpsだが表示は60fpsで補間するため、16ms周期で再描画を要求する。
	ui.ctx()
		.request_repaint_after(Duration::from_secs_f32(1.0 / DISPLAY_FPS));
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_overlay_data() -> FftOverlayData {
		FftOverlayData {
			track_path: "test_track".to_owned(),
			bin_freqs_hz: vec![100.0, 1000.0],
			min_freq_hz: 50.0,
			max_freq_hz: 20_000.0,
			frames_db: vec![vec![-12.0, -24.0], vec![-6.0, -18.0]],
			analysis_frame_ms: 100.0,
		}
	}

	#[test]
	fn interpolated_db_frame_clamps_after_last_frame() {
		let data = test_overlay_data();

		let out = data
			.interpolated_db_frame(100_000)
			.expect("frame should be returned");

		assert_eq!(out, vec![-6.0, -18.0]);
	}

	#[test]
	fn interpolated_db_frame_clamps_out_of_range_time() {
		let data = test_overlay_data();

		let out = data
			.interpolated_db_frame(u64::MAX)
			.expect("frame should be returned");

		assert_eq!(out, vec![-6.0, -18.0]);
	}
}
