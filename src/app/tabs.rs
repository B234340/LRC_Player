use super::types::{AlbumView, ArtistKind, ArtistView, DirectoryNode, LibraryTrackView, QueuePlaybackMode};
use super::utils::{artwork_placeholder_label, format_duration, insert_track_into_tree, placeholder_color};
use super::LrcPlayerApp;
use crate::core::models::{QueueMode, RepeatMode, Track, TrackId};
use crate::core::player::PlaybackStatus;
use crate::core::state::AppCommand;
use eframe::egui;
use image::ImageReader;
use rfd::FileDialog;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::Path;

impl LrcPlayerApp {
    pub(super) fn render_queue_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Queue");
        ui.label("Current playback order. Click a row to jump there. Drag rows to change the playback order.");

        ui.add_space(12.0);
        ui.group(|ui| {
            let mut playback_mode = self.queue_playback_mode();
            ui.horizontal_wrapped(|ui| {
                ui.label("Playback mode");
                ui.radio_value(&mut playback_mode, QueuePlaybackMode::SingleRepeat, "Single repeat");
                ui.radio_value(&mut playback_mode, QueuePlaybackMode::QueueRepeat, "Queue repeat");
                ui.radio_value(&mut playback_mode, QueuePlaybackMode::QueueShuffle, "Queue shuffle");
            });
            self.set_queue_playback_mode(playback_mode);
        });

        if self.state.queue.track_ids.is_empty() {
            self.revealed_queue_anchor_id = None;
            ui.add_space(12.0);
            ui.label("Queue is empty. Click a track in Album, Artist, Playlist, or Library to build a new queue.");
            return;
        }

        let queue_track_ids = self.state.queue.track_ids.clone();
        let current_track_id = self.current_track_id().cloned();
        if current_track_id != self.last_queue_current_track_id {
            self.last_queue_current_track_id = current_track_id.clone();
            if self.state.config.queue_follow_current {
                self.queue_follow_paused_by_user = false;
                self.revealed_queue_anchor_id = None;
            }
        }
        let queue_anchor_id = if self.state.config.queue_follow_current && !self.queue_follow_paused_by_user {
            self.state
                .queue
                .current_index
                .and_then(|current_index| queue_track_ids.get(current_index.saturating_sub(2)).cloned())
                .or_else(|| queue_track_ids.first().cloned())
        } else {
            None
        };
        let should_reveal_anchor = queue_anchor_id.is_some()
            && self.revealed_queue_anchor_id.as_ref() != queue_anchor_id.as_ref();

        ui.add_space(12.0);
        let queue_list_rect = ui.available_rect_before_wrap();
        let user_scrolled_queue = ui.ctx().input(|input| {
            (input.raw_scroll_delta != egui::Vec2::ZERO || input.smooth_scroll_delta != egui::Vec2::ZERO)
                && input
                    .pointer
                    .hover_pos()
                    .map(|position| queue_list_rect.contains(position))
                    .unwrap_or(false)
        });
        if user_scrolled_queue {
            self.queue_follow_paused_by_user = true;
        }

        let mut pending_move = None;

