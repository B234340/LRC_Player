//! Automatic PEQ creation ("自動作成"): imported target / measured frequency responses, the
//! AutoEq-style fit on a worker thread, applying the result to the affected channel(s) and
//! undoing it.
//!
//! | method | effect |
//! |---|---|
//! | [`GuiApp::pick_auto_eq_curve`] | native file dialog + read + parse on a worker |
//! | [`GuiApp::set_auto_eq_curve`] / [`GuiApp::clear_auto_eq_curve`] | a curve slot (path saved) |
//! | [`GuiApp::set_auto_eq_target_flat`], [`GuiApp::set_auto_eq_same_source`], [`GuiApp::set_auto_eq_options`] | saved in `config.auto_eq` |
//! | [`GuiApp::start_auto_eq`] / [`GuiApp::poll_auto_eq`] | fit on a worker, then replace the filters + preamp |
//! | [`GuiApp::undo_auto_eq`] | restore the PEQ from before the last auto-create |
//!
//! Which source is fitted: linked mode fits one curve for both channels (the shared
//! measurement, or the average of L and R when two files are used); separate mode fits L and
//! R separately (or the shared measurement once for both).

use std::path::{Path, PathBuf};

use eframe::egui;
use lrc_player_application::infrastructure::audio::engine::{
    auto_eq, parse_frequency_response, AutoEqOptions, AutoEqResult, CurveParseError, FrequencyResponse, ParametricEq,
    PeqChannel, PeqChannelMode, PeqSide, CURVE_MAX_BYTES,
};

use crate::presentation::app::GuiApp;
use crate::presentation::ui::i18n::Tr;
pub(crate) use crate::presentation::ui::views::eq_auto::CurveSlot;
use crate::presentation::ui::views::eq_auto::FitSummary;

/// An imported curve.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LoadedCurve {
    pub(crate) path: PathBuf,
    /// File name shown in the panel.
    pub(crate) name: String,
    pub(crate) curve: FrequencyResponse,
}

impl LoadedCurve {
    pub(crate) fn new(path: PathBuf, curve: FrequencyResponse) -> Self {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned());
        Self { path, name, curve }
    }
}

/// Why a curve file could not be loaded.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CurveLoadError {
    /// I/O error (OS message).
    Read(String),
    Parse(CurveParseError),
}

impl CurveLoadError {
    /// Localized reason.
    pub(crate) fn text(&self, tr: Tr) -> String {
        match self {
            CurveLoadError::Read(e) => e.clone(),
            CurveLoadError::Parse(CurveParseError::TooLarge) => tr.t("autoeq.err_too_large").to_owned(),
            CurveLoadError::Parse(CurveParseError::NoData) => tr.t("autoeq.err_no_data").to_owned(),
            CurveLoadError::Parse(CurveParseError::TooFewPoints(_)) => tr.t("autoeq.err_too_few").to_owned(),
        }
    }
}

/// Result of the curve picker: `None` = cancelled.
pub(crate) type CurvePick = Option<(PathBuf, Result<FrequencyResponse, CurveLoadError>)>;

/// What to fit: the target and, per fit, the sides it applies to and the measurement.
pub(crate) type FitPlan = (FrequencyResponse, Vec<(Vec<PeqSide>, FrequencyResponse)>);

/// One fit of the worker: the sides it applies to and its result.
pub(crate) type FitOutput = Vec<(Vec<PeqSide>, AutoEqResult)>;

/// Reads and parses a curve file (size-capped).
pub(crate) fn load_curve_file(path: &Path) -> Result<FrequencyResponse, CurveLoadError> {
    let meta = std::fs::metadata(path).map_err(|e| CurveLoadError::Read(e.to_string()))?;
    if meta.len() > CURVE_MAX_BYTES as u64 {
        return Err(CurveLoadError::Parse(CurveParseError::TooLarge));
    }
    let bytes = std::fs::read(path).map_err(|e| CurveLoadError::Read(e.to_string()))?;
    parse_frequency_response(&bytes).map_err(CurveLoadError::Parse)
}

