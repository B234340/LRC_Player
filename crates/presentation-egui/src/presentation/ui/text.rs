//! Per-element CJK font selection.
//!
//! Han characters are shared by Japanese and Chinese but drawn differently (直, 骨, 写 …), and a
//! Japanese font lacks many Chinese characters. Mixing fonts inside one element looks broken, so
//! every *element* (label, title, artist, lyric line, …) is drawn entirely in the font of its
//! detected language: [`lang_of`] classifies a string, [`family_for`] maps it to one of the font
//! families registered by [`super::fonts::build_definitions`].
//!
//! # Detection rules ([`lang_of`])
//! 1. Any hiragana / katakana (incl. half-width kana) → [`TextLang::Ja`].
//! 2. Otherwise any Hangul → [`TextLang::Ko`].
//! 3. Otherwise, without Han characters → [`TextLang::Latin`] (UI default font).
//! 4. Han text (plus Latin, digits, punctuation) is Japanese unless at least one Han character
//!    is outside the Japanese repertoire: not Shift_JIS-encodable (≈ JIS X 0208 + the NEC/IBM
//!    extensions, via `encoding_rs`) and not in a small JIS X 0213 allowlist, or one of the
//!    JIS characters modern Japanese does not use (們 這 麼 裡 沒 嘛), or missing from the
//!    installed Japanese font, or two or more traditional forms modern Japanese replaced
//!    (會 點 邊 戀 …, see `TRADITIONAL_MARKERS`) without any Japan-only shinjitai (駆 歩 …).
//!    Then it is Chinese:
//!    - each Han character in GB2312 but not in Big5 counts as Simplified evidence, each one in
//!      Big5 (standard hanzi block) but not in GB2312 as Traditional evidence (GB2312 is the PRC
//!      simplified-character standard, Big5 the Taiwan traditional one; characters in both or
//!      neither are neutral);
//!    - more Traditional than Simplified evidence → [`TextLang::ZhHant`], otherwise (including
//!      ties / no evidence) → [`TextLang::ZhHans`].
//!
//! Results for Han strings are cached in a small per-thread cache (pure-ASCII, kana and Hangul
//! strings are decided by one scan without caching).
//!
//! # Rendering
//! Lists / documents that share a language (a song's lyric lines, an album's track titles) can
//! pass a hint: [`dominant_lang`] + [`lang_in_context`] / [`family_in_context`] make ambiguous
//! all-JIS Han lines follow a Chinese context instead of defaulting to Japanese.
//!
//! Use [`rich`] for widgets, [`job`] / [`append`] / [`format`] for [`LayoutJob`]s, or
//! [`font_id`] / [`family_for`] for anything else. Before the CJK fonts are live (font worker
//! still running, or a context without them) the helpers fall back to the default
//! `Proportional` / bold families, so they never reference a missing family. Custom galley
//! caches should include [`epoch`] in their keys (the family of a string is a pure function of
//! the string and the epoch).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock};

use eframe::egui::{self, text::LayoutJob, Color32, FontFamily, FontId, Galley, RichText, TextFormat, Ui};
use encoding_rs::{EncoderResult, BIG5, GBK, SHIFT_JIS};
use skrifa::MetadataProvider;

use super::theme::BOLD_FAMILY;

// ---------------------------------------------------------------------------------------
// Languages and families
// ---------------------------------------------------------------------------------------

/// Script / language of a UI string, as far as font choice is concerned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextLang {
    /// No CJK: the default UI families (`Proportional` / bold).
    #[default]
    Latin,
    Ja,
    ZhHans,
    ZhHant,
    Ko,
}

/// Font family names registered by [`super::fonts::build_definitions`].
pub const FAMILY_JA: &str = "ui-ja";
pub const FAMILY_JA_BOLD: &str = "ui-ja-bold";
pub const FAMILY_ZH_HANS: &str = "ui-zh-hans";
pub const FAMILY_ZH_HANS_BOLD: &str = "ui-zh-hans-bold";
pub const FAMILY_ZH_HANT: &str = "ui-zh-hant";
pub const FAMILY_ZH_HANT_BOLD: &str = "ui-zh-hant-bold";
pub const FAMILY_KO: &str = "ui-ko";
pub const FAMILY_KO_BOLD: &str = "ui-ko-bold";

