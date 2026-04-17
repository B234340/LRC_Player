use crate::presentation::egui_app::GuiApp;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    ui.heading("Queue");
    ui.label(format!("{} tracks in queue", app.app_vm.queue.tracks.len()));
    ui.separator();
    if app.app_vm.queue.tracks.is_empty() {
        ui.label("Queue is empty.");
    } else {
        let mut pending_move_up: Option<usize> = None;
        let mut pending_move_down: Option<usize> = None;
        let mut pending_remove: Option<usize> = None;

        for (index, track) in app.app_vm.queue.tracks.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}: {}", index + 1, track));
                if ui.button("Up").clicked() {
                    pending_move_up = Some(index);
                }
                if ui.button("Down").clicked() {
                    pending_move_down = Some(index);
                }
                if ui.button("Remove").clicked() {
                    pending_remove = Some(index);
                }
            });
        }

        if let Some(index) = pending_move_up {
            app.move_queue_item_up(index);
        } else if let Some(index) = pending_move_down {
            app.move_queue_item_down(index);
        } else if let Some(index) = pending_remove {
            app.remove_queue_item(index);
        }
    }
}
