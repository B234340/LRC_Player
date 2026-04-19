use eframe::egui;

#[derive(Debug, Clone, Default)]
pub struct TrackContextMenuOutcome {
    pub play_now_path: Option<String>,
    pub next_path: Option<String>,
    pub enqueue_path: Option<String>,
    pub add_to_playlist: Option<(usize, String)>,
    pub show_details_path: Option<String>,
}

pub fn render_track_context_menu<F>(
    ui: &mut egui::Ui,
    track_path: &str,
    playlist_targets: &[(usize, String)],
    enqueue_label: &str,
    extra_items: F,
) -> TrackContextMenuOutcome
where
    F: FnOnce(&mut egui::Ui, &mut TrackContextMenuOutcome),
{
    let mut outcome = TrackContextMenuOutcome::default();

    if ui.button("Play now").clicked() {
        outcome.play_now_path = Some(track_path.to_string());
        ui.close_menu();
    }
    if ui.button("Play next").clicked() {
        outcome.next_path = Some(track_path.to_string());
        ui.close_menu();
    }
    if ui.button(enqueue_label).clicked() {
        outcome.enqueue_path = Some(track_path.to_string());
        ui.close_menu();
    }

    ui.separator();
    ui.label("Add to playlist");
    for (target_index, target_name) in playlist_targets {
        if ui.button(target_name).clicked() {
            outcome.add_to_playlist = Some((*target_index, track_path.to_string()));
            ui.close_menu();
        }
    }

    ui.separator();
    extra_items(ui, &mut outcome);

    if ui.button("Show details").clicked() {
        outcome.show_details_path = Some(track_path.to_string());
        ui.close_menu();
    }

    outcome
}