impl TextLang {
    /// The languages with their own font family.
    pub const CJK: [TextLang; 4] = [TextLang::Ja, TextLang::ZhHans, TextLang::ZhHant, TextLang::Ko];

    pub const fn is_cjk(self) -> bool {
        !matches!(self, TextLang::Latin)
    }

    /// Name of this language's font family (`None` for [`TextLang::Latin`]).
    pub const fn family_name(self, bold: bool) -> Option<&'static str> {
        Some(match (self, bold) {
            (TextLang::Latin, _) => return None,
            (TextLang::Ja, false) => FAMILY_JA,
            (TextLang::Ja, true) => FAMILY_JA_BOLD,
            (TextLang::ZhHans, false) => FAMILY_ZH_HANS,
            (TextLang::ZhHans, true) => FAMILY_ZH_HANS_BOLD,
            (TextLang::ZhHant, false) => FAMILY_ZH_HANT,
            (TextLang::ZhHant, true) => FAMILY_ZH_HANT_BOLD,
            (TextLang::Ko, false) => FAMILY_KO,
            (TextLang::Ko, true) => FAMILY_KO_BOLD,
        })
    }

    fn slot(self, bold: bool) -> usize {
        let base = match self {
            TextLang::Latin => return 8,
            TextLang::Ja => 0,
            TextLang::ZhHans => 2,
            TextLang::ZhHant => 4,
            TextLang::Ko => 6,
        };
        base + usize::from(bold)
    }
}

/// Interned families (cloning a `FontFamily::Name` is an `Arc` bump, not an allocation).
static FAMILIES: LazyLock<[FontFamily; 9]> = LazyLock::new(|| {
    [
        FAMILY_JA,
        FAMILY_JA_BOLD,
        FAMILY_ZH_HANS,
        FAMILY_ZH_HANS_BOLD,
        FAMILY_ZH_HANT,
        FAMILY_ZH_HANT_BOLD,
        FAMILY_KO,
        FAMILY_KO_BOLD,
        BOLD_FAMILY,
    ]
    .map(|n| FontFamily::Name(n.into()))
});

// ---------------------------------------------------------------------------------------
// Which families are live (per pass)
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PassFonts {
    /// All `ui-*` families exist in the fonts of the pass being run on this thread.
    cjk: bool,
    /// The legacy bold family exists.
    bold: bool,
}

thread_local! {
    static PASS: Cell<PassFonts> = const { Cell::new(PassFonts { cjk: false, bold: false }) };
    static PASS_STACK: RefCell<Vec<PassFonts>> = const { RefCell::new(Vec::new()) };
}

/// egui plugin that records, at the start of each pass, which of our families the context's
/// *current* fonts contain (fonts change between passes, and a missing family panics in egui),
/// and restores the previous state at the end (nested immediate viewports, several contexts on
/// one thread).
struct FontsPlugin;

impl egui::plugin::Plugin for FontsPlugin {
    fn debug_name(&self) -> &'static str {
        "lrc_ui_text_fonts"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let state = ui.ctx().fonts(|f| {
            let fams = &f.definitions().families;
            PassFonts {
                cjk: FAMILIES[..8].iter().all(|fam| fams.contains_key(fam)),
                bold: fams.contains_key(&FAMILIES[8]),
            }
        });
        PASS_STACK.with(|s| s.borrow_mut().push(PASS.get()));
        PASS.set(state);
    }

    fn on_end_pass(&mut self, _ui: &mut Ui) {
        let prev = PASS_STACK.with(|s| s.borrow_mut().pop()).unwrap_or_default();
        PASS.set(prev);
    }
}

/// Register the pass tracker on `ctx` (idempotent; called by `fonts::install_definitions`).
pub(crate) fn register(ctx: &egui::Context) {
    ctx.add_plugin(FontsPlugin);
}

/// Whether the per-language families are usable in the pass currently running on this thread.
pub fn active() -> bool {
    PASS.get().cjk
}

/// Whether the bold family is usable in the pass currently running on this thread.
pub fn bold_active() -> bool {
    PASS.get().bold
}

