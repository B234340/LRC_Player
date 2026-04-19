use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use std::path::Path;
use std::time::{Duration, Instant};

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    ui.heading("Play");
    ui.label("Enter a file path or select a queued track, then press Play.");
    ui.add_space(4.0);
    app.handle_play_keyboard_shortcuts(ui.ctx());
    ui.label("Space: play/pause, ←/→: seek 10s (hold for continuous seek)");
    ui.separator();

    ui.horizontal(|ui| {
        if ui.button("Previous").clicked() {
            app.play_previous();
        }
        let play_label = match app.playback_state {
            crate::domain::model::PlaybackState::Playing => "Pause",
            crate::domain::model::PlaybackState::Paused => "Resume",
            _ => "Play",
        };
        if ui.button(play_label).clicked() {
            app.toggle_play_pause();
        }
        if ui.button("Next").clicked() {
            app.play_next();
        }
    });

    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.play_path);
        if ui.button("Play path").clicked() {
            app.play_position_ms = 0;
            app.playback_state = crate::domain::model::PlaybackState::Buffering;
            app.discover_lyrics_for_current_play_path();
            app.request_play();
        }
        if ui.button("Discover metadata").clicked() {
            app.refresh_play_metadata();
            app.discover_lyrics_for_current_play_path();
        }
    });

    ui.label(format!("Playback state: {}", app.playback_state));
    if let Some(status) = &app.play_status {
        ui.label(format!("Status: {}", status));
    }

    let mut mode_changed = false;
    ui.horizontal(|ui| {
        ui.label("Playback mode:");
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, crate::domain::model::PlaybackMode::Sequential, "Sequential")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, crate::domain::model::PlaybackMode::RepeatOne, "Repeat One")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, crate::domain::model::PlaybackMode::RepeatAll, "Repeat All")
            .changed();
        mode_changed |= ui
            .selectable_value(&mut app.playback_mode, crate::domain::model::PlaybackMode::Shuffle, "Shuffle")
            .changed();
    });
    if mode_changed {
        app.save_state();
    }

    if !app.play_path.trim().is_empty() {
        app.ensure_play_artwork(ui.ctx());
        if let Some(texture) = &app.play_artwork_handle {
            let available = ui.available_size();
            let max_side = (available.x.min(available.y) * 0.45).clamp(128.0, 320.0);
            let mut image_size = texture.size_vec2();
            let scale = (max_side / image_size.x).min(max_side / image_size.y).min(1.0);
            image_size *= scale;
            ui.image(texture, image_size);
            ui.add_space(8.0);
        }

        let path = Path::new(&app.play_path);
        ui.label(format!("Track path: {}", app.play_path));
        ui.label(format!("Exists: {}", path.exists()));
        if let Some(title) = &app.play_title {
            ui.label(format!("Title: {}", title));
        }
        if let Some(artist) = &app.play_artist {
            ui.label(format!("Artist: {}", artist));
        }
        if let Some(album) = &app.play_album {
            ui.label(format!("Album: {}", album));
        }
        if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
            ui.label(format!("Filename: {}", filename));
        }
        if let Some(duration_ms) = app.play_duration_ms {
            let mins = duration_ms / 60000;
            let secs = (duration_ms / 1000) % 60;
            let millis = duration_ms % 1000;
            ui.label(format!("Duration: {:02}:{:02}.{:03}", mins, secs, millis));

            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut app.lyrics_offset_ms, -30_000..=30_000).text("Lyrics offset ms"));
                ui.label(format!("({:+} ms)", app.lyrics_offset_ms));
            });

            let slider = egui::Slider::new(&mut app.play_position_ms, 0..=duration_ms).text("Position ms");
            let response = ui
                .scope(|ui| {
                    ui.spacing_mut().slider_width = ui.available_width();
                    ui.add(slider)
                })
                .inner;
            if response.changed() {
                app.queue_seek_position(app.play_position_ms);
                app.save_state();
            }

            let pointer_down = response.is_pointer_button_down_on();
            let mut slider_interacting = response.dragged() || response.has_focus();
            if pointer_down {
                if app.seek_long_press_elapsed().is_none() {
                    app.set_seek_long_press_start(Some(Instant::now()));
                } else if !app.is_seek_interacting()
                    && app.seek_long_press_elapsed().unwrap() >= Duration::from_millis(350)
                {
                    slider_interacting = true;
                }
            } else {
                app.set_seek_long_press_start(None);
            }

            app.set_seek_interacting(slider_interacting);

            let pos = app.play_position_ms.min(duration_ms);
            let mins = pos / 60000;
            let secs = (pos / 1000) % 60;
            let millis = pos % 1000;
            ui.label(format!("Current position: {:02}:{:02}.{:03}", mins, secs, millis));
        } else {
            ui.label("Duration: unknown. Discover metadata to enable seek.");
        }
    }

    ui.separator();
    if let Some(index) = app.selected_queue_index {
        if let Some(track) = app.app_vm.queue.tracks.get(index) {
            ui.label(format!("Selected queue item: {}", track));
        }
    } else {
        ui.label("No queue item selected.");
    }

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Play selected queue item").clicked() {
            app.play_selected_queue_item();
        }
        if ui.button("Clear play path").clicked() {
            app.play_path.clear();
            app.lyrics_files.clear();
            app.loaded_lyrics = None;
            app.selected_lyrics_index = None;
            app.play_duration_ms = None;
            app.play_title = None;
            app.play_artist = None;
            app.play_album = None;
            app.play_artwork_handle = None;
            app.play_artwork_source = None;
            app.lyrics_offset_ms = 0;
            app.play_position_ms = 0;
            app.save_state();
        }
    });

    ui.separator();
    ui.label(format!("{} tracks in queue.", app.app_vm.queue.tracks.len()));

    ui.separator();
    ui.heading("Lyrics");
    if app.lyrics_files.is_empty() {
        ui.label("No lyrics files discovered for the current track.");
    } else {
        for index in 0..app.lyrics_files.len() {
            let file_path = app.lyrics_files[index].clone();
            let file_name = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("lyrics");
            let label = format!("{} — {}", file_name, file_path.display());
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(app.selected_lyrics_index == Some(index), label)
                    .clicked()
                {
                    app.load_lyrics_file(index);
                }
                if ui.button("Load").clicked() {
                    app.load_lyrics_file(index);
                }
            });
        }
    }

    if let Some(lyrics) = &app.loaded_lyrics {
        ui.separator();
        ui.label("Lyrics preview:");

        let current_time_ms = if app.lyrics_offset_ms.is_negative() {
            app.play_position_ms.saturating_sub((-app.lyrics_offset_ms) as u64)
        } else {
            app.play_position_ms.saturating_add(app.lyrics_offset_ms as u64)
        };
        let highlighted_line_index = lyrics
            .lines
            .iter()
            .enumerate()
            .rev()
            .find(|(_, line)| line.time_ms <= current_time_ms)
            .map(|(index, _)| index);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(300.0)
            .show(ui, |ui| {
                for (index, line) in lyrics.lines.iter().enumerate() {
                    let is_current_line = Some(index) == highlighted_line_index;
                    let label_text = line.text.clone();

                    let response = if is_current_line {
                        if line.segments.len() > 1 {
                            let font_id = egui::TextStyle::Body.resolve(ui.style());
                            let text_color = ui.style().visuals.text_color();
                            let highlight_color = egui::Color32::from_rgb(40, 40, 80);

                            let mut prefix_text = String::new();
                            let mut highlight_width = 0.0;
                            let full_text_width = ui.fonts(|fonts| {
                                let mut job = egui::text::LayoutJob::default();
                                job.append(&line.text, 0.0, egui::TextFormat::simple(font_id.clone(), text_color));
                                fonts.layout_job(job).size().x
                            });

                            for (seg_index, segment) in line.segments.iter().enumerate() {
                                let seg_start = segment.time_ms;
                                let seg_end = line.segments.get(seg_index + 1).map(|next| next.time_ms);

                                if let Some(end_ms) = seg_end {
                                    if current_time_ms >= end_ms {
                                        prefix_text.push_str(&segment.text);
                                        continue;
                                    }
                                }

                                let prefix_width = ui.fonts(|fonts| {
                                    let mut job = egui::text::LayoutJob::default();
                                    job.append(&prefix_text, 0.0, egui::TextFormat::simple(font_id.clone(), text_color));
                                    fonts.layout_job(job).size().x
                                });

                                if current_time_ms < seg_start {
                                    highlight_width = prefix_width;
                                } else if let Some(end_ms) = seg_end {
                                    let elapsed = current_time_ms.saturating_sub(seg_start);
                                    let duration = end_ms.saturating_sub(seg_start).max(1);
                                    let ratio = (elapsed as f32 / duration as f32).clamp(0.0, 1.0);
                                    let segment_width = ui.fonts(|fonts| {
                                        let mut job = egui::text::LayoutJob::default();
                                        job.append(&segment.text, 0.0, egui::TextFormat::simple(font_id.clone(), text_color));
                                        fonts.layout_job(job).size().x
                                    });
                                    highlight_width = prefix_width + segment_width * ratio;
                                } else {
                                    highlight_width = full_text_width;
                                }
                                break;
                            }

                            let text_size = egui::Vec2::new(full_text_width, ui.fonts(|fonts| fonts.row_height(&font_id)));
                            let (rect, response) = ui.allocate_exact_size(text_size, egui::Sense::hover());
                            let highlight_rect = egui::Rect::from_min_size(rect.min, egui::vec2(highlight_width, rect.height()));
                            ui.painter().rect_filled(highlight_rect, 0.0, highlight_color);
                            ui.painter().text(rect.min, egui::Align2::LEFT_TOP, &line.text, font_id.clone(), text_color);
                            response
                        } else {
                            ui.label(
                                egui::RichText::new(label_text)
                                    .color(egui::Color32::WHITE)
                                    .background_color(egui::Color32::from_rgb(40, 40, 80)),
                            )
                        }
                    } else {
                        ui.label(label_text)
                    };

                    if is_current_line {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            });
    }
}
