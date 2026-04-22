//! フォント設定を担当
//! - CJKフォールバックフォントの検出と設定

use eframe::CreationContext;
use eframe::egui;
use fontdb::{Database, Family, Source, Style};

use crate::presentation::app::GuiApp;

impl GuiApp {
    pub(crate) fn setup_fonts(cc: &CreationContext<'_>) {
        let mut fonts = egui::FontDefinitions::default();
        let fallback_fonts = Self::discover_cjk_fallback_fonts();

        if !fallback_fonts.is_empty() {
            for (name, bytes) in &fallback_fonts {
                fonts
                    .font_data
                    .insert(name.clone(), egui::FontData::from_owned(bytes.clone()));
            }

            let family = fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default();
            for (name, _) in fallback_fonts.iter().rev() {
                if !family.contains(name) {
                    family.insert(0, name.clone());
                }
            }

            let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
            for (name, _) in fallback_fonts.iter().rev() {
                if !monospace.contains(name) {
                    monospace.insert(0, name.clone());
                }
            }
        }

        cc.egui_ctx.set_fonts(fonts);
    }

    pub(crate) fn discover_cjk_fallback_fonts() -> Vec<(String, Vec<u8>)> {
        let mut database = Database::new();
        database.load_system_fonts();

        let candidates = [
            "Noto Sans CJK SC",
            "Source Han Sans SC",
            "Microsoft YaHei",
            "PingFang SC",
            "Noto Sans CJK TC",
            "Source Han Sans TC",
            "Microsoft JhengHei",
            "Noto Sans CJK JP",
            "Source Han Sans JP",
            "Noto Sans CJK KR",
            "Source Han Sans KR",
            "Noto Sans JP",
            "Noto Sans SC",
            "Noto Sans TC",
            "IPAPGothic",
            "WenQuanYi Zen Hei",
            "Hiragino Sans",
        ];

        let mut loaded = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for family_name in &candidates {
            let query = fontdb::Query {
                families: &[Family::Name(family_name)],
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: Style::Normal,
            };

            if let Some(face_id) = database.query(&query) {
                if let Some(face) = database.face(face_id) {
                    let font_bytes = match &face.source {
                        Source::File(path) => std::fs::read(path).ok(),
                        Source::Binary(data) => Some(data.as_ref().as_ref().to_vec()),
                        _ => None,
                    };

                    if let Some(bytes) = font_bytes {
                        let key = family_name.replace(' ', "_");
                        if seen.insert(key.clone()) {
                            loaded.push((key, bytes));
                        }
                    }
                }
            }
        }

        loaded
    }
}