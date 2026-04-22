//! ライブラリのキャッシュ管理を担当
//! - アルバムアートワークのキャッシュ
//! - 遅延読み込みキュー処理

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui;
use image::io::Reader as ImageReader;
use lrc_player_application::infrastructure::persistence::MetaCache;

use crate::presentation::app::GuiApp;

impl GuiApp {
    /// アルバムアートワーク関連の全キャッシュをクリアします。
    /// 
    /// 以下のキャッシュがクリアされます：
    /// - アルバムタイトル → アートワークパスのキャッシュ
    /// - アルバムタイトル → テクスチャハンドルのキャッシュ
    /// - 読み込み待ちキュー
    /// - キュー重複防止用セット
    /// - 現在のアルバムタイトルリスト
    /// - メタデータキャッシュ参照
    /// 
    /// 主にライブラリ全体がクリアされた際に呼び出されます。
    pub(crate) fn clear_album_artwork_cache(&mut self) {
        self.album_artwork_path_cache.clear();
        self.album_artwork_texture_cache.clear();
        self.album_artwork_pending.clear();
        self.album_artwork_queued.clear();
        self.album_artwork_album_titles.clear();
        self.album_meta_cache = None;
    }

    /// アルバムアートワークキャッシュをライブラリの現在の状態と同期します。
    /// 
    /// ライブラリに存在しなくなったアルバムのキャッシュエントリを削除します。
    /// これにより、削除されたアルバムのアートワークがメモリに残り続けることを防ぎます。
    /// 
    /// 処理の流れ：
    /// 1. 現在のライブラリに存在する全アルバムタイトルを取得
    /// 2. 前回のタイトルリストと比較
    /// 3. 変更がある場合、存在しないタイトルのキャッシュを削除
    /// 4. 新しいタイトルリストを保存
    pub(crate) fn sync_album_artwork_cache_with_library(&mut self) {
        // 現在ライブラリに存在する全アルバムタイトルを取得
        let current_titles: Vec<String> = self
            .app_vm
            .library
            .albums
            .iter()
            .map(|album| album.title.clone())
            .collect();

        // 変更がなければ何もしない
        if current_titles == self.album_artwork_album_titles {
            return;
        }

        // 存在すべきタイトルのセットを作成
        let keep: HashSet<String> = current_titles.iter().cloned().collect();
        
        // 各キャッシュから、存在しないタイトルのエントリを削除
        self.album_artwork_path_cache
            .retain(|title, _| keep.contains(title));
        self.album_artwork_texture_cache
            .retain(|title, _| keep.contains(title));
        self.album_artwork_queued
            .retain(|title| keep.contains(title));
        self.album_artwork_pending
            .retain(|title| keep.contains(title));
        
        // 新しいタイトルリストを保存
        self.album_artwork_album_titles = current_titles;
    }

    /// 指定されたアルバムタイトルのアートワークパスを解決します。
    /// 
    /// メタデータキャッシュを使用して、そのアルバムに属するトラックの
    /// ファイルパスを最大32件取得し、各トラックのディレクトリから
    /// アートワーク画像を検索します。
    /// 
    /// # 引数
    /// * `album_title` - アートワークを検索するアルバムのタイトル
    /// 
    /// # 戻り値
    /// 見つかったアートワーク画像のパス。
    /// アルバムに属するトラックが見つからない、またはアートワークが
    /// 見つからない場合は`None`を返します。
    pub(crate) fn resolve_album_artwork_path(&mut self, album_title: &str) -> Option<PathBuf> {
        // メタキャッシュが未初期化の場合は初期化
        if self.album_meta_cache.is_none() {
            self.album_meta_cache = MetaCache::open_default().ok();
        }

        let cache = self.album_meta_cache.as_ref()?;
        
        // アルバムに属するトラックのパスを最大32件取得
        let candidates = cache.album_location_paths(album_title, 32).ok()?;
        
        // 各トラックの場所からアートワークを検索
        for location in candidates {
            if let Some(path) = self.discover_artwork_for_track(Path::new(&location)) {
                return Some(path);
            }
        }
        
        None
    }

