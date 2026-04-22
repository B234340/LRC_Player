use lrc_player_application::infrastructure::persistence::MetaCache;
use eframe::egui;
use image::io::Reader as ImageReader;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardVisualKey};
use symphonia::default::get_probe;

const TRACK_ARTWORK_CACHE_MAX_ENTRIES: usize = 256;
const TRACK_ARTWORK_CACHE_MAX_TEXTURE_SIDE: u32 = 192;
const EMBEDDED_ARTWORK_PROBES_PER_FRAME: usize = 1;

#[derive(Clone, Copy)]
struct EmbeddedProbeBudget {
    window_started_at: Instant,
    used: usize,
}

impl Default for EmbeddedProbeBudget {
    fn default() -> Self {
        Self {
            window_started_at: Instant::now(),
            used: 0,
        }
    }
}

fn consume_embedded_probe_budget(ctx: &egui::Context) -> bool {
    let budget_id = egui::Id::new("track_display_embedded_probe_budget");
    let now = Instant::now();
    let mut budget = ctx
        .data(|data| data.get_temp::<EmbeddedProbeBudget>(budget_id))
        .unwrap_or_default();

    if now.duration_since(budget.window_started_at) >= Duration::from_millis(32) {
        budget.window_started_at = now;
        budget.used = 0;
    }

    if budget.used >= EMBEDDED_ARTWORK_PROBES_PER_FRAME {
        ctx.data_mut(|data| {
            data.insert_temp(budget_id, budget);
        });
        return false;
    }

    budget.used += 1;
    ctx.data_mut(|data| {
        data.insert_temp(budget_id, budget);
    });
    true
}

/// トラック表示用の情報をまとめた構造体
#[derive(Debug, Clone)]
pub struct TrackDisplayInfo {
    /// ファイルパス
    pub path: String,
    /// 曲名
    pub title: String,
    /// アーティスト名
    pub artist: String,
    /// アルバム名
    pub album: String,
}

