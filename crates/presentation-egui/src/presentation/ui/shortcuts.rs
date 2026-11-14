//! Global keyboard shortcuts as data (one table drives both the handler and the list shown
//! in Settings).

use eframe::egui::{Context, Key, KeyboardShortcut, Modifiers};

use super::{
    actions::{Page, RightPanel, UiAction},
    widgets::{gain_to_ui, ui_to_gain, VOLUME_STEP},
};

/// Seek step for ←/→.
pub const SEEK_STEP_MS: i64 = 5_000;

/// One row of the shortcut list: (keys label, localization key of the description).
pub const SHORTCUT_LIST: &[(&str, &str)] = &[
    ("Space", "shortcut.play_pause"),
    ("← / →", "shortcut.seek"),
    ("Shift + ← / →", "shortcut.prev_next"),
    ("↑ / ↓", "shortcut.volume"),
    ("M", "shortcut.mute"),
    ("Ctrl + F", "shortcut.search"),
    ("Ctrl + L", "shortcut.lyrics"),
    ("Ctrl + Shift + L", "shortcut.desktop_lyrics"),
    ("Ctrl + Alt + L", "shortcut.desktop_lyrics_lock"),
    ("Ctrl + ,", "shortcut.settings"),
    ("Ctrl + U", "shortcut.queue"),
    ("Alt + ←", "shortcut.back"),
    ("Esc", "shortcut.escape"),
];

/// Consume global shortcuts for this frame and return the resulting actions.
///
/// Call at the *start* of `update`, before drawing widgets. Shortcuts are ignored while a
/// text field has keyboard focus; plain arrows/Space are also left alone while any widget
/// has focus (e.g. the track table uses ↑/↓ and Enter, a focused button uses Space).
/// `volume_gain` is the current linear gain (for ↑/↓).
pub fn global_shortcuts(ctx: &Context, volume_gain: f32) -> Vec<UiAction> {
    let mut out = Vec::new();
    if ctx.egui_wants_keyboard_input() {
        return out;
    }
    let widget_focused = ctx.memory(|m| m.focused().is_some());
    let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
    ctx.input_mut(|i| {
        if i.consume_shortcut(&cmd(Key::F)) {
            out.push(UiAction::FocusSearch);
        }
        if i.consume_shortcut(&cmd(Key::L)) {
            out.push(UiAction::ToggleRightPanel(RightPanel::Lyrics));
        }
        if i.consume_shortcut(&cmd(Key::Comma)) {
            out.push(UiAction::Navigate(Page::Settings));
        }
        if i.consume_shortcut(&cmd(Key::U)) {
            out.push(UiAction::Navigate(Page::Queue));
        }
        if i.consume_key(Modifiers::ALT, Key::ArrowLeft)
            || i.pointer.button_pressed(eframe::egui::PointerButton::Extra1)
        {
            out.push(UiAction::Back);
        }
        if i.consume_key(Modifiers::SHIFT, Key::ArrowLeft) {
            out.push(UiAction::Previous);
        }
        if i.consume_key(Modifiers::SHIFT, Key::ArrowRight) {
            out.push(UiAction::Next);
        }
        if i.consume_key(Modifiers::NONE, Key::M) {
            out.push(UiAction::ToggleMute);
        }
        if widget_focused {
            return;
        }
        if i.consume_key(Modifiers::NONE, Key::Space) {
            out.push(UiAction::PlayPause);
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
            out.push(UiAction::SeekRelative(-SEEK_STEP_MS));
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
            out.push(UiAction::SeekRelative(SEEK_STEP_MS));
        }
        let up = i.count_and_consume_key(Modifiers::NONE, Key::ArrowUp) as f32;
        let down = i.count_and_consume_key(Modifiers::NONE, Key::ArrowDown) as f32;
        if up != down {
            let pos = (gain_to_ui(volume_gain) + (up - down) * VOLUME_STEP).clamp(0.0, 1.0);
            out.push(UiAction::SetVolume(ui_to_gain(pos)));
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, RawInput};

    fn press(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn maps_keys_to_actions() {
        let ctx = Context::default();
        let input = RawInput {
            events: vec![
                press(Key::Space, Modifiers::NONE),
                press(Key::ArrowRight, Modifiers::NONE),
                press(Key::ArrowRight, Modifiers::SHIFT),
                press(Key::ArrowUp, Modifiers::NONE),
                press(Key::F, Modifiers::COMMAND),
            ],
            ..Default::default()
        };
        let mut got = Vec::new();
        let _ = ctx.run_ui(input, |ui| got = global_shortcuts(ui.ctx(), 0.125));
        assert!(got.contains(&UiAction::PlayPause));
        assert!(got.contains(&UiAction::SeekRelative(SEEK_STEP_MS)));
        assert!(got.contains(&UiAction::Next));
        assert!(got.contains(&UiAction::FocusSearch));
        let vol = got.iter().find_map(|a| if let UiAction::SetVolume(v) = a { Some(*v) } else { None }).unwrap();
        assert!((gain_to_ui(vol) - 0.55).abs() < 1e-4);
    }
}
