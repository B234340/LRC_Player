//! アートワーク画像の管理を担当
//! - 画像ファイルの検索と読み込み
//! - 埋め込みアートワークの抽出
//! - テクスチャ生成（通常/ぼかし）
//! - クロップ処理

use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui;
use image::{self, GenericImageView, io::Reader as ImageReader};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardVisualKey};
use symphonia::default::get_probe;

use crate::presentation::app::GuiApp;

impl GuiApp {
    /// 再生中のトラックのアートワークを読み込み、テクスチャを準備します。
    /// 
    /// この関数は以下の処理を行います：
    /// 1. 再生パスの有効性チェック
    /// 2. アートワーク画像の検索（外部ファイル → 埋め込み）
    /// 3. 通常テクスチャとぼかしテクスチャの生成
    /// 
    /// パフォーマンス最適化のため、ソースが変更されていない場合は再生成をスキップします。
    /// LRCオーバーレイ有効時は、通常アートワークを先に表示し、ぼかし版は次フレームで生成します。
    /// 
    /// # 引数
    /// * `ctx` - eguiコンテキスト（テクスチャ生成用）
    pub(crate) fn ensure_play_artwork(&mut self, ctx: &egui::Context) {
        let path = Path::new(&self.play_path);
        
        // 再生パスが無効な場合は全アートワーク状態をクリア
        if path.as_os_str().is_empty() || !path.exists() {
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            self.play_artwork_blur_handle = None;
            self.play_artwork_blur_source = None;
            return;
        }

        // アートワーク画像のパスを検索
        let artwork_path = self.discover_artwork_for_track(path);
        
        // アートワークのソース識別子（ファイルパス）
        // 外部ファイルが見つからなければ、トラックファイル自体をソースとする（埋め込み用）
        let artwork_source = artwork_path
            .as_ref()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf());

        // テクスチャ再生成が必要か判定
        // - 通常テクスチャ：ソース変更 or 未生成
        // - ぼかしテクスチャ：LRCオーバーレイ有効かつ（ソース変更 or 未生成）
        let need_regular_texture = self.play_artwork_source.as_ref() != Some(&artwork_source)
            || self.play_artwork_handle.is_none();
        let need_blur_texture = self.lrc_overlay_enabled
            && (self.play_artwork_blur_source.as_ref() != Some(&artwork_source)
                || self.play_artwork_blur_handle.is_none());