/// 指定されたトラックファイルのディレクトリからアートワーク画像を検索する。
///
/// 以下の優先順位で画像を探す：
/// 1. 一般的なカバーアート名（cover.jpg, folder.jpg, front.jpg など）
/// 2. トラックファイル名と一致する画像ファイル名
///
/// 複数見つかった場合は、優先度の高いもの、さらにファイルサイズの大きいものを選択する。
/// 
/// # 引数
/// * `path` - トラックファイルのパス
/// 
/// # 戻り値
/// アートワーク画像が見つかった場合はそのパス、見つからなければ `None`
fn discover_artwork_for_track(path: &Path) -> Option<PathBuf> {
    // 親ディレクトリを取得（なければ探索終了）
    let parent = path.parent()?;
    // トラックファイル名（拡張子なし）を取得
    let stem = path.file_stem()?.to_string_lossy().to_string();
    
    // トラック名に基づくパターン（例：track1.jpg）
    let stem_patterns = [
        format!("{}.jpg", stem),
        format!("{}.jpeg", stem),
        format!("{}.png", stem),
        format!("{}.webp", stem),
    ];
    
    // 優先度の高い共通アートワーク名パターン
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

    // 最適な候補を保持する変数：(優先度, ファイルサイズ, パス)
    let mut best_match: Option<(u8, u64, PathBuf)> = None;
    
    // ファイルパターンをチェックするクロージャ
    let mut check_pattern = |name: &str, path: PathBuf| {
        let lower = name.to_lowercase();
        // 優先度の判定
        let priority = if preferred_patterns.contains(&lower.as_str()) {
            1 // 高優先度
        } else if stem_patterns.iter().any(|pattern| pattern.eq_ignore_ascii_case(&lower)) {
            2 // 中優先度（トラック名一致）
        } else {
            return; // パターンに一致しない場合は無視
        };
        
        // ファイルメタデータを取得してサイズを確認
        if let Ok(metadata) = std::fs::metadata(&path) {
            let size = metadata.len();
            match &best_match {
                // より高い優先度が見つかった場合のみ更新
                Some((best_priority, best_size, _)) if *best_priority < priority => {}
                // 同じ優先度だがサイズが同じか小さい場合は更新しない
                Some((best_priority, best_size, _)) if *best_priority == priority && *best_size >= size => {}
                // それ以外の場合は更新（より良い候補）
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
            if !path.is_file() {
                continue; // ファイルのみ対象
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                check_pattern(name, path.clone());
            }
        }
    }

    // 最適な候補のパスを返す
    best_match.map(|(_, _, path)| path)
}

/// 画像ファイルから egui のテクスチャを読み込む。
/// 
/// # 引数
/// * `ctx` - egui のコンテキスト
/// * `artwork_path` - 画像ファイルのパス
/// * `id` - テクスチャの識別子
/// 
/// # 戻り値
/// 読み込み成功時は `TextureHandle`、失敗時は `None`
fn load_artwork_texture(ctx: &egui::Context, artwork_path: &Path, id: &str) -> Option<egui::TextureHandle> {
    // 画像ファイルを開く
    if let Ok(reader) = ImageReader::open(artwork_path) {
        // フォーマットを推測
        if let Ok(reader) = reader.with_guessed_format() {
            // 画像をデコード
            if let Ok(image) = reader.decode() {
                return load_artwork_texture_from_image(ctx, image, id);
            }
        }
    }
    None
}

fn insert_cached_texture(
    ctx: &egui::Context,
    cache_key: String,
    texture: egui::TextureHandle,
) {
    let cache_id = egui::Id::new("track_display_artwork_texture_cache");
    let order_id = egui::Id::new("track_display_artwork_texture_cache_order");
    ctx.data_mut(|data| {
        let is_new_key = {
            let cache = data.get_temp_mut_or_default::<HashMap<String, egui::TextureHandle>>(cache_id);
            let is_new = !cache.contains_key(&cache_key);
            cache.insert(cache_key.clone(), texture);
            is_new
        };

        if is_new_key {
            let order = data.get_temp_mut_or_default::<VecDeque<String>>(order_id);
            order.push_back(cache_key);
        }

        loop {
            let over_limit = {
                let cache = data.get_temp_mut_or_default::<HashMap<String, egui::TextureHandle>>(cache_id);
                cache.len() > TRACK_ARTWORK_CACHE_MAX_ENTRIES
            };
            if !over_limit {
                break;
            }

            let oldest_key = {
                let order = data.get_temp_mut_or_default::<VecDeque<String>>(order_id);
                order.pop_front()
            };

            let Some(oldest_key) = oldest_key else {
                break;
            };

            let cache = data.get_temp_mut_or_default::<HashMap<String, egui::TextureHandle>>(cache_id);
            cache.remove(&oldest_key);
        }
    });
}

/// アートワークテクスチャをキャッシュから取得する。
/// キャッシュにない場合は読み込んでキャッシュに保存する。
/// 
/// # 引数
/// * `ctx` - egui のコンテキスト
/// * `artwork_path` - 画像ファイルのパス
/// * `id` - テクスチャの識別子
/// 
/// # 戻り値
/// テクスチャのハンドル、取得失敗時は `None`
fn get_cached_artwork_texture(
    ctx: &egui::Context,
    artwork_path: &Path,
    id: &str,
) -> Option<egui::TextureHandle> {
    // キャッシュの識別子
    let cache_id = egui::Id::new("track_display_artwork_texture_cache");
    let cache_key = artwork_path.to_string_lossy().to_string();

    // キャッシュに存在するか確認
    let existing = ctx.data(|data| {
        data.get_temp::<HashMap<String, egui::TextureHandle>>(cache_id)
            .and_then(|cache| cache.get(&cache_key).cloned())
    });
    if existing.is_some() {
        return existing; // キャッシュヒット
    }

    // キャッシュミス：新規読み込み
    let texture = load_artwork_texture(ctx, artwork_path, id)?;

    // キャッシュに保存（上限付き）
    insert_cached_texture(ctx, cache_key, texture.clone());
    Some(texture)
}

fn load_artwork_texture_from_image(
    ctx: &egui::Context,
    image: image::DynamicImage,
    id: &str,
) -> Option<egui::TextureHandle> {
    let image = if image.width() > TRACK_ARTWORK_CACHE_MAX_TEXTURE_SIDE
        || image.height() > TRACK_ARTWORK_CACHE_MAX_TEXTURE_SIDE
    {
        image.thumbnail(
            TRACK_ARTWORK_CACHE_MAX_TEXTURE_SIDE,
            TRACK_ARTWORK_CACHE_MAX_TEXTURE_SIDE,
        )
    } else {
        image
    };
    let size = [image.width() as usize, image.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(
        size,
        &image.to_rgba8().into_vec(),
    );
    Some(ctx.load_texture(id, color_image, egui::TextureOptions::default()))
}

fn extract_embedded_artwork(path: &Path) -> Option<image::DynamicImage> {
    let src = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
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

    let artwork_visual = metadata.visuals().iter().find(|visual| {
        visual.usage == Some(StandardVisualKey::FrontCover)
    }).or_else(|| {
        metadata.visuals().iter().max_by_key(|visual| {
            visual.dimensions
                .map(|dimensions| dimensions.width.saturating_mul(dimensions.height))
                .unwrap_or(0)
        })
    })?;

    image::load_from_memory(&artwork_visual.data).ok()
}

fn get_cached_embedded_artwork_texture(
    ctx: &egui::Context,
    track_path: &Path,
    id: &str,
) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::new("track_display_artwork_texture_cache");
    let miss_cache_id = egui::Id::new("track_display_embedded_artwork_miss_cache");
    let cache_key = track_path.to_string_lossy().to_string();

    let existing = ctx.data(|data| {
        data.get_temp::<HashMap<String, egui::TextureHandle>>(cache_id)
            .and_then(|cache| cache.get(&cache_key).cloned())
    });
    if existing.is_some() {
        return existing;
    }

    let known_miss = ctx.data(|data| {
        data.get_temp::<HashSet<String>>(miss_cache_id)
            .map(|misses| misses.contains(&cache_key))
            .unwrap_or(false)
    });
    if known_miss {
        return None;
    }

    // 埋め込みアート探索は重いため、1フレームの探索数を抑えてクリック時の引っかかりを防ぐ。
    if !consume_embedded_probe_budget(ctx) {
        return None;
    }

    let Some(image) = extract_embedded_artwork(track_path) else {
        ctx.data_mut(|data| {
            let misses = data.get_temp_mut_or_default::<HashSet<String>>(miss_cache_id);
            misses.insert(cache_key);
        });
        return None;
    };
    let texture = load_artwork_texture_from_image(ctx, image, id)?;
    insert_cached_texture(ctx, cache_key.clone(), texture.clone());
    ctx.data_mut(|data| {
        let misses = data.get_temp_mut_or_default::<HashSet<String>>(miss_cache_id);
        misses.remove(&cache_key);
    });
    Some(texture)
}

pub fn render_track_artwork_with_metadata_first(
    ui: &mut egui::Ui,
    track_path: &str,
    texture_id: &str,
    art_size: egui::Vec2,
    is_playing: bool,
) {
    let art_bg = if is_playing {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().faint_bg_color
    };

    let path = Path::new(track_path);
    if let Some(texture) = get_cached_embedded_artwork_texture(ui.ctx(), path, texture_id) {
        ui.image(&texture, art_size);
        return;
    }

    if let Some(art_path) = discover_artwork_for_track(path) {
        if let Some(texture) = get_cached_artwork_texture(ui.ctx(), &art_path, texture_id) {
            ui.image(&texture, art_size);
            return;
        }
    }

    let (art_rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
    ui.painter().rect_filled(art_rect, 4.0, art_bg);
    ui.painter().text(
        art_rect.center(),
        egui::Align2::CENTER_CENTER,
        "ART",
        egui::TextStyle::Small.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}

pub fn render_track_artwork_with_file_first(
    ui: &mut egui::Ui,
    track_path: &str,
    texture_id: &str,
    art_size: egui::Vec2,
    is_playing: bool,
) {
    let art_bg = if is_playing {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().faint_bg_color
    };

    let path = Path::new(track_path);
    if let Some(art_path) = discover_artwork_for_track(path) {
        if let Some(texture) = get_cached_artwork_texture(ui.ctx(), &art_path, texture_id) {
            ui.image(&texture, art_size);
            return;
        }
    }

    if let Some(texture) = get_cached_embedded_artwork_texture(ui.ctx(), path, texture_id) {
        ui.image(&texture, art_size);
        return;
    }

    let (art_rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
    ui.painter().rect_filled(art_rect, 4.0, art_bg);
    ui.painter().text(
        art_rect.center(),
        egui::Align2::CENTER_CENTER,
        "ART",
        egui::TextStyle::Small.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}

/// トラックの表示情報（タイトル、アーティスト、アルバム）を解決する。
/// 
/// 以下の順序で情報を取得する：
/// 1. インメモリキャッシュ (`metadata_cache`)
/// 2. 永続化メタデータキャッシュ (`meta_cache`)
/// 3. ファイル名からのフォールバック値
/// 
/// # 引数
/// * `track_path` - トラックファイルのパス
/// * `meta_cache` - 永続化されたメタデータキャッシュ（オプション）
/// * `metadata_cache` - インメモリのメタデータキャッシュ
/// 
/// # 戻り値
/// 解決されたトラック表示情報
pub fn resolve_track_display(
    track_path: &str,
    meta_cache: Option<&MetaCache>,
    metadata_cache: &mut HashMap<String, (String, String, String)>,
) -> TrackDisplayInfo {
    // ファイル名をフォールバック用のタイトルとして取得
    let fallback_title = Path::new(track_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(track_path)
        .to_string();

    // メタデータキャッシュから取得（なければ永続化キャッシュから読み込んで保存）
    let (title, artist, album) = metadata_cache
        .entry(track_path.to_string())
        .or_insert_with(|| {
            meta_cache
                .and_then(|cache| cache.track_metadata_by_location(track_path).ok().flatten())
                .map(|(title, artist, album)| {
                    // 空文字列の場合はフォールバック値を使用
                    let title = if title.trim().is_empty() {
                        fallback_title.clone()
                    } else {
                        title
                    };
                    let artist = if artist.trim().is_empty() {
                        "Unknown artist".to_string()
                    } else {
                        artist
                    };
                    let album = if album.trim().is_empty() {
                        "Unknown album".to_string()
                    } else {
                        album
                    };
                    (title, artist, album)
                })
                .unwrap_or_else(|| {
                    // メタデータが全くない場合のデフォルト値
                    (
                        fallback_title.clone(),
                        "Unknown artist".to_string(),
                        "Unknown album".to_string(),
                    )
                })
        })
        .clone(); // HashMap から借用の問題を避けるためクローン

    TrackDisplayInfo {
        path: track_path.to_string(),
        title,
        artist,
        album,
    }
}

/// トラックのアートワークを描画する。
/// 
/// アートワーク画像が見つかればそれを表示し、
/// 見つからない場合は「ART」と書かれたプレースホルダーを表示する。
/// 再生中フラグが立っている場合は、ハイライト表示する。
/// 
/// # 引数
/// * `ui` - egui の UI ビルダー
/// * `track_path` - トラックファイルのパス
/// * `texture_id` - テクスチャのキャッシュ用識別子
/// * `art_size` - 表示サイズ
/// * `is_playing` - 再生中かどうか（ハイライト表示用）
pub fn render_track_artwork(
    ui: &mut egui::Ui,
    track_path: &str,
    texture_id: &str,
    art_size: egui::Vec2,
    is_playing: bool,
) {
    // 再生中は選択色、それ以外は薄い背景色を使用
    let art_bg = if is_playing {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().faint_bg_color
    };

    // アートワーク画像の検索
    if let Some(art_path) = discover_artwork_for_track(Path::new(track_path)) {
        // キャッシュからテクスチャを取得して表示
        if let Some(texture) = get_cached_artwork_texture(ui.ctx(), &art_path, texture_id) {
            ui.image(&texture, art_size);
            return; // 画像表示成功
        }
    }

    // アートワークが見つからない場合：プレースホルダーを描画
    let (art_rect, _) = ui.allocate_exact_size(art_size, egui::Sense::hover());
    // 背景を塗りつぶし
    ui.painter().rect_filled(art_rect, 4.0, art_bg);
    // 中央に "ART" テキストを描画
    ui.painter().text(
        art_rect.center(),
        egui::Align2::CENTER_CENTER,
        "ART",
        egui::TextStyle::Small.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}