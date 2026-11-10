//! Font installation for the redesigned UI.
//!
//! Families:
//! - `Proportional` (UI chrome): a crisp system Latin UI font (Segoe UI / Inter / Noto Sans /
//!   ...), then the Japanese font (the UI defaults to Japanese), then egui's bundled fonts
//!   (emoji, symbols), then the Chinese / Korean fonts as last-resort fallbacks.
//! - [`super::theme::BOLD_FAMILY`]: bold faces of the same, then the regular chain.
//! - One family per CJK language, used by [`super::text`] so that an element is drawn entirely
//!   in one language's glyph forms: `ui-ja`, `ui-zh-hans`, `ui-zh-hant`, `ui-ko` and their
//!   `-bold` variants. Order: Latin UI font, that language's CJK font, the other CJK fonts,
//!   egui's defaults (bold variants: bold faces first, then the regular chain).
//!
//! CJK fonts are chosen per OS so they pair with the platform UI font (see [`CJK_CANDIDATES`]);
//! the other platforms' names are tried afterwards (useful under WSL with `/mnt/c/Windows/Fonts`).
//!
//! Vertical metrics: egui takes a row's height and ascent from the *first* font of the family
//! (the Latin font here) and centres fallback faces in it. Each CJK face gets a [`FontTweak`]
//! (`scale`, `y_offset_factor`) computed from its metrics so that its ideographs have the same
//! size and vertical position as the Japanese font's, which itself sits on the Latin baseline.
//! A Chinese or Korean row therefore has the same baseline and height as a Japanese row.
//!
//! Memory: font files are memory-mapped once per file and shared by every face / family that
//! uses them (`.ttc` collections such as `msyh.ttc` or `NotoSansCJK-Regular.ttc` are mapped once
//! for all their faces). egui gets `Cow::Borrowed` data, so it never copies the bytes (it would
//! clone `Cow::Owned` data on every font-atlas rebuild); only touched pages become resident.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui::{self, epaint::text::VariationCoords, Context, FontData, FontDefinitions, FontFamily, FontTweak};
use fontdb::{Database, Family, Query, Source, Stretch, Style, Weight, ID};
use skrifa::{
    instance::{LocationRef, Size},
    MetadataProvider,
};

use super::text::{self, TextLang};
use super::theme::BOLD_FAMILY;

const LATIN_FAMILIES: &[&str] = &[
    "Segoe UI",
    "Inter",
    "SF Pro Text",
    "Helvetica Neue",
    "Noto Sans",
    "Cantarell",
    "Ubuntu",
    "DejaVu Sans",
];

/// CJK font candidates for one language, per platform (preferred platform first at runtime).
pub struct CjkCandidates {
    pub lang: TextLang,
    /// A character the font must contain (rejects mislabelled / partial fonts).
    pub probe: char,
    pub windows: &'static [&'static str],
    pub macos: &'static [&'static str],
    pub linux: &'static [&'static str],
}