/// Auto-EQ state of the app (options and flags live in `config.auto_eq`).
#[derive(Default)]
pub(crate) struct AutoEqState {
    pub(crate) target: Option<LoadedCurve>,
    /// Shared measurement, or the left one when `same_source` is off.
    pub(crate) source: Option<LoadedCurve>,
    pub(crate) source_r: Option<LoadedCurve>,
    /// Running file dialog (slot, result channel).
    pub(crate) picker: Option<(CurveSlot, crossbeam_channel::Receiver<CurvePick>)>,
    /// Running fit.
    pub(crate) job: Option<crossbeam_channel::Receiver<FitOutput>>,
    /// RMS errors of the last auto-create (shown in the panel).
    pub(crate) last: Vec<FitSummary>,
    /// PEQ before the last auto-create.
    pub(crate) undo: Option<ParametricEq>,
    /// The section is expanded.
    pub(crate) expanded: bool,
    /// Bumped whenever a curve or a flag that changes the graph overlay changes.
    pub(crate) rev: u64,
}

impl AutoEqState {
    pub(crate) fn slot(&self, slot: CurveSlot) -> Option<&LoadedCurve> {
        match slot {
            CurveSlot::Target => self.target.as_ref(),
            CurveSlot::Source => self.source.as_ref(),
            CurveSlot::SourceRight => self.source_r.as_ref(),
        }
    }

    fn slot_mut(&mut self, slot: CurveSlot) -> &mut Option<LoadedCurve> {
        match slot {
            CurveSlot::Target => &mut self.target,
            CurveSlot::Source => &mut self.source,
            CurveSlot::SourceRight => &mut self.source_r,
        }
    }
}

/// What is missing before a fit can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FitMissing {
    Target,
    Source,
    SourceRight,
}

impl GuiApp {
    // ---- options ----

    /// Fitting options from the config (sanitized).
    pub(crate) fn auto_eq_options(&self) -> AutoEqOptions {
        let c = &self.config.auto_eq;
        AutoEqOptions {
            max_filters: c.max_filters,
            max_gain_db: c.max_gain_db,
            min_q: c.min_q,
            max_q: c.max_q,
            freq_min_hz: c.freq_min_hz,
            freq_max_hz: c.freq_max_hz,
            treble_boost_limit_db: c.treble_boost_limit_db,
            use_shelves: c.use_shelves,
            ..AutoEqOptions::default()
        }
        .sanitized()
    }

    pub fn set_auto_eq_options(&mut self, o: AutoEqOptions) {
        let o = o.sanitized();
        if o == self.auto_eq_options() {
            return;
        }
        let c = &mut self.config.auto_eq;
        c.max_filters = o.max_filters;
        c.max_gain_db = o.max_gain_db;
        c.min_q = o.min_q;
        c.max_q = o.max_q;
        c.freq_min_hz = o.freq_min_hz;
        c.freq_max_hz = o.freq_max_hz;
        c.treble_boost_limit_db = o.treble_boost_limit_db;
        c.use_shelves = o.use_shelves;
        self.save_config_soon();
    }

    pub fn set_auto_eq_target_flat(&mut self, flat: bool) {
        if self.config.auto_eq.target_flat == flat {
            return;
        }
        self.config.auto_eq.target_flat = flat;
        self.auto_eq.rev += 1;
        self.save_config_soon();
    }

    pub fn set_auto_eq_same_source(&mut self, same: bool) {
        if self.config.auto_eq.same_source == same {
            return;
        }
        self.config.auto_eq.same_source = same;
        self.auto_eq.rev += 1;
        self.save_config_soon();
    }

    // ---- curves ----

    /// Puts a curve into `slot` (and its path into the config). A target file turns the flat
    /// target off.
    pub(crate) fn set_auto_eq_curve(&mut self, slot: CurveSlot, curve: LoadedCurve) {
        let path = Some(curve.path.to_string_lossy().into_owned());
        let c = &mut self.config.auto_eq;
        match slot {
            CurveSlot::Target => {
                c.target_path = path;
                c.target_flat = false;
            }
            CurveSlot::Source => c.source_path = path,
            CurveSlot::SourceRight => c.source_r_path = path,
        }
        *self.auto_eq.slot_mut(slot) = Some(curve);
        self.auto_eq.rev += 1;
        self.save_config_soon();
    }

