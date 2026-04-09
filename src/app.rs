use crate::core::config::ConfigStore;
use crate::core::models::TrackId;
use crate::core::state::{AppCommand, AppState};
use eframe::egui;
use rfd::FileDialog;
use std::fs;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppTab {
    Queue,
    Play,
    Library,
    Scan,
}

#[derive(Default)]
struct DirectoryNode {
    children: BTreeMap<String, DirectoryNode>,
    tracks: Vec<LibraryTrackView>,
}

#[derive(Clone)]
struct LibraryTrackView {
    id: TrackId,
    title: String,
    full_path: String,
    lyric_path: Option<String>,
}

pub struct LrcPlayerApp {
    config_store: ConfigStore,
    state: AppState,
    directory_path_input: String,
    active_tab: AppTab,
}

impl LrcPlayerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);

        let config_store = ConfigStore::new();
        let state = AppState::bootstrap(&config_store);

        Self {
            config_store,
            state,
            directory_path_input: String::new(),
            active_tab: AppTab::Play,
        }
    }
}

impl eframe::App for LrcPlayerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.style_mut(|style| {
            style.spacing.interact_size = egui::vec2(80.0, 40.0);
            style.spacing.button_padding = egui::vec2(12.0, 10.0);
            style.spacing.scroll = egui::style::ScrollStyle {
                bar_width: 24.0,
                bar_inner_margin: 6.0,
                bar_outer_margin: 4.0,
                ..egui::style::ScrollStyle::solid()
            };
            style.text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::new(22.0, egui::FontFamily::Proportional),
            );
        });

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("LRC Player");
                ui.separator();
                ui.label("Portable audio player scaffold");
            });

            ui.add_space(8.0);
            self.render_tab_bar(ui);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&self.state.status_line);
                ui.separator();
                ui.label(format!("Theme: {}", self.state.config.theme));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                self.render_active_tab(ui);
                });
        });
    }
}

