use crate::core::config::ConfigStore;
use crate::core::models::{QueueMode, RepeatMode, Track, TrackId};
use crate::core::player::PlaybackStatus;
use crate::core::state::{AppCommand, AppState};
use eframe::egui;
use image::ImageReader;
use rfd::FileDialog;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppTab {
    Queue,
    Play,
    Album,
    Artist,
    Playlist,
    Library,
    Scan,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArtistKind {
    Artist,
    AlbumArtist,
    Composer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QueuePlaybackMode {
    SingleRepeat,
    QueueRepeat,
    QueueShuffle,
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
    artist: String,
    album: String,
    full_path: String,
    lyric_path: Option<String>,
}

#[derive(Clone)]
struct AlbumView {
    key: String,
    title: String,
    subtitle: String,
    track_ids: Vec<TrackId>,
    preview_track_id: TrackId,
}

#[derive(Clone)]
struct ArtistView {
    name: String,
    track_ids: Vec<TrackId>,
}

pub struct LrcPlayerApp {
    config_store: ConfigStore,
    state: AppState,
    include_path_input: String,
    exclude_path_input: String,
    playlist_name_input: String,
    active_tab: AppTab,
    artist_kind: ArtistKind,
    selected_album_key: Option<String>,
    selected_artist_name: Option<String>,
    revealed_queue_anchor_id: Option<TrackId>,
    detail_track_id: Option<TrackId>,
    artwork_cache: HashMap<String, egui::TextureHandle>,
}

impl LrcPlayerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);

        let config_store = ConfigStore::new();
        let state = AppState::bootstrap(&config_store);

        Self {
            config_store,
            state,
            include_path_input: String::new(),
            exclude_path_input: String::new(),
            playlist_name_input: String::new(),
            active_tab: AppTab::Play,
            artist_kind: ArtistKind::Artist,
            selected_album_key: None,
            selected_artist_name: None,
            revealed_queue_anchor_id: None,
            detail_track_id: None,
            artwork_cache: HashMap::new(),
        }
    }
}

impl eframe::App for LrcPlayerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.state.refresh();
        self.apply_theme(ctx);

        ctx.style_mut(|style| {
            style.spacing.interact_size = egui::vec2(84.0, 42.0);
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
                ui.label("Portable queue-driven music player");
            });

            ui.add_space(8.0);
            self.render_tab_bar(ui);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(&self.state.status_line);
                ui.separator();
                ui.label(format!("Theme: {}", self.state.config.theme));
                ui.separator();
                ui.label(format!("Library: {} tracks", self.state.library.track_count()));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    self.render_active_tab(ui, ctx);
                });
        });

        self.render_track_detail_window(ctx);
    }
}

impl LrcPlayerApp {
    fn apply_theme(&self, ctx: &egui::Context) {
        match self.state.config.theme.as_str() {
            "light" => ctx.set_visuals(egui::Visuals::light()),
            "dark" => ctx.set_visuals(egui::Visuals::dark()),
            _ => {}
        }
    }

