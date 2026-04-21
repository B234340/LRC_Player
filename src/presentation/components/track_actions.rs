use crate::presentation::components::track_list::TrackListOutcome;
use crate::presentation::egui_app::GuiApp;

/// 再生アクションを `TrackListOutcome` に登録する。
pub fn apply_play_action(outcome: &mut TrackListOutcome, index: usize, displayed_tracks: &[String]) {
    outcome.play_index = Some(index);
    outcome.play_list = displayed_tracks.to_vec();
}

/// お気に入りアクションを実行する。
pub fn apply_toggle_favorite_action(app: &mut GuiApp, track_path: &str) {
    perform_toggle_favorite_action(app, track_path);
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

/// お気に入り切り替えを実行する。
pub fn perform_toggle_favorite_action(app: &mut GuiApp, track_path: &str) {
    app.toggle_favorite_track(track_path);
}

/// 末尾にトラックを追加する。
pub fn perform_enqueue_track_path(app: &mut GuiApp, path: &str) {
    app.enqueue_track_path(path);
}

/// 次に再生する位置にトラックを挿入する。
pub fn perform_add_track_next_in_queue(app: &mut GuiApp, track: &str) {
    app.add_track_next_in_queue(track);
}

/// 表示中のリストを再生キューに設定し、指定トラックを選択して再生開始する。
pub fn perform_play_from_displayed_list(
    app: &mut GuiApp,
    displayed_tracks: &[String],
    clicked_index: usize,
) {
    app.play_from_displayed_list(displayed_tracks, clicked_index);
}

/// 指定プレイリストにトラックを追加する。
pub fn perform_add_track_to_playlist(app: &mut GuiApp, playlist_index: usize, track: &str) {
    app.add_track_to_playlist(playlist_index, track);
}

/// 選択中プレイリストからトラックを削除する。
pub fn perform_remove_track_from_selected_playlist(app: &mut GuiApp, track_index: usize) {
    app.remove_track_from_selected_playlist(track_index);
}

/// 今すぐ再生を実行する。
pub fn perform_play_track_now(app: &mut GuiApp, path: &str) {
    app.play_track_now(path);
}

/// トラック詳細ダイアログを開く。
pub fn perform_open_track_details(app: &mut GuiApp, path: &str) {
    app.open_track_details(path);
}
