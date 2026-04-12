//! UI 層（egui）アプリケーション
//!
//! - `LrcPlayerEguiApp` は `eframe::App` を実装し、ViewModel を描画してユーザー操作を `AppCommand` として発行します。
//! - UI 側の振る舞いは軽量に保ち、状態や永続化は `Application` / `AppFacade` に委譲します。
//!
//! 注: 多くのレンダリングヘルパー関数（タブごとの描画、行レンダリング、フォントインストールなど）を含む大きなファイルです。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::{CreationContext, egui};
use rfd::FileDialog;

use crate::app::Application;
use crate::app::commands::{AppCommand, TrackListOrigin};
use crate::app::view_models::{
    AlbumEntryVm, AppViewModel, ArtistEntryVm, DirectoryEntryVm, PlaylistEntryVm, TrackRowVm,
};
use crate::core::library::ArtistKind;
use crate::core::ids::{AlbumId, ArtistId};
use crate::core::persistence::TabId;
use crate::core::scan::ScanCommand;

/// egui ベースのアプリケーション実装。
///
/// - `application` は内部ロジックのエントリ（`Application`）で、UI はここにコマンドを渡す。
/// - `last_highlighted_*` は UI のログ目的で、最後に選択された要素のトレースに使う。
/// - `cached_visible_tracks`: トラック ID リストのキャッシュ。再描画時のクローンを避けるために使用。
pub struct LrcPlayerEguiApp {
    application: Application,
    new_playlist_name: String,
    last_highlighted_album: Option<AlbumId>,
    last_highlighted_artist: Option<ArtistId>,
    cached_visible_tracks: Option<(TrackListOrigin, Vec<crate::core::ids::TrackId>)>,
}

impl LrcPlayerEguiApp {
    pub fn new(cc: &CreationContext<'_>, mut application: Application) -> Self {
        // フォントの補完（CJK）を行い、表示崩れを低減
        install_cjk_fonts(&cc.egui_ctx);

        // アプリの永続化状態を復元する（失敗しても UI は起動する）
        if let Err(error) = application.initialize() {
            eprintln!("failed to restore application state: {error}");
        }

        Self {
            application,
            new_playlist_name: String::new(),
            last_highlighted_album: None,
            last_highlighted_artist: None,
            cached_visible_tracks: None,
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

    /// 上部タブ（Queue/Play/Album/...）を描画して、選択時に `SelectTab` コマンドを送る。
    
    /// トラック行リストを描画するヘルパー。
    /// - `rows` は UI 表示用のスナップショットで、フレーム内での行インデックスは安定していることを前提とする。
    /// - クリック処理では `visible_tracks` のクローンと `clicked_index` を渡し、アプリ側がモデルの並行変更に影響されず動作できるようにする。
    /// - `origin` はトラックリストの発生元（Album/Artist/Playlist/...）を示す。コマンドはこれに基づき振る舞いを変える。
    /// - トラックIDリストのキャッシュを使用して、毎フレームのクローンを避ける。
    fn render_track_rows(
        &mut self,
        ui: &mut egui::Ui,
        rows: &[TrackRowVm],
        origin: TrackListOrigin,
    ) {
        // キャッシュを確認し、変更がなければ再利用する
        let visible_tracks = if let Some((cached_origin, cached)) = &self.cached_visible_tracks {
            if *cached_origin == origin && cached.len() == rows.len() {
                cached.clone()
            } else {
                let new_cache: Vec<_> = rows.iter().map(|row| row.track_id).collect();
                self.cached_visible_tracks = Some((origin, new_cache.clone()));
                new_cache
            }
        } else {
            let new_cache: Vec<_> = rows.iter().map(|row| row.track_id).collect();
            self.cached_visible_tracks = Some((origin, new_cache.clone()));
            new_cache
        };

        let row_height = ui.text_style_height(&egui::TextStyle::Body)
            + ui.spacing().button_padding.y * 2.0;
        egui::ScrollArea::vertical().show_rows(
            ui,
            row_height,
            rows.len(),
            |ui, range| {
                for i in range {
                    let row = &rows[i];
                    ui.horizontal(|ui| {
                        // 再生中を示すマーカー
                        let playing = if row.is_current { "▶" } else { " " };
                        if ui
                            .selectable_label(
                                row.is_current,
                                format!("{} {} - {} [{}]", playing, row.title, row.artist, row.album),
                            )
                            .clicked()
                        {
                            // トラッククリック -> visible_tracks + index を渡して再生等の動作をトリガー
                            self.application.handle_command(AppCommand::ClickTrack {
                                visible_tracks: visible_tracks.clone(),
                                clicked_index: i,
                                origin,
                            });
                        }

                        // お気に入りトグル
                        let favorite_label = match row.favorite_state {
                            crate::app::view_models::FavoriteStateVm::Included => "★",
                            crate::app::view_models::FavoriteStateVm::Excluded => "☆",
                        };
                        if ui.button(favorite_label).clicked() {
                            self.application.handle_command(AppCommand::ToggleFavorite {
                                track_id: row.track_id,
                            });
                        }
                    });
                }
            },
        );
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

    /// 再生キュータブの描画（プレイコントロール + キューのトラック行）

    fn render_play_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.heading("Now Playing");
        if let Some(track) = &view_model.play.current_track {
            ui.label(&*track.title);
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
                self.application
                    .handle_command(AppCommand::SeekBy { delta_ms: -10_000 });
            }
            if ui.button("Play/Pause").clicked() {
                self.application.handle_command(AppCommand::TogglePlayPause);
            }
            if ui.button("+10s").clicked() {
                self.application
                    .handle_command(AppCommand::SeekBy { delta_ms: 10_000 });
            }
        });
    }

