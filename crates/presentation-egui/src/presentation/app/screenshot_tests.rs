//! Screenshots of the REAL `GuiApp` (every page, dark and light) rendered headless with
//! egui_kittest + wgpu (lavapipe works). The app runs with a temp library database, temp
//! album folders with cover images, no persistence, no audio engine and no media controls;
//! fonts come from the system (plus `/mnt/c/Windows/Fonts` under WSL).
//!
//! ```text
//! APP_SCREENS_DIR=/some/dir cargo test -p lrc_player --lib app::screenshot_tests -- --ignored --test-threads=1
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ColorImage, Vec2};
use egui_kittest::Harness;
use lrc_player_application::config::{Config, Language, Theme};
use lrc_player_application::dto::{PlaylistDto, StateDto};
use lrc_player_application::infrastructure::persistence::MetaCache;
use lrc_player_domain::model::PlaybackState;

use crate::presentation::app::{AppOptions, GuiApp};
use crate::presentation::ui::{theme, Page, RightPanel};

const SIZE: Vec2 = Vec2::new(1280.0, 800.0);

const SONGS: &[(&str, &str, &str, u64, i32)] = &[
    ("夜に駆ける", "YOASOBI", "THE BOOK", 261_000, 2021),
    ("アイドル", "YOASOBI", "THE BOOK 3", 213_000, 2023),
    ("群青", "YOASOBI", "THE BOOK", 249_000, 2021),
    ("Lemon", "米津玄師", "STRAY SHEEP", 255_000, 2020),
    ("感電", "米津玄師", "STRAY SHEEP", 268_000, 2020),
    ("馬と鹿", "米津玄師", "STRAY SHEEP", 249_000, 2020),
    ("Pretender", "Official髭男dism", "Traveler", 326_000, 2019),
    ("Subtitle", "Official髭男dism", "Subtitle", 305_000, 2022),
    ("紅蓮華", "LiSA", "LEO-NiNE", 239_000, 2020),
    ("炎", "LiSA", "LEO-NiNE", 275_000, 2020),
    ("Blinding Lights", "The Weeknd", "After Hours", 200_000, 2020),
    ("Save Your Tears", "The Weeknd", "After Hours", 215_000, 2020),
    ("Levitating", "Dua Lipa", "Future Nostalgia", 203_000, 2020),
    ("Don't Start Now", "Dua Lipa", "Future Nostalgia", 183_000, 2020),
    ("ベテルギウス", "優里", "壱", 271_000, 2022),
    ("ドライフラワー", "優里", "壱", 285_000, 2022),
    ("怪獣の花唄", "Vaundy", "strobo", 223_000, 2020),
    ("踊り子", "Vaundy", "strobo", 208_000, 2020),
    ("きらり", "藤井 風", "LOVE ALL SERVE ALL", 205_000, 2022),
    ("花", "藤井 風", "LOVE ALL SERVE ALL", 216_000, 2022),
];

const LRC: &str = "[ti:夜に駆ける]\n[ar:YOASOBI]\n\
[00:10.00]沈むように溶けてゆくように\n\
[00:14.00]二人だけの空が広がる夜に\n\
[00:18.00]<00:18.00>「さよなら」<00:19.20>だけ<00:19.80>だった\n\
[00:22.00]<00:22.00>その<00:22.60>一言<00:23.40>で<00:23.80>全て<00:24.60>が<00:25.00>分かった\n\
[00:26.00]日が沈み出した空と君の姿\n\
[00:30.00]フェンス越しに重なっていた\n\
[00:34.00]\n\
[00:36.00]初めて会った日から\n\
[00:40.00]僕の心の全てを奪った\n\
[00:44.00]どこか儚い空気を纏う君は\n\
[00:48.00]寂しい目をしてたんだ\n";

fn out_dir() -> PathBuf {
    let dir = std::env::var_os("APP_SCREENS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/app-screens"));
    std::fs::create_dir_all(&dir).expect("create screenshot dir");
    dir
}

