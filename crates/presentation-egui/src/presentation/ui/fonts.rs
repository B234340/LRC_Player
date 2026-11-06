//! Font installation for the redesigned UI.
//!
//! Order per family: a crisp system Latin UI font (Segoe UI / Inter / Noto Sans / ...),
//! then a Japanese font (Yu Gothic UI / Meiryo / Hiragino / Noto Sans CJK JP / ...) so that
//! kanji use Japanese glyph forms, then egui's bundled fonts (emoji, symbols) as fallbacks.
//! A second family, [`super::theme::BOLD_FAMILY`], holds bold faces of the same fonts.
//! `.ttc` face indices are honored.

use std::path::Path;
use std::sync::Arc;

use eframe::egui::{self, Context, FontData, FontDefinitions, FontFamily};
use fontdb::{Database, Family, Query, Stretch, Style, Weight};

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

const JAPANESE_FAMILIES: &[&str] = &[
    "Yu Gothic UI",
    "Yu Gothic",
    "Meiryo UI",
    "Meiryo",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Noto Sans CJK JP",
    "Noto Sans JP",
    "Source Han Sans JP",
    "BIZ UDPGothic",
    "IPAexGothic",
    "IPAPGothic",
    "TakaoPGothic",
    "VL PGothic",
];

/// What [`install`] ended up using (for logs / diagnostics).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontReport {
    pub latin: Option<String>,
    pub latin_bold: Option<String>,
    pub japanese: Option<String>,
    pub japanese_bold: Option<String>,
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

/// Font definitions built from system fonts (plus any `extra_dirs`). Expensive (scans and
/// reads font files): build once, ideally off the UI thread, then pass to [`install_definitions`].
pub fn build_definitions(extra_dirs: &[&Path]) -> (FontDefinitions, FontReport) {
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

    let mut regular: Vec<String> = Vec::new();
    let mut bold: Vec<String> = Vec::new();

    let mut add = |db: &Database, families: &[&str], weight: Weight, out: &mut Vec<String>| {
        for name in families {
            if let Some((key, data)) = load_face(db, name, weight) {
                if !out.contains(&key) {
                    defs.font_data.entry(key.clone()).or_insert_with(|| Arc::new(data));
                    out.push(key.clone());
                }
                return Some(key);
            }
        }
        None
    };

    report.latin = add(&db, LATIN_FAMILIES, Weight::NORMAL, &mut regular);
    report.japanese = add(&db, JAPANESE_FAMILIES, Weight::NORMAL, &mut regular);
    report.latin_bold = add(&db, LATIN_FAMILIES, Weight::BOLD, &mut bold);
    report.japanese_bold = add(&db, JAPANESE_FAMILIES, Weight::BOLD, &mut bold);

    // Proportional: system Latin, Japanese, then egui defaults (emoji etc.).
    let mut prop = regular.clone();
    for f in &default_prop {
        if !prop.contains(f) {
            prop.push(f.clone());
        }
    }
    // Bold: bold faces first, then the regular chain as fallback for missing glyphs.
    let mut bold_chain = bold.clone();
    for f in &prop {
        if !bold_chain.contains(f) {
            bold_chain.push(f.clone());
        }
    }
    defs.families.insert(FontFamily::Proportional, prop);
    defs.families
        .insert(FontFamily::Name(BOLD_FAMILY.into()), bold_chain);

    // Monospace: keep egui's Hack first, Japanese as fallback.
    if let Some(jp) = &report.japanese {
        let mono = defs.families.entry(FontFamily::Monospace).or_default();
        if !mono.contains(jp) {
            mono.push(jp.clone());
        }
    }
    (defs, report)
}

/// Install prepared definitions and record that the bold family exists.
pub fn install_definitions(ctx: &Context, defs: FontDefinitions) {
    let bold = defs
        .families
        .contains_key(&FontFamily::Name(BOLD_FAMILY.into()));
    ctx.set_fonts(defs);
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

fn load_face(db: &Database, family: &str, weight: Weight) -> Option<(String, FontData)> {
    let id = db.query(&Query {
        families: &[Family::Name(family)],
        weight,
        stretch: Stretch::Normal,
        style: Style::Normal,
    })?;
    let face = db.face(id)?;
    // fontdb picks the closest weight; reject a regular face when bold was requested so the
    // bold chain does not just duplicate the regular font.
    if weight == Weight::BOLD && face.weight.0 < 600 {
        return None;
    }
    let (bytes, index) = db.with_face_data(id, |data, index| (data.to_vec(), index))?;
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    let key = format!("{}#{}#{}", face.post_script_name, index, face.weight.0);
    Some((key, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_always_have_bold_family() {
        let (defs, _report) = build_definitions(&[]);
        assert!(defs
            .families
            .contains_key(&FontFamily::Name(BOLD_FAMILY.into())));
        assert!(!defs.families[&FontFamily::Proportional].is_empty());
    }
}
