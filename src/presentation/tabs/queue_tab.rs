use crate::domain::model::PlaybackMode;
use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use std::time::{Duration, Instant};

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    let (ctrl_pressed, shift_pressed, esc_pressed) = ui.ctx().input(|input| {
        (
            input.modifiers.ctrl,
            input.modifiers.shift,
            input.key_pressed(egui::Key::Escape),
        )
    });
    if esc_pressed {
        app.clear_queue_selection();
        app.cancel_queue_drag();
    }

    if !ui.ctx().input(|input| input.pointer.primary_down()) {
        app.clear_queue_handle_hold();
    }

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

    if let Some(source) = app.queue_drag_source_index {
        ui.label(format!(
            "Moving track {}. Click a destination row/handle to place, or click the same row/handle to cancel.",
            source + 1
        ));
    }

    let tracks = app.app_vm.queue.tracks.clone();
    if tracks.is_empty() {
        ui.label("Queue is empty.");
        return;
    }

    let mut pending_move_up: Option<usize> = None;
    let mut pending_move_down: Option<usize> = None;
    let mut pending_remove: Option<usize> = None;
    let mut pending_play: Option<usize> = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, track) in tracks.iter().enumerate() {
                let is_selected = app.is_queue_item_selected(index);
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
                if app.queue_drag_source_index == Some(index) {
                    frame = frame.stroke(egui::Stroke::new(2.0, ui.visuals().warn_fg_color));
                }

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let handle_label = if app.queue_drag_source_index == Some(index) {
                            "⇅"
                        } else {
                            "☰"
                        };
                        let handle_response = ui
                            .button(handle_label)
                            .on_hover_text("Handle: click to select. Hold 350ms to start moving one track.");

                        if let Some(source) = app.queue_drag_source_index {
                            if handle_response.clicked() {
                                if source == index {
                                    app.cancel_queue_drag();
                                } else {
                                    app.move_queue_item_to(source, index);
                                    app.cancel_queue_drag();
                                }
                            }
                        } else {
                            if handle_response.is_pointer_button_down_on() {
                                if app.queue_handle_hold_index != Some(index) {
                                    app.queue_handle_hold_index = Some(index);
                                    app.queue_handle_hold_started_at = Some(Instant::now());
                                }

                                if let Some(started_at) = app.queue_handle_hold_started_at {
                                    let elapsed = started_at.elapsed();
                                    let hold_delay = Duration::from_millis(350);
                                    if elapsed >= hold_delay {
                                        app.queue_drag_source_index = Some(index);
                                        app.select_queue_item_single(index);
                                        app.clear_queue_handle_hold();
                                    } else {
                                        ui.ctx().request_repaint_after(hold_delay - elapsed);
                                    }
                                }
                            }

                            if handle_response.clicked() {
                                if shift_pressed {
                                    app.select_queue_item_range_to(index);
                                } else if ctrl_pressed {
                                    app.toggle_queue_item_selection(index);
                                } else {
                                    app.select_queue_item_single(index);
                                }
                            }
                        }

                        ui.vertical(|ui| {
                            let label_text = if is_playing {
                                format!("{}: {} [Playing]", index + 1, file_name)
                            } else {
                                format!("{}: {}", index + 1, file_name)
                            };

                            let response = ui
                                .selectable_label(is_selected, label_text)
                                .on_hover_text("Left click: play now (queue order unchanged). Right click: remove.");

                            if response.clicked() {
                                if let Some(source) = app.queue_drag_source_index {
                                    if source == index {
                                        app.cancel_queue_drag();
                                    } else {
                                        app.move_queue_item_to(source, index);
                                        app.cancel_queue_drag();
                                    }
                                } else {
                                    app.select_queue_item_single(index);
                                    app.play_selected_queue_item();
                                }
                            }
                            if response.secondary_clicked() {
                                pending_remove = Some(index);
                            }

                            if !location.is_empty() {
                                ui.label(location);
                            }

                            if is_playing && app.queue_scroll_to_playing {
                                response.scroll_to_me(Some(egui::Align::Center));
                                app.queue_scroll_to_playing = false;
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
        });

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
        app.select_queue_item_single(index);
        app.play_selected_queue_item();
    }

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Play selected").clicked() {
            app.play_selected_queue_item();
        }
        if ui.button("Remove selected").clicked() {
            app.remove_selected_queue_items();
        }
        if ui.button("Clear queue").clicked() {
            app.clear_queue();
        }
    });
}