    fn render_tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            self.render_tab_button(ui, AppTab::Queue, "Queue");
            self.render_tab_button(ui, AppTab::Play, "Play");
            self.render_tab_button(ui, AppTab::Album, "Album");
            self.render_tab_button(ui, AppTab::Artist, "Artist");
            self.render_tab_button(ui, AppTab::Playlist, "Playlist");
            self.render_tab_button(ui, AppTab::Library, "Library");
            self.render_tab_button(ui, AppTab::Scan, "Scan");
            self.render_tab_button(ui, AppTab::Settings, "Settings");
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
            if tab == AppTab::Queue {
                self.revealed_queue_anchor_id = None;
            }
        }
    }

    fn render_active_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        match self.active_tab {
            AppTab::Queue => self.render_queue_tab(ui, ctx),
            AppTab::Play => self.render_play_tab(ui, ctx),
            AppTab::Album => self.render_album_tab(ui, ctx),
            AppTab::Artist => self.render_artist_tab(ui, ctx),
            AppTab::Playlist => self.render_playlist_tab(ui, ctx),
            AppTab::Library => self.render_library_tab(ui),
            AppTab::Scan => self.render_scan_tab(ui),
            AppTab::Settings => self.render_settings_tab(ui),
        }
    }

    fn render_queue_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Queue");
        ui.label("Current playback order. Click a row to jump there. Drag rows to change the playback order.");

        ui.add_space(12.0);
        let mut playback_mode = self.queue_playback_mode();
        ui.horizontal_wrapped(|ui| {
            ui.label("Playback mode");
            ui.radio_value(&mut playback_mode, QueuePlaybackMode::SingleRepeat, "Single repeat");
            ui.radio_value(&mut playback_mode, QueuePlaybackMode::QueueRepeat, "Queue repeat");
            ui.radio_value(&mut playback_mode, QueuePlaybackMode::QueueShuffle, "Queue shuffle");
        });
        self.set_queue_playback_mode(playback_mode);

        if self.state.queue.track_ids.is_empty() {
            self.revealed_queue_anchor_id = None;
            ui.add_space(12.0);
            ui.label("Queue is empty. Click a track in Album, Artist, Playlist, or Library to build a new queue.");
            return;
        }

        let queue_track_ids = self.state.queue.track_ids.clone();
        let current_track_id = self.current_track_id().cloned();
        let queue_anchor_id = self
            .state
            .queue
            .current_index
            .and_then(|current_index| queue_track_ids.get(current_index.saturating_sub(1)).cloned());
        let should_reveal_anchor = queue_anchor_id.is_some()
            && self.revealed_queue_anchor_id.as_ref() != queue_anchor_id.as_ref();

        let mut pending_move = None;

        for (index, track_id) in queue_track_ids.iter().enumerate() {
            let Some(track) = self.state.library.track_by_id(track_id).cloned() else {
                continue;
            };

            let is_current = current_track_id.as_ref() == Some(&track.id);
            let frame = egui::Frame::group(ui.style()).fill(if is_current {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().extreme_bg_color
            });
            let (inner, dropped_payload) = ui.dnd_drop_zone::<usize, _>(frame, |ui| {
                let drag_id = ui.make_persistent_id(("queue-row", &track.id.0));
                ui.dnd_drag_source(drag_id, index, |ui| {
                    ui.horizontal(|ui| {
                        self.render_track_artwork(ui, ctx, &track, 72.0);
                        ui.vertical(|ui| {
                            let response = ui.selectable_label(is_current, &track.title);
                            if response.clicked() {
                                self.state
                                    .handle(&self.config_store, AppCommand::JumpToQueueTrack(track.id.clone()));
                            }
                            if should_reveal_anchor && queue_anchor_id.as_ref() == Some(&track.id) {
                                response.scroll_to_me(Some(egui::Align::TOP));
                            }
                            self.render_track_context_menu(ui, response, &track.id, &queue_track_ids);
                            ui.label(self.track_secondary_line(&track));
                            ui.small(track.album.clone().unwrap_or_else(|| String::from("Unknown album")));
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.strong(format!("#{}", index + 1));
                            ui.small("Drag");
                        });
                    });
                })
                .inner
            });
            let _ = inner;
            if let Some(from_index) = dropped_payload.map(|payload| *payload) {
                if from_index != index {
                    pending_move = Some((from_index, index));
                }
            }
            ui.add_space(8.0);
        }

        if let Some((from, to)) = pending_move {
            self.state
                .handle(&self.config_store, AppCommand::MoveQueueItem { from, to });
        }

        self.revealed_queue_anchor_id = queue_anchor_id;
    }

    fn render_play_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Play");
        ui.label(self.state.playback_summary());

        ui.add_space(12.0);

        let Some(track) = self.current_track().cloned().or_else(|| self.state.selected_track().cloned()) else {
            ui.label("No track selected.");
            return;
        };

        let available = ui.available_size_before_wrap();
        let art_size = available.x.min(available.y.max(280.0)) * 0.6;
        ui.vertical_centered(|ui| {
            self.render_track_artwork(ui, ctx, &track, art_size.clamp(180.0, 420.0));
            ui.add_space(12.0);
            ui.heading(&track.title);
            ui.label(self.track_secondary_line(&track));
            ui.label(track.album.clone().unwrap_or_else(|| String::from("Unknown album")));
        });

        ui.add_space(18.0);

        let snapshot = self.state.playback_snapshot();
        let duration_ms = track.duration_ms.unwrap_or(0);
        let mut seek_value = snapshot.position_ms.min(duration_ms) as f64;
        let slider = egui::Slider::new(&mut seek_value, 0.0..=duration_ms.max(1) as f64)
            .show_value(false)
            .trailing_fill(true)
            .text("Position");
        let response = ui.add_sized([ui.available_width(), 40.0], slider);
        if response.changed() {
            self.state
                .handle(&self.config_store, AppCommand::Seek(seek_value as u64));
        }

        ui.horizontal(|ui| {
            ui.label(format_duration(snapshot.position_ms));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format_duration(duration_ms));
            });
        });

        ui.add_space((ui.available_height() - 80.0).max(18.0));
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.horizontal_centered(|ui| {
                if ui.button("Previous").clicked() {
                    self.state.handle(&self.config_store, AppCommand::PreviousTrack);
                }

                match snapshot.status {
                    PlaybackStatus::Playing => {
                        if ui.button("Pause").clicked() {
                            self.state.handle(&self.config_store, AppCommand::Pause);
                        }
                    }
                    PlaybackStatus::Paused => {
                        if ui.button("Resume").clicked() {
                            self.state.handle(&self.config_store, AppCommand::Resume);
                        }
                    }
                    PlaybackStatus::Stopped => {
                        if ui.button("Play").clicked() {
                            self.state.handle(&self.config_store, AppCommand::PlaySelected);
                        }
                    }
                }

                if ui.button("Next").clicked() {
                    self.state.handle(&self.config_store, AppCommand::NextTrack);
                }
            });
        });
    }

    fn render_album_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Album");
        ui.label("Albums are grouped from tags, with tracks available below the selected album.");

        let albums = self.build_album_views();
        if albums.is_empty() {
            ui.add_space(12.0);
            ui.label("No albums available. Scan at least one directory first.");
            return;
        }

        ui.add_space(12.0);
        let card_width = 200.0;
        let column_count = ((ui.available_width() + 12.0) / (card_width + 12.0)).floor() as usize;
        let column_count = column_count.max(1);
        for row in albums.chunks(column_count) {
            ui.columns(row.len(), |columns| {
                for (column, album) in columns.iter_mut().zip(row.iter()) {
                    let selected = self.selected_album_key.as_deref() == Some(album.key.as_str());
                    egui::Frame::group(column.style()).show(column, |ui| {
                        ui.set_width(card_width);
                        if let Some(track) = self.state.library.track_by_id(&album.preview_track_id).cloned() {
                            self.render_track_artwork(ui, ctx, &track, 172.0);
                        }

                        if ui.selectable_label(selected, &album.title).clicked() {
                            self.selected_album_key = Some(album.key.clone());
                        }
                        ui.small(&album.subtitle);
                        ui.small(format!("{} tracks", album.track_ids.len()));
                    });
                }
            });
            ui.add_space(12.0);
        }

        if self.selected_album_key.is_none() {
            self.selected_album_key = albums.first().map(|album| album.key.clone());
        }

        let Some(selected_album) = albums
            .iter()
            .find(|album| self.selected_album_key.as_deref() == Some(album.key.as_str()))
            .cloned()
        else {
            return;
        };

        ui.add_space(18.0);
        ui.heading(format!("Album Tracks: {}", selected_album.title));
        let mut tracks = selected_album
            .track_ids
            .iter()
            .filter_map(|track_id| self.state.library.track_by_id(track_id).cloned())
            .collect::<Vec<_>>();
        tracks.sort_by(|left, right| {
            left.track_number
                .unwrap_or(u32::MAX)
                .cmp(&right.track_number.unwrap_or(u32::MAX))
                .then_with(|| left.file_name.cmp(&right.file_name))
        });
        self.render_track_list(ui, ctx, &tracks, &selected_album.track_ids);
    }

    fn render_artist_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Artist");
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.artist_kind, ArtistKind::Artist, "Artist");
            ui.selectable_value(&mut self.artist_kind, ArtistKind::AlbumArtist, "Album Artist");
            ui.selectable_value(&mut self.artist_kind, ArtistKind::Composer, "Composer");
        });

        let artists = self.build_artist_views();
        if artists.is_empty() {
            ui.add_space(12.0);
            ui.label("No artist information is available in the scanned library.");
            return;
        }

        if self.selected_artist_name.is_none() {
            self.selected_artist_name = artists.first().map(|artist| artist.name.clone());
        }

        ui.add_space(12.0);
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                for artist in &artists {
                    let selected = self.selected_artist_name.as_deref() == Some(artist.name.as_str());
                    if ui.selectable_label(selected, &artist.name).clicked() {
                        self.selected_artist_name = Some(artist.name.clone());
                    }
                }
            });

            egui::ScrollArea::vertical().show(&mut columns[1], |ui| {
                let Some(selected_artist) = artists
                    .iter()
                    .find(|artist| self.selected_artist_name.as_deref() == Some(artist.name.as_str()))
                    .cloned()
                else {
                    return;
                };

                let mut tracks = selected_artist
                    .track_ids
                    .iter()
                    .filter_map(|track_id| self.state.library.track_by_id(track_id).cloned())
                    .collect::<Vec<_>>();
                tracks.sort_by(|left, right| {
                    left.album
                        .clone()
                        .unwrap_or_default()
                        .cmp(&right.album.clone().unwrap_or_default())
                        .then_with(|| left.file_name.cmp(&right.file_name))
                });

                self.render_track_list(ui, ctx, &tracks, &selected_artist.track_ids);
            });
        });
    }

    fn render_playlist_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Playlist");
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.playlist_name_input)
                    .hint_text("New playlist name")
                    .desired_width(260.0),
            );
            if ui.button("Create playlist").clicked() {
                self.state.handle(
                    &self.config_store,
                    AppCommand::CreatePlaylist(self.playlist_name_input.clone()),
                );
                self.playlist_name_input.clear();
            }
            if let Some(track) = self.state.selected_track().cloned() {
                if let Some(selected_playlist_name) = self.state.selected_playlist_name.clone() {
                    if ui.button("Add selected track").clicked() {
                        self.state.handle(
                            &self.config_store,
                            AppCommand::AddTrackToPlaylist {
                                playlist_name: selected_playlist_name,
                                track_id: track.id,
                            },
                        );
                    }
                }
            }
        });

        ui.add_space(12.0);
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                if self.state.playlists.is_empty() {
                    ui.label("No playlists yet.");
                    return;
                }

                let playlist_names = self
                    .state
                    .playlists
                    .iter()
                    .map(|playlist| playlist.name.clone())
                    .collect::<Vec<_>>();

                for playlist_name in playlist_names {
                    let selected = self.state.selected_playlist_name.as_deref() == Some(playlist_name.as_str());
                    ui.horizontal(|ui| {
                        if ui.selectable_label(selected, &playlist_name).clicked() {
                            self.state.handle(
                                &self.config_store,
                                AppCommand::SelectPlaylist(Some(playlist_name.clone())),
                            );
                        }
                        if ui.button("Delete").clicked() {
                            self.state.handle(
                                &self.config_store,
                                AppCommand::DeletePlaylist(playlist_name.clone()),
                            );
                        }
                    });
                }
            });

            egui::ScrollArea::vertical().show(&mut columns[1], |ui| {
                let Some(selected_playlist_name) = self.state.selected_playlist_name.clone() else {
                    ui.label("Select a playlist.");
                    return;
                };

                let playlist_track_ids = self
                    .state
                    .playlists
                    .iter()
                    .find(|playlist| playlist.name == selected_playlist_name)
                    .map(|playlist| playlist.track_ids.clone())
                    .unwrap_or_default();
                let tracks = playlist_track_ids
                    .iter()
                    .filter_map(|track_id| self.state.library.track_by_id(track_id).cloned())
                    .collect::<Vec<_>>();

                self.render_playlist_tracks(ui, ctx, &selected_playlist_name, &tracks, &playlist_track_ids);
            });
        });
    }

    fn render_library_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Library");
        ui.label("Directory tree view of the registered library folders.");

        if self.state.library.tracks().is_empty() {
            ui.add_space(12.0);
            ui.label("No directory scanned.");
            return;
        }

        let tree = self.build_library_tree();
        let library_track_ids = self
            .state
            .library
            .tracks()
            .iter()
            .map(|track| track.id.clone())
            .collect::<Vec<_>>();

        for (root_label, root_node) in tree {
            egui::CollapsingHeader::new(root_label)
                .default_open(false)
                .show(ui, |ui| {
                    self.render_directory_node(ui, &root_node, &library_track_ids);
                });
            ui.add_space(8.0);
        }
    }

    fn render_scan_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scan");
        ui.label("Register include and exclude folders for library scanning.");

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.label("Include folder");
            ui.add(
                egui::TextEdit::singleline(&mut self.include_path_input)
                    .desired_width(f32::INFINITY)
                    .hint_text("/path/to/music"),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("Browse...").clicked() {
                    self.pick_folder(true);
                }
                if ui.button("Add scan folder").clicked() {
                    self.state.handle(
                        &self.config_store,
                        AppCommand::AddLibraryFolder(self.include_path_input.clone()),
                    );
                }
            });
        });

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.label("Exclude folder");
            ui.add(
                egui::TextEdit::singleline(&mut self.exclude_path_input)
                    .desired_width(f32::INFINITY)
                    .hint_text("/path/to/skip"),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("Browse...").clicked() {
                    self.pick_folder(false);
                }
                if ui.button("Add excluded folder").clicked() {
                    self.state.handle(
                        &self.config_store,
                        AppCommand::AddExcludedFolder(self.exclude_path_input.clone()),
                    );
                }
            });
        });

        ui.add_space(16.0);
        ui.columns(2, |columns| {
            columns[0].group(|ui| {
                ui.label("Scan folders");
                let folders = self.state.config.library_folders.clone();
                if folders.is_empty() {
                    ui.label("No registered folders.");
                }
                for folder in folders {
                    ui.horizontal(|ui| {
                        ui.monospace(&folder);
                        if ui.button("Remove").clicked() {
                            self.state.handle(
                                &self.config_store,
                                AppCommand::RemoveLibraryFolder(folder.clone()),
                            );
                        }
                    });
                }
            });

            columns[1].group(|ui| {
                ui.label("Excluded folders");
                let folders = self.state.config.excluded_folders.clone();
                if folders.is_empty() {
                    ui.label("No exclusions configured.");
                }
                for folder in folders {
                    ui.horizontal(|ui| {
                        ui.monospace(&folder);
                        if ui.button("Remove").clicked() {
                            self.state.handle(
                                &self.config_store,
                                AppCommand::RemoveExcludedFolder(folder.clone()),
                            );
                        }
                    });
                }
            });
        });

        ui.add_space(12.0);
        if ui.button("Rescan now").clicked() {
            self.state.handle(&self.config_store, AppCommand::RescanLibrary);
        }
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.label("Current settings are kept in the local portable data directory.");

        ui.add_space(12.0);
        let mut theme = self.state.config.theme.clone();
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut theme, String::from("system"), "System");
            ui.selectable_value(&mut theme, String::from("light"), "Light");
            ui.selectable_value(&mut theme, String::from("dark"), "Dark");
        });
        if theme != self.state.config.theme {
            self.state.handle(&self.config_store, AppCommand::SetTheme(theme));
        }

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.label("Portable layout");
            ui.label("- Data stays under ./data relative to the executable.");
            ui.label("- Scan folders and exclusions are saved to config.json.");
            ui.label("- Playlists are saved to playlists.json in the same data directory.");
            ui.label("- Queue order is session-local and updates immediately when rows are dragged.");
        });
    }

    fn render_track_list(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tracks: &[Track],
        visible_track_ids: &[TrackId],
    ) {
        if tracks.is_empty() {
            ui.label("No tracks to display.");
            return;
        }

        for track in tracks {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    self.render_track_artwork(ui, ctx, track, 56.0);
                    ui.vertical(|ui| {
                        let response = ui.selectable_label(
                            self.current_track_id() == Some(&track.id),
                            &track.title,
                        );
                        if response.clicked() {
                            self.state.handle(
                                &self.config_store,
                                AppCommand::BuildQueueAndPlay {
                                    track_ids: visible_track_ids.to_vec(),
                                    start_track_id: track.id.clone(),
                                },
                            );
                            self.active_tab = AppTab::Play;
                        }
                        self.render_track_context_menu(ui, response, &track.id, visible_track_ids);
                        ui.label(self.track_secondary_line(track));
                        ui.small(track.file_name.clone());
                    });
                });
            });
            ui.add_space(8.0);
        }
    }

    fn render_track_context_menu(
        &mut self,
        ui: &mut egui::Ui,
        response: egui::Response,
        track_id: &TrackId,
        visible_track_ids: &[TrackId],
    ) {
        let track_id = track_id.clone();
        let visible_track_ids = visible_track_ids.to_vec();
        response.context_menu(|ui| {
            if ui.button("Play from here").clicked() {
                self.state.handle(
                    &self.config_store,
                    AppCommand::BuildQueueAndPlay {
                        track_ids: visible_track_ids.clone(),
                        start_track_id: track_id.clone(),
                    },
                );
                self.active_tab = AppTab::Play;
                ui.close_menu();
            }

            if ui.button("Play next").clicked() {
                self.state
                    .handle(&self.config_store, AppCommand::QueueTrackNext(track_id.clone()));
                ui.close_menu();
            }

            if ui.button("Add to queue end").clicked() {
                self.state.handle(
                    &self.config_store,
                    AppCommand::AppendTrackToQueue(track_id.clone()),
                );
                ui.close_menu();
            }

            if !self.state.playlists.is_empty() {
                ui.separator();
                ui.label("Add to playlist");
                let playlists = self.state.playlists.clone();
                for playlist in playlists {
                    if ui.button(&playlist.name).clicked() {
                        self.state.handle(
                            &self.config_store,
                            AppCommand::AddTrackToPlaylist {
                                playlist_name: playlist.name,
                                track_id: track_id.clone(),
                            },
                        );
                        ui.close_menu();
                    }
                }
            }

            ui.separator();
            if ui.button("Show details").clicked() {
                self.detail_track_id = Some(track_id.clone());
                ui.close_menu();
            }
        });
        let _ = ui;
    }

    fn render_directory_node(
        &mut self,
        ui: &mut egui::Ui,
        node: &DirectoryNode,
        visible_track_ids: &[TrackId],
    ) {
        for (directory_name, child_node) in &node.children {
            egui::CollapsingHeader::new(directory_name)
                .default_open(false)
                .show(ui, |ui| {
                    self.render_directory_node(ui, child_node, visible_track_ids);
                });
        }

        for track in &node.tracks {
            let selected = self.current_track_id() == Some(&track.id);
            let response = ui.selectable_label(selected, &track.title);
            if response.clicked() {
                self.state.handle(
                    &self.config_store,
                    AppCommand::BuildQueueAndPlay {
                        track_ids: visible_track_ids.to_vec(),
                        start_track_id: track.id.clone(),
                    },
                );
                self.active_tab = AppTab::Play;
            }
            self.render_track_context_menu(ui, response, &track.id, visible_track_ids);
            ui.small(format!("{} / {}", track.artist, track.album));
            ui.monospace(&track.full_path);
            if let Some(lyric_path) = &track.lyric_path {
                ui.small(format!("LRC: {lyric_path}"));
            }
            ui.add_space(8.0);
        }
    }

    fn pick_folder(&mut self, include_folder: bool) {
        let dialog = FileDialog::new().set_title("Select folder");
        if let Some(path) = dialog.pick_folder() {
            let selected = path.to_string_lossy().to_string();
            if include_folder {
                self.include_path_input = selected.clone();
                self.state
                    .handle(&self.config_store, AppCommand::AddLibraryFolder(selected));
            } else {
                self.exclude_path_input = selected.clone();
                self.state
                    .handle(&self.config_store, AppCommand::AddExcludedFolder(selected));
            }
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
                    artist: track.artist.clone().unwrap_or_else(|| String::from("Unknown artist")),
                    album: track.album.clone().unwrap_or_else(|| String::from("Unknown album")),
                    full_path: track.file_path().to_string(),
                    lyric_path: track.lyric_path.clone(),
                };
                insert_track_into_tree(&mut root_node, relative_path, track_view);
            }

            roots.push((folder.clone(), root_node));
        }

        roots
    }

    fn build_album_views(&self) -> Vec<AlbumView> {
        let mut albums = BTreeMap::<String, AlbumView>::new();

        for track in self.state.library.tracks() {
            let album_title = track.album.clone().unwrap_or_else(|| String::from("Unknown album"));
            let subtitle = track
                .album_artist
                .clone()
                .or_else(|| track.artist.clone())
                .unwrap_or_else(|| String::from("Unknown artist"));
            let key = format!("{subtitle}\u{1f}{album_title}");

            albums
                .entry(key.clone())
                .and_modify(|album| album.track_ids.push(track.id.clone()))
                .or_insert_with(|| AlbumView {
                    key,
                    title: album_title,
                    subtitle,
                    track_ids: vec![track.id.clone()],
                    preview_track_id: track.id.clone(),
                });
        }

        albums.into_values().collect()
    }

    fn build_artist_views(&self) -> Vec<ArtistView> {
        let mut artists = BTreeMap::<String, Vec<TrackId>>::new();

        for track in self.state.library.tracks() {
            let Some(name) = (match self.artist_kind {
                ArtistKind::Artist => track.artist.clone(),
                ArtistKind::AlbumArtist => track.album_artist.clone(),
                ArtistKind::Composer => track.composer.clone(),
            }) else {
                continue;
            };

            artists.entry(name).or_default().push(track.id.clone());
        }

        artists
            .into_iter()
            .map(|(name, track_ids)| ArtistView { name, track_ids })
            .collect()
    }

    fn render_playlist_tracks(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        playlist_name: &str,
        tracks: &[Track],
        visible_track_ids: &[TrackId],
    ) {
        if tracks.is_empty() {
            ui.label("No tracks in this playlist.");
            return;
        }

        let mut pending_move = None;
        let mut pending_remove = None;

        for (index, track) in tracks.iter().enumerate() {
            let frame = egui::Frame::group(ui.style()).fill(if self.current_track_id() == Some(&track.id) {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().extreme_bg_color
            });

            let (_inner, dropped_payload) = ui.dnd_drop_zone::<usize, _>(frame, |ui| {
                let drag_id = ui.make_persistent_id(("playlist-row", playlist_name, &track.id.0));
                ui.dnd_drag_source(drag_id, index, |ui| {
                    ui.horizontal(|ui| {
                        self.render_track_artwork(ui, ctx, track, 56.0);
                        ui.vertical(|ui| {
                            let response = ui.selectable_label(
                                self.current_track_id() == Some(&track.id),
                                &track.title,
                            );
                            if response.clicked() {
                                self.state.handle(
                                    &self.config_store,
                                    AppCommand::BuildQueueAndPlay {
                                        track_ids: visible_track_ids.to_vec(),
                                        start_track_id: track.id.clone(),
                                    },
                                );
                                self.active_tab = AppTab::Play;
                            }
                            self.render_track_context_menu(ui, response, &track.id, visible_track_ids);
                            ui.label(self.track_secondary_line(track));
                            ui.small(track.file_name.clone());
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Remove").clicked() {
                                pending_remove = Some(track.id.clone());
                            }
                            ui.strong(format!("#{}", index + 1));
                            ui.small("Drag");
                        });
                    });
                })
                .inner
            });

            if let Some(from_index) = dropped_payload.map(|payload| *payload) {
                if from_index != index {
                    pending_move = Some((from_index, index));
                }
            }

            ui.add_space(8.0);
        }

        if let Some((from, to)) = pending_move {
            self.state.handle(
                &self.config_store,
                AppCommand::MovePlaylistItem {
                    playlist_name: playlist_name.to_string(),
                    from,
                    to,
                },
            );
        }

        if let Some(track_id) = pending_remove {
            self.state.handle(
                &self.config_store,
                AppCommand::RemoveTrackFromPlaylist {
                    playlist_name: playlist_name.to_string(),
                    track_id,
                },
            );
        }
    }

    fn render_track_detail_window(&mut self, ctx: &egui::Context) {
        let Some(track_id) = self.detail_track_id.clone() else {
            return;
        };

        let Some(track) = self.state.library.track_by_id(&track_id).cloned() else {
            self.detail_track_id = None;
            return;
        };

        let mut open = true;
        egui::Window::new("Track Details")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.heading(&track.title);
                ui.add_space(12.0);
                ui.label(format!(
                    "Artist: {}",
                    track.artist.clone().unwrap_or_else(|| String::from("Unknown artist"))
                ));
                ui.label(format!(
                    "Album: {}",
                    track.album.clone().unwrap_or_else(|| String::from("Unknown album"))
                ));
                ui.label(format!(
                    "Album Artist: {}",
                    track.album_artist.clone().unwrap_or_else(|| String::from("Unknown"))
                ));
                ui.label(format!(
                    "Composer: {}",
                    track.composer.clone().unwrap_or_else(|| String::from("Unknown"))
                ));
                if let Some(duration_ms) = track.duration_ms {
                    ui.label(format!("Duration: {}", format_duration(duration_ms)));
                }
                ui.separator();
                ui.label("File path");
                ui.monospace(track.file_path());
            });

        if !open {
            self.detail_track_id = None;
        }
    }

    fn queue_playback_mode(&self) -> QueuePlaybackMode {
        if self.state.queue.repeat_mode == RepeatMode::One {
            QueuePlaybackMode::SingleRepeat
        } else if self.state.queue.queue_mode == QueueMode::Shuffle {
            QueuePlaybackMode::QueueShuffle
        } else {
            QueuePlaybackMode::QueueRepeat
        }
    }

    fn set_queue_playback_mode(&mut self, mode: QueuePlaybackMode) {
        match mode {
            QueuePlaybackMode::SingleRepeat => {
                self.state.queue.repeat_mode = RepeatMode::One;
                self.state.queue.queue_mode = QueueMode::Sequential;
            }
            QueuePlaybackMode::QueueRepeat => {
                self.state.queue.repeat_mode = RepeatMode::All;
                self.state.queue.queue_mode = QueueMode::Sequential;
            }
            QueuePlaybackMode::QueueShuffle => {
                self.state.queue.repeat_mode = RepeatMode::Off;
                self.state.queue.queue_mode = QueueMode::Shuffle;
            }
        }

        self.state
            .handle(&self.config_store, AppCommand::SetRepeatMode(self.state.queue.repeat_mode));
        self.state
            .handle(&self.config_store, AppCommand::SetQueueMode(self.state.queue.queue_mode));
    }

    fn render_track_artwork(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        track: &Track,
        size: f32,
    ) {
        let cache_key = format!("art:{}", track.id.0);
        if let Some(texture_id) = self.ensure_artwork_texture(ctx, &cache_key, track.artwork.as_deref()) {
            ui.image((texture_id, egui::vec2(size, size)));
            return;
        }

        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let fill = placeholder_color(&cache_key);
        painter.rect_filled(rect, 10.0, fill);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            artwork_placeholder_label(track),
            egui::FontId::proportional((size * 0.18).clamp(16.0, 36.0)),
            egui::Color32::WHITE,
        );
    }

    fn ensure_artwork_texture(
        &mut self,
        ctx: &egui::Context,
        cache_key: &str,
        artwork_bytes: Option<&[u8]>,
    ) -> Option<egui::TextureId> {
        if !self.artwork_cache.contains_key(cache_key) {
            let bytes = artwork_bytes?;
            let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
            let image = reader.decode().ok()?.to_rgba8();
            let image_size = [image.width() as usize, image.height() as usize];
            let pixels = image.into_raw();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(image_size, &pixels);
            let texture = ctx.load_texture(cache_key.to_string(), color_image, egui::TextureOptions::LINEAR);
            self.artwork_cache.insert(cache_key.to_string(), texture);
        }

        self.artwork_cache.get(cache_key).map(|texture| texture.id())
    }

    fn current_track(&self) -> Option<&Track> {
        let track_id = self.current_track_id()?;
        self.state.library.track_by_id(track_id)
    }

    fn current_track_id(&self) -> Option<&TrackId> {
        self.state.queue.current_track_id()
    }

    fn track_secondary_line(&self, track: &Track) -> String {
        let artist = track.artist.clone().unwrap_or_else(|| String::from("Unknown artist"));
        let album = track.album.clone().unwrap_or_else(|| String::from("Unknown album"));
        format!("{artist} / {album}")
    }

}

fn configure_fonts(ctx: &egui::Context) {
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

fn format_duration(duration_ms: u64) -> String {
    let total_seconds = duration_ms / 1_000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn placeholder_color(key: &str) -> egui::Color32 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    egui::Color32::from_rgb(
        64 + ((hash >> 16) & 0x7f) as u8,
        72 + ((hash >> 8) & 0x7f) as u8,
        88 + (hash & 0x7f) as u8,
    )
}

fn artwork_placeholder_label(track: &Track) -> String {
    track
        .album
        .as_deref()
        .and_then(|album| album.chars().next())
        .or_else(|| track.title.chars().next())
        .map(|value| value.to_string())
        .unwrap_or_else(|| String::from("?"))
}