use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use std::path::Path;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    ui.heading("Play");
    ui.label("Enter a file path or select a queued track, then press Play.");
    ui.separator();

    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.play_path);
        if ui.button("Play path").clicked() {
            app.request_play();
        }
        if ui.button("Discover metadata").clicked() {
            app.refresh_play_metadata();
            app.discover_lyrics_for_current_play_path();
        }
    });

    if let Some(status) = &app.play_status {
        ui.label(format!("Status: {}", status));
    }

    if !app.play_path.trim().is_empty() {
        app.ensure_play_artwork(ui.ctx());
        if let Some(texture) = &app.play_artwork_handle {
            let mut image_size = texture.size_vec2();
            let max_size = egui::Vec2::splat(256.0);
            if image_size.x > max_size.x || image_size.y > max_size.y {
                let scale = (max_size.x / image_size.x).min(max_size.y / image_size.y);
                image_size *= scale;
            }
            ui.image(texture, image_size);
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

            let slider = egui::Slider::new(&mut app.play_position_ms, 0..=duration_ms)
                .text("Position ms");
            if ui.add(slider).changed() {
                app.save_state();
            }

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
            let label = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("lyrics");
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
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for line in lyrics.lines.iter() {
                    let time_ms = if app.lyrics_offset_ms.is_negative() {
                        line.time_ms.saturating_sub((-app.lyrics_offset_ms) as u64)
                    } else {
                        line.time_ms.saturating_add(app.lyrics_offset_ms as u64)
                    };
                    ui.label(format!(
                        "[{:02}:{:02}.{:03}] {}",
                        time_ms / 60000,
                        (time_ms / 1000) % 60,
                        time_ms % 1000,
                        line.text
                    ));
                }
            });
    }
}
