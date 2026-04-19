use crate::infrastructure::persistence::MetaCache;
use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackActivation {
    SingleClick,
    DoubleClick,
}

#[derive(Debug, Clone)]
pub struct TrackDisplay {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
}

#[derive(Debug, Clone, Default)]
pub struct TrackListOutcome {
    pub play_index: Option<usize>,
    pub play_list: Vec<String>,
    pub play_now_path: Option<String>,
    pub next_path: Option<String>,
    pub enqueue_path: Option<String>,
    pub add_to_playlist: Option<(usize, String)>,
    pub show_details_path: Option<String>,
}

fn resolve_track_display(
    track_path: &str,
    meta_cache: Option<&MetaCache>,
    metadata_cache: &mut HashMap<String, (String, String, String)>,
) -> TrackDisplay {
    let fallback_title = Path::new(track_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(track_path)
        .to_string();

    let (title, artist, album) = metadata_cache
        .entry(track_path.to_string())
        .or_insert_with(|| {
            meta_cache
                .and_then(|cache| cache.track_metadata_by_location(track_path).ok().flatten())
                .map(|(title, artist, album)| {
                    let title = if title.trim().is_empty() {
                        fallback_title.clone()
                    } else {
                        title
                    };
                    let artist = if artist.trim().is_empty() {
                        "Unknown artist".to_string()
                    } else {
                        artist
                    };
                    let album = if album.trim().is_empty() {
                        "Unknown album".to_string()
                    } else {
                        album
                    };
                    (title, artist, album)
                })
                .unwrap_or_else(|| {
                    (
                        fallback_title.clone(),
                        "Unknown artist".to_string(),
                        "Unknown album".to_string(),
                    )
                })
        })
        .clone();

    TrackDisplay {
        path: track_path.to_string(),
        title,
        artist,
        album,
    }
}

pub fn render_standard_track_list<F>(
    ui: &mut egui::Ui,
    app: &mut GuiApp,
    id_source: &str,
    tracks: &[String],
    playlist_targets: &[(usize, String)],
    activation: TrackActivation,
    row_height: f32,
    max_height: Option<f32>,
    mut label_for: F,
) -> TrackListOutcome
where
    F: FnMut(usize, &TrackDisplay) -> String,
{
    let mut outcome = TrackListOutcome::default();
    let mut metadata_cache: HashMap<String, (String, String, String)> = HashMap::new();
    let meta_cache = MetaCache::open_default().ok();
    let displayed_tracks = tracks.to_vec();

    let mut render_rows = |ui: &mut egui::Ui| {
        for (index, track) in tracks.iter().enumerate() {
            let display = resolve_track_display(track, meta_cache.as_ref(), &mut metadata_cache);
            let label = label_for(index, &display);

            ui.horizontal(|ui| {
                let is_favorite = app.is_track_favorite(track);
                let star_label = if is_favorite { "★" } else { "☆" };
                let star_area_height = row_height.max(22.0);
                let vertical_pad = ((star_area_height - 22.0) * 0.5).max(0.0);
                let toggle_response = ui
                    .allocate_ui_with_layout(
                        egui::vec2(28.0, star_area_height),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.add_space(vertical_pad);
                            ui.add_sized([22.0, 22.0], egui::Button::new(star_label))
                        },
                    )
                    .inner
                    .on_hover_text("Toggle favorite");

                if toggle_response.clicked() {
                    app.toggle_favorite_track(track);
                }

                ui.add_space(4.0);
                let response = ui.selectable_label(false, label);

                let activated = match activation {
                    TrackActivation::SingleClick => response.clicked(),
                    TrackActivation::DoubleClick => response.double_clicked(),
                };
                if activated {
                    outcome.play_index = Some(index);
                    outcome.play_list = displayed_tracks.clone();
                }

                response.context_menu(|ui| {
                    if ui.button("Play now").clicked() {
                        outcome.play_now_path = Some(track.clone());
                        ui.close_menu();
                    }
                    if ui.button("Play next").clicked() {
                        outcome.next_path = Some(track.clone());
                        ui.close_menu();
                    }
                    if ui.button("Add to queue").clicked() {
                        outcome.enqueue_path = Some(track.clone());
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label("Add to playlist");
                    for (target_index, target_name) in playlist_targets {
                        if ui.button(target_name).clicked() {
                            outcome.add_to_playlist = Some((*target_index, track.clone()));
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("Show details").clicked() {
                        outcome.show_details_path = Some(track.clone());
                        ui.close_menu();
                    }
                });
            });
        }
    };

    let mut area = egui::ScrollArea::vertical()
        .id_source(id_source)
        .auto_shrink([false, false]);
    if let Some(height) = max_height {
        area = area.max_height(height);
    }
    area.show(ui, |ui| {
        render_rows(ui);
    });

    outcome
}