/// Synthetic cover art, deterministic per seed.
fn cover_image(seed: usize, size: usize) -> ColorImage {
    let hues = [0.58, 0.03, 0.78, 0.12, 0.45, 0.92, 0.33, 0.67];
    let h0 = hues[seed % hues.len()];
    let h1 = (h0 + 0.18) % 1.0;
    let mut px = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 / size as f32;
            let fy = y as f32 / size as f32;
            let d = ((fx - 0.65).powi(2) + (fy - 0.35).powi(2)).sqrt();
            let ring = (1.0 - (d * 3.2)).clamp(0.0, 1.0);
            let base: Color32 = egui::ecolor::Hsva::new(h0, 0.65, 0.35 + 0.35 * fy, 1.0).into();
            let glow: Color32 = egui::ecolor::Hsva::new(h1, 0.55, 0.95, 1.0).into();
            px.push(theme::blend(base, glow, ring * ring));
        }
    }
    ColorImage::new([size, size], px)
}

fn save_png(img: &ColorImage, path: &Path) {
    let [w, h] = img.size;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    image::RgbaImage::from_raw(w as u32, h as u32, bytes).unwrap().save(path).unwrap();
}

struct Fixture {
    _dir: tempfile::TempDir,
    db: PathBuf,
    paths: Vec<String>,
}

/// Temp library: album folders with covers (one album left without art) + a database.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("meta_cache.db");
    let cache = MetaCache::new(&db).unwrap();
    let mut albums: Vec<&str> = Vec::new();
    let mut paths = Vec::new();
    for (i, (title, artist, album, _, _)) in SONGS.iter().enumerate() {
        let folder = dir.path().join("music").join(artist).join(album);
        std::fs::create_dir_all(&folder).unwrap();
        if !albums.contains(album) {
            albums.push(album);
            if albums.len() % 5 != 4 {
                save_png(&cover_image(albums.len() - 1, 240), &folder.join("cover.png"));
            }
        }
        let path = folder.join(format!("{:02} {title}.flac", i + 1)).to_string_lossy().into_owned();
        let cid = format!("cid{i}");
        cache.upsert_track(&cid, Some(title), Some(artist), Some(artist), None, Some(album), None, 0).unwrap();
        cache.upsert_location(&cid, &path, None, None, 1, 0, i as i64, true, true).unwrap();
        if i == 0 {
            // Sidecar lyrics, found by the real discovery on the track-info worker.
            std::fs::write(Path::new(&path).with_extension("lrc"), LRC).unwrap();
        }
        paths.push(path);
    }
    Fixture { _dir: dir, db, paths }
}

fn new_app(ctx: &egui::Context, fx: &Fixture, dark: bool, lang: Language) -> GuiApp {
    let config = Config {
        theme: if dark { Theme::Dark } else { Theme::Light },
        language: lang,
        show_spectrum: false,
        ..Config::default()
    };
    let state = StateDto {
        scan_targets: vec![fx.db.parent().unwrap().join("music").to_string_lossy().into_owned()],
        last_scan_status: Some("Scan completed".into()),
        scan_scanned: Some(20),
        scan_parsed: Some(20),
        scan_registered: Some(20),
        ..Default::default()
    };
    let playlists = vec![
        PlaylistDto { id: "fav".into(), name: "favorite".into(), tracks: fx.paths[..6].to_vec() },
        PlaylistDto { id: "p2".into(), name: "作業用 BGM".into(), tracks: fx.paths[6..16].to_vec() },
        PlaylistDto { id: "p3".into(), name: "Night Drive".into(), tracks: fx.paths[10..14].to_vec() },
    ];
    GuiApp::with_options(
        ctx,
        AppOptions {
            persist: false,
            engine: false,
            meta_path: Some(fx.db.clone()),
            state: Some(state),
            config: Some(config),
            playlists: Some(playlists),
            load_fonts: true,
            media_controls: false,
        },
    )
}

