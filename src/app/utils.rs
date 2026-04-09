use super::types::{DirectoryNode, LibraryTrackView};
use crate::core::models::Track;
use eframe::egui;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

pub(super) fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    for (font_name, font_data) in load_system_fallback_fonts() {
        fonts
            .font_data
            .insert(font_name.clone(), Arc::new(egui::FontData::from_owned(font_data)));
        if let Some(proportional_fonts) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            proportional_fonts.insert(0, font_name.clone());
        }
        if let Some(monospace_fonts) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            monospace_fonts.push(font_name);
        }
    }

    ctx.set_fonts(fonts);
}

fn load_system_fallback_fonts() -> Vec<(String, Vec<u8>)> {
    let candidates = [
        ("noto-sans-cjk-jp", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        ("noto-sans-cjk-jp", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.otf"),
        ("noto-sans-cjk-sc", "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf"),
        ("noto-sans-cjk-sc", "/usr/share/fonts/opentype/noto/NotoSansCJK-SC-Regular.otf"),
        ("noto-sans-sc", "/usr/share/fonts/truetype/noto/NotoSansSC-Regular.otf"),
        ("source-han-sans-sc", "/usr/share/fonts/opentype/adobe-source-han-sans/SourceHanSansSC-Regular.otf"),
        ("wqy-zenhei", "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"),
        ("noto-sans-jp", "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc"),
        ("noto-sans-jp", "/usr/share/fonts/truetype/noto/NotoSansJP-Regular.ttf"),
        ("noto-sans-cjk-jp", "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"),
        ("pingfang-sc", "/System/Library/Fonts/PingFang.ttc"),
        ("stheiti", "/System/Library/Fonts/STHeiti Light.ttc"),
        ("hiragino-sans", "/System/Library/Fonts/Hiragino Sans GB.ttc"),
        ("microsoft-yahei", "C:/Windows/Fonts/msyh.ttc"),
        ("microsoft-yahei-bold", "C:/Windows/Fonts/msyhbd.ttc"),
        ("simsun", "C:/Windows/Fonts/simsun.ttc"),
        ("simhei", "C:/Windows/Fonts/simhei.ttf"),
        ("meiryo", "C:/Windows/Fonts/meiryo.ttc"),
        ("msgothic", "C:/Windows/Fonts/msgothic.ttc"),
    ];

    let mut loaded_fonts = Vec::new();
    for (font_name, font_path) in candidates {
        if let Ok(font_data) = fs::read(font_path) {
            loaded_fonts.push((font_name.to_string(), font_data));
        }
    }

    loaded_fonts
}

pub(super) fn insert_track_into_tree(node: &mut DirectoryNode, relative_path: &Path, track: LibraryTrackView) {
    if let Some(parent) = relative_path.parent() {
        let mut cursor = node;
        for component in parent.components() {
            let directory_name = component.as_os_str().to_string_lossy().to_string();
            if directory_name.is_empty() || directory_name == "." {
                continue;
            }
            cursor = cursor.children.entry(directory_name).or_default();
        }
        cursor.tracks.push(track);
        return;
    }

    node.tracks.push(track);
}

pub(super) fn format_duration(duration_ms: u64) -> String {
    let total_seconds = duration_ms / 1_000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

pub(super) fn placeholder_color(key: &str) -> egui::Color32 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    egui::Color32::from_rgb(
        64 + ((hash >> 16) & 0x7f) as u8,
        72 + ((hash >> 8) & 0x7f) as u8,
        88 + (hash & 0x7f) as u8,
    )
}

pub(super) fn artwork_placeholder_label(track: &Track) -> String {
    track
        .album
        .as_deref()
        .and_then(|album| album.chars().next())
        .or_else(|| track.title.chars().next())
        .map(|value| value.to_string())
        .unwrap_or_else(|| String::from("?"))
}