    pub fn clear_auto_eq_curve(&mut self, slot: CurveSlot) {
        let c = &mut self.config.auto_eq;
        match slot {
            CurveSlot::Target => c.target_path = None,
            CurveSlot::Source => c.source_path = None,
            CurveSlot::SourceRight => c.source_r_path = None,
        }
        *self.auto_eq.slot_mut(slot) = None;
        self.auto_eq.rev += 1;
        self.save_config_soon();
    }

    /// Loads the curve files named in the config (start-up). Missing or unreadable files are
    /// skipped quietly (their paths stay in the config).
    pub(crate) fn reload_auto_eq_curves(&mut self) {
        let c = &self.config.auto_eq;
        for (slot, path) in [
            (CurveSlot::Target, c.target_path.clone()),
            (CurveSlot::Source, c.source_path.clone()),
            (CurveSlot::SourceRight, c.source_r_path.clone()),
        ] {
            let Some(path) = path.map(PathBuf::from) else { continue };
            if !path.is_file() {
                continue;
            }
            match load_curve_file(&path) {
                Ok(curve) => *self.auto_eq.slot_mut(slot) = Some(LoadedCurve::new(path, curve)),
                Err(e) => tracing::info!("auto EQ curve {} not loaded: {e:?}", path.display()),
            }
        }
        self.auto_eq.rev += 1;
    }

    /// Opens the native file dialog for `slot` on a worker thread; the file is read and parsed
    /// there too.
    pub(crate) fn pick_auto_eq_curve(&mut self, ctx: &egui::Context, slot: CurveSlot) {
        if self.auto_eq.picker.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded::<CurvePick>(1);
        let wake = ctx.clone();
        let tr = self.tr();
        let title = tr.t("autoeq.file_title");
        let filter_name = tr.t("autoeq.file_filter");
        let start_dir = self.auto_eq.slot(slot).or(self.auto_eq.source.as_ref()).or(self.auto_eq.target.as_ref()).and_then(|c| c.path.parent().map(Path::to_path_buf));
        let spawned = std::thread::Builder::new().name("autoeq-curve-picker".into()).spawn(move || {
            let picked = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()
                .and_then(|rt| {
                    let mut dialog = rfd::AsyncFileDialog::new()
                        .set_title(title)
                        .add_filter(filter_name, &["txt", "csv", "frd", "dat"])
                        .add_filter("*", &["*"]);
                    if let Some(dir) = start_dir {
                        dialog = dialog.set_directory(dir);
                    }
                    rt.block_on(dialog.pick_file())
                })
                .map(|h| {
                    let path = h.path().to_path_buf();
                    let curve = load_curve_file(&path);
                    (path, curve)
                });
            let _ = tx.send(picked);
            wake.request_repaint();
        });
        if spawned.is_ok() {
            self.auto_eq.picker = Some((slot, rx));
        }
    }

    pub(crate) fn poll_auto_eq_curve_picker(&mut self) {
        let Some((slot, rx)) = &self.auto_eq.picker else { return };
        let slot = *slot;
        let picked = match rx.try_recv() {
            Ok(p) => p,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => None,
        };
        self.auto_eq.picker = None;
        self.apply_curve_pick(slot, picked);
    }

    /// The picker's result: the curve goes into `slot`, errors become a toast.
    pub(crate) fn apply_curve_pick(&mut self, slot: CurveSlot, picked: CurvePick) {
        let Some((path, result)) = picked else { return };
        match result {
            Ok(curve) => self.set_auto_eq_curve(slot, LoadedCurve::new(path, curve)),
            Err(err) => {
                let tr = self.tr();
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.toast_error(tr.f("toast.autoeq_read_failed", &[&name, &err.text(tr)]));
            }
        }
    }

    // ---- what to fit ----

    /// The target curve (flat or the file), if chosen.
    pub(crate) fn auto_eq_target(&self) -> Option<FrequencyResponse> {
        if self.config.auto_eq.target_flat {
            Some(FrequencyResponse::flat())
        } else {
            self.auto_eq.target.as_ref().map(|c| c.curve.clone())
        }
    }