    /// Now Playing タブの描画：現在トラック情報と基本的なシーク/再生操作を提供

    /// アルバム一覧の1エントリを描画する。
    /// - `expanded` が true のとき選択表示（右カラムにトラックを表示）。
    /// - `ui.allocate_exact_size` で矩形領域を確保し、その領域全体でクリックを受け取る。
    /// - クリックで `AppCommand::SelectAlbum` を発行してトグル選択を行う。
    /// - `last_highlighted_album` はデバッグ/ログ表示のため最後にハイライトされた ID を保持する。
    fn render_album_entry(&mut self, ui: &mut egui::Ui, album: &AlbumEntryVm, expanded: bool) {
        let log_label = if album.is_provisional { " [log]" } else { "" };
        let text = format!(
            "{} - {} ({} tracks){}",
            &*album.title,
            &*album.album_artist,
            album.track_count,
            log_label
        );
        // ラベルの高さを計測して、行全体の矩形を確保し、その矩形でクリックを受け取る
        let label_height = ui.text_style_height(&egui::TextStyle::Body);
        let desired_height = label_height + ui.spacing().button_padding.y * 2.0;
        let desired_size = egui::Vec2::new(ui.available_width(), desired_height);
        // 領域を確保して矩形を得る（この矩形全体がクリック可能になる）
        let (rect, reserve) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        let response = ui.put(rect, egui::SelectableLabel::new(expanded, text));
        // When an album becomes highlighted (expanded) update last_highlighted_album for tracing
        if expanded && self.last_highlighted_album != Some(album.album_id) {
            self.last_highlighted_album = Some(album.album_id);
        }
        // クリックされたら選択状態のトグルを行い、AppCommand を発行する
        if response.clicked() || reserve.clicked() {
            let next = if expanded { None } else { Some(album.album_id) };
            self.application
                .handle_command(AppCommand::SelectAlbum(next));
            ui.ctx().request_repaint();
        }
    }