/// Font candidates per language. Windows: Yu Gothic UI / Microsoft YaHei UI / Microsoft
/// JhengHei UI / Malgun Gothic (UI variants pair with Segoe UI). macOS: Hiragino Sans /
/// PingFang SC / PingFang TC / Apple SD Gothic Neo. Linux: Noto Sans CJK JP/SC/TC/KR (faces of
/// one `.ttc`), then Source Han Sans, then WenQuanYi / Droid Sans Fallback.
pub const CJK_CANDIDATES: [CjkCandidates; 4] = [
    CjkCandidates {
        lang: TextLang::Ja,
        probe: 'あ',
        windows: &["Yu Gothic UI", "Yu Gothic", "Meiryo UI", "Meiryo", "BIZ UDPGothic"],
        macos: &["Hiragino Sans", "Hiragino Kaku Gothic ProN", "Hiragino Kaku Gothic Pro"],
        linux: &[
            "Noto Sans CJK JP",
            "Noto Sans JP",
            "Source Han Sans JP",
            "Source Han Sans",
            "IPAexGothic",
            "IPAPGothic",
            "TakaoPGothic",
            "VL PGothic",
        ],
    },
    CjkCandidates {
        lang: TextLang::ZhHans,
        probe: '们',
        windows: &["Microsoft YaHei UI", "Microsoft YaHei", "DengXian"],
        macos: &["PingFang SC", "Hiragino Sans GB", "Heiti SC", "STHeiti"],
        linux: &[
            "Noto Sans CJK SC",
            "Noto Sans SC",
            "Source Han Sans SC",
            "Source Han Sans CN",
            "WenQuanYi Micro Hei",
            "WenQuanYi Zen Hei",
            "Droid Sans Fallback",
        ],
    },
    CjkCandidates {
        lang: TextLang::ZhHant,
        probe: '們',
        windows: &["Microsoft JhengHei UI", "Microsoft JhengHei"],
        macos: &["PingFang TC", "PingFang HK", "Heiti TC"],
        linux: &[
            "Noto Sans CJK TC",
            "Noto Sans TC",
            "Source Han Sans TC",
            "Source Han Sans TW",
            "Noto Sans CJK HK",
            "WenQuanYi Micro Hei",
            "WenQuanYi Zen Hei",
            "Droid Sans Fallback",
        ],
    },
    CjkCandidates {
        lang: TextLang::Ko,
        probe: '한',
        windows: &["Malgun Gothic"],
        macos: &["Apple SD Gothic Neo", "AppleGothic"],
        linux: &[
            "Noto Sans CJK KR",
            "Noto Sans KR",
            "Source Han Sans KR",
            "Source Han Sans K",
            "NanumGothic",
            "NanumBarunGothic",
            "UnDotum",
            "Droid Sans Fallback",
        ],
    },
];

/// Fallback order of the *other* CJK fonts inside a language family (closest glyph forms first:
/// a missing Simplified font falls back to Traditional before Japanese, Korean hanja to
/// Traditional, ...).
fn fallback_order(lang: TextLang) -> [TextLang; 3] {
    use TextLang::*;
    match lang {
        ZhHans => [ZhHant, Ja, Ko],
        ZhHant => [ZhHans, Ja, Ko],
        Ko => [ZhHant, Ja, ZhHans],
        Ja | Latin => [ZhHans, ZhHant, Ko],
    }
}

/// Reference ideographs for size / position normalization (present in JA, SC, TC and KR fonts).
const REF_CHARS: [char; 3] = ['日', '永', '中'];

/// What [`install`] ended up using (for logs / diagnostics).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontReport {
    pub latin: Option<String>,
    pub latin_bold: Option<String>,
    pub japanese: Option<String>,
    pub japanese_bold: Option<String>,
    pub chinese_simplified: Option<String>,
    pub chinese_simplified_bold: Option<String>,
    pub chinese_traditional: Option<String>,
    pub chinese_traditional_bold: Option<String>,
    pub korean: Option<String>,
    pub korean_bold: Option<String>,
}

#[derive(Clone, Copy)]
struct FontsInstalled {
    bold: bool,
    /// Pass in which `set_fonts` was called; the fonts are live from the next pass on.
    pass: u64,
}

fn installed_id() -> egui::Id {
    egui::Id::new("lrc_ui_fonts_installed")
}

/// Whether the bold family is registered *and active* (used by `theme::bold_family`).
pub fn bold_available(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<FontsInstalled>(installed_id()))
        .is_some_and(|f| f.bold && ctx.cumulative_pass_nr() > f.pass)
}

/// A face picked from the database: font-data key plus shared bytes.
#[derive(Clone)]
struct Face {
    key: String,
    bytes: &'static [u8],
    index: u32,
    /// `wght` axis value to pin for variable fonts (their default instance may be Thin).
    wght: Option<f32>,
}

/// Which platform's font names are tried first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontPlatform {
    Windows,
    MacOs,
    Linux,
}