    /// The measured curve used for `side` (see the module docs). In linked mode `side` is
    /// ignored.
    pub(crate) fn auto_eq_source(&self, side: PeqSide) -> Option<FrequencyResponse> {
        let a = &self.auto_eq;
        let l = a.source.as_ref().map(|c| &c.curve);
        if self.config.auto_eq.same_source {
            return l.cloned();
        }
        let r = a.source_r.as_ref().map(|c| &c.curve);
        match self.peq.channel_mode {
            PeqChannelMode::Linked => match (l, r) {
                (Some(l), Some(r)) => Some(FrequencyResponse::average(l, r)),
                (Some(c), None) | (None, Some(c)) => Some(c.clone()),
                (None, None) => None,
            },
            PeqChannelMode::Separate => match side {
                PeqSide::Left => l.cloned(),
                PeqSide::Right => r.cloned(),
            },
        }
    }

    /// The fits to run: (sides, source). `Err` names what is missing.
    pub(crate) fn auto_eq_plan(&self) -> Result<FitPlan, FitMissing> {
        let target = self.auto_eq_target().ok_or(FitMissing::Target)?;
        let separate = self.peq.channel_mode == PeqChannelMode::Separate;
        let plan = if !separate || self.config.auto_eq.same_source {
            let sides = if separate { vec![PeqSide::Left, PeqSide::Right] } else { vec![PeqSide::Left] };
            vec![(sides, self.auto_eq_source(PeqSide::Left).ok_or(FitMissing::Source)?)]
        } else {
            vec![
                (vec![PeqSide::Left], self.auto_eq_source(PeqSide::Left).ok_or(FitMissing::Source)?),
                (vec![PeqSide::Right], self.auto_eq_source(PeqSide::Right).ok_or(FitMissing::SourceRight)?),
            ]
        };
        Ok((target, plan))
    }

    // ---- the fit ----

    /// Starts the fit on a worker thread (no-op while one runs). Missing curves become a
    /// toast. `ctx` (if any) is woken when it is done.
    pub fn start_auto_eq(&mut self, ctx: Option<&egui::Context>) {
        if self.auto_eq.job.is_some() {
            return;
        }
        let tr = self.tr();
        let (target, plan) = match self.auto_eq_plan() {
            Ok(p) => p,
            Err(missing) => {
                let key = match missing {
                    FitMissing::Target => "toast.autoeq_missing_target",
                    FitMissing::Source => "toast.autoeq_missing_source",
                    FitMissing::SourceRight => "toast.autoeq_missing_source_r",
                };
                self.toast_error(tr.t(key).to_owned());
                return;
            }
        };
        let opts = self.auto_eq_options();
        let (tx, rx) = crossbeam_channel::bounded::<FitOutput>(1);
        let wake = ctx.cloned();
        let spawned = std::thread::Builder::new().name("autoeq-fit".into()).spawn(move || {
            let out: FitOutput = plan.into_iter().map(|(sides, source)| (sides, auto_eq(&source, &target, &opts))).collect();
            let _ = tx.send(out);
            if let Some(ctx) = wake {
                ctx.request_repaint();
            }
        });
        if spawned.is_ok() {
            self.auto_eq.job = Some(rx);
        }
    }

    pub fn auto_eq_running(&self) -> bool {
        self.auto_eq.job.is_some()
    }

    pub(crate) fn poll_auto_eq(&mut self) {
        let Some(rx) = &self.auto_eq.job else { return };
        let out = match rx.try_recv() {
            Ok(out) => out,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => Vec::new(),
        };
        self.auto_eq.job = None;
        if !out.is_empty() {
            self.apply_auto_eq(out);
        }
    }