/// Fill in what probing real files would provide (durations, years, track numbers) and
/// make the first track "now playing" with karaoke lyrics.
fn stage(app: &mut GuiApp, fx: &Fixture) {
    {
        let rows = Arc::make_mut(&mut app.library.rows);
        for row in rows.iter_mut() {
            if let Some((_, _, _, d, y)) = SONGS.iter().find(|s| s.0 == row.title) {
                row.duration_ms = Some(*d);
                row.year = Some(*y);
            }
            let n: u32 = row.path.file_name().unwrap().to_string_lossy()[..2].parse().unwrap_or(0);
            row.track_no = Some(n);
        }
    }
    app.library.rebuild();
    app.library.revision += 1;

    app.queue = fx.paths[..14].to_vec();
    app.current_queue_index = Some(0);
    app.on_track_changed(Some(0), fx.paths[0].clone());
    app.play_duration_ms = Some(SONGS[0].3);
    app.play_position_ms = 24_100;
    app.playback_state = PlaybackState::Playing;
    app.eq.enabled = true;
    app.eq.bands_db = [4.0, 3.0, 1.5, 0.0, -1.0, -0.5, 1.0, 2.5, 3.5, 4.0];
}

type H<'a> = Harness<'a, Option<GuiApp>>;

fn harness<'a>(dark: bool) -> H<'a> {
    Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(1.0)
        .with_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light })
        .build_ui_state(
            |ui, app: &mut Option<GuiApp>| {
                if let Some(app) = app.as_mut() {
                    let mut frame = eframe::Frame::_new_kittest();
                    eframe::App::ui(app, ui, &mut frame);
                }
            },
            None,
        )
}