/// Changes whenever the string → family mapping may change (fonts became live / were replaced,
/// Japanese font coverage changed). Include it in custom galley-cache keys.
pub fn epoch() -> u64 {
    GENERATION.load(Ordering::Relaxed) << 2 | u64::from(PASS.get().cjk) << 1 | u64::from(PASS.get().bold)
}

// ---------------------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------------------

/// Font family for a language (falls back to the default families while CJK fonts are not live).
pub fn family_for_lang(lang: TextLang, bold: bool) -> FontFamily {
    let pass = PASS.get();
    if lang.is_cjk() && pass.cjk {
        FAMILIES[lang.slot(bold)].clone()
    } else if bold && pass.bold {
        FAMILIES[8].clone()
    } else {
        FontFamily::Proportional
    }
}

/// Font family for a whole element, decided from its text (see the module docs).
pub fn family_for(text: &str, bold: bool) -> FontFamily {
    family_for_lang(lang_of(text), bold)
}

/// `FontId` for an element.
pub fn font_id(text: &str, size: f32, bold: bool) -> FontId {
    FontId::new(size, family_for(text, bold))
}

/// `RichText` drawn entirely in the font of its language.
pub fn rich(text: impl Into<String>, size: f32, bold: bool) -> RichText {
    let text = text.into();
    let font = font_id(&text, size, bold);
    RichText::new(text).font(font)
}

/// `TextFormat` for an element (for building multi-section [`LayoutJob`]s by hand).
pub fn format(text: &str, size: f32, bold: bool, color: Color32) -> TextFormat {
    TextFormat::simple(font_id(text, size, bold), color)
}

/// A single-element [`LayoutJob`] (like `LayoutJob::simple`, with the element's family).
/// Use `wrap_width = f32::INFINITY` for no wrapping.
pub fn job(text: impl Into<String>, size: f32, bold: bool, color: Color32, wrap_width: f32) -> LayoutJob {
    let text = text.into();
    let font = font_id(&text, size, bold);
    LayoutJob::simple(text, font, color, wrap_width)
}

/// Append one element to a [`LayoutJob`]; its family is decided from `text` alone, so e.g.
/// "title · artist" can mix a Chinese title with a Japanese artist without mixing glyph forms
/// inside either.
pub fn append(job: &mut LayoutJob, text: &str, size: f32, bold: bool, color: Color32) {
    job.append(text, 0.0, format(text, size, bold, color));
}

/// Lay out a single element (egui caches the galley; the key includes the family).
pub fn galley(ui: &Ui, text: impl Into<String>, size: f32, bold: bool, color: Color32, wrap_width: f32) -> Arc<Galley> {
    let job = job(text, size, bold, color, wrap_width);
    ui.fonts_mut(|f| f.layout_job(job))
}

/// Like [`family_for`], but ambiguous Han-only text (Japanese by default) follows `context`
/// when that is Chinese — see [`lang_in_context`].
pub fn family_in_context(text: &str, context: TextLang, bold: bool) -> FontFamily {
    family_for_lang(lang_in_context(text, context), bold)
}

// ---------------------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------------------

/// [`lang_of`] with a document-level hint: a Han-only string that is Japanese only by default
/// (no kana, every character in the Japanese repertoire, e.g. 夜里的小白船) takes `context` when
/// `context` is Chinese, so a Chinese song's lyrics or a Chinese album's track list do not
/// switch to Japanese glyph forms on such lines. Strings with kana / Hangul / non-JIS hanzi are
/// unaffected.
pub fn lang_in_context(text: &str, context: TextLang) -> TextLang {
    let lang = lang_of(text);
    if lang == TextLang::Ja && matches!(context, TextLang::ZhHans | TextLang::ZhHant) && !text.chars().any(is_kana) {
        context
    } else {
        lang
    }
}