impl FontPlatform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Font definitions built from system fonts (plus any `extra_dirs`). Expensive (scans font
/// files): build once, ideally off the UI thread, then pass to [`install_definitions`].
pub fn build_definitions(extra_dirs: &[&Path]) -> (FontDefinitions, FontReport) {
    build_definitions_for(extra_dirs, FontPlatform::current())
}

/// [`build_definitions`] with an explicit platform preference (tests, diagnostics).
pub fn build_definitions_for(extra_dirs: &[&Path], platform: FontPlatform) -> (FontDefinitions, FontReport) {
    let mut db = Database::new();
    db.load_system_fonts();
    for dir in extra_dirs {
        if dir.is_dir() {
            db.load_fonts_dir(dir);
        }
    }

    let mut defs = FontDefinitions::default();
    let mut report = FontReport::default();
    let default_prop = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    // ---- pick faces ----
    let latin_reg = pick(&mut db, LATIN_FAMILIES, false, None);
    let latin_bold = pick(&mut db, LATIN_FAMILIES, true, None);
    let mut cjk_reg: HashMap<TextLang, Face> = HashMap::new();
    let mut cjk_bold: HashMap<TextLang, Face> = HashMap::new();
    for cand in &CJK_CANDIDATES {
        let names = ordered_names(cand, platform);
        if let Some(f) = pick(&mut db, &names, false, Some(cand.probe)) {
            cjk_reg.insert(cand.lang, f);
        } else {
            warn_missing_once(cand.lang);
        }
        if let Some(f) = pick(&mut db, &names, true, Some(cand.probe)) {
            cjk_bold.insert(cand.lang, f);
        }
    }
    let name = |f: Option<&Face>| f.map(|f| f.key.clone());
    report.latin = name(latin_reg.as_ref());
    report.latin_bold = name(latin_bold.as_ref());
    report.japanese = name(cjk_reg.get(&TextLang::Ja));
    report.japanese_bold = name(cjk_bold.get(&TextLang::Ja));
    report.chinese_simplified = name(cjk_reg.get(&TextLang::ZhHans));
    report.chinese_simplified_bold = name(cjk_bold.get(&TextLang::ZhHans));
    report.chinese_traditional = name(cjk_reg.get(&TextLang::ZhHant));
    report.chinese_traditional_bold = name(cjk_bold.get(&TextLang::ZhHant));
    report.korean = name(cjk_reg.get(&TextLang::Ko));
    report.korean_bold = name(cjk_bold.get(&TextLang::Ko));

    // ---- register font data (Latin untweaked, CJK normalized) ----
    let insert = |defs: &mut FontDefinitions, f: &Face, mut tweak: FontTweak| {
        defs.font_data.entry(f.key.clone()).or_insert_with(|| {
            let mut data = FontData::from_static(f.bytes);
            data.index = f.index;
            if let Some(w) = f.wght {
                tweak.coords = VariationCoords::new([(b"wght", w)]);
            }
            data.tweak = tweak;
            Arc::new(data)
        });
    };
    for f in [&latin_reg, &latin_bold].into_iter().flatten() {
        insert(&mut defs, f, FontTweak::default());
    }
    // Metrics of the font heading each chain (egui's first default font if no system Latin font).
    let head_reg = latin_reg
        .as_ref()
        .and_then(|f| VMetrics::of(f.bytes, f.index))
        .or_else(|| default_prop.first().and_then(|k| defs.font_data.get(k)).and_then(|d| VMetrics::of(&d.font, d.index)));
    let head_bold = latin_bold.as_ref().and_then(|f| VMetrics::of(f.bytes, f.index)).or(head_reg);
    for (faces, head) in [(&cjk_reg, head_reg), (&cjk_bold, head_bold)] {
        // Reference: the Japanese face of this weight (else the first CJK face found).
        let reference = [TextLang::Ja, TextLang::ZhHans, TextLang::ZhHant, TextLang::Ko]
            .iter()
            .find_map(|l| faces.get(l))
            .and_then(|f| VMetrics::of(f.bytes, f.index));
        for lang in TextLang::CJK {
            let Some(face) = faces.get(&lang) else { continue };
            let tweak = match (head, reference, VMetrics::of(face.bytes, face.index)) {
                (Some(head), Some(reference), Some(m)) => normalize(&head, &reference, &m),
                _ => FontTweak::default(),
            };
            tracing::debug!(
                "font {} ({lang:?}): scale {:.3}, y_offset_factor {:.4}",
                face.key,
                tweak.scale,
                tweak.y_offset_factor
            );
            insert(&mut defs, face, tweak);
        }
    }

    // ---- families ----
    let latin_reg_key = latin_reg.as_ref().map(|f| f.key.clone()).or_else(|| default_prop.first().cloned());
    let latin_bold_key = latin_bold.as_ref().map(|f| f.key.clone());
    let reg_key = |l: TextLang| cjk_reg.get(&l).map(|f| f.key.clone());
    let bold_key = |l: TextLang| cjk_bold.get(&l).map(|f| f.key.clone());

    // Proportional: Latin, Japanese, egui defaults (as before), then the other CJK fonts.
    let mut prop = Vec::new();
    push_all(&mut prop, [latin_reg_key.clone(), reg_key(TextLang::Ja)].into_iter().flatten());
    push_all(&mut prop, default_prop.iter().cloned());
    push_all(&mut prop, fallback_order(TextLang::Ja).into_iter().filter_map(reg_key));
    // Legacy bold family: bold faces first (Latin, Japanese, the other CJK bold faces so a
    // glyph missing from the Japanese font stays bold), then the regular chain.
    let mut bold_chain = Vec::new();
    push_all(&mut bold_chain, [latin_bold_key.clone(), bold_key(TextLang::Ja)].into_iter().flatten());
    push_all(&mut bold_chain, fallback_order(TextLang::Ja).into_iter().filter_map(bold_key));
    push_all(&mut bold_chain, prop.iter().cloned());

    for lang in TextLang::CJK {
        let mut regular = Vec::new();
        push_all(&mut regular, latin_reg_key.clone());
        push_all(&mut regular, reg_key(lang));
        push_all(&mut regular, fallback_order(lang).into_iter().filter_map(reg_key));
        push_all(&mut regular, default_prop.iter().cloned());
        let mut bold = Vec::new();
        push_all(&mut bold, latin_bold_key.clone());
        push_all(&mut bold, bold_key(lang));
        push_all(&mut bold, fallback_order(lang).into_iter().filter_map(bold_key));
        push_all(&mut bold, regular.iter().cloned());
        if let (Some(r), Some(b)) = (lang.family_name(false), lang.family_name(true)) {
            defs.families.insert(FontFamily::Name(r.into()), regular);
            defs.families.insert(FontFamily::Name(b.into()), bold);
        }
    }
    defs.families.insert(FontFamily::Proportional, prop);
    defs.families.insert(FontFamily::Name(BOLD_FAMILY.into()), bold_chain);

    // Monospace: keep egui's Hack first, Japanese as fallback.
    if let Some(jp) = reg_key(TextLang::Ja) {
        let mono = defs.families.entry(FontFamily::Monospace).or_default();
        if !mono.contains(&jp) {
            mono.push(jp);
        }
    }
    (defs, report)
}

/// Install prepared definitions and record that the bold family exists. Also registers the
/// per-pass tracker that lets [`super::text`] use the per-language families once they are live,
/// and tells language detection which Japanese font is installed.
pub fn install_definitions(ctx: &Context, defs: FontDefinitions) {
    let bold = defs
        .families
        .contains_key(&FontFamily::Name(BOLD_FAMILY.into()));
    text::set_japanese_face(japanese_face(&defs));
    ctx.set_fonts(defs);
    text::register(ctx);
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(installed_id(), FontsInstalled { bold, pass }));
}