    /// 表示中のアルバムのアートワーク読み込みをキューに追加します。
    /// 
    /// この関数は、UIに表示されているアルバムのリストを受け取り、
    /// まだキャッシュされていないアルバムのアートワーク読み込みを
    /// バックグラウンドキューに追加します。
    /// 
    /// すでにキャッシュ済みのアルバムや、既にキューに入っているアルバムは
    /// スキップされます（重複防止）。
    /// 
    /// # 引数
    /// * `visible_album_titles` - 現在表示中のアルバムタイトルのリスト
    pub(crate) fn queue_album_artwork_for_visible(&mut self, visible_album_titles: &[String]) {
        for title in visible_album_titles {
            // 既にキャッシュ済みの場合はスキップ
            if self.album_artwork_texture_cache.contains_key(title) {
                continue;
            }
            
            // キューに追加（重複防止のため、queuedセットで管理）
            if self.album_artwork_queued.insert(title.clone()) {
                self.album_artwork_pending.push_back(title.clone());
            }
        }
    }

    /// アルバムアートワーク読み込みキューを処理します。
    /// 
    /// この関数は毎フレーム呼び出され、指定された予算（budget）の数だけ
    /// キューからアルバムタイトルを取り出し、アートワークの読み込みと
    /// テクスチャ生成を行います。
    /// 
    /// 処理の流れ：
    /// 1. キューからアルバムタイトルを1つ取り出す
    /// 2. アートワークパスを解決（キャッシュがあればそれを使用）
    /// 3. 画像を読み込み（外部ファイル → 埋め込み）
    /// 4. テクスチャを生成してキャッシュに保存
    /// 
    /// キューが残っている場合は、次フレームでの継続処理を要求します。
    /// 
    /// # 引数
    /// * `ctx` - eguiコンテキスト（テクスチャ生成用）
    /// * `budget` - このフレームで処理する最大アイテム数
    pub(crate) fn process_album_artwork_queue(&mut self, ctx: &egui::Context, budget: usize) {
        // 予算が0の場合は何もしない
        if budget == 0 {
            return;
        }

        // 予算の数だけキューを処理
        for _ in 0..budget {
            // キューから次のタイトルを取得
            let Some(title) = self.album_artwork_pending.pop_front() else {
                break; // キューが空
            };
            
            // queuedセットからも削除
            self.album_artwork_queued.remove(&title);

            // 既にキャッシュ済みの場合はスキップ（競合防止）
            if self.album_artwork_texture_cache.contains_key(&title) {
                continue;
            }

            // アートワークパスを取得（キャッシュ優先）
            let artwork_path = if let Some(cached) = self.album_artwork_path_cache.get(&title).cloned() {
                cached
            } else {
                // キャッシュにない場合は解決
                let resolved = self.resolve_album_artwork_path(&title);
                self.album_artwork_path_cache
                    .insert(title.clone(), resolved.clone());
                resolved
            };

            // パスが見つからない場合はスキップ
            let Some(path) = artwork_path else {
                continue;
            };

            // 画像の読み込みを試みる
            // まず外部ファイルとして読み込み、失敗したら埋め込みアートワークを試行
            let image_result: Result<image::DynamicImage, image::ImageError> = (|| {
                let reader = ImageReader::open(&path).map_err(image::ImageError::IoError)?;
                let reader = reader
                    .with_guessed_format()
                    .map_err(image::ImageError::IoError)?;
                reader.decode()
            })();

            let image = match image_result {
                Ok(image) => Some(image),
                Err(_) => self.extract_embedded_artwork(&path), // 埋め込みを試行
            };

            // 画像が取得できた場合、テクスチャを生成してキャッシュ
            if let Some(image) = image {
                let size = [image.width() as usize, image.height() as usize];
                let color_image =
                    egui::ColorImage::from_rgba_unmultiplied(size, &image.to_rgba8().into_vec());
                
                // テクスチャIDは "album_artwork:アルバムタイトル" 形式
                let texture_id = format!("album_artwork:{}", title);
                let texture =
                    ctx.load_texture(texture_id, color_image, egui::TextureOptions::default());
                
                self.album_artwork_texture_cache.insert(title, texture);
            }
        }

        // キューがまだ残っている場合、次フレームでの処理を要求
        // 16ms後（約60fps）に再描画
        if !self.album_artwork_pending.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    /// 指定されたアルバムタイトルに対応するアートワークテクスチャを取得します。
    /// 
    /// この関数はキャッシュからの読み取りのみを行い、
    /// キャッシュに存在しない場合は`None`を返します。
    /// 読み込みが必要な場合は、事前に`queue_album_artwork_for_visible`で
    /// キューに追加しておく必要があります。
    /// 
    /// # 引数
    /// * `album_title` - アルバムのタイトル
    /// 
    /// # 戻り値
    /// キャッシュに存在する場合はテクスチャハンドルのクローン。
    /// 存在しない場合は`None`。
    pub(crate) fn album_artwork_texture(&self, album_title: &str) -> Option<egui::TextureHandle> {
        self.album_artwork_texture_cache.get(album_title).cloned()
    }
}