/// Dominant language of a set of strings (lyric lines, an album's titles), for
/// [`lang_in_context`]: Chinese when more strings are unambiguously Chinese than contain kana
/// (Simplified unless Traditional strings are the majority of those), else Korean when Korean
/// strings outnumber kana ones, else Japanese if any string is CJK, else Latin.
pub fn dominant_lang<'a>(texts: impl IntoIterator<Item = &'a str>) -> TextLang {
    let (mut hans, mut hant, mut kana, mut ko, mut cjk) = (0usize, 0usize, 0usize, 0usize, false);
    for t in texts {
        match lang_of(t) {
            TextLang::ZhHans => hans += 1,
            TextLang::ZhHant => hant += 1,
            TextLang::Ko => ko += 1,
            TextLang::Ja => kana += usize::from(t.chars().any(is_kana)),
            TextLang::Latin => continue,
        }
        cjk = true;
    }
    if hans + hant > kana && hans + hant >= ko {
        if hant > hans {
            TextLang::ZhHant
        } else {
            TextLang::ZhHans
        }
    } else if ko > kana {
        TextLang::Ko
    } else if cjk {
        TextLang::Ja
    } else {
        TextLang::Latin
    }
}

/// Bumped when the Japanese font (and therefore its coverage) changes; clears the caches.
static GENERATION: AtomicU64 = AtomicU64::new(0);

struct JaFace {
    id: (usize, usize, u32),
    font: skrifa::FontRef<'static>,
}

static JA_FACE: RwLock<Option<JaFace>> = RwLock::new(None);

/// Tell detection which Japanese font is installed, so Han characters it lacks count as
/// non-Japanese. Called by `fonts::install_definitions`.
pub(crate) fn set_japanese_face(face: Option<(&'static [u8], u32)>) {
    let id = face.map(|(b, i)| (b.as_ptr() as usize, b.len(), i));
    let mut guard = JA_FACE.write().unwrap_or_else(|e| e.into_inner());
    if guard.as_ref().map(|f| f.id) == id {
        return;
    }
    *guard = face.and_then(|(bytes, index)| {
        let font = skrifa::FontRef::from_index(bytes, index).ok()?;
        Some(JaFace { id: id?, font })
    });
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

const CACHE_CAP: usize = 2048;

struct LangCache {
    generation: u64,
    map: HashMap<Box<str>, TextLang>,
}

thread_local! {
    static CACHE: RefCell<LangCache> = RefCell::new(LangCache { generation: 0, map: HashMap::new() });
}

/// Language of a string (see the module docs for the rules). Cheap; Han strings are cached.
pub fn lang_of(text: &str) -> TextLang {
    let mut has_han = false;
    let mut has_hangul = false;
    for c in text.chars() {
        if (c as u32) < 0x1100 {
            continue;
        }
        if is_kana(c) {
            return TextLang::Ja;
        }
        has_hangul |= is_hangul(c);
        has_han |= is_han(c);
    }
    if has_hangul {
        return TextLang::Ko;
    }
    if !has_han {
        return TextLang::Latin;
    }
    let generation = GENERATION.load(Ordering::Relaxed);
    if let Some(hit) = CACHE.with(|c| {
        let c = c.borrow();
        (c.generation == generation).then(|| c.map.get(text).copied()).flatten()
    }) {
        return hit;
    }
    let lang = classify_han(text);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.generation != generation || c.map.len() >= CACHE_CAP {
            c.map.clear();
            c.generation = generation;
        }
        c.map.insert(text.into(), lang);
    });
    lang
}

fn classify_han(text: &str) -> TextLang {
    let ja_face = JA_FACE.read().unwrap_or_else(|e| e.into_inner());
    let mut chinese = false;
    let (mut hans, mut hant) = (0u32, 0u32);
    let mut kyujitai = 0u32;
    let mut japan_only = false;
    for c in text.chars().filter(|&c| is_han(c)) {
        let sjis = in_japanese_repertoire(c);
        if !chinese {
            let in_font = ja_face.as_ref().is_none_or(|f| f.font.charmap().map(c).is_some());
            chinese = !in_font || !sjis;
        }
        let (gb, big5) = (in_gb2312(c), in_big5(c));
        match (gb, big5) {
            (true, false) => hans += 1,
            (false, true) => hant += 1,
            (false, false) if sjis => japan_only = true,
            _ => {}
        }
        kyujitai += u32::from(TRADITIONAL_MARKERS.contains(&c));
    }
    // All-JIS Traditional Chinese (還要走多遠 才會看見海邊): several traditional forms that modern
    // Japanese replaced, and no Japan-only shinjitai (駆, 歩, 険 …) to say otherwise.
    chinese |= kyujitai >= 2 && !japan_only;
    if !chinese {
        TextLang::Ja
    } else if hant > hans {
        TextLang::ZhHant
    } else {
        TextLang::ZhHans
    }
}