        egui::ScrollArea::vertical()
            .id_salt("queue-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
                        ui.horizontal(|ui| {
                            let drag_id = ui.make_persistent_id(("queue-row", &track.id.0));
                            ui.allocate_ui_with_layout(
                                egui::vec2(28.0, 72.0),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    ui.dnd_drag_source(drag_id, index, |ui| {
                                        ui.add_space(12.0);
                                        ui.monospace("==");
                                        ui.monospace("==");
                                    });
                                });
                            

                            ui.add_space(8.0);
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
                                ui.label(self.track_artist_label(&track));
                                ui.small(track.album.clone().unwrap_or_else(|| String::from("Unknown album")));
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.strong(format!("#{}", index + 1));
                            });
                        });
                    });
                    let _ = inner;
                    if let Some(from_index) = dropped_payload.map(|payload| *payload) {
                        if from_index != index {
                            pending_move = Some((from_index, index));
                        }
                    }
                    ui.add_space(8.0);
                }
            });

        if let Some((from, to)) = pending_move {
            self.state
                .handle(&self.config_store, AppCommand::MoveQueueItem { from, to });
        }

        if should_reveal_anchor {
            self.revealed_queue_anchor_id = queue_anchor_id;
        }
    }

    pub(super) fn render_play_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Play");
        ui.label(self.state.playback_summary());

        ui.add_space(12.0);

        let Some(track) = self.current_track().cloned().or_else(|| self.state.selected_track().cloned()) else {
            ui.label("No track selected.");
            return;
        };

        let available = ui.available_size_before_wrap();
        let art_region_height = (available.y - 180.0).max(220.0);
        let short_edge = available.x.min(art_region_height);
        let art_size = (short_edge * 0.78).clamp(220.0, 520.0).min((available.x - 24.0).max(180.0));
        ui.vertical_centered(|ui| {
            self.render_track_artwork(ui, ctx, &track, art_size);
            ui.add_space(12.0);
            ui.heading(&track.title);
            ui.label(self.track_artist_label(&track));
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

    pub(super) fn render_album_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Album");
        ui.label("Albums are grouped from tags, with tracks available below the selected album.");

        let albums = self.cached_album_views();
        if albums.is_empty() {
            ui.add_space(12.0);
            ui.label("No albums available. Scan at least one directory first.");
            return;
        }

        if self.selected_album_key.is_none() {
            self.selected_album_key = albums.first().map(|album| album.key.clone());
        }

        ui.add_space(12.0);
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical()
                .id_salt("album-grid")
                .show(&mut columns[0], |ui| {
                    let card_width = 200.0;
                    let column_count = ((ui.available_width() + 12.0) / (card_width + 12.0)).floor() as usize;
                    let column_count = column_count.max(1);
                    for row in albums.chunks(column_count) {
                        ui.columns(row.len(), |row_columns| {
                            for (column, album) in row_columns.iter_mut().zip(row.iter()) {
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
                });

            egui::ScrollArea::vertical()
                .id_salt("album-track-list")
                .show(&mut columns[1], |ui| {
                    let Some(selected_album) = albums
                        .iter()
                        .find(|album| self.selected_album_key.as_deref() == Some(album.key.as_str()))
                        .cloned()
                    else {
                        ui.label("Select an album.");
                        return;
                    };

                    ui.heading(format!("Tracks: {}", selected_album.title));
                    ui.small(&selected_album.subtitle);
                    ui.add_space(12.0);

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

                    for track in &tracks {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            let response = ui.selectable_label(
                                self.current_track_id() == Some(&track.id),
                                &track.title,
                            );
                            if response.clicked() {
                                self.state.handle(
                                    &self.config_store,
                                    AppCommand::BuildQueueAndPlay {
                                        track_ids: selected_album.track_ids.clone(),
                                        start_track_id: track.id.clone(),
                                    },
                                );
                                self.active_tab = super::types::AppTab::Play;
                            }
                            self.render_track_context_menu(ui, response, &track.id, &selected_album.track_ids);
                            if let Some(track_number) = track.track_number {
                                ui.small(format!("{:02} / {}", track_number, self.track_secondary_line(track)));
                            } else {
                                ui.small(self.track_secondary_line(track));
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
        });
    }

    pub(super) fn render_artist_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Artist");
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.artist_kind, ArtistKind::Artist, "Artist");
            ui.selectable_value(&mut self.artist_kind, ArtistKind::AlbumArtist, "Album Artist");
            ui.selectable_value(&mut self.artist_kind, ArtistKind::Composer, "Composer");
        });

        let artists = self.cached_artist_views();
        if artists.is_empty() {
            ui.add_space(12.0);
            ui.label("No artist information is available in the scanned library.");
            return;
        }

        if self.selected_artist_name.is_none()
            || !artists
                .iter()
                .any(|artist| self.selected_artist_name.as_deref() == Some(artist.name.as_str()))
        {
            self.selected_artist_name = artists.first().map(|artist| artist.name.clone());
        }

        ui.add_space(12.0);
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                for artist in &artists {
                    let selected = self.selected_artist_name.as_deref() == Some(artist.name.as_str());
                    let response = ui.add_sized(
                        [ui.available_width(), 40.0],
                        egui::Button::new(&artist.name).selected(selected),
                    );
                    if response.clicked() {
                        self.selected_artist_name = Some(artist.name.clone());
                    }
                    ui.add_space(4.0);
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

    pub(super) fn render_playlist_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
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

    pub(super) fn render_library_tab(&mut self, ui: &mut egui::Ui) {
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
            let header = format!(
                "{} ({} folders, {} tracks)",
                root_label,
                root_node.descendant_folder_count(),
                root_node.descendant_track_count()
            );
            egui::CollapsingHeader::new(egui::RichText::new(header).strong())
                .default_open(false)
                .show(ui, |ui| {
                    self.render_directory_node(ui, &root_node, &library_track_ids);
                });
            ui.add_space(8.0);
        }
    }

    pub(super) fn render_scan_tab(&mut self, ui: &mut egui::Ui) {
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

    pub(super) fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.label("Current settings are kept in the local portable data directory.");

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.label("Appearance");
            let mut theme = self.state.config.theme.clone();
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut theme, String::from("system"), "System");
                ui.selectable_value(&mut theme, String::from("light"), "Light");
                ui.selectable_value(&mut theme, String::from("dark"), "Dark");
            });
            if theme != self.state.config.theme {
                self.state.handle(&self.config_store, AppCommand::SetTheme(theme));
            }
        });

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.label("Queue");
            let mut follow_current = self.state.config.queue_follow_current;
            if ui
                .checkbox(&mut follow_current, "Keep the queue aligned to the current track")
                .changed()
            {
                self.state
                    .handle(&self.config_store, AppCommand::SetQueueFollowCurrent(follow_current));
            }
            ui.small("When enabled, the queue keeps the two tracks before the current song near the top.");
        });

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.label("Library");
            let mut show_track_paths = self.state.config.show_library_track_paths;
            if ui
                .checkbox(&mut show_track_paths, "Show audio file paths in the Library tree")
                .changed()
            {
                self.state.handle(
                    &self.config_store,
                    AppCommand::SetShowLibraryTrackPaths(show_track_paths),
                );
            }

            let mut show_lyric_paths = self.state.config.show_library_lyric_paths;
            if ui
                .checkbox(&mut show_lyric_paths, "Show linked LRC paths in the Library tree")
                .changed()
            {
                self.state.handle(
                    &self.config_store,
                    AppCommand::SetShowLibraryLyricPaths(show_lyric_paths),
                );
            }
        });

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
                            self.active_tab = super::types::AppTab::Play;
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
                self.active_tab = super::types::AppTab::Play;
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
            let directory_label = format!(
                "{} ({} folders, {} tracks)",
                directory_name,
                child_node.descendant_folder_count(),
                child_node.descendant_track_count()
            );
            egui::CollapsingHeader::new(egui::RichText::new(directory_label).strong())
                .default_open(false)
                .show(ui, |ui| {
                    self.render_directory_node(ui, child_node, visible_track_ids);
                });
        }

        for track in &node.tracks {
            let selected = self.current_track_id() == Some(&track.id);
            let frame = egui::Frame::group(ui.style()).fill(if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().extreme_bg_color
            });
            frame.show(ui, |ui| {
                let response = ui.selectable_label(selected, &track.title);
                if response.clicked() {
                    self.state.handle(
                        &self.config_store,
                        AppCommand::BuildQueueAndPlay {
                            track_ids: visible_track_ids.to_vec(),
                            start_track_id: track.id.clone(),
                        },
                    );
                    self.active_tab = super::types::AppTab::Play;
                }
                self.render_track_context_menu(ui, response, &track.id, visible_track_ids);
                ui.small(format!("{} / {}", track.artist, track.album));
                if self.state.config.show_library_track_paths {
                    ui.monospace(&track.full_path);
                }
                if self.state.config.show_library_lyric_paths {
                    if let Some(lyric_path) = &track.lyric_path {
                        ui.small(format!("LRC: {lyric_path}"));
                    }
                }
            });
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
                                self.active_tab = super::types::AppTab::Play;
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

    pub(super) fn render_track_detail_window(&mut self, ctx: &egui::Context) {
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

    fn track_artist_label(&self, track: &Track) -> String {
        track.artist.clone().unwrap_or_else(|| String::from("Unknown artist"))
    }

    fn track_secondary_line(&self, track: &Track) -> String {
        let artist = track.artist.clone().unwrap_or_else(|| String::from("Unknown artist"));
        let album = track.album.clone().unwrap_or_else(|| String::from("Unknown album"));
        format!("{artist} / {album}")
    }

    fn library_revision(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.state.config.library_folders.hash(&mut hasher);
        self.state.config.excluded_folders.hash(&mut hasher);
        for track in self.state.library.tracks() {
            track.id.0.hash(&mut hasher);
            track.file_name.hash(&mut hasher);
            track.file_path().hash(&mut hasher);
        }
        hasher.finish()
    }

    fn cached_album_views(&mut self) -> Vec<AlbumView> {
        let revision = self.library_revision();
        if self.cached_library_revision != revision {
            self.cached_albums = self.build_album_views();
            self.cached_library_revision = revision;
            self.cached_artist_revision = 0;
        }
        self.cached_albums.clone()
    }

    fn cached_artist_views(&mut self) -> Vec<ArtistView> {
        let revision = self.library_revision();
        if self.cached_artist_revision != revision || self.cached_artist_kind != self.artist_kind {
            self.cached_artists = self.build_artist_views();
            self.cached_artist_revision = revision;
            self.cached_artist_kind = self.artist_kind;
        }
        self.cached_artists.clone()
    }
}