/// Build and install in one go. The bold family becomes usable on the next frame; call
/// [`super::theme::sync`] every frame so the strong text styles switch to it.
pub fn install(ctx: &Context, extra_dirs: &[&Path]) -> FontReport {
    let (defs, report) = build_definitions(extra_dirs);
    install_definitions(ctx, defs);
    report
}

/// The Japanese face of the `ui-ja` family (first font with kana), if its data is `'static`.
fn japanese_face(defs: &FontDefinitions) -> Option<(&'static [u8], u32)> {
    let family = defs.families.get(&FontFamily::Name(text::FAMILY_JA.into()))?;
    family.iter().find_map(|key| {
        let data = defs.font_data.get(key)?;
        let Cow::Borrowed(bytes) = data.font else { return None };
        let font = skrifa::FontRef::from_index(bytes, data.index).ok()?;
        font.charmap().map('あ').is_some().then_some((bytes, data.index))
    })
}

fn push_all(out: &mut Vec<String>, items: impl IntoIterator<Item = String>) {
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
}

/// Candidate names for `platform` first, then the other platforms' names.
fn ordered_names(c: &CjkCandidates, platform: FontPlatform) -> Vec<&'static str> {
    let lists = match platform {
        FontPlatform::Windows => [c.windows, c.macos, c.linux],
        FontPlatform::MacOs => [c.macos, c.windows, c.linux],
        FontPlatform::Linux => [c.linux, c.windows, c.macos],
    };
    let mut out: Vec<&'static str> = Vec::new();
    for name in lists.into_iter().flatten() {
        if !out.contains(name) {
            out.push(name);
        }
    }
    out
}