/// JIS X 0213 kanji (not in JIS X 0208) common in Japanese text: the 2010 jōyō forms 𠮟 塡 剝 頰
/// and frequent name / literary kanji.
const JIS_X_0213_ALLOW: &[char] = &['𠮟', '塡', '剝', '頰', '𠮷', '鷗', '俱', '噓', '瘦', '繫', '蟬', '屛', '搔', '摑', '攢'];

/// In JIS X 0208 (Shift_JIS-encodable) but not used in modern Japanese, while being very
/// frequent in Chinese (mostly Traditional function words): 們 這 麼 裡 沒 嘛. Without this,
/// e.g. 我們的愛 would be classified as Japanese.
const CHINESE_ONLY: &[char] = &['們', '這', '麼', '裡', '沒', '嘛'];

/// Traditional forms that are in JIS X 0208 but replaced in modern Japanese (會→会, 點→点,
/// 邊→辺, 戀→恋 …) plus a few very frequent Traditional-Chinese characters (還, 為). One of them
/// alone stays Japanese (names such as 渡邊); two or more in a Han-only string mean Chinese.
const TRADITIONAL_MARKERS: &[char] = &[
    '會', '點', '邊', '學', '國', '體', '來', '當', '對', '發', '經', '實', '變', '歡', '樂', '舊', '聽',
    '圖', '廣', '爲', '與', '還', '為', '讓', '氣', '傳', '戀', '聲', '雙', '寫', '畫', '從',
];

fn in_japanese_repertoire(c: char) -> bool {
    if CHINESE_ONLY.contains(&c) {
        return false;
    }
    JIS_X_0213_ALLOW.contains(&c) || encode_char(SHIFT_JIS.new_encoder(), c).is_some()
}

/// GB2312 = GBK two-byte codes with both bytes in 0xA1..=0xFE (hanzi rows 0xB0..=0xF7).
fn in_gb2312(c: char) -> bool {
    matches!(encode_char(GBK.new_encoder(), c), Some([0xB0..=0xF7, 0xA1..=0xFE]))
}

/// Big5 standard hanzi blocks (0xA440..=0xC67E frequent, 0xC940..=0xF9D5 less frequent);
/// excludes the HKSCS / ETEN extension areas, which contain simplified forms.
fn in_big5(c: char) -> bool {
    match encode_char(BIG5.new_encoder(), c) {
        Some([lead, trail]) => {
            let code = u16::from(lead) << 8 | u16::from(trail);
            (0xA440..=0xC67E).contains(&code) || (0xC940..=0xF9D5).contains(&code)
        }
        None => false,
    }
}

/// Encode one char as a two-byte code without replacement (None if unmappable / not 2 bytes).
fn encode_char(mut enc: encoding_rs::Encoder, c: char) -> Option<[u8; 2]> {
    let mut src = [0u8; 4];
    let mut dst = [0u8; 8];
    let (res, _, written) = enc.encode_from_utf8_without_replacement(c.encode_utf8(&mut src), &mut dst, true);
    (matches!(res, EncoderResult::InputEmpty) && written == 2).then(|| [dst[0], dst[1]])
}

fn is_kana(c: char) -> bool {
    matches!(c as u32,
        0x3041..=0x309F            // hiragana
        | 0x30A0..=0x30FA | 0x30FC..=0x30FF // katakana (minus U+30FB middle dot, used in Chinese names)
        | 0x31F0..=0x31FF          // katakana phonetic extensions
        | 0xFF66..=0xFF9D          // half-width katakana
        | 0x1B000..=0x1B16F)       // kana supplement / extended-A / small kana
}

fn is_hangul(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7AF | 0xD7B0..=0xD7FF
        | 0xFFA0..=0xFFDC)
}

