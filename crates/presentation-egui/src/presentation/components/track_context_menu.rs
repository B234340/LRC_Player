use eframe::egui;

/// トラックのコンテキストメニュー（右クリックメニュー）操作の結果を保持する構造体。
/// 各フィールドは、ユーザーがメニュー内の特定のアクションを選択した場合に値が設定される。
#[derive(Debug, Clone, Default)]
pub struct TrackContextMenuOutcome {
    /// 「今すぐ再生」がクリックされた場合のトラックパス
    pub play_now_path: Option<String>,
    /// 「次に再生」がクリックされた場合のトラックパス
    pub next_path: Option<String>,
    /// 「キューに追加」がクリックされた場合のトラックパス
    pub enqueue_path: Option<String>,
    /// 特定のプレイリストに追加する操作。タプルは (プレイリストのインデックス, トラックパス)
    pub add_to_playlist: Option<(usize, String)>,
    /// 「詳細を表示」がクリックされた場合のトラックパス
    pub show_details_path: Option<String>,
}

/// トラック用のコンテキストメニューを描画する。
/// 
/// # 引数
/// * `ui` - eguiのUIビルダー
/// * `track_path` - 対象となるトラックのファイルパス文字列
/// * `playlist_targets` - 追加先候補となるプレイリストのリスト (インデックス, 名前)
/// * `enqueue_label` - キュー追加ボタンに表示するカスタムラベル
/// * `extra_items` - メニューに追加の項目を挿入するためのコールバック関数
pub fn render_track_context_menu<F>(
    ui: &mut egui::Ui,
    track_path: &str,
    playlist_targets: &[(usize, String)],
    enqueue_label: &str,
    extra_items: F,
) -> TrackContextMenuOutcome
where
    F: FnOnce(&mut egui::Ui, &mut TrackContextMenuOutcome), // 追加項目描画用のクロージャ制約
{
    // 戻り値となる結果オブジェクトを初期化
    let mut outcome = TrackContextMenuOutcome::default();

    // --- 基本再生操作 ---
    if ui.button("Play now").clicked() {
        outcome.play_now_path = Some(track_path.to_string());
        ui.close(); // アクション実行後はメニューを閉じる
    }
    if ui.button("Play next").clicked() {
        outcome.next_path = Some(track_path.to_string());
        ui.close();
    }
    if ui.button(enqueue_label).clicked() {
        outcome.enqueue_path = Some(track_path.to_string());
        ui.close();
    }

    ui.separator(); // 区切り線

    // --- プレイリスト追加サブメニュー ---
    ui.label("Add to playlist");
    for (target_index, target_name) in playlist_targets {
        if ui.button(target_name).clicked() {
            // 選択されたプレイリストのインデックスとトラックパスを結果に格納
            outcome.add_to_playlist = Some((*target_index, track_path.to_string()));
            ui.close();
        }
    }

    ui.separator();

    // --- 呼び出し元が定義する拡張項目エリア ---
    extra_items(ui, &mut outcome);

    // --- 共通メニュー項目 ---
    if ui.button("Show details").clicked() {
        outcome.show_details_path = Some(track_path.to_string());
        ui.close();
    }

    // クリックされたアクションの情報を返却
    outcome
}