fn warn_missing_once(lang: TextLang) {
    static WARNED: AtomicU8 = AtomicU8::new(0);
    let bit = 1u8 << (lang as u8);
    if WARNED.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        tracing::warn!("no {lang:?} font found; text in that language falls back to the other CJK fonts");
    }
}

// ---------------------------------------------------------------------------------------
// Face lookup and shared data
// ---------------------------------------------------------------------------------------

/// First face (in `names` order) of the wanted weight class that contains `probe`.
fn pick(db: &mut Database, names: &[&str], bold: bool, probe: Option<char>) -> Option<Face> {
    for name in names {
        let Some((id, wght)) = find_face(db, name, bold) else { continue };
        if let Some(probe) = probe {
            let has = db.with_face_data(id, |data, index| {
                skrifa::FontRef::from_index(data, index).is_ok_and(|f| f.charmap().map(probe).is_some())
            });
            if has != Some(true) {
                continue;
            }
        }
        let face = db.face(id)?;
        let weight = wght.map_or(face.weight.0, |w| w as u16);
        let key = format!("{}#{}#{}", face.post_script_name, face.index, weight);
        let (bytes, index) = shared_bytes(db, id)?;
        return Some(Face { key, bytes, index, wght });
    }
    None
}

fn weight_ok(weight: Weight, bold: bool) -> bool {
    if bold {
        weight.0 >= 600
    } else {
        (300..600).contains(&weight.0)
    }
}

/// Range of the `wght` axis if the face is a variable font.
fn wght_axis(db: &Database, id: ID) -> Option<(f32, f32)> {
    db.with_face_data(id, |data, index| {
        let font = skrifa::FontRef::from_index(data, index).ok()?;
        let axis = font.axes().iter().find(|a| a.tag() == skrifa::Tag::new(b"wght"))?;
        Some((axis.min_value(), axis.max_value()))
    })?
}

/// Exact family match via fontdb (CSS-style weight matching), else faces whose family is
/// `name` plus a weight suffix ("PingFang SC Semibold", "Hiragino Sans W6", ...). Variable
/// fonts match either weight; the returned `wght` value is then pinned via `FontTweak::coords`.
fn find_face(db: &Database, name: &str, bold: bool) -> Option<(ID, Option<f32>)> {
    let weight = if bold { Weight::BOLD } else { Weight::NORMAL };
    if let Some(id) = db.query(&Query {
        families: &[Family::Name(name)],
        weight,
        stretch: Stretch::Normal,
        style: Style::Normal,
    }) {
        // fontdb picks the closest weight; reject the wrong weight class so the bold chain
        // does not just duplicate the regular font (and vice versa).
        if db.face(id).is_some_and(|f| weight_ok(f.weight, bold)) {
            return Some((id, None));
        }
        if let Some((lo, hi)) = wght_axis(db, id) {
            let want = f32::from(weight.0).clamp(lo, hi);
            if weight_ok(Weight(want as u16), bold) {
                return Some((id, Some(want)));
            }
        }
    }
    let prefix = format!("{name} ");
    db.faces()
        .iter()
        .filter(|f| {
            f.style == Style::Normal
                && weight_ok(f.weight, bold)
                && f.family.strip_prefix(&prefix).is_some_and(|rest| !rest.is_empty() && !rest.contains(' '))
        })
        .min_by_key(|f| (i32::from(f.weight.0) - i32::from(weight.0)).abs())
        .map(|f| (f.id, None))
}