    /// アルバムタブ全体を描画する。
    /// - 左カラム: アルバム一覧（`render_album_entry` を使用）。
    /// - 右カラム: 選択アルバムのトラックと読み込み進捗表示（`render_track_rows` を使用）。
    fn render_album_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.horizontal(|ui| {
            ui.label(format!(
                "Albums hydrated: {}/{}",
                view_model.album.loading.hydrated_entries, view_model.album.loading.total_entries
            ));
            if view_model.album.loading.is_loading {
                ui.label("using log snapshot while background hydration continues");
            }
        });

        ui.columns(2, |columns| {
            let row_height = columns[0].text_style_height(&egui::TextStyle::Body)
                + columns[0].spacing().button_padding.y * 2.0;
            egui::ScrollArea::vertical().show_rows(
                &mut columns[0],
                row_height,
                view_model.album.albums.len(),
                |ui, range| {
                    for i in range {
                        let album = &view_model.album.albums[i];
                        self.render_album_entry(
                            ui,
                            album,
                            view_model.album.expanded_album == Some(album.album_id),
                        );
                    }
                },
            );
            // Right column: show loading progress for tracks if background hydrate is in progress
            egui::ScrollArea::vertical().show(&mut columns[1], |ui| {
                if view_model.album.tracks_loading {
                    if view_model.album.tracks_total > 0 {
                        let fraction = view_model.album.tracks_loaded as f32
                            / view_model.album.tracks_total as f32;
                        ui.label(format!(
                            "Loading tracks: {}/{}",
                            view_model.album.tracks_loaded, view_model.album.tracks_total
                        ));
                        ui.add(egui::ProgressBar::new(fraction).show_percentage());
                    } else {
                        ui.label("Loading tracks...");
                    }
                    ui.separator();
                }

                self.render_track_rows(
                    ui,
                    &view_model.album.visible_tracks,
                    TrackListOrigin::Album,
                );
            });
        });
    }

    /// アーティスト一覧の1エントリを描画する。
    /// - `selected` が true のとき選択表示（右カラムにトラックを表示）。
    /// - `ui.allocate_exact_size` で矩形領域を確保し、その領域全体でクリックを受け取る。
    /// - クリックで `AppCommand::SelectArtist` を発行してトグル選択を行う。
    /// - `last_highlighted_artist` はデバッグ/ログ表示のため最後にハイライトされた ID を保持する。
    fn render_artist_entry(&mut self, ui: &mut egui::Ui, artist: &ArtistEntryVm, selected: bool) {
        let log_label = if artist.is_provisional { " [log]" } else { "" };
        let text = format!("{} ({}){}", &*artist.name, artist.track_count, log_label);
        let label_height = ui.text_style_height(&egui::TextStyle::Body);
        let desired_height = label_height + ui.spacing().button_padding.y * 2.0;
        let desired_size = egui::Vec2::new(ui.available_width(), desired_height);
        let (rect, reserve) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        let response = ui.put(rect, egui::SelectableLabel::new(selected, text));
        // When an artist becomes highlighted (selected)
        if selected && self.last_highlighted_artist != Some(artist.artist_id) {
            self.last_highlighted_artist = Some(artist.artist_id);
        }
        if response.clicked() || reserve.clicked() {
            let next = if selected {
                None
            } else {
                Some(artist.artist_id)
            };
            self.application
                .handle_command(AppCommand::SelectArtist(next));
            ui.ctx().request_repaint();
        }
    }

    /// アーティストタブ全体を描画する。
    /// - 上部で `ArtistKind` を切り替え、左カラムにアーティスト一覧、右カラムに選択アーティストのトラックを表示する。
    /// - トラック読み込み中は進捗表示を行い、`render_track_rows` で行を描画する。
    fn render_artist_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.horizontal(|ui| {
            for (kind, label) in [
                (ArtistKind::TrackArtist, "Artist"),
                (ArtistKind::AlbumArtist, "Album Artist"),
                (ArtistKind::Composer, "Composer"),
            ] {
                if ui
                    .selectable_label(view_model.artist.kind == kind, label)
                    .clicked()
                {
                    self.application
                        .handle_command(AppCommand::SelectArtistKind(kind));
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label(format!(
                "Artists hydrated: {}/{}",
                view_model.artist.loading.hydrated_entries, view_model.artist.loading.total_entries
            ));
            if view_model.artist.loading.is_loading {
                ui.label("using log snapshot while background hydration continues");
            }
        });

        ui.columns(2, |columns| {
            let row_height = columns[0].text_style_height(&egui::TextStyle::Body)
                + columns[0].spacing().button_padding.y * 2.0;
            egui::ScrollArea::vertical().show_rows(
                &mut columns[0],
                row_height,
                view_model.artist.artists.len(),
                |ui, range| {
                    for i in range {
                        let artist = &view_model.artist.artists[i];
                        self.render_artist_entry(
                            ui,
                            artist,
                            view_model.artist.selected_artist == Some(artist.artist_id),
                        );
                    }
                },
            );
            egui::ScrollArea::vertical().show(&mut columns[1], |ui| {
                if view_model.artist.tracks_loading {
                    if view_model.artist.tracks_total > 0 {
                        let fraction = view_model.artist.tracks_loaded as f32
                            / view_model.artist.tracks_total as f32;
                        ui.label(format!(
                            "Loading tracks: {}/{}",
                            view_model.artist.tracks_loaded, view_model.artist.tracks_total
                        ));
                        ui.add(egui::ProgressBar::new(fraction).show_percentage());
                    } else {
                        ui.label("Loading tracks...");
                    }
                    ui.separator();
                }

                self.render_track_rows(
                    ui,
                    &view_model.artist.visible_tracks,
                    TrackListOrigin::Artist,
                );
            });
        });
    }

    fn render_playlist_entry(
        &mut self,
        ui: &mut egui::Ui,
        playlist: &PlaylistEntryVm,
        selected: bool,
    ) {
        let label = if playlist.is_system {
            format!("{} ({}) [system]", &*playlist.name, playlist.track_count)
        } else {
            format!("{} ({})", &*playlist.name, playlist.track_count)
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
            self.render_track_rows(
                &mut columns[1],
                &view_model.playlist.visible_tracks,
                TrackListOrigin::Playlist,
            );
        });
    }

    fn render_library_entry(&mut self, ui: &mut egui::Ui, node: &DirectoryEntryVm, selected: bool) {
        if ui
            .selectable_label(
                selected,
                format!("{} ({}/{} )", &*node.name, node.child_count, node.track_count),
            )
            .clicked()
        {
            let next = if selected { None } else { Some(node.node_id) };
            self.application
                .handle_command(AppCommand::SelectDirectory(next));
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
            self.render_track_rows(
                &mut columns[1],
                &view_model.library.visible_tracks,
                TrackListOrigin::Library,
            );
        });
    }

    fn render_scan_tab(&mut self, ui: &mut egui::Ui, view_model: &AppViewModel) {
        ui.horizontal(|ui| {
            if ui.button("Start Scan").clicked() {
                self.application
                    .handle_command(AppCommand::Scan(ScanCommand::StartFullScan));
            }
            if view_model.scan.cancel_visible && ui.button("Cancel").clicked() {
                self.application
                    .handle_command(AppCommand::Scan(ScanCommand::Stop));
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
        egui::Grid::new("settings-grid")
            .striped(true)
            .show(ui, |ui| {
                for item in &view_model.settings.items {
                    ui.label(&item.key);
                    ui.label(&item.value);
                    ui.end_row();
                }
            });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Output device");
            let selected = view_model.settings.preferred_output_device.clone();
            let selected_text = selected.clone().unwrap_or_else(|| "Default".to_string());
            egui::ComboBox::from_label("")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(selected.is_none(), "Default").clicked() {
                        self.application
                            .handle_command(AppCommand::SetPreferredOutputDevice {
                                device_name: None,
                            });
                    }

                    for device in &view_model.settings.available_output_devices {
                        if ui
                            .selectable_label(selected == Some(device.clone()), device)
                            .clicked()
                        {
                            self.application
                                .handle_command(AppCommand::SetPreferredOutputDevice {
                                    device_name: Some(device.clone()),
                                });
                        }
                    }
                });
        });

        // Debug mode toggle
        ui.separator();
        ui.horizontal(|ui| {
            let mut debug_enabled = view_model.settings.debug_mode;
            if ui.checkbox(&mut debug_enabled, "Enable debug mode").changed() {
                self.application
                    .handle_command(AppCommand::SetDebugMode { enabled: debug_enabled });
            }
        });
        // When debug mode is enabled, allow selecting which log categories to output.
        if view_model.settings.debug_mode {
            ui.horizontal(|ui| {
                ui.label("Log filters");
                let mut selected: Vec<String> = view_model.settings.selected_log_filters.clone();
                let categories = &view_model.settings.available_log_categories;
                let display = if selected.is_empty() { "All".to_string() } else { selected.join(", ") };
                egui::ComboBox::from_label("")
                    .selected_text(display)
                    .show_ui(ui, |ui| {
                        for cat in categories.iter() {
                            let mut checked = selected.contains(cat);
                            if ui.checkbox(&mut checked, cat).changed() {
                                if checked {
                                    if !selected.contains(cat) {
                                        selected.push(cat.clone());
                                    }
                                } else {
                                    selected.retain(|s| s != cat);
                                }
                                self.application.handle_command(AppCommand::SetLogFilters { filters: selected.clone() });
                            }
                        }
                    });
            });
        }
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