    /// Replaces the filters + preamp of the fitted side(s) (PEQ enabled), remembers the old
    /// PEQ for undo and toasts the errors before / after.
    pub(crate) fn apply_auto_eq(&mut self, out: FitOutput) {
        let tr = self.tr();
        let before = self.peq.clone();
        let mut peq = before.clone();
        let mut summary = Vec::new();
        for (sides, res) in &out {
            for &side in sides {
                peq.set_channel(side, PeqChannel { preamp_db: res.preamp_db, filters: res.filters.clone() });
            }
            let side = (peq.channel_mode == PeqChannelMode::Separate && sides.len() == 1).then(|| sides[0]);
            summary.push(FitSummary { side, before_db: res.rms_error_before_db, after_db: res.rms_error_after_db, filters: res.filters.len() });
        }
        peq.enabled = true;
        self.set_parametric_eq(peq);
        self.auto_eq.undo = Some(before);
        let all_empty = out.iter().all(|(_, r)| r.filters.is_empty());
        let msg = if all_empty {
            tr.t("toast.autoeq_nothing").to_owned()
        } else if let [l, r] = summary.as_slice() {
            tr.f("toast.autoeq_created_lr", &[&fmt_db(l.before_db), &fmt_db(l.after_db), &fmt_db(r.before_db), &fmt_db(r.after_db)])
        } else {
            let s = summary[0];
            tr.f("toast.autoeq_created", &[&s.filters, &fmt_db(s.before_db), &fmt_db(s.after_db)])
        };
        self.toast_success(msg);
        self.auto_eq.last = summary;
    }

    /// Restores the PEQ from before the last auto-create.
    pub fn undo_auto_eq(&mut self) {
        let Some(prev) = self.auto_eq.undo.take() else { return };
        self.set_parametric_eq(prev);
        self.auto_eq.last.clear();
        let tr = self.tr();
        self.toast_info(tr.t("toast.autoeq_undone").to_owned());
    }
}