/// The bytes of the file backing `id`, memory-mapped once per file for the process lifetime
/// and shared by all faces / families / font builds using it. Falls back to a private copy.
fn shared_bytes(db: &mut Database, id: ID) -> Option<(&'static [u8], u32)> {
    static MAPPED: OnceLock<Mutex<HashMap<PathBuf, &'static [u8]>>> = OnceLock::new();
    let (source, index) = db.face_source(id)?;
    let path = match &source {
        Source::File(p) | Source::SharedFile(p, _) => Some(p.clone()),
        Source::Binary(_) => None,
    };
    let mapped = MAPPED.get_or_init(Default::default);
    if let Some(path) = &path {
        if let Some(bytes) = mapped.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
            return Some((bytes, index));
        }
        // SAFETY: maps the font file read-only. Like fontdb's own scanning, this assumes the
        // file is not truncated or rewritten in place while mapped (system fonts are replaced
        // by rename, which keeps the old mapping valid). The mapping is leaked on purpose: egui
        // needs `'static` bytes to avoid copying, and fonts live as long as the app.
        if let Some((data, _)) = unsafe { db.make_shared_face_data(id) } {
            let data: &'static Arc<dyn AsRef<[u8]> + Send + Sync> = Box::leak(Box::new(data));
            let bytes: &'static [u8] = (**data).as_ref();
            mapped.lock().unwrap_or_else(|e| e.into_inner()).insert(path.clone(), bytes);
            return Some((bytes, index));
        }
    }
    let copy = db.with_face_data(id, |data, _| data.to_vec())?;
    Some((Box::leak(copy.into_boxed_slice()), index))
}

// ---------------------------------------------------------------------------------------
// Vertical normalization
// ---------------------------------------------------------------------------------------

/// A glyph's vertical extent `(y_min, y_max)` in em units (y up, baseline 0).
type YRange = (f32, f32);

/// Vertical metrics in em units (y up, baseline 0), as epaint reads them.
#[derive(Clone, Copy, Debug)]
struct VMetrics {
    ascent: f32,
    /// ascent - descent + line gap (egui's row height).
    height: f32,
    /// Per [`REF_CHARS`] entry: (y_min, y_max) of the glyph, if present.
    ideo: [Option<YRange>; 3],
}

impl VMetrics {
    fn of(bytes: &[u8], index: u32) -> Option<Self> {
        let font = skrifa::FontRef::from_index(bytes, index).ok()?;
        let m = font.metrics(Size::unscaled(), LocationRef::default());
        let upm = f32::from(m.units_per_em.max(1));
        let glyphs = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        let charmap = font.charmap();
        let ideo = REF_CHARS.map(|c| {
            let b = glyphs.bounds(charmap.map(c)?)?;
            Some((b.y_min / upm, b.y_max / upm))
        });
        Some(Self { ascent: m.ascent / upm, height: (m.ascent - m.descent + m.leading) / upm, ideo })
    }

    /// Average ideograph box over the reference chars present in both fonts.
    fn common_box(&self, other: &Self) -> Option<(YRange, YRange)> {
        let pairs: Vec<_> = self.ideo.iter().zip(&other.ideo).filter_map(|(a, b)| Some(((*a)?, (*b)?))).collect();
        if pairs.is_empty() {
            return None;
        }
        let n = pairs.len() as f32;
        let avg = |f: fn(&(YRange, YRange)) -> f32| pairs.iter().map(f).sum::<f32>() / n;
        Some(((avg(|p| p.0 .0), avg(|p| p.0 .1)), (avg(|p| p.1 .0), avg(|p| p.1 .1))))
    }
}

/// Tweak for a CJK face `m` in a family headed by `head` (the Latin font), so that its
/// ideographs match `reference` (the Japanese face, itself placed on the Latin baseline) in
/// height and vertical centre. Without common reference glyphs it is just baseline-aligned.
///
/// epaint places a fallback glyph's baseline at `a + (H - h) / 2 + y_offset` below the row top
/// (`a`, `h`: the face's scaled ascent / row height; `H`: the head font's row height) while the
/// head font's baseline is at `A`; `scale` multiplies the face's em size including `a` and `h`.
fn normalize(head: &VMetrics, reference: &VMetrics, m: &VMetrics) -> FontTweak {
    let (scale, ref_center, own_center) = match reference.common_box(m) {
        Some(((r0, r1), (m0, m1))) if m1 - m0 > 0.1 => {
            let k = ((r1 - r0) / (m1 - m0)).clamp(0.85, 1.15);
            (k, (r0 + r1) / 2.0, (m0 + m1) / 2.0)
        }
        _ => (1.0, 0.0, 0.0),
    };
    let a = scale * m.ascent;
    let h = scale * m.height;
    // Ideograph centre (from row top) = baseline - scale * own_center; want head.ascent - ref_center.
    let offset_em = head.ascent - ref_center - a - 0.5 * (head.height - h) + scale * own_center;
    FontTweak { scale, y_offset_factor: offset_em / scale, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_always_have_bold_and_language_families() {
        let (defs, _report) = build_definitions(&[]);
        assert!(defs
            .families
            .contains_key(&FontFamily::Name(BOLD_FAMILY.into())));
        assert!(!defs.families[&FontFamily::Proportional].is_empty());
        for lang in TextLang::CJK {
            for bold in [false, true] {
                let fam = FontFamily::Name(lang.family_name(bold).unwrap().into());
                let keys = &defs.families[&fam];
                assert!(!keys.is_empty());
                assert!(keys.iter().all(|k| defs.font_data.contains_key(k)), "{fam:?}");
            }
        }
    }

    #[test]
    fn normalize_identity_and_baseline() {
        let segoe = VMetrics { ascent: 1.079, height: 1.330, ideo: [None; 3] };
        let yu = VMetrics { ascent: 1.079, height: 1.330, ideo: [Some((-0.04, 0.765)), Some((-0.064, 0.828)), Some((-0.05, 0.8))] };
        // Yu Gothic UI shares Segoe UI's metrics: no change.
        let t = normalize(&segoe, &yu, &yu);
        assert!((t.scale - 1.0).abs() < 1e-6 && t.y_offset_factor.abs() < 1e-6, "{t:?}");
        // A face with larger, lower ideographs is shrunk and moved so the boxes coincide.
        let big = VMetrics { ascent: 1.016, height: 1.270, ideo: [Some((-0.12, 0.767)), Some((-0.122, 0.836)), Some((-0.12, 0.8))] };
        let t = normalize(&segoe, &yu, &big);
        assert!(t.scale < 0.95);
        let k = t.scale;
        let baseline = k * big.ascent + 0.5 * (segoe.height - k * big.height) + t.y_offset_factor * k;
        let center = baseline - k * (-0.1207 + 0.801) / 2.0;
        let want = segoe.ascent - (-0.0513 + 0.7977) / 2.0;
        assert!((center - want).abs() < 0.01, "{center} vs {want}");
    }

    #[test]
    fn candidate_order_prefers_current_platform() {
        for (platform, first) in [
            (FontPlatform::Windows, "Microsoft YaHei UI"),
            (FontPlatform::MacOs, "PingFang SC"),
            (FontPlatform::Linux, "Noto Sans CJK SC"),
        ] {
            let names = ordered_names(&CJK_CANDIDATES[1], platform);
            assert_eq!(names[0], first);
            assert!(names.contains(&"Droid Sans Fallback"));
        }
    }
}
