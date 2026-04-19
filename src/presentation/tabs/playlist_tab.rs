use crate::presentation::egui_app::GuiApp;
use eframe::egui;
use image::io::Reader as ImageReader;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// トラックに対応するアートワーク画像を検出する関数
/// 
/// トラックファイルと同じディレクトリから、以下の優先順位でアートワークを探す：
/// 1. トラック名と同じベース名の画像ファイル（.jpg, .jpeg, .png, .webp）
/// 2. 一般的なアートワークファイル名（folder.jpg, cover.jpg, front.jpg など）
/// 
/// # 引数
/// * `path` - トラックファイルのパス
/// 
/// # 戻り値
/// アートワークが見つかった場合はそのパス、見つからない場合は None
fn discover_artwork_for_track(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;  // 親ディレクトリを取得
    let stem = path.file_stem()?.to_string_lossy();  // 拡張子を除いたファイル名
    
    // 検索候補のファイル名リスト
    let candidates = [
        format!("{}.jpg", stem),   // トラック名.jpg
        format!("{}.jpeg", stem),  // トラック名.jpeg
        format!("{}.png", stem),   // トラック名.png
        format!("{}.webp", stem),  // トラック名.webp
        "folder.jpg".to_string(),  // フォルダ画像
        "cover.jpg".to_string(),   // カバー画像
        "front.jpg".to_string(),   // フロントカバー
        "folder.png".to_string(),
        "cover.png".to_string(),
        "front.png".to_string(),
    ];
    
    // 候補を順にチェックし、存在するものを返す
    for candidate in candidates.iter() {
        let candidate_path = parent.join(candidate);
        if candidate_path.exists() {
            return Some(candidate_path);
        }
    }
    None
}

/// アートワーク画像を読み込み、eguiのテクスチャとして返す関数
/// 
/// # 引数
/// * `ctx` - eguiのコンテキスト
/// * `artwork_path` - アートワーク画像のパス
/// * `id` - テクスチャの識別子
/// 
/// # 戻り値
/// 読み込みに成功した場合は TextureHandle、失敗した場合は None
fn load_artwork_texture(ctx: &egui::Context, artwork_path: &Path, id: &str) -> Option<egui::TextureHandle> {
    // 画像ファイルを開く
    if let Ok(reader) = ImageReader::open(artwork_path) {
        // フォーマットを推測
        if let Ok(reader) = reader.with_guessed_format() {
            // デコード
            if let Ok(image) = reader.decode() {
                let size = [image.width() as usize, image.height() as usize];
                // RGBA形式に変換してeguiのカラー画像を作成
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    size,
                    &image.to_rgba8().into_vec(),
                );
                // テクスチャとして読み込み
                return Some(ctx.load_texture(id, color_image, egui::TextureOptions::default()));
            }
        }
    }
    None
}

/// キャッシュされたアートワークテクスチャを取得する関数
/// 
/// メモリ内にキャッシュがあればそれを返し、なければ読み込んでキャッシュする
/// 
/// # 引数
/// * `ctx` - eguiのコンテキスト
/// * `artwork_path` - アートワーク画像のパス
/// * `id` - テクスチャの識別子
/// 
/// # 戻り値
/// キャッシュまたは新規読み込みした TextureHandle、失敗した場合は None
fn get_cached_artwork_texture(
    ctx: &egui::Context,
    artwork_path: &Path,
    id: &str,
) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::new("playlist_artwork_texture_cache");
    let cache_key = artwork_path.to_string_lossy().to_string();

    // キャッシュから既存のテクスチャを検索
    let existing = ctx.data(|data| {
        data.get_temp::<HashMap<String, egui::TextureHandle>>(cache_id)
            .and_then(|cache| cache.get(&cache_key).cloned())
    });
    if existing.is_some() {
        return existing;  // キャッシュヒット
    }

    // キャッシュミス：新規に読み込み
    let texture = load_artwork_texture(ctx, artwork_path, id)?;
    
    // キャッシュに保存
    ctx.data_mut(|data| {
        let cache = data.get_temp_mut_or_default::<HashMap<String, egui::TextureHandle>>(cache_id);
        cache.insert(cache_key, texture.clone());
    });
    Some(texture)
}

