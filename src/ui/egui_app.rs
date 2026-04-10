use eframe::egui;
use rfd::FileDialog;

use crate::app::commands::{AppCommand, TrackListOrigin};
use crate::app::view_models::{
    AlbumEntryVm, AppViewModel, ArtistEntryVm, DirectoryEntryVm, PlaylistEntryVm, TrackRowVm,
};
use crate::app::Application;
use crate::core::library::ArtistKind;
use crate::core::persistence::TabId;
use crate::core::scan::ScanCommand;

pub struct LrcPlayerEguiApp {
    application: Application,
    new_playlist_name: String,
}

impl LrcPlayerEguiApp {
    pub fn new(mut application: Application) -> Self {
        if let Err(error) = application.initialize() {
            eprintln!("failed to restore application state: {error}");
        }

        Self {
            application,
            new_playlist_name: String::new(),
        }
    }

    fn top_tabs(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in [
                (TabId::Queue, "Queue"),
                (TabId::Play, "Play"),
                (TabId::Album, "Album"),
                (TabId::Artist, "Artist"),
                (TabId::Playlist, "Playlist"),
                (TabId::Library, "Library"),
                (TabId::Scan, "Scan"),
                (TabId::Settings, "Settings"),
            ] {
                let selected = view_model.current_tab == tab;
                if ui.selectable_label(selected, label).clicked() {
                    self.application.handle_command(AppCommand::SelectTab(tab));
                }
            }
        });
    }

    fn render_track_rows(
        &mut self,
        ui: &mut egui::Ui,
        rows: &[TrackRowVm],
        origin: TrackListOrigin,
    ) {
        let visible_tracks: Vec<_> = rows.iter().map(|row| row.track_id).collect();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, row) in rows.iter().enumerate() {
                ui.horizontal(|ui| {
                    let playing = if row.is_current { "▶" } else { " " };
                    if ui
                        .selectable_label(
                            row.is_current,
                            format!("{} {} - {} [{}]", playing, row.title, row.artist, row.album),
                        )
                        .clicked()
                    {
                        self.application.handle_command(AppCommand::ClickTrack {
                            visible_tracks: visible_tracks.clone(),
                            clicked_index: index,
                            origin,
                        });
                    }

                    let favorite_label = match row.favorite_state {
                        crate::app::view_models::FavoriteStateVm::Included => "★",
                        crate::app::view_models::FavoriteStateVm::Excluded => "☆",
                    };
                    if ui.button(favorite_label).clicked() {
                        self.application
                            .handle_command(AppCommand::ToggleFavorite { track_id: row.track_id });
                    }
                });
            }
        });
    }

    fn render_queue_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.horizontal(|ui| {
            ui.label(format!("Mode: {:?}", view_model.queue.repeat_mode));
            if ui.button("Previous").clicked() {
                self.application.handle_command(AppCommand::PreviousTrack);
            }
            if ui.button("Play/Pause").clicked() {
                self.application.handle_command(AppCommand::TogglePlayPause);
            }
            if ui.button("Next").clicked() {
                self.application.handle_command(AppCommand::NextTrack);
            }
        });
        self.render_track_rows(ui, &view_model.queue.rows, TrackListOrigin::Queue);
    }

    fn render_play_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.heading("Now Playing");
        if let Some(track) = &view_model.play.current_track {
            ui.label(&track.title);
            ui.label(format!("{} / {}", track.artist, track.album));
        } else {
            ui.label("No track selected");
        }

        ui.label(format!(
            "State: {:?} | Position: {} ms",
            view_model.play.playback_state, view_model.play.position_ms
        ));

        ui.horizontal(|ui| {
            if ui.button("-10s").clicked() {
                self.application.handle_command(AppCommand::SeekBy { delta_ms: -10_000 });
            }
            if ui.button("Play/Pause").clicked() {
                self.application.handle_command(AppCommand::TogglePlayPause);
            }
            if ui.button("+10s").clicked() {
                self.application.handle_command(AppCommand::SeekBy { delta_ms: 10_000 });
            }
        });
    }

    fn render_album_entry(&mut self, ui: &mut egui::Ui, album: &AlbumEntryVm, expanded: bool) {
        if ui
            .selectable_label(
                expanded,
                format!("{} - {} ({} tracks)", album.title, album.album_artist, album.track_count),
            )
            .clicked()
        {
            let next = if expanded { None } else { Some(album.album_id) };
            self.application.handle_command(AppCommand::SelectAlbum(next));
        }
    }

    fn render_album_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                for album in &view_model.album.albums {
                    self.render_album_entry(ui, album, view_model.album.expanded_album == Some(album.album_id));
                }
            });
            self.render_track_rows(&mut columns[1], &view_model.album.visible_tracks, TrackListOrigin::Album);
        });
    }

    fn render_artist_entry(&mut self, ui: &mut egui::Ui, artist: &ArtistEntryVm, selected: bool) {
        if ui
            .selectable_label(selected, format!("{} ({})", artist.name, artist.track_count))
            .clicked()
        {
            let next = if selected { None } else { Some(artist.artist_id) };
            self.application.handle_command(AppCommand::SelectArtist(next));
        }
    }

    fn render_artist_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.horizontal(|ui| {
            for (kind, label) in [
                (ArtistKind::TrackArtist, "Artist"),
                (ArtistKind::AlbumArtist, "Album Artist"),
                (ArtistKind::Composer, "Composer"),
            ] {
                if ui.selectable_label(view_model.artist.kind == kind, label).clicked() {
                    self.application.handle_command(AppCommand::SelectArtistKind(kind));
                }
            }
        });

        ui.columns(2, |columns| {
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                for artist in &view_model.artist.artists {
                    self.render_artist_entry(
                        ui,
                        artist,
                        view_model.artist.selected_artist == Some(artist.artist_id),
                    );
                }
            });
            self.render_track_rows(&mut columns[1], &view_model.artist.visible_tracks, TrackListOrigin::Artist);
        });
    }

    fn render_playlist_entry(&mut self, ui: &mut egui::Ui, playlist: &PlaylistEntryVm, selected: bool) {
        let label = if playlist.is_system {
            format!("{} ({}) [system]", playlist.name, playlist.track_count)
        } else {
            format!("{} ({})", playlist.name, playlist.track_count)
        };
        if ui.selectable_label(selected, label).clicked() {
            self.application
                .handle_command(AppCommand::SelectPlaylist(Some(playlist.playlist_id)));
        }
    }

    fn render_playlist_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.new_playlist_name);
            if ui.button("Create").clicked() && !self.new_playlist_name.trim().is_empty() {
                self.application.handle_command(AppCommand::CreatePlaylist {
                    name: self.new_playlist_name.trim().to_string(),
                });
                self.new_playlist_name.clear();
            }
        });

        ui.columns(2, |columns| {
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                for playlist in &view_model.playlist.playlists {
                    self.render_playlist_entry(
                        ui,
                        playlist,
                        view_model.playlist.selected_playlist == Some(playlist.playlist_id),
                    );
                }
            });
            self.render_track_rows(&mut columns[1], &view_model.playlist.visible_tracks, TrackListOrigin::Playlist);
        });
    }

    fn render_library_entry(&mut self, ui: &mut egui::Ui, node: &DirectoryEntryVm, selected: bool) {
        if ui
            .selectable_label(
                selected,
                format!("{} ({}/{} )", node.name, node.child_count, node.track_count),
            )
            .clicked()
        {
            let next = if selected { None } else { Some(node.node_id) };
            self.application.handle_command(AppCommand::SelectDirectory(next));
        }
    }

    fn render_library_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical().show(&mut columns[0], |ui| {
                for node in &view_model.library.nodes {
                    self.render_library_entry(
                        ui,
                        node,
                        view_model.library.selected_directory == Some(node.node_id),
                    );
                }
            });
            self.render_track_rows(&mut columns[1], &view_model.library.visible_tracks, TrackListOrigin::Library);
        });
    }

    fn render_scan_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.horizontal(|ui| {
            if ui.button("Start Scan").clicked() {
                self.application
                    .handle_command(AppCommand::Scan(ScanCommand::StartFullScan));
            }
            if view_model.scan.cancel_visible && ui.button("Cancel").clicked() {
                self.application.handle_command(AppCommand::Scan(ScanCommand::Stop));
            }
            ui.label(format!("Status: {:?}", view_model.scan.visible_status));
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.heading("Include directories");
            if ui.button("Add...").clicked() {
                self.pick_scan_directory(true);
            }
        });
        for path in &view_model.scan.include_dirs {
            ui.horizontal(|ui| {
                ui.label(path);
                if ui.button("Remove").clicked() {
                    self.application.handle_command(AppCommand::Scan(
                        ScanCommand::RemoveIncludeDir(path.into()),
                    ));
                }
            });
        }
        if view_model.scan.include_dirs.is_empty() {
            ui.label("No include directories configured");
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.heading("Exclude directories");
            if ui.button("Add...").clicked() {
                self.pick_scan_directory(false);
            }
        });
        for path in &view_model.scan.exclude_dirs {
            ui.horizontal(|ui| {
                ui.label(path);
                if ui.button("Remove").clicked() {
                    self.application.handle_command(AppCommand::Scan(
                        ScanCommand::RemoveExcludeDir(path.into()),
                    ));
                }
            });
        }
        if view_model.scan.exclude_dirs.is_empty() {
            ui.label("No exclude directories configured");
        }
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        egui::Grid::new("settings-grid").striped(true).show(ui, |ui| {
            for item in &view_model.settings.items {
                ui.label(&item.key);
                ui.label(&item.value);
                ui.end_row();
            }
        });
    }

    fn pick_scan_directory(&mut self, include: bool) {
        let Some(path) = FileDialog::new().pick_folder() else {
            return;
        };

        let command = if include {
            ScanCommand::AddIncludeDir(path)
        } else {
            ScanCommand::AddExcludeDir(path)
        };
        self.application.handle_command(AppCommand::Scan(command));
    }
}

impl eframe::App for LrcPlayerEguiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let view_model = self.application.view_model();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            self.top_tabs(ui, &view_model);
        });

        egui::CentralPanel::default().show(ctx, |ui| match view_model.current_tab {
            TabId::Queue => self.render_queue_tab(ui, &view_model),
            TabId::Play => self.render_play_tab(ui, &view_model),
            TabId::Album => self.render_album_tab(ui, &view_model),
            TabId::Artist => self.render_artist_tab(ui, &view_model),
            TabId::Playlist => self.render_playlist_tab(ui, &view_model),
            TabId::Library => self.render_library_tab(ui, &view_model),
            TabId::Scan => self.render_scan_tab(ui, &view_model),
            TabId::Settings => self.render_settings_tab(ui, &view_model),
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(error) = self.application.persist() {
            eprintln!("failed to persist application state: {error}");
        }
    }
}