impl LrcPlayerApp {
    fn render_tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            self.render_tab_button(ui, AppTab::Queue, "Queue");
            self.render_tab_button(ui, AppTab::Play, "Play");
            self.render_tab_button(ui, AppTab::Library, "Library");
            self.render_tab_button(ui, AppTab::Scan, "Scan");
        });
    }

    fn render_tab_button(&mut self, ui: &mut egui::Ui, tab: AppTab, label: &str) {
        let selected = self.active_tab == tab;
        let button = egui::Button::new(
            egui::RichText::new(label)
                .size(26.0)
                .family(egui::FontFamily::Proportional),
        )
        .selected(selected)
        .min_size(egui::vec2(140.0, 56.0));

        if ui.add(button).clicked() {
            self.active_tab = tab;
        }
    }

    fn render_active_tab(&mut self, ui: &mut egui::Ui) {
        match self.active_tab {
            AppTab::Queue => self.render_queue_tab(ui),
            AppTab::Play => self.render_play_tab(ui),
            AppTab::Library => self.render_library_tab(ui),
            AppTab::Scan => self.render_scan_tab(ui),
        }
    }

    fn render_queue_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Queue");
        ui.label("Current queue order follows the scanned library order.");

        ui.add_space(12.0);

        if self.state.library.tracks().is_empty() {
            ui.label("No tracks available. Open the Scan tab and scan a directory first.");
            return;
        }

        if let Some(track) = self.state.selected_track() {
            ui.label(format!("Current track: {}", track.title));
            ui.label(self.queue_track_label(track.title.as_str(), track.artist.as_deref()));
        } else {
            ui.label("No track selected.");
        }

        ui.add_space(16.0);
        ui.group(|ui| {
            ui.label("Queue order");

            let mut clicked_track_id = None;
            for track in self.state.library.tracks() {
                let selected = self.is_selected_track(&track.id);
                if ui
                    .selectable_label(
                        selected,
                        self.queue_track_label(track.title.as_str(), track.artist.as_deref()),
                    )
                    .clicked()
                {
                    clicked_track_id = Some(track.id.clone());
                }
                ui.add_space(8.0);
            }

            if let Some(track_id) = clicked_track_id {
                self.state
                    .handle(&self.config_store, AppCommand::SelectTrack(track_id));
            }
        });
    }

    fn render_play_tab(&mut self, ui: &mut egui::Ui) {
        let playback_snapshot = self.state.playback_snapshot();

        ui.heading("Now Playing");
        ui.label(self.state.playback_summary());

        if let Some(track) = self.state.selected_track() {
            ui.add_space(8.0);
            ui.label(format!("Selected track: {}", track.title));
            ui.monospace(track.file_path());
        }

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Play").clicked() {
                self.state
                    .handle(&self.config_store, AppCommand::PlaySelected);
            }

            match playback_snapshot.status {
                crate::core::player::PlaybackStatus::Playing => {
                    if ui.button("Pause").clicked() {
                        self.state.handle(&self.config_store, AppCommand::Pause);
                    }
                }
                crate::core::player::PlaybackStatus::Paused => {
                    if ui.button("Resume").clicked() {
                        self.state.handle(&self.config_store, AppCommand::Resume);
                    }
                }
                crate::core::player::PlaybackStatus::Stopped => {}
            }

            if ui.button("Stop").clicked() {
                self.state.handle(&self.config_store, AppCommand::Stop);
            }
        });

        ui.add_space(16.0);
        ui.group(|ui| {
            ui.label("Current design constraints");
            ui.label("- All mutable files stay inside the application directory.");
            ui.label("- Core logic remains independent from UI toolkit details.");
            ui.label("- Audio playback is decoded with symphonia and output via cpal.");
        });
    }

    fn render_library_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Library");
        ui.label("Scanned tracks by directory");

        ui.add_space(12.0);

        if self.state.library.tracks().is_empty() {
            ui.label("No directory scanned.");
        } else {
            let tree = self.build_library_tree();
            let mut clicked_track_id = None;

            for (root_label, root_node) in tree {
                egui::CollapsingHeader::new(root_label)
                    .default_open(true)
                    .show(ui, |ui| {
                        self.render_directory_node(ui, &root_node, &mut clicked_track_id);
                    });
                ui.add_space(8.0);
            }

            if let Some(track_id) = clicked_track_id {
                self.state
                    .handle(&self.config_store, AppCommand::SelectTrack(track_id));
            }
        }

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.label(format!("Indexed tracks: {}", self.state.library.track_count()));
            ui.label("Library shows the scanned folder structure. Selecting a track changes the target for Play.");
        });
    }

    fn render_scan_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scan");
        ui.label("Choose a music directory and scan it recursively.");

        ui.add_space(16.0);
        ui.label("Music directory path");
        ui.add(
            egui::TextEdit::singleline(&mut self.directory_path_input)
                .desired_width(f32::INFINITY)
                .hint_text("/path/to/music or C:/Music"),
        );

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Browse folder...").clicked() {
                self.pick_audio_directory();
            }

            if ui.button("Scan directory").clicked() {
                self.state.handle(
                    &self.config_store,
                    AppCommand::OpenAudioDirectory(self.directory_path_input.clone()),
                );
            }

            if ui.button("Save current config").clicked() {
                self.state
                    .handle(&self.config_store, AppCommand::SaveConfig);
            }
        });

        ui.add_space(16.0);
        ui.group(|ui| {
            ui.label("Scanned directories");
            if self.state.config.library_folders.is_empty() {
                ui.label("No directory saved yet.");
            } else {
                for folder in &self.state.config.library_folders {
                    ui.monospace(folder);
                }
            }
        });

        ui.add_space(16.0);
        ui.group(|ui| {
            ui.label("Portable data");
            ui.label("Config, cache, playlists, and logs stay under ./data.");
        });
    }

    fn pick_audio_directory(&mut self) {
        let dialog = FileDialog::new().set_title("Select a music directory");

        if let Some(path) = dialog.pick_folder() {
            self.directory_path_input = path.to_string_lossy().to_string();
            self.state.handle(
                &self.config_store,
                AppCommand::OpenAudioDirectory(self.directory_path_input.clone()),
            );
        }
    }

    fn is_selected_track(&self, track_id: &TrackId) -> bool {
        self.state
            .selected_track_id
            .as_ref()
            .map(|selected_track_id| selected_track_id == track_id)
            .unwrap_or(false)
    }

    fn queue_track_label(&self, title: &str, artist: Option<&str>) -> String {
        match artist.filter(|artist_name| !artist_name.trim().is_empty()) {
            Some(artist_name) => format!("{title} / {artist_name}"),
            None => format!("{title} / Unknown artist"),
        }
    }

    fn build_library_tree(&self) -> Vec<(String, DirectoryNode)> {
        let mut roots = Vec::new();

        for folder in &self.state.config.library_folders {
            let root_path = Path::new(folder);
            let mut root_node = DirectoryNode::default();

            for track in self.state.library.tracks() {
                let track_path = Path::new(track.file_path());
                let Ok(relative_path) = track_path.strip_prefix(root_path) else {
                    continue;
                };

                let track_view = LibraryTrackView {
                    id: track.id.clone(),
                    title: track.title.clone(),
                    full_path: track.file_path().to_string(),
                    lyric_path: track.lyric_path.clone(),
                };
                insert_track_into_tree(&mut root_node, relative_path, track_view);
            }

            roots.push((folder.clone(), root_node));
        }

        if roots.is_empty() {
            let mut root_node = DirectoryNode::default();
            for track in self.state.library.tracks() {
                let track_view = LibraryTrackView {
                    id: track.id.clone(),
                    title: track.title.clone(),
                    full_path: track.file_path().to_string(),
                    lyric_path: track.lyric_path.clone(),
                };
                insert_track_into_tree(&mut root_node, Path::new(track.file_path()), track_view);
            }
            roots.push((String::from("Library"), root_node));
        }

        roots
    }

    fn render_directory_node(
        &mut self,
        ui: &mut egui::Ui,
        node: &DirectoryNode,
        clicked_track_id: &mut Option<TrackId>,
    ) {
        for (directory_name, child_node) in &node.children {
            egui::CollapsingHeader::new(directory_name)
                .default_open(true)
                .show(ui, |ui| {
                    self.render_directory_node(ui, child_node, clicked_track_id);
                });
        }

        for track in &node.tracks {
            let selected = self.is_selected_track(&track.id);
            if ui.selectable_label(selected, &track.title).clicked() {
                *clicked_track_id = Some(track.id.clone());
            }
            ui.monospace(&track.full_path);
            if let Some(lyric_path) = &track.lyric_path {
                ui.label(format!("LRC: {lyric_path}"));
            }
            ui.add_space(8.0);
        }
    }
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    for (font_name, font_data) in load_system_fallback_fonts() {
        fonts.font_data.insert(font_name.clone(), Arc::new(egui::FontData::from_owned(font_data)));
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
        ("noto-sans-jp", "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc"),
        ("noto-sans-jp", "/usr/share/fonts/truetype/noto/NotoSansJP-Regular.ttf"),
        ("noto-sans-cjk-jp", "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"),
        ("hiragino-sans", "/System/Library/Fonts/Hiragino Sans GB.ttc"),
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

fn insert_track_into_tree(node: &mut DirectoryNode, relative_path: &Path, track: LibraryTrackView) {
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