/// "4.2" (one decimal).
pub(crate) fn fmt_db(v: f32) -> String {
    format!("{v:.1}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::app::tests::test_app;
    use lrc_player_application::infrastructure::audio::engine::{
        filters_response_db, FilterKind, PeqFilter, AudioEngine, EngineConfig, OutputMode,
    };
    use std::time::{Duration, Instant};

    /// Smooth IEM-like target (absolute SPL).
    fn target_db(f: f32) -> f32 {
        let f = f as f64;
        let bass = 6.0 / (1.0 + (f / 120.0).powi(2));
        let pinna = 9.0 * (-(f / 2800.0).log2().powi(2) / (2.0 * 0.49)).exp();
        let roll = -8.0 / (1.0 + (9000.0 / f).powi(4));
        (75.0 + bass + pinna + roll) as f32
    }

    fn grid() -> Vec<f32> {
        (0..=240).map(|i| 20.0 * 1000f32.powf(i as f32 / 240.0)).collect()
    }

    fn curve(f: impl Fn(f32) -> f32) -> FrequencyResponse {
        let freqs = grid();
        let db = freqs.iter().map(|&x| f(x)).collect();
        FrequencyResponse { freqs, db }
    }

    fn pk(freq_hz: f32, gain_db: f32, q: f32) -> PeqFilter {
        PeqFilter { kind: FilterKind::Peak, freq_hz, gain_db, q }
    }

    /// A measured curve that needs `need` to reach the target.
    fn needs(need: &[PeqFilter]) -> FrequencyResponse {
        let freqs = grid();
        let corr = filters_response_db(need, &freqs);
        FrequencyResponse { db: freqs.iter().zip(&corr).map(|(&f, c)| target_db(f) + 18.0 - c).collect(), freqs }
    }

    fn wait_fit(app: &mut GuiApp) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.auto_eq_running() {
            app.poll_auto_eq();
            assert!(Instant::now() < deadline, "fit did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Stereo 16-bit 48 kHz WAV of a 5 kHz sine at 0.1 (both channels).
    fn write_sine_wav(path: &Path, seconds: f32) {
        let rate = 48_000u32;
        let frames = (rate as f32 * seconds) as u32;
        let mut data = Vec::new();
        for i in 0..frames {
            let v = ((i as f32 * 5000.0 * std::f32::consts::TAU / rate as f32).sin() * 0.1 * 32767.0).round() as i16;
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        std::fs::write(path, out).unwrap();
    }

    fn write_rew(path: &Path, c: &FrequencyResponse) {
        let mut s = String::from("* Measurement data measured by IEM_Measure 0.1.0\n* Freq(Hz) SPL(dB) Phase(degrees)\n");
        for (f, d) in c.freqs.iter().zip(&c.db) {
            s.push_str(&format!("{f:.3} {d:.3} 0.0000\n"));
        }
        std::fs::write(path, s).unwrap();
    }

    fn write_csv(path: &Path, c: &FrequencyResponse) {
        let mut s = String::from("frequency,raw\n");
        for (f, d) in c.freqs.iter().zip(&c.db) {
            s.push_str(&format!("{f:.3},{d:.3}\n"));
        }
        std::fs::write(path, s).unwrap();
    }

    /// Linked mode: one fit, the filters replace the main set, undo restores the old PEQ.
    #[test]
    fn auto_create_linked_replaces_filters_and_undo_restores() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        // Nothing loaded: an error toast, no job.
        app.start_auto_eq(None);
        assert!(!app.auto_eq_running());
        assert!(app.ui.last_toast.as_ref().is_some_and(|t| t.0.contains("ターゲット")), "{:?}", app.ui.last_toast);

        let manual = ParametricEq { enabled: false, preamp_db: -1.0, filters: vec![pk(500.0, 2.0, 1.0)], ..Default::default() };
        app.set_parametric_eq(manual.clone());
        app.set_auto_eq_curve(CurveSlot::Target, LoadedCurve::new("t.csv".into(), curve(target_db)));
        app.start_auto_eq(None);
        assert!(app.ui.last_toast.as_ref().is_some_and(|t| t.0.contains("測定データ")), "{:?}", app.ui.last_toast);
        let need = [pk(3000.0, -5.0, 2.0), pk(6000.0, 4.0, 3.0), pk(150.0, 3.0, 1.0)];
        app.set_auto_eq_curve(CurveSlot::Source, LoadedCurve::new("m.txt".into(), needs(&need)));
        app.start_auto_eq(None);
        assert!(app.auto_eq_running());
        wait_fit(&mut app);
        assert!(app.peq.enabled);
        assert_eq!(app.peq.channel_mode, PeqChannelMode::Linked);
        assert!(!app.peq.filters.is_empty() && app.peq.filters.len() <= 10);
        assert!(app.peq.preamp_db <= 0.0);
        let s = app.auto_eq.last[0];
        assert!(s.after_db < 0.5 && s.before_db > 1.5, "{s:?}");
        assert_eq!(app.config.parametric_eq, super::super::audio_quality::peq_to_config(&app.peq));
        assert!(app.ui.last_toast.as_ref().is_some_and(|t| t.0.contains("作成しました")), "{:?}", app.ui.last_toast);
        // Undo.
        app.undo_auto_eq();
        assert_eq!(app.peq, manual);
        assert!(app.auto_eq.undo.is_none() && app.auto_eq.last.is_empty());
    }

    /// Separate mode with L and R files: each side gets its own fit; the shared measurement
    /// gives both sides the same filters; linked mode with two files fits their average.
    #[test]
    fn auto_create_separate_and_average() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        app.set_peq_channel_mode(PeqChannelMode::Separate);
        app.set_auto_eq_target_flat(true);
        app.set_auto_eq_same_source(false);
        // Narrow corrections outside the 250–2500 Hz level-matching band (a correction inside
        // it also shifts the level, which is just a different preamp).
        let need_l = [pk(3500.0, 6.0, 3.0)];
        let need_r = [pk(7000.0, -6.0, 3.0)];
        let flat_need = |need: &[PeqFilter]| {
            let freqs = grid();
            let c = filters_response_db(need, &freqs);
            FrequencyResponse { db: c.iter().map(|v| 90.0 - v).collect(), freqs }
        };
        app.set_auto_eq_curve(CurveSlot::Source, LoadedCurve::new("l.txt".into(), flat_need(&need_l)));
        app.start_auto_eq(None);
        assert!(!app.auto_eq_running(), "R missing");
        assert!(app.ui.last_toast.as_ref().is_some_and(|t| t.0.contains('R')), "{:?}", app.ui.last_toast);
        app.set_auto_eq_curve(CurveSlot::SourceRight, LoadedCurve::new("r.txt".into(), flat_need(&need_r)));
        app.start_auto_eq(None);
        wait_fit(&mut app);
        let l = app.peq.channel(PeqSide::Left);
        let r = app.peq.channel(PeqSide::Right);
        // Correction relative to 1 kHz (the absolute level is arbitrary).
        let at = |c: &PeqChannel, hz: f32| {
            let v = filters_response_db(&c.filters, &[hz, 1000.0]);
            v[0] - v[1]
        };
        assert!((at(&l, 3500.0) - 6.0).abs() < 0.6 && at(&l, 7000.0).abs() < 1.0, "{l:?}");
        assert!((at(&r, 7000.0) + 6.0).abs() < 0.6 && at(&r, 3500.0).abs() < 1.0, "{r:?}");
        assert_eq!(app.auto_eq.last.len(), 2);
        assert_eq!(app.auto_eq.last[1].side, Some(PeqSide::Right));
        assert!(app.ui.last_toast.as_ref().is_some_and(|t| t.0.contains("L・R")), "{:?}", app.ui.last_toast);
        // Same measurement for both: identical sets.
        app.set_auto_eq_same_source(true);
        app.start_auto_eq(None);
        wait_fit(&mut app);
        assert_eq!(app.peq.channel(PeqSide::Left), app.peq.channel(PeqSide::Right));
        // Linked with two files: the average (+3 dB at 3.5 kHz, -3 dB at 7 kHz).
        app.set_auto_eq_same_source(false);
        app.set_peq_channel_mode(PeqChannelMode::Linked);
        app.start_auto_eq(None);
        wait_fit(&mut app);
        let m = app.peq.channel(PeqSide::Left);
        assert!((at(&m, 3500.0) - 3.0).abs() < 0.6 && (at(&m, 7000.0) + 3.0).abs() < 0.6, "{m:?}");
    }

    /// Curve files: the picker result (read on the worker) lands in the slot and the config;
    /// broken files toast in Japanese; paths are reloaded by a new app (missing ones skipped).
    #[test]
    fn curve_files_load_persist_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        let rew = dir.path().join("left.txt");
        let csv = dir.path().join("target.csv");
        write_rew(&rew, &curve(|f| target_db(f) + 20.0));
        write_csv(&csv, &curve(target_db));
        app.apply_curve_pick(CurveSlot::Source, Some((rew.clone(), load_curve_file(&rew))));
        app.apply_curve_pick(CurveSlot::Target, Some((csv.clone(), load_curve_file(&csv))));
        assert_eq!(app.auto_eq.source.as_ref().map(|c| c.name.as_str()), Some("left.txt"));
        assert_eq!(app.auto_eq.target.as_ref().map(|c| c.curve.freqs.len()), Some(241));
        assert_eq!(app.config.auto_eq.source_path.as_deref(), Some(rew.to_string_lossy().as_ref()));
        assert!(!app.config.auto_eq.target_flat);
        let bad = dir.path().join("bad.txt");
        std::fs::write(&bad, "Filter 1: ON PK Fc 100 Hz Gain 1 dB Q 1\n").unwrap();
        app.apply_curve_pick(CurveSlot::SourceRight, Some((bad.clone(), load_curve_file(&bad))));
        assert!(app.auto_eq.source_r.is_none());
        let toast = app.ui.last_toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
        assert!(toast.contains("bad.txt") && toast.contains("見つかりません"), "{toast}");
        let missing = dir.path().join("nope.txt");
        app.apply_curve_pick(CurveSlot::SourceRight, Some((missing.clone(), load_curve_file(&missing))));
        assert!(app.auto_eq.source_r.is_none());
        // Options.
        let mut o = app.auto_eq_options();
        o.max_filters = 7;
        o.use_shelves = false;
        app.set_auto_eq_options(o);
        assert_eq!(app.config.auto_eq.max_filters, 7);
        assert!(!app.config.auto_eq.use_shelves);
        // A new app from that config reloads the curves; a missing file is skipped.
        let mut cfg = app.config.clone();
        cfg.auto_eq.source_r_path = Some(missing.to_string_lossy().into_owned());
        let app2 = GuiApp::with_options(
            &ctx,
            crate::presentation::app::AppOptions {
                persist: false,
                engine: false,
                meta_path: Some(dir.path().join("meta2.db")),
                config: Some(cfg),
                load_fonts: false,
                media_controls: false,
                ..Default::default()
            },
        );
        assert_eq!(app2.auto_eq.target.as_ref().map(|c| &c.curve), app.auto_eq.target.as_ref().map(|c| &c.curve));
        assert_eq!(app2.auto_eq.source.as_ref().map(|c| &c.curve), app.auto_eq.source.as_ref().map(|c| &c.curve));
        assert!(app2.auto_eq.source_r.is_none());
        assert_eq!(app2.auto_eq_options().max_filters, 7);
        // Clearing a slot clears its path.
        app.clear_auto_eq_curve(CurveSlot::Target);
        assert!(app.auto_eq.target.is_none() && app.config.auto_eq.target_path.is_none());
    }

    /// The engine gets the per-channel settings after an L/R auto-create (manual output).
    #[test]
    fn auto_create_reaches_the_engine_per_channel() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = test_app(&ctx, dir.path(), false);
        app.engine = Some(
            AudioEngine::new(EngineConfig {
                output: OutputMode::Manual { sample_rate: 48_000, channels: 2 },
                ring_ms: 500,
                prebuffer_ms: 100,
                ..Default::default()
            })
            .unwrap(),
        );
        app.set_peq_channel_mode(PeqChannelMode::Separate);
        app.set_auto_eq_target_flat(true);
        app.set_auto_eq_same_source(false);
        let flat_need = |need: &[PeqFilter]| {
            let freqs = grid();
            let c = filters_response_db(need, &freqs);
            FrequencyResponse { db: c.iter().map(|v| 90.0 - v).collect(), freqs }
        };
        app.set_auto_eq_curve(CurveSlot::Source, LoadedCurve::new("l.txt".into(), flat_need(&[pk(5000.0, -6.0, 2.0)])));
        app.set_auto_eq_curve(CurveSlot::SourceRight, LoadedCurve::new("r.txt".into(), flat_need(&[pk(5000.0, 4.0, 2.0)])));
        // Play a 5 kHz stereo sine through the manual output.
        let wav = dir.path().join("sine.wav");
        write_sine_wav(&wav, 4.0);
        app.engine.as_ref().unwrap().load(wav, 0, true);
        app.start_auto_eq(None);
        wait_fit(&mut app);
        let l = app.peq.channel(PeqSide::Left);
        let r = app.peq.channel(PeqSide::Right);
        assert!(l.filters.iter().any(|f| f.gain_db < -4.0), "{l:?}");
        assert!(r.filters.iter().any(|f| f.gain_db > 3.0), "{r:?}");
        assert!(r.preamp_db <= -3.5, "{r:?}");
        let engine = app.engine.as_ref().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        // Until the engine reports the PEQ (the settings reached the renderer)…
        loop {
            let mut buf = vec![0.0f32; 960];
            engine.render_manual(&mut buf);
            if engine.snapshot().signal.is_some_and(|s| s.parametric_eq) {
                break;
            }
            assert!(Instant::now() < deadline, "engine never reported the PEQ");
            std::thread::sleep(Duration::from_millis(1));
        }
        // …then one second of audio (the crossfade is long over in its second half).
        let mut out = Vec::new();
        while out.len() < 48_000 * 2 {
            let mut buf = vec![0.0f32; 960];
            if engine.render_manual(&mut buf) && buf.iter().any(|v| *v != 0.0) {
                out.extend_from_slice(&buf);
            }
            assert!(Instant::now() < deadline, "no audio");
            std::thread::sleep(Duration::from_millis(1));
        }
        let tail = &out[out.len() / 2..];
        let level = |ch: usize| {
            let peak = tail.iter().skip(ch).step_by(2).fold(0.0f32, |m, v| m.max(v.abs()));
            20.0 * (peak / 0.1).log10()
        };
        let signal = engine.snapshot().signal.expect("signal path");
        assert!(signal.parametric_eq);
        let expect = |side| {
            lrc_player_application::infrastructure::audio::engine::combined_response_db(&app.eq, &app.peq, side, 48_000, &[5000.0])[0]
                + signal.headroom_db
        };
        let (got_l, got_r) = (level(0), level(1));
        assert!((got_l - expect(PeqSide::Left)).abs() < 0.3, "L {got_l} vs {}", expect(PeqSide::Left));
        assert!((got_r - expect(PeqSide::Right)).abs() < 0.3, "R {got_r} vs {}", expect(PeqSide::Right));
        assert!(got_r - got_l > 4.5, "R is ~6 dB louder than L at 5 kHz: {got_l} {got_r}");
    }
}