fn pump(h: &mut H<'_>, secs: f32, until: impl Fn(&GuiApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs_f32(secs);
    while Instant::now() < deadline {
        h.run_steps(1);
        if h.state().as_ref().is_some_and(&until) {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Builds the app inside the harness context, waits for fonts / library / artwork.
fn start<'a>(fx: &Fixture, dark: bool, lang: Language) -> H<'a> {
    let mut h = harness(dark);
    let ctx = h.ctx.clone();
    *h.state_mut() = Some(new_app(&ctx, fx, dark, lang));
    pump(&mut h, 20.0, |a| a.ui.fonts_rx.is_none() && a.library.loaded);
    {
        let app = h.state_mut().as_mut().unwrap();
        stage(app, fx);
        app.ui.toasts = Default::default();
    }
    // Artwork (Now Playing + covers) and lyrics load on workers.
    pump(&mut h, 5.0, |a| a.play_artwork_handle.is_some() && a.loaded_timeline.is_some());
    with_app(&mut h, |a| {
        a.play_duration_ms = Some(SONGS[0].3);
        a.play_position_ms = 24_100;
    });
    h
}

fn shoot(h: &mut H<'_>, name: &str) {
    // Let covers requested by this page arrive.
    for _ in 0..8 {
        h.run_steps(2);
        std::thread::sleep(Duration::from_millis(60));
    }
    h.run_steps(4);
    let img = h.render().expect("render (needs a wgpu adapter)");
    let path = out_dir().join(format!("{name}.png"));
    img.save(&path).expect("save png");
    eprintln!("wrote {}", path.display());
}

fn with_app(h: &mut H<'_>, f: impl FnOnce(&mut GuiApp)) {
    f(h.state_mut().as_mut().unwrap());
}

fn all_pages(dark: bool) {
    let fx = fixture();
    let suffix = if dark { "dark" } else { "light" };
    let mut h = start(&fx, dark, Language::Ja);
    let paths = fx.paths.clone();

    with_app(&mut h, |a| {
        a.navigate(Page::NowPlaying);
        a.ui.right_panel = None;
    });
    shoot(&mut h, &format!("now_playing_{suffix}"));

    with_app(&mut h, |a| {
        a.navigate(Page::Library);
        a.ui.right_panel = Some(RightPanel::Queue);
        // Sticky: the harness advances time in large steps.
        a.ui.toasts.push(crate::presentation::ui::widgets::ToastKind::Success, "「夜に駆ける」をキューに追加しました", f32::INFINITY);
    });
    shoot(&mut h, &format!("library_queue_{suffix}"));

    with_app(&mut h, |a| {
        a.ui.toasts = Default::default();
        a.ui.right_panel = Some(RightPanel::Lyrics);
        a.ui.library_query = "yoasobi".into();
    });
    shoot(&mut h, &format!("library_search_lyrics_{suffix}"));

    with_app(&mut h, |a| {
        a.ui.library_query.clear();
        a.ui.right_panel = None;
        a.navigate(Page::Albums);
    });
    shoot(&mut h, &format!("albums_{suffix}"));

    with_app(&mut h, |a| {
        let key = a.library.album_key_of(Path::new(&paths[3])).unwrap();
        a.open_album(key);
    });
    shoot(&mut h, &format!("album_detail_{suffix}"));

    with_app(&mut h, |a| a.navigate(Page::Artists));
    shoot(&mut h, &format!("artists_{suffix}"));

    with_app(&mut h, |a| a.open_artist("米津玄師".into()));
    shoot(&mut h, &format!("artist_detail_{suffix}"));

    with_app(&mut h, |a| a.navigate(Page::Playlists));
    shoot(&mut h, &format!("playlists_{suffix}"));

    with_app(&mut h, |a| a.open_playlist("p2".into()));
    shoot(&mut h, &format!("playlist_detail_{suffix}"));

    with_app(&mut h, |a| a.navigate(Page::Folders));
    shoot(&mut h, &format!("folders_{suffix}"));

    with_app(&mut h, |a| a.navigate(Page::Settings));
    shoot(&mut h, &format!("settings_{suffix}"));

    with_app(&mut h, |a| {
        a.navigate(Page::Library);
        a.ui.eq_open = true;
    });
    shoot(&mut h, &format!("eq_popover_{suffix}"));

    with_app(&mut h, |a| {
        a.ui.eq_open = false;
        a.ui.modal = Some(super::Modal::DeletePlaylist { id: "p3".into(), name: "Night Drive".into() });
    });
    shoot(&mut h, &format!("dialog_delete_{suffix}"));
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_pages_dark() {
    all_pages(true);
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_pages_light() {
    all_pages(false);
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_english_narrow() {
    let fx = fixture();
    let mut h = start(&fx, true, Language::En);
    h.set_size(Vec2::new(860.0, 640.0));
    with_app(&mut h, |a| {
        a.ui.sidebar.collapsed = true;
        a.navigate(Page::NowPlaying);
    });
    shoot(&mut h, "now_playing_en_narrow");
    with_app(&mut h, |a| a.navigate(Page::Library));
    shoot(&mut h, "library_en_narrow");
    with_app(&mut h, |a| {
        a.ui.sidebar.collapsed = false;
        a.navigate(Page::Settings);
    });
    shoot(&mut h, "settings_en_narrow");
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_empty_library() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(true);
    let ctx = h.ctx.clone();
    *h.state_mut() = Some(GuiApp::with_options(
        &ctx,
        AppOptions {
            persist: false,
            engine: false,
            meta_path: Some(dir.path().join("meta_cache.db")),
            config: Some(Config { theme: Theme::Dark, ..Config::default() }),
            load_fonts: true,
            media_controls: false,
            ..Default::default()
        },
    ));
    pump(&mut h, 20.0, |a| a.ui.fonts_rx.is_none() && a.library.loaded);
    with_app(&mut h, |a| a.navigate(Page::Library));
    shoot(&mut h, "library_empty_dark");
    with_app(&mut h, |a| a.navigate(Page::NowPlaying));
    shoot(&mut h, "now_playing_empty_dark");
    with_app(&mut h, |a| a.navigate(Page::Folders));
    shoot(&mut h, "folders_empty_dark");
}
