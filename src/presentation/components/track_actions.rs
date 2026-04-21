use crate::presentation::components::track_list::TrackListOutcome;
use crate::presentation::egui_app::GuiApp;

/// 再生アクションを `TrackListOutcome` に登録する。
pub fn apply_play_action(outcome: &mut TrackListOutcome, index: usize, displayed_tracks: &[String]) {
    outcome.play_index = Some(index);
    outcome.play_list = displayed_tracks.to_vec();
}

/// お気に入り切り替えアクションを実行する。
pub fn apply_toggle_favorite_action(app: &mut GuiApp, track_path: &str) {
    app.toggle_favorite_track(track_path);
}

/// キュー追加アクションを `TrackListOutcome` に登録する。
pub fn apply_enqueue_action(outcome: &mut TrackListOutcome, track_path: &str) {
    outcome.enqueue_path = Some(track_path.to_string());
}

/// 削除アクションを `TrackListOutcome` に登録する。
pub fn apply_remove_action(outcome: &mut TrackListOutcome, index: usize) {
    outcome.remove_indices = Some(vec![index]);
}

/// 詳細表示アクションを `TrackListOutcome` に登録する。
pub fn apply_show_details_action(outcome: &mut TrackListOutcome, track_path: &str) {
    outcome.show_details_path = Some(track_path.to_string());
}
