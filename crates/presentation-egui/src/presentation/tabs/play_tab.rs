use crate::presentation::app::GuiApp;
use crate::presentation::components::fft_overlay::render_fft_overlay;
use crate::presentation::components::lrc_overlay::render_lrc_overlay;
use eframe::egui;
use lrc_player_domain::model::PlaybackState;

fn truncate_end(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    if max_chars <= 3 {
        return "...".to_owned();
    }
    let start: String = text.chars().take(max_chars - 3).collect();
    format!("{}...", start)
}

fn format_ms(ms: u64) -> String {
    format!("{:02}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000)
}

/// 再生コントロール画面のUIを描画する関数。
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    ui.heading("Play");
    // Shortcuts are handled globally (GuiApp::handle_global_keyboard_shortcuts).
    ui.label("Space: play/pause, ←/→: seek 5 s (hold to scan faster)");
    ui.separator();

    // ========== 再生モード（シャッフル + リピート） ==========
    ui.horizontal(|ui| {
        ui.label("Playback mode:");
        let shuffle = app.shuffle();
        if ui.selectable_label(shuffle, "Shuffle").clicked() {
            app.set_shuffle(!shuffle);
        }
        let repeat = app.repeat();
        if ui
            .button(repeat.label())
            .on_hover_text("Click to cycle: off → all → one")
            .clicked()
        {
            app.cycle_repeat();
        }
    });

    // ========== 歌詞表示セクション ==========
    ui.separator();
    ui.heading("Lyrics");

    let mut overlay_setting_changed = false;
    ui.horizontal(|ui| {
        overlay_setting_changed |= ui.checkbox(&mut app.lrc_overlay_enabled, "LRC overlay").changed();
        overlay_setting_changed |= ui.checkbox(&mut app.fft_overlay_enabled, "FFT overlay").changed();
    });
    if overlay_setting_changed {
        app.save_state();
    }

    if app.lyrics_files.is_empty() {
        ui.label("No lyrics files discovered for the current track.");
    } else {
        let mut load_index = None;
        for index in 0..app.lyrics_files.len() {
            let file_path = &app.lyrics_files[index];
            let file_name = file_path.file_name().and_then(|name| name.to_str()).unwrap_or("lyrics");
            let label = truncate_end(&format!("{} — {}", file_name, file_path.display()), 96);
            let is_selected = app.selected_lyrics_index == Some(index);
            if ui
                .selectable_label(is_selected, label)
                .on_hover_text(file_path.display().to_string())
                .clicked()
            {
                load_index = Some(index);
            }
        }
        if let Some(index) = load_index {
            app.load_lyrics_file(index);
        }
    }

    // ========== 現在再生中のトラック情報表示 ==========
    if !app.play_path.trim().is_empty() {
        app.ensure_play_artwork();

        let overlays = app.lrc_overlay_enabled || app.fft_overlay_enabled;
        let display_texture = if overlays {
            app.play_artwork_blur_handle.clone().or(app.play_artwork_handle.clone())
        } else {
            app.play_artwork_handle.clone()
        };

        let available = ui.available_size();
        let available_width = available.x.max(1.0);
        let text_height = ui.text_style_height(&egui::TextStyle::Body);
        let control_height = ui.spacing().interact_size.y.max(text_height) * 2.0;
        // Rows below the artwork: duration, offset, offset save, seek, position, transport,
        // volume, queue buttons + separators.
        let bottom_height = text_height * 3.0 + control_height * 5.0 + 40.0;
        let max_side = available_width
            .min((available.y - bottom_height).max(1.0))
            .clamp(1.0, 1080.0);
        let image_size = egui::vec2(max_side, max_side);

        if display_texture.is_some() || overlays {
            let response = ui
                .allocate_ui_with_layout(
                    egui::vec2(available_width, image_size.y),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| match &display_texture {
                        Some(texture) => ui.image((texture.id(), image_size)),
                        None => {
                            let (rect, response) = ui.allocate_exact_size(image_size, egui::Sense::hover());
                            ui.painter().rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
                            response
                        }
                    },
                )
                .inner;
            let rect = response.rect;

            if app.fft_overlay_enabled {
                // Analyze only while new audio arrives (no work when paused).
                if app.playback_state == PlaybackState::Playing {
                    if let Some(tap) = app.spectrum_tap() {
                        app.fft.update(&tap);
                    }
                }
                render_fft_overlay(ui, rect, &app.fft);
            }

            if app.lrc_overlay_enabled {
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_black_alpha(100));
                if let Some(lyrics) = &app.loaded_lyrics {
                    // The timeline applies [offset:] + the user offset exactly once.
                    let lyric_time = crate::presentation::app::lyric_clock_ms(
                        app.loaded_timeline.as_ref(),
                        app.play_position_ms,
                    );
                    render_lrc_overlay(ui, rect, lyrics, lyric_time, &mut app.lrc_layout);
                }
            }
            ui.add_space(8.0);
        }

        // ========== 歌詞オフセット ==========
        if app.loaded_lyrics.is_some() {
            ui.horizontal(|ui| {
                let mut new_offset = None;
                for (label, delta) in [("-250 ms", -250), ("-50 ms", -50)] {
                    if ui.button(label).clicked() {
                        new_offset = Some(app.lyrics_offset_ms + delta);
                    }
                }
                let mut value = app.lyrics_offset_ms;
                let drag = ui.add(
                    egui::DragValue::new(&mut value)
                        .speed(10)
                        .range(-30_000..=30_000)
                        .prefix("Offset: ")
                        .suffix(" ms"),
                );
                if drag.changed() {
                    new_offset = Some(value);
                }
                for (label, delta) in [("+50 ms", 50), ("+250 ms", 250)] {
                    if ui.button(label).clicked() {
                        new_offset = Some(app.lyrics_offset_ms + delta);
                    }
                }
                if let Some(offset) = new_offset {
                    app.set_lyrics_offset(offset);
                }
            });
            ui.horizontal(|ui| {
                let save_enabled = app.selected_lyrics_index.is_some() && app.lyrics_offset_ms != 0;
                if ui
                    .add_enabled(save_enabled, egui::Button::new("Save offset to LRC"))
                    .on_hover_text("Rewrites the .lrc timestamps (previous file kept as .lrc.bak)")
                    .clicked()
                {
                    app.save_lyrics_offset_to_selected_file();
                }
                if ui
                    .add_enabled(app.lyrics_offset_ms != 0, egui::Button::new("Reset offset"))
                    .clicked()
                {
                    app.set_lyrics_offset(0);
                }
            });
        }

        // ========== シークバー ==========
        match app.play_duration_ms {
            Some(duration_ms) => {
                ui.label(format!("Duration: {}", format_ms(duration_ms)));
                let mut position = app.play_position_ms.min(duration_ms);
                let response = ui
                    .scope(|ui| {
                        ui.spacing_mut().slider_width = ui.available_width();
                        ui.add(egui::Slider::new(&mut position, 0..=duration_ms).show_value(false))
                    })
                    .inner;
                // The slider must not keep keyboard focus: the global ←/→ shortcuts already
                // seek, and a focused slider would leave "scrubbing" stuck on.
                if response.has_focus() {
                    response.surrender_focus();
                }
                if response.changed() {
                    app.seek_to(position);
                }
                app.set_seek_interacting(response.dragged() || response.is_pointer_button_down_on());
                ui.label(format!(
                    "Current position: {} / -{}",
                    format_ms(app.play_position_ms.min(duration_ms)),
                    format_ms(duration_ms.saturating_sub(app.play_position_ms))
                ));
            }
            None => {
                app.set_seek_interacting(false);
                ui.label("Duration: unknown");
            }
        }
    }

    // ========== 再生コントロール ==========
    ui.horizontal(|ui| {
        let button_size = ui.spacing().interact_size * egui::vec2(2.0, 1.6);
        if ui.add_sized(button_size, egui::Button::new("Previous")).clicked() {
            app.previous();
        }
        let play_label = match app.playback_state {
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => "Pause",
            PlaybackState::Paused => "Resume",
            PlaybackState::Stopped => "Play",
        };
        if ui.add_sized(button_size, egui::Button::new(play_label)).clicked() {
            app.toggle_play_pause();
        }
        if ui.add_sized(button_size, egui::Button::new("Stop")).clicked() {
            app.stop();
        }
        if ui.add_sized(button_size, egui::Button::new("Next")).clicked() {
            app.next();
        }
        if app.playback_state == PlaybackState::Buffering {
            ui.spinner();
        }
    });

    // ========== 音量 ==========
    ui.horizontal(|ui| {
        let muted = app.is_muted();
        if ui.selectable_label(muted, if muted { "Muted" } else { "Mute" }).clicked() {
            app.toggle_mute();
        }
        let mut volume = app.volume();
        let label = format!("Volume {:.0}%", volume * 100.0);
        let response = ui.add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false).text(label));
        if response.has_focus() {
            response.surrender_focus();
        }
        if response.changed() {
            app.set_volume(volume);
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Play selected queue item").clicked() {
            app.play_selected_queue_item();
        }
        // Stops playback too (the old button left audio playing).
        if ui.button("Clear play path").clicked() {
            app.clear_current_track();
        }
    });

    ui.separator();
    ui.label(format!("{} tracks in queue.", app.app_vm.queue.tracks.len()));
}
