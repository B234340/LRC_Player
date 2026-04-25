use crate::presentation::app::GuiApp;
use crate::presentation::components::track_list::{TrackActivation, TrackListOutcome};
use eframe::egui;

/// トラックリストのクリック/ダブルクリックを処理し、再生要求を `TrackListOutcome` に登録します。
///
/// `render_standard_track_list` や `render_track_list_with_artwork` から呼び出します。
///
/// # 引数
/// * `response` - クリック対象の `egui::Response`
/// * `activation` - シングルクリックまたはダブルクリックの指定
/// * `index` - クリックされたトラックのインデックス
/// * `displayed_tracks` - 表示中のトラックリスト全体
/// * `outcome` - トラックリスト結果を格納する構造体
pub fn process_track_activation(
    response: &egui::Response,
    activation: TrackActivation,
    index: usize,
    displayed_tracks: &[String],
    outcome: &mut TrackListOutcome,
) -> bool {
    let activated = match activation {
        TrackActivation::SingleClick => response.clicked(),
        TrackActivation::DoubleClick => response.double_clicked(),
    };

    if activated {
        outcome.play_index = Some(index);
        outcome.play_list = displayed_tracks.to_vec();
    }

    activated
}

fn track_list_selection_id(id_source: &str) -> egui::Id {
    egui::Id::new(format!("track_list_selection_{}", id_source))
}

/// 現在選択されているトラック行インデックスを取得します。
pub fn selected_track_index(ui: &egui::Ui, id_source: &str) -> Option<usize> {
    let id = track_list_selection_id(id_source);
    ui.ctx().data(|data| data.get_temp::<usize>(id))
}

/// トラック行を選択状態として保持します。
pub fn set_selected_track_index(ui: &egui::Ui, id_source: &str, index: usize) {
    let id = track_list_selection_id(id_source);
    ui.ctx().data_mut(|data| {
        data.insert_temp(id, index);
    });
}

/// トラックリストの選択状態をリセットします。
pub fn clear_selected_track_index(ui: &egui::Ui, id_source: &str) {
    let id = track_list_selection_id(id_source);
    ui.ctx().data_mut(|data| {
        data.remove::<usize>(id);
    });
}

/// シングルクリックでの選択処理を共通化します。
///
/// Queue タブの行選択機能などで利用します。
///
/// # 引数
/// * `response` - 対象行の `egui::Response`
/// * `app` - アプリケーション状態
/// * `index` - クリックされたトラックのインデックス
/// * `shift_pressed` - Shift キー押下かどうか
/// * `ctrl_pressed` - Ctrl キー押下かどうか
pub fn process_single_click_selection(
    response: &egui::Response,
    app: &mut GuiApp,
    index: usize,
    shift_pressed: bool,
    ctrl_pressed: bool,
) -> bool {
    if !response.clicked() {
        return false;
    }

    if shift_pressed {
        app.select_queue_item_range_to(index);
    } else if ctrl_pressed {
        app.toggle_queue_item_selection(index);
    } else if app.selected_queue_indices.contains(&index) {
        app.toggle_queue_item_selection(index);
    } else {
        app.select_queue_item_single(index);
    }

    true
}