fn install_cjk_fonts(context: &egui::Context) {
    let mut font_definitions = egui::FontDefinitions::default();
    let mut installed_font_names = Vec::new();

    for path in candidate_cjk_font_paths() {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let font_name = format!("cjk-{file_name}");
        if font_definitions.font_data.contains_key(&font_name) {
            continue;
        }

        let Ok(bytes) = fs::read(&path) else {
            continue;
        };

        font_definitions
            .font_data
            .insert(font_name.clone(), egui::FontData::from_owned(bytes).into());
        installed_font_names.push(font_name);
    }

    if installed_font_names.is_empty() {
        return;
    }

    if let Some(proportional) = font_definitions
        .families
        .get_mut(&egui::FontFamily::Proportional)
    {
        for font_name in installed_font_names.iter().rev() {
            proportional.insert(0, font_name.clone());
        }
    }
    if let Some(monospace) = font_definitions
        .families
        .get_mut(&egui::FontFamily::Monospace)
    {
        for font_name in installed_font_names.iter().rev() {
            monospace.insert(0, font_name.clone());
        }
    }

    context.set_fonts(font_definitions);
}

fn candidate_cjk_font_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for path in [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansJP-Regular.otf",
        "/usr/share/fonts/opentype/noto/NotoSansSC-Regular.otf",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/usr/share/fonts/truetype/arphic/ukai.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/Hiragino Sans W3.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
        "C:/Windows/Fonts/msgothic.ttc",
        "C:/Windows/Fonts/YuGothR.ttc",
        "C:/Windows/Fonts/meiryo.ttc",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simsun.ttc",
        "C:/Windows/Fonts/msjh.ttc",
    ] {
        push_if_exists(&mut candidates, path);
    }

    candidates
}

fn push_if_exists(paths: &mut Vec<PathBuf>, candidate: impl AsRef<Path>) {
    let path = candidate.as_ref();
    if path.exists() {
        paths.push(path.to_path_buf());
    }
}

impl eframe::App for LrcPlayerEguiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.application.poll_background_tasks();
        
        // ViewModel はキャッシュから取得（内部で dirty フラグに基づき必要時のみ再構築）
        let view_model = self.application.view_model().clone();

        if view_model.scan.cancel_visible
            || view_model.album.loading.is_loading
            || view_model.artist.loading.is_loading
            || matches!(
                view_model.play.playback_state,
                crate::core::playback::PlaybackState::Playing
                    | crate::core::playback::PlaybackState::Buffering
                    | crate::core::playback::PlaybackState::Seeking
            )
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }

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