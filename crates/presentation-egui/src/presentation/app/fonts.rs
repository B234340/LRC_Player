//! フォント設定を担当
//! - CJK フォールバックフォントの検出と設定
//!
//! egui's built-in fonts stay first so Latin text and icons render with them; at most two
//! system CJK fonts are appended as *fallbacks*: a Japanese face first (so kanji use Japanese
//! glyph forms), plus one Chinese face only when the Japanese font is not a pan-CJK family.
//! The font-collection face index (`.ttc`) is honoured, and font bytes are moved into egui
//! without extra copies.

use eframe::egui;
use eframe::CreationContext;
use fontdb::{Database, Family, Source, Style};
use std::fs;
use std::sync::Arc;

use crate::presentation::app::GuiApp;

/// Japanese-capable families, most preferred first.
const JAPANESE_CANDIDATES: &[&str] = &[
    "Noto Sans CJK JP",
    "Noto Sans JP",
    "Source Han Sans JP",
    "Source Han Sans",
    "Yu Gothic UI",
    "Yu Gothic",
    "Meiryo UI",
    "Meiryo",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "BIZ UDPGothic",
    "IPAexGothic",
    "IPAPGothic",
    "MS UI Gothic",
];

/// Second fallback for Chinese-only / Korean characters (skipped when the Japanese font is a
/// pan-CJK family that already covers them).
const OTHER_CJK_CANDIDATES: &[&str] = &[
    "Noto Sans CJK SC",
    "Source Han Sans SC",
    "Microsoft YaHei UI",
    "Microsoft YaHei",
    "PingFang SC",
    "Noto Sans SC",
    "WenQuanYi Zen Hei",
    "Malgun Gothic",
    "Apple SD Gothic Neo",
];

/// A discovered system font face.
pub(crate) struct DiscoveredFont {
    pub(crate) key: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) index: u32,
}

fn is_pan_cjk(family: &str) -> bool {
    family.starts_with("Noto Sans CJK") || family.starts_with("Source Han Sans")
}

fn load_face(database: &Database, family: &str) -> Option<DiscoveredFont> {
    let query = fontdb::Query {
        families: &[Family::Name(family)],
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        style: Style::Normal,
    };
    let id = database.query(&query)?;
    let face = database.face(id)?;
    let bytes = match &face.source {
        Source::File(path) => fs::read(path).ok()?,
        Source::Binary(data) => data.as_ref().as_ref().to_vec(),
        #[allow(unreachable_patterns)]
        _ => return None,
    };
    Some(DiscoveredFont {
        key: family.replace(' ', "_"),
        bytes,
        index: face.index,
    })
}

/// Inserts `fonts` as fallbacks after the existing families (egui's defaults stay first).
pub(crate) fn install_fallback_fonts(definitions: &mut egui::FontDefinitions, fonts: Vec<DiscoveredFont>) {
    for font in fonts {
        let mut data = egui::FontData::from_owned(font.bytes);
        data.index = font.index;
        definitions.font_data.insert(font.key.clone(), Arc::new(data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let list = definitions.families.entry(family).or_default();
            if !list.contains(&font.key) {
                list.push(font.key.clone());
            }
        }
    }
}

impl GuiApp {
    /// アプリケーションのフォントをセットアップします（CJK はフォールバックとして末尾に追加）。
    pub(crate) fn setup_fonts(cc: &CreationContext<'_>) {
        let mut fonts = egui::FontDefinitions::default();
        let fallback = Self::discover_cjk_fallback_fonts();
        if !fallback.is_empty() {
            tracing::info!(
                "CJK fallback fonts: {}",
                fallback.iter().map(|f| f.key.as_str()).collect::<Vec<_>>().join(", ")
            );
            install_fallback_fonts(&mut fonts, fallback);
        }
        cc.egui_ctx.set_fonts(fonts);
    }

    /// Finds at most two CJK fallback faces (Japanese first).
    pub(crate) fn discover_cjk_fallback_fonts() -> Vec<DiscoveredFont> {
        let mut database = Database::new();
        database.load_system_fonts();

        let mut loaded = Vec::new();
        let mut japanese_is_pan_cjk = false;
        for family in JAPANESE_CANDIDATES {
            if let Some(font) = load_face(&database, family) {
                japanese_is_pan_cjk = is_pan_cjk(family);
                loaded.push(font);
                break;
            }
        }
        if !japanese_is_pan_cjk {
            for family in OTHER_CJK_CANDIDATES {
                if let Some(font) = load_face(&database, family) {
                    loaded.push(font);
                    break;
                }
            }
        }
        loaded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallbacks_are_appended_after_default_fonts_with_face_index() {
        let mut defs = egui::FontDefinitions::default();
        let first_before = defs.families[&egui::FontFamily::Proportional][0].clone();
        install_fallback_fonts(
            &mut defs,
            vec![DiscoveredFont {
                key: "Test_CJK".into(),
                bytes: vec![0u8; 4],
                index: 2,
            }],
        );
        let prop = &defs.families[&egui::FontFamily::Proportional];
        assert_eq!(prop[0], first_before);
        assert_eq!(prop.last().map(String::as_str), Some("Test_CJK"));
        assert_eq!(defs.font_data["Test_CJK"].index, 2);
        let mono = &defs.families[&egui::FontFamily::Monospace];
        assert_eq!(mono.last().map(String::as_str), Some("Test_CJK"));
    }
}
