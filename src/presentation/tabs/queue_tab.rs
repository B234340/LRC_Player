use crate::domain::model::PlaybackMode;
use crate::presentation::egui_app::GuiApp;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    ui.heading("Queue");
    ui.label(format!("{} tracks in queue", app.app_vm.queue.tracks.len()));
    ui.separator();

    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.queue_item_input);
        if ui.button("Add to queue").clicked() {
            app.add_queue_item();
        }
    });

    let mut mode_changed = false;
    ui.horizontal(|ui| {
        ui.label("Playback mode:");
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, PlaybackMode::Sequential, "Sequential")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, PlaybackMode::RepeatOne, "Repeat One")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, PlaybackMode::RepeatAll, "Repeat All")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, PlaybackMode::Shuffle, "Shuffle")
            .changed();
    });
    if mode_changed {
        app.save_state();
    }

    ui.add_space(8.0);

    let tracks = app.app_vm.queue.tracks.clone();
    if tracks.is_empty() {
        ui.label("Queue is empty.");
        return;
    }

    let mut pending_move_up: Option<usize> = None;
    let mut pending_move_down: Option<usize> = None;
    let mut pending_remove: Option<usize> = None;
    let mut pending_play: Option<usize> = None;

    for (index, track) in tracks.iter().enumerate() {
        let is_selected = app.selected_queue_index == Some(index);
        let is_playing = app.current_queue_index == Some(index);
        let path = std::path::Path::new(track);
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(track);
        let location = path
            .parent()
            .and_then(|parent| parent.to_str())
            .unwrap_or("");

        let mut frame = egui::Frame::group(ui.style());
        if is_playing {
            frame = frame.fill(ui.visuals().selection.bg_fill);
        } else if is_selected {
            frame = frame.stroke(egui::Stroke::new(1.0, ui.visuals().widgets.active.bg_stroke.color));
        }

        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("☰");
                ui.vertical(|ui| {
                    let label_text = if is_playing {
                        format!("{}: {} [Playing]", index + 1, file_name)
                    } else {
                        format!("{}: {}", index + 1, file_name)
                    };

                    let response = ui
                        .selectable_label(is_selected, label_text)
                        .on_hover_text("Left click to select, double-click to play, right click to remove");

                    if response.clicked() {
                        app.selected_queue_index = Some(index);
                    }
                    if response.double_clicked() {
                        app.selected_queue_index = Some(index);
                        app.play_selected_queue_item();
                    }
                    if response.secondary_clicked() {
                        pending_remove = Some(index);
                    }

                    if !location.is_empty() {
                        ui.label(location);
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Play").clicked() {
                        pending_play = Some(index);
                    }
                    if ui.button("Remove").clicked() {
                        pending_remove = Some(index);
                    }
                    if ui.button("Up").clicked() {
                        pending_move_up = Some(index);
                    }
                    if ui.button("Down").clicked() {
                        pending_move_down = Some(index);
                    }
                });
            });
        });

        ui.add_space(4.0);
    }

    if let Some(index) = pending_move_up {
        app.move_queue_item_up(index);
    }
    if let Some(index) = pending_move_down {
        app.move_queue_item_down(index);
    }
    if let Some(index) = pending_remove {
        app.remove_queue_item(index);
    }
    if let Some(index) = pending_play {
        app.selected_queue_index = Some(index);
        app.play_selected_queue_item();
    }

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Play selected").clicked() {
            app.play_selected_queue_item();
        }
        if ui.button("Remove selected").clicked() {
            if let Some(index) = app.selected_queue_index {
                app.remove_queue_item(index);
            }
        }
        if ui.button("Clear queue").clicked() {
            app.clear_queue();
        }
    });
}