        // どちらかのテクスチャ生成が必要な場合のみ画像を読み込む
        if need_regular_texture || need_blur_texture {
            // 画像の読み込み（外部ファイル優先、なければ埋め込み）
            let artwork_image = if let Some(ref art_path) = artwork_path {
                self.load_image_from_path(art_path)
            } else {
                self.extract_embedded_artwork(path)
            };

            // 画像が取得できなければ全アートワーク状態をクリア
            if artwork_image.is_none() {
                self.play_artwork_handle = None;
                self.play_artwork_source = None;
                self.play_artwork_blur_handle = None;
                self.play_artwork_blur_source = None;
                return;
            }

            let artwork_image = artwork_image.unwrap();

            // 通常テクスチャの生成
            if need_regular_texture {
                self.play_artwork_handle =
                    self.load_artwork_texture_from_image(ctx, artwork_image.clone(), "play_artwork");
                self.play_artwork_source =
                    self.play_artwork_handle.as_ref().map(|_| artwork_source.clone());
            }

            // ぼかしテクスチャの生成
            if self.lrc_overlay_enabled {
                if need_blur_texture {
                    // 通常テクスチャも同時に生成する場合、ぼかし生成は次フレームに遅延
                    // これにより初回表示時の引っかかりを軽減する
                    if need_regular_texture {
                        ctx.request_repaint_after(Duration::from_millis(16));
                    } else {
                        self.play_artwork_blur_handle = self.load_blurred_artwork_texture_from_image(
                            ctx,
                            artwork_image,
                            "play_artwork_blur",
                        );
                        self.play_artwork_blur_source =
                            self.play_artwork_blur_handle.as_ref().map(|_| artwork_source.clone());
                    }
                }
            } else {
                // LRCオーバーレイ無効時はぼかしテクスチャをクリア
                self.play_artwork_blur_handle = None;
                self.play_artwork_blur_source = None;
            }
        }
    }

    /// ファイルパスから画像を読み込みます。
    /// 
    /// 画像フォーマットは自動判別されます。
    /// 
    /// # 引数
    /// * `artwork_path` - 画像ファイルのパス
    /// 
    /// # 戻り値
    /// 読み込みに成功した場合は`DynamicImage`、失敗した場合は`None`
    pub(crate) fn load_image_from_path(&self, artwork_path: &Path) -> Option<image::DynamicImage> {
        let reader = ImageReader::open(artwork_path).ok()?;
        let reader = reader.with_guessed_format().ok()?;
        reader.decode().ok()
    }

    /// 画像を正方形にクロップします。
    /// 
    /// 画像の中央部分を切り出し、短辺の長さを一辺とする正方形にします。
    /// すでに正方形の場合はそのまま返します。
    /// 
    /// # 引数
    /// * `image` - クロップ対象の画像
    /// 
    /// # 戻り値
    /// 正方形にクロップされた画像
    pub(crate) fn crop_image_to_square(&self, image: image::DynamicImage) -> image::DynamicImage {
        let (width, height) = image.dimensions();
        if width == height {
            return image;
        }
        let side: u32 = width.min(height);
        let x = (width - side) / 2;
        let y = (height - side) / 2;
        let buffer = image.to_rgba8();
        let cropped = image::imageops::crop_imm(&buffer, x, y, side, side).to_image();
        image::DynamicImage::ImageRgba8(cropped)
    }

    /// 画像からeguiテクスチャを生成します。
    /// 
    /// 画像は正方形にクロップされてテクスチャ化されます。
    /// 大きすぎる画像は最大2048pxまで縮小されます。
    /// 
    /// # 引数
    /// * `ctx` - eguiコンテキスト
    /// * `image` - 元画像
    /// * `id` - テクスチャの識別子（eguiのテクスチャキャッシュ用）
    /// 
    /// # 戻り値
    /// 生成されたテクスチャハンドル、失敗時は`None`
    pub(crate) fn load_artwork_texture_from_image(
        &self,
        ctx: &egui::Context,
        image: image::DynamicImage,
        id: &str,
    ) -> Option<egui::TextureHandle> {
        // 大きすぎる画像はテクスチャ負荷軽減のため最大2048pxに縮小
        let image = image.thumbnail(2048, 2048);
        let image = self.crop_image_to_square(image);
        let size = [image.width() as usize, image.height() as usize];
        
        // egui用のカラー画像に変換
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &image.to_rgba8().into_vec());
        
        // テクスチャとしてロード
        Some(ctx.load_texture(id, color_image, egui::TextureOptions::default()))
    }

    /// 画像をぼかし加工してeguiテクスチャを生成します。
    /// 
    /// パフォーマンス最適化のため、以下の処理を行います：
    /// 1. 512x512に縮小（ぼかし処理の負荷軽減）
    /// 2. ガウシアンブラーを適用（半径10.0）
    /// 3. テクスチャとしてロード
    /// 
    /// # 引数
    /// * `ctx` - eguiコンテキスト
    /// * `image` - 元画像
    /// * `id` - テクスチャの識別子
    /// 
    /// # 戻り値
    /// 生成されたぼかしテクスチャハンドル、失敗時は`None`
    pub(crate) fn load_blurred_artwork_texture_from_image(
        &self,
        ctx: &egui::Context,
        image: image::DynamicImage,
        id: &str,
    ) -> Option<egui::TextureHandle> {
        // ぼかし処理の負荷を軽減するため512x512に縮小
        let image = image.thumbnail(512, 512);
        let image = self.crop_image_to_square(image);
        
        // ガウシアンブラーを適用（半径10.0）
        let blurred = image::imageops::blur(&image, 10.0);
        
        // テクスチャとしてロード
        self.load_artwork_texture_from_image(ctx, image::DynamicImage::ImageRgba8(blurred), id)
    }

    /// オーディオファイルから埋め込みアートワークを抽出します。
    /// 
    /// Symphoniaライブラリを使用してメタデータから画像を取得します。
    /// 画像の選択優先順位：
    /// 1. FrontCover（表ジャケット）としてマークされた画像
    /// 2. なければ最大サイズの画像
    /// 
    /// # 引数
    /// * `path` - オーディオファイルのパス
    /// 
    /// # 戻り値
    /// 抽出された画像、失敗時は`None`
    pub(crate) fn extract_embedded_artwork(&self, path: &Path) -> Option<image::DynamicImage> {
        // ファイルを開く
        let src = File::open(path).ok()?;
        
        // MediaSourceStreamを作成
        let mss = MediaSourceStream::new(Box::new(src), Default::default());
        
        // フォーマットをプローブ（検出）
        let probe = get_probe()
            .format(
                &Default::default(),
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .ok()?;
        
        let mut format = probe.format;
        let metadata_source = format.metadata();
        let metadata = metadata_source.current()?;

        // アートワーク画像を検索
        // 優先1: FrontCoverとしてマークされた画像
        // 優先2: 最大サイズの画像
        let artwork_visual = metadata
            .visuals()
            .iter()
            .find(|visual| visual.usage == Some(StandardVisualKey::FrontCover))
            .or_else(|| {
                metadata.visuals().iter().max_by_key(|visual| {
                    visual
                        .dimensions
                        .map(|dimensions| dimensions.width.saturating_mul(dimensions.height))
                        .unwrap_or(0)
                })
            })?;

        // メモリから画像をデコード
        image::load_from_memory(&artwork_visual.data).ok()
    }

    /// トラックファイルのディレクトリからアートワーク画像を検索します。
    /// 
    /// 以下の優先順位で最適な画像を選択します：
    /// 
    /// **優先度1**（最も優先）：
    /// - cover.jpg / cover.jpeg / cover.png / cover.webp
    /// - folder.jpg / folder.jpeg / folder.png / folder.webp
    /// - front.jpg / front.jpeg / front.png / front.webp
    /// 
    /// **優先度2**：
    /// - トラック名と同じベース名を持つ画像
    ///   （例: "song.flac" → "song.jpg", "song.png" など）
    /// 
    /// 同じ優先度内では、ファイルサイズが大きいものが選択されます。
    /// 
    /// # 引数
    /// * `path` - トラックファイルのパス
    /// 
    /// # 戻り値
    /// 検出されたアートワーク画像のパス、見つからなければ`None`
    pub(crate) fn discover_artwork_for_track(&self, path: &Path) -> Option<PathBuf> {
        // 親ディレクトリを取得
        let parent = path.parent()?;
        
        // ファイル名（拡張子なし）を取得
        let stem = path.file_stem()?.to_string_lossy().to_string();

        // トラック名と一致するパターン
        let stem_patterns = [
            format!("{}.jpg", stem),
            format!("{}.jpeg", stem),
            format!("{}.png", stem),
            format!("{}.webp", stem),
        ];

        // 優先される一般的なアートワーク名
        let preferred_patterns = [
            "cover.jpg",
            "cover.jpeg",
            "cover.png",
            "cover.webp",
            "folder.jpg",
            "folder.jpeg",
            "folder.png",
            "folder.webp",
            "front.jpg",
            "front.jpeg",
            "front.png",
            "front.webp",
        ];

        // 最適なマッチを保持: (優先度, ファイルサイズ, パス)
        let mut best_match: Option<(u8, u64, PathBuf)> = None;

        // パターンマッチング用クロージャ
        let mut check_pattern = |name: &str, path: PathBuf| {
            let lower = name.to_lowercase();
            
            // 優先度を判定
            let priority = if preferred_patterns.contains(&lower.as_str()) {
                1  // 優先パターン
            } else if stem_patterns
                .iter()
                .any(|pattern| pattern.eq_ignore_ascii_case(&lower))
            {
                2  // トラック名一致
            } else {
                return;  // マッチしない
            };

            // ファイルサイズを取得して最適なものを選択
            if let Ok(metadata) = fs::metadata(&path) {
                let size = metadata.len();
                match &best_match {
                    // より高い優先度のものが見つかった場合は置き換え
                    Some((best_priority, _, _)) if *best_priority < priority => {}
                    // 同じ優先度で、より大きいサイズのものはスキップ（現在のものを維持）
                    Some((best_priority, best_size, _))
                        if *best_priority == priority && *best_size >= size => {}
                    // それ以外（優先度が高い、または同じ優先度でサイズが大きい）は置き換え
                    _ => {
                        best_match = Some((priority, size, path));
                    }
                }
            }
        };

        // ディレクトリ内のファイルを走査
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                
                // ファイルのみを対象
                if !path.is_file() {
                    continue;
                }
                
                // ファイル名を取得してチェック
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    check_pattern(name, path.clone());
                }
            }
        }

        // パスのみを返す
        best_match.map(|(_, _, path)| path)
    }
}