fn is_han(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F | 0x30000..=0x323AF)
}

#[cfg(test)]
mod screenshot_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese() {
        for s in ["群青", "夜に駆ける", "優里", "壱", "怪獣の花唄", "米津玄師", "紅蓮華", "馬と鹿", "Official髭男dism", "𠮟る", "ｱｲﾄﾞﾙ", "渡邊", "生還", "歩く會"] {
            assert_eq!(lang_of(s), TextLang::Ja, "{s}");
        }
    }

    #[test]
    fn simplified_chinese() {
        for s in ["优里", "我们的爱", "林晓风 - 晴空", "说", "爱", "们", "陈小雨 - 远行"] {
            assert_eq!(lang_of(s), TextLang::ZhHans, "{s}");
        }
    }

    #[test]
    fn traditional_chinese() {
        for s in ["我們的愛", "們", "說", "這是什麼", "林曉風 - 說好的海", "還要走多遠 才會看見海邊", "等到天亮的時候 我會再一次回來"] {
            assert_eq!(lang_of(s), TextLang::ZhHant, "{s}");
        }
    }

    #[test]
    fn korean_and_latin() {
        assert_eq!(lang_of("사랑"), TextLang::Ko);
        assert_eq!(lang_of("아이유 (IU) - 좋은 날"), TextLang::Ko);
        assert_eq!(lang_of("Lemon"), TextLang::Latin);
        assert_eq!(lang_of(""), TextLang::Latin);
        assert_eq!(lang_of("Beyoncé — Halo ♪"), TextLang::Latin);
    }

    #[test]
    fn marker_lists_are_jis() {
        let bad: String = TRADITIONAL_MARKERS
            .iter()
            .chain(CHINESE_ONLY)
            .filter(|&&c| encode_char(SHIFT_JIS.new_encoder(), c).is_none())
            .collect();
        assert!(bad.is_empty(), "not in Shift_JIS: {bad}");
    }

    #[test]
    fn cache_is_consistent_and_bounded() {
        for i in 0..(CACHE_CAP + 10) {
            let s = format!("{i}我们");
            assert_eq!(lang_of(&s), TextLang::ZhHans);
        }
        assert!(CACHE.with(|c| c.borrow().map.len()) <= CACHE_CAP);
        assert_eq!(lang_of("我們的愛"), TextLang::ZhHant);
        assert_eq!(lang_of("我們的愛"), TextLang::ZhHant);
    }

    #[test]
    fn context_hint() {
        let song = ["夜里的小白船", "慢慢开过安静的海湾", "月亮跟着我们走", ""];
        let ctx = dominant_lang(song);
        assert_eq!(ctx, TextLang::ZhHans);
        assert_eq!(lang_of("夜里的小白船"), TextLang::Ja);
        assert_eq!(lang_in_context("夜里的小白船", ctx), TextLang::ZhHans);
        // Kana, Hangul and Latin are never overridden; Japanese context changes nothing.
        assert_eq!(lang_in_context("夜に駆ける", ctx), TextLang::Ja);
        assert_eq!(lang_in_context("사랑", ctx), TextLang::Ko);
        assert_eq!(lang_in_context("Lemon", ctx), TextLang::Latin);
        let jp = ["最終電車を見送ってから歩く", "潮騒", "街灯の影が線路に伸びる夜"];
        assert_eq!(dominant_lang(jp), TextLang::Ja);
        assert_eq!(lang_in_context("潮騒", dominant_lang(jp)), TextLang::Ja);
        assert_eq!(dominant_lang(["我們的愛", "還要走多遠 才會看見海邊", "我们"]), TextLang::ZhHant);
        assert_eq!(dominant_lang(["Lemon", ""]), TextLang::Latin);
    }

    #[test]
    fn falls_back_without_live_fonts() {
        // No pass running on this thread: never hand out a family that may not exist.
        assert_eq!(family_for("我们的爱", false), FontFamily::Proportional);
        assert_eq!(family_for("Lemon", true), FontFamily::Proportional);
        assert_eq!(TextLang::ZhHant.family_name(true), Some(FAMILY_ZH_HANT_BOLD));
    }
}