/// プレイリスト画面のUIを描画する関数
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    // ヘッダー表示
    ui.heading("Playlists");
    ui.label(format!("Saved playlists: {}", app.playlists.len()));
    ui.separator();

    // 新規プレイリスト作成用の入力欄とボタン
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.new_playlist_name);
        if ui.button("Create playlist").clicked() {
            app.add_playlist();
        }
    });

    if app.playlists.is_empty() {
        ui.label("No playlists available.");
        return;
    }

    ui.separator();
    
    // 遅延実行するアクションを保持する変数群（プレイリスト操作）
    let mut pending_select_playlist: Option<Option<usize>> = None;  // プレイリスト選択（Noneで選択解除）
    let mut pending_delete_playlist: Option<usize> = None;  // プレイリスト削除
    let mut pending_move_playlist_up: Option<usize> = None;  // プレイリストを上に移動
    let mut pending_move_playlist_down: Option<usize> = None;  // プレイリストを下に移動
    
    // プレイリスト一覧の表示
    for (index, playlist) in app.playlists.iter().enumerate() {
        let selected = app.selected_playlist == Some(index);
        let is_favorite = playlist.name.eq_ignore_ascii_case("favorite");  // お気に入りプレイリストは特別扱い
        
        // 選択状態に応じてフレームのスタイルを変更
        let mut frame = egui::Frame::group(ui.style());
        if selected {
            frame = frame
                .fill(ui.visuals().selection.bg_fill)
                .stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
        }

        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                // お気に入りの場合は★マークを付ける
                let label = if is_favorite {
                    format!("★ {}", playlist.name)
                } else {
                    playlist.name.clone()
                };
                
                // プレイリスト選択ラベル
                let response = ui.add(egui::SelectableLabel::new(selected, label));
                if response.clicked() {
                    if selected {
                        pending_select_playlist = Some(None);  // 選択解除
                    } else {
                        pending_select_playlist = Some(Some(index));  // 選択
                    }
                }
                
                // 右寄せで操作ボタンを配置
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 下に移動ボタン（お気に入り以外、かつ最後以外）
                    if ui.add_enabled(!is_favorite && index < app.playlists.len() - 1, egui::Button::new("↓")).clicked() {
                        pending_move_playlist_down = Some(index);
                    }
                    // 上に移動ボタン（お気に入り以外、かつ2番目以降）
                    if ui.add_enabled(index > 1, egui::Button::new("↑")).clicked() {
                        pending_move_playlist_up = Some(index);
                    }
                    // 削除ボタン（お気に入り以外）
                    if ui.add_enabled(!is_favorite, egui::Button::new("Delete")).clicked() {
                        pending_delete_playlist = Some(index);
                    }
                });
            });
        });
        ui.add_space(4.0);
    }

    // プレイリスト削除アクションの実行（早期リターン）
    if let Some(index) = pending_delete_playlist {
        app.remove_playlist(index);
        return;
    }

    // プレイリスト並び替えアクションの実行
    if let Some(index) = pending_move_playlist_up {
        app.move_playlist_up(index);
    }
    if let Some(index) = pending_move_playlist_down {
        app.move_playlist_down(index);
    }

    // プレイリスト選択アクションの実行
    if let Some(selection) = pending_select_playlist {
        app.selected_playlist = selection;
        app.update_app_vm();
    }

    // 選択中のプレイリストの詳細表示
    if let Some(selected_index) = app.selected_playlist {
        let mut selected_name: Option<String> = None;
        let mut selected_len: usize = 0;
        
        // トラック操作の遅延アクション変数
        let mut pending_remove_track: Option<usize> = None;  // トラック削除
        let mut pending_play_index: Option<usize> = None;  // 再生開始インデックス
        let mut pending_play_list: Vec<String> = Vec::new();  // 再生リスト
        let mut pending_play_now_path: Option<String> = None;  // 即時再生
        let mut pending_next_path: Option<String> = None;  // 次に再生
        let mut pending_enqueue_path: Option<String> = None;  // キューに追加
        let mut pending_add_to_playlist: Option<(usize, String)> = None;  // プレイリストに追加
        let mut pending_show_details_path: Option<String> = None;  // 詳細表示
        
        let meta_cache = crate::infrastructure::persistence::MetaCache::open_default().ok();
        
        // プレイリスト一覧（追加先用）
        let playlist_targets: Vec<(usize, String)> = app
            .playlists
            .iter()
            .enumerate()
            .map(|(idx, p)| (idx, p.name.clone()))
            .collect();
        
        let playlist = app.playlists.get(selected_index).cloned();
        if let Some(playlist) = playlist {
            selected_name = Some(playlist.name.clone());
            selected_len = playlist.tracks.len();
            
            ui.separator();
            ui.heading(format!("Playlist: {}", playlist.name));
            ui.label(format!("{} tracks", playlist.tracks.len()));

            if playlist.tracks.is_empty() {
                ui.label("No tracks in this playlist.");
            } else {
                ui.label("Left click: create queue from this list and play. Right click: common actions.");
                
                // スクロール可能なトラック一覧
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(ui.available_height().max(120.0))
                    .show(ui, |ui| {
                        for (track_index, track) in playlist.tracks.iter().enumerate() {
                            let path = std::path::Path::new(track);
                            
                            // 表示用の情報を取得
                            let fallback_title = path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or(track);
                            
                            // メタデータキャッシュから情報を取得
                            let metadata = meta_cache
                                .as_ref()
                                .and_then(|cache| cache.track_metadata_by_location(track).ok().flatten());
                            
                            let title = metadata
                                .as_ref()
                                .map(|(title, _, _)| title.as_str())
                                .filter(|text| !text.trim().is_empty())
                                .unwrap_or(fallback_title);
                            
                            let artist = metadata
                                .as_ref()
                                .map(|(_, artist, _)| artist.as_str())
                                .filter(|text| !text.trim().is_empty())
                                .unwrap_or("Unknown artist");
                            
                            let album = metadata
                                .as_ref()
                                .map(|(_, _, album)| album.as_str())
                                .filter(|text| !text.trim().is_empty())
                                .unwrap_or("Unknown album");
                            
                            let is_playing = app.play_path == *track;  // 現在再生中かどうか

                            // 再生中トラックはハイライト表示
                            let mut frame = egui::Frame::group(ui.style());
                            if is_playing {
                                frame = frame
                                    .fill(ui.visuals().selection.bg_fill)
                                    .stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
                            }

                            frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // --- お気に入り切り替えボタン ---
                                    let art_size = egui::vec2(44.0, 44.0);
                                    let handle_pad_y = ((art_size.y - 24.0) * 0.5).max(0.0);
                                    let toggle_response = ui
                                        .allocate_ui_with_layout(
                                            egui::vec2(28.0, art_size.y),
                                            egui::Layout::top_down(egui::Align::Center),
                                            |ui| {
                                                ui.add_space(handle_pad_y);
                                                let is_favorite = app.is_track_favorite(track);
                                                let star_label = if is_favorite { "★" } else { "☆" };
                                                ui.add_sized([24.0, 24.0], egui::Button::new(star_label))
                                            },
                                        )
                                        .inner
                                        .on_hover_text("Toggle favorite");
                                    if toggle_response.clicked() {
                                        app.toggle_favorite_track(track);
                                    }
                                    ui.add_space(4.0);

                                    // --- アートワーク表示 ---
                                    if let Some(art_path) = discover_artwork_for_track(Path::new(track)) {
                                        if let Some(texture) =
                                            get_cached_artwork_texture(ui.ctx(), &art_path, &format!("playlist_art_{}_{}", selected_index, track_index))
                                        {
                                            ui.image(&texture, art_size);
                                        } else {
                                            // 読み込み失敗時のプレースホルダー
                                            let (art_rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
                                            ui.painter()
                                                .rect_filled(art_rect, 4.0, ui.visuals().faint_bg_color);
                                            ui.painter().text(
                                                art_rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "ART",
                                                egui::TextStyle::Small.resolve(ui.style()),
                                                ui.visuals().text_color(),
                                            );
                                        }
                                    } else {
                                        // アートワークが見つからない場合のプレースホルダー
                                        let (art_rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
                                        ui.painter()
                                            .rect_filled(art_rect, 4.0, ui.visuals().faint_bg_color);
                                        ui.painter().text(
                                            art_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "ART",
                                            egui::TextStyle::Small.resolve(ui.style()),
                                            ui.visuals().text_color(),
                                        );
                                    }

                                    // --- トラック情報表示 ---
                                    ui.vertical(|ui| {
                                        // トラックタイトル（再生中の場合は [Playing] を付加）
                                        let title_text = if is_playing {
                                            format!("{}: {} [Playing]", track_index + 1, title)
                                        } else {
                                            format!("{}: {}", track_index + 1, title)
                                        };
                                        
                                        let response = ui.selectable_label(false, title_text);
                                        if response.clicked() {
                                            pending_play_index = Some(track_index);
                                            pending_play_list = playlist.tracks.clone();
                                        }
                                        
                                        // 右クリックコンテキストメニュー
                                        response.context_menu(|ui| {
                                            if ui.button("Play now").clicked() {
                                                pending_play_now_path = Some(track.clone());
                                                ui.close_menu();
                                            }
                                            if ui.button("Play next").clicked() {
                                                pending_next_path = Some(track.clone());
                                                ui.close_menu();
                                            }
                                            if ui.button("Add to queue").clicked() {
                                                pending_enqueue_path = Some(track.clone());
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            ui.label("Add to playlist");
                                            for (target_index, target_name) in &playlist_targets {
                                                if ui.button(target_name).clicked() {
                                                    pending_add_to_playlist = Some((*target_index, track.clone()));
                                                    ui.close_menu();
                                                }
                                            }
                                            ui.separator();
                                            if ui.button("Show details").clicked() {
                                                pending_show_details_path = Some(track.clone());
                                                ui.close_menu();
                                            }
                                        });

                                        // アーティスト名とアルバム名を表示
                                        ui.label(format!("Artist: {}", artist));
                                        ui.label(format!("Album: {}", album));
                                    });

                                    // --- 右寄せ操作ボタン ---
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button("Remove").clicked() {
                                            pending_remove_track = Some(track_index);
                                        }
                                        if ui.button("Play").clicked() {
                                            pending_play_index = Some(track_index);
                                            pending_play_list = playlist.tracks.clone();
                                        }
                                    });
                                });
                            });
                            ui.add_space(4.0);
                        }
                    });
            }
        }

        // トラック追加用の入力欄
        if selected_name.is_some() {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut app.playlist_track_input);
                if ui.button("Add track").clicked() {
                    app.add_track_to_selected_playlist();
                }
                if selected_len == 0 {
                    ui.label("(empty)");
                }
            });
        }

        // 保留中のアクションを実行
        if let Some(track_index) = pending_remove_track {
            app.remove_track_from_selected_playlist(track_index);
        }
        if let Some(index) = pending_play_index {
            app.play_from_displayed_list(&pending_play_list, index);
        }
        if let Some(path) = pending_play_now_path {
            app.play_track_now(&path);
        }
        if let Some(path) = pending_next_path {
            app.add_track_next_in_queue(&path);
        }
        if let Some(path) = pending_enqueue_path {
            app.enqueue_track_path(&path);
        }
        if let Some((target_index, track)) = pending_add_to_playlist {
            app.add_track_to_playlist(target_index, &track);
        }
        if let Some(path) = pending_show_details_path {
            app.open_track_details(&path);
        }
    }
}