//! Screenshots of per-element CJK font selection (headless wgpu via egui_kittest, like
//! `ui::gallery_tests`). Ignored by default; run with:
//!
//! ```text
//! UI_SCREENS_DIR=/some/dir cargo test -p lrc_player --lib ui::text::screenshot_tests -- --ignored --test-threads=1
//! ```
//!
//! PNGs go to `$UI_SCREENS_DIR/` (default: `target/ui-screens/cjk`).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use eframe::egui::{self, pos2, text::LayoutJob, vec2, Align, Color32, Context, FontDefinitions, FontFamily, FontId, Rect, Stroke, Ui, Vec2};
use egui_kittest::Harness;

use super::{append, family_for, lang_of};
use crate::presentation::ui::{
    fonts::{self, FontPlatform},
    i18n::Tr,
    theme::{self, Accent, ThemeMode},
    views::{lyrics_view, LyricLine, LyricsProps, LyricsViewState},
};

fn out_dir() -> PathBuf {
    let dir = std::env::var_os("UI_SCREENS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-screens/cjk"));
    std::fs::create_dir_all(&dir).expect("create screenshot dir");
    dir
}

/// The Windows font table (the target platform), from `/mnt/c/Windows/Fonts` under WSL.
/// `CJK_FONT_PLATFORM=linux` / `macos` renders the other platform orders instead.
fn font_defs() -> &'static FontDefinitions {
    static DEFS: OnceLock<FontDefinitions> = OnceLock::new();
    DEFS.get_or_init(|| {
        let platform = match std::env::var("CJK_FONT_PLATFORM").as_deref() {
            Ok("linux") => FontPlatform::Linux,
            Ok("macos") => FontPlatform::MacOs,
            _ => FontPlatform::Windows,
        };
        let (defs, report) = fonts::build_definitions_for(&[Path::new("/mnt/c/Windows/Fonts")], platform);
        eprintln!("fonts ({platform:?}): {report:#?}");
        defs
    })
}

fn setup(ctx: &Context) {
    let id = egui::Id::new("cjk_screens_setup");
    if ctx.data(|d| d.get_temp::<bool>(id).is_none()) {
        fonts::install_definitions(ctx, font_defs().clone());
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
    theme::sync(ctx, ThemeMode::Dark, Accent::Teal);
}

fn shoot(size: Vec2, ppp: f32, name: &str, mut app: impl FnMut(&mut Ui)) {
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(ppp)
        .with_theme(egui::Theme::Dark)
        .build_ui(move |ui| {
            setup(ui.ctx());
            let pal = theme::palette(ui);
            ui.painter().rect_filled(ui.max_rect(), 0.0, pal.bg);
            app(ui);
        });
    harness.run_steps(6);
    let img = harness.render().expect("render (needs wgpu adapter)");
    let suffix = std::env::var("CJK_FONT_PLATFORM").map(|p| format!("_{p}")).unwrap_or_default();
    let path = out_dir().join(format!("{name}{suffix}.png"));
    img.save(&path).expect("save png");
    eprintln!("wrote {}", path.display());
}

const ROWS: &[&str] = &[
    "群青",
    "夜に駆ける",
    "怪獣の花唄",
    "优里 - 我们的爱",
    "周杰伦 - 晴天",
    "我們的愛",
    "這是什麼 說好的幸福",
    "사랑 - 아이유 (IU)",
    "Lemon",
    "Blinding Lights",
];

/// One row: language tag, the text in its own family (with a Latin baseline guide), and the
/// same text in plain `Proportional` (the old rendering) for comparison.
fn rows(ui: &mut Ui, size: f32, bold: bool) {
    let pal = theme::palette(ui);
    let row_h = size * 2.0;
    let left = ui.max_rect().left() + 12.0;
    let mut y = ui.max_rect().top() + 10.0;
    let painter = ui.painter().clone();
    let head = |x: f32, y: f32, t: &str| {
        painter.text(pos2(x, y), egui::Align2::LEFT_TOP, t, FontId::proportional(11.0), pal.text_muted);
    };
    head(left, y, "lang");
    head(left + 80.0, y, "per-element family (guide = Latin baseline / row box)");
    head(left + 470.0, y, "for comparison: Proportional / bold (chrome chain)");
    y += 18.0;
    for (i, t) in ROWS.iter().enumerate() {
        let lang = lang_of(t);
        let bg = if i % 2 == 0 { pal.surface } else { pal.bg };
        let row = Rect::from_min_size(pos2(left - 6.0, y), vec2(ui.max_rect().width() - 12.0, row_h));
        painter.rect_filled(row, 0.0, bg);
        painter.text(pos2(left, row.center().y), egui::Align2::LEFT_CENTER, format!("{lang:?}"), FontId::proportional(11.0), pal.text_muted);
        // Prefix with digits so the Latin baseline is visible inside the element itself.
        let shown = format!("0{i} {t}");
        for (x, family) in [(left + 80.0, family_for(t, bold)), (left + 470.0, if bold { theme::bold_family(ui.ctx()) } else { FontFamily::Proportional })] {
            let job = LayoutJob::simple_singleline(shown.clone(), FontId::new(size, family), pal.text);
            let g = ui.fonts_mut(|f| f.layout_job(job));
            let top = row.center().y - g.size().y / 2.0;
            let baseline = top + g.rows[0].row.glyphs.first().map_or(0.0, |gl| gl.pos.y);
            painter.hline(x..=x + g.size().x, baseline, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 80, 80, 140)));
            painter.rect_stroke(Rect::from_min_size(pos2(x, top), g.size()), 0.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(80, 160, 255, 90)), egui::StrokeKind::Outside);
            painter.galley(pos2(x, top), g, pal.text);
        }
        y += row_h;
    }
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn cjk_rows() {
    shoot(vec2(820.0, 470.0), 1.0, "cjk_rows_14", |ui| rows(ui, 14.0, false));
    shoot(vec2(820.0, 470.0), 2.0, "cjk_rows_14_2x", |ui| rows(ui, 14.0, false));
    shoot(vec2(980.0, 900.0), 1.0, "cjk_rows_32_bold", |ui| rows(ui, 32.0, true));
    shoot(vec2(820.0, 640.0), 1.0, "cjk_rows_22_bold", |ui| rows(ui, 22.0, true));
}

fn chinese_lyrics() -> Vec<LyricLine<'static>> {
    let texts = [
        ("故事的小黄花", false),
        ("从出生那年就飘着", false),
        ("童年的荡秋千", true),
        ("随记忆一直晃到现在", true),
        ("刮风这天 我试过握着你手", false),
        ("但偏偏 雨渐渐 大到我看你不见", false),
        ("", false),
        ("還要多久 我才能在你身邊", false),
        ("等到放晴的那天 也許我會比較好一點", false),
        ("夜に駆ける (Japanese line)", false),
        ("사랑해요 (Korean line)", false),
    ];
    texts
        .iter()
        .enumerate()
        .map(|(i, (t, k))| LyricLine { text: t, start_ms: 10_000 + i as u64 * 4_000, karaoke: *k })
        .collect()
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn cjk_lyrics_view() {
    for (ppp, name) in [(1.0, "cjk_lyrics"), (2.0, "cjk_lyrics_2x")] {
        lyrics_shot(ppp, name);
    }
}

fn lyrics_shot(ppp: f32, name: &str) {
    let lines = chinese_lyrics();
    let mut state = LyricsViewState::default();
    shoot(vec2(640.0, 720.0), ppp, name, move |ui| {
        lyrics_view(
            ui,
            &LyricsProps {
                lines: &lines,
                revision: 1,
                current: Some(3),
                highlighted_chars: 5.4,
                offset_ms: 0,
                font_size: 30.0,
                align: Align::Center,
                on_backdrop: false,
                show_controls: true,
                can_save_offset: false,
                line_shift: 0,
                char_shift: 0,
                can_char_shift: true,
                can_generate_timing: false,
                generating_timing: false,
                desktop_lyrics: false,
                tr: Tr::JA,
            },
            &mut state,
        );
    });
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn cjk_paragraph() {
    shoot(vec2(720.0, 360.0), 1.0, "cjk_paragraph", |ui| {
        let pal = theme::palette(ui);
        let mut job = LayoutJob { wrap: egui::text::TextWrapping { max_width: 680.0, ..Default::default() }, ..Default::default() };
        let parts: &[(&str, bool)] = &[
            ("Now playing: ", false),
            ("周杰伦 - 晴天", true),
            (" · ", false),
            ("我們的愛", true),
            (" · ", false),
            ("夜に駆ける", true),
            (" · ", false),
            ("사랑", true),
            (" · ", false),
            ("Lemon", true),
            (". 歌词: ", false),
            ("刮风这天 我试过握着你手 但偏偏 雨渐渐 大到我看你不见", false),
            (" / 歌詞: ", false),
            ("沈むように溶けてゆくように 二人だけの空が広がる夜に", false),
            (" / 가사: ", false),
            ("이 밤 그날의 반딧불을 당신의 창 가까이 보낼게요", false),
        ];
        for (t, bold) in parts {
            append(&mut job, t, 18.0, *bold, if *bold { pal.text } else { pal.text_muted });
        }
        let g = ui.fonts_mut(|f| f.layout_job(job));
        ui.painter().galley(ui.max_rect().min + vec2(20.0, 20.0), g, pal.text);
    });
}
