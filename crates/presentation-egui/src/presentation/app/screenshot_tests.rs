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
    ("夜の灯台", "SORAMICHI", "BLUE HOUR", 261_000, 2021),
    ("ハミング", "SORAMICHI", "BLUE HOUR 3", 213_000, 2023),
    ("海鳴り", "SORAMICHI", "BLUE HOUR", 249_000, 2021),
    ("Lantern", "朝霧航太", "PAPER MOON", 255_000, 2020),
    ("雷鳴", "朝霧航太", "PAPER MOON", 268_000, 2020),
    ("鳥と舟", "朝霧航太", "PAPER MOON", 249_000, 2020),
    ("Wanderer", "Morning鯨club", "Coastline", 326_000, 2019),
    ("Footnote", "Morning鯨club", "Footnote", 305_000, 2022),
    ("紅の帆", "RiKO", "SEA-GLASS", 239_000, 2020),
    ("灯", "RiKO", "SEA-GLASS", 275_000, 2020),
    ("Neon Harbor", "The Lowtides", "Night Tram", 200_000, 2020),
    ("Salt and Silver", "The Lowtides", "Night Tram", 215_000, 2020),
    ("Floating", "Mira Vale", "Quiet Satellite", 203_000, 2020),
    ("Don't Look Back Yet", "Mira Vale", "Quiet Satellite", 183_000, 2020),
    ("シリウス", "湊人", "零", 271_000, 2022),
    ("ガラスの花", "湊人", "零", 285_000, 2022),
    ("鯨の子守唄", "Nagare", "drift", 223_000, 2020),
    ("踊り場", "Nagare", "drift", 208_000, 2020),
    ("ひかり", "柚木 凪", "SLOW TRAIN HOME", 205_000, 2022),
    ("波", "柚木 凪", "SLOW TRAIN HOME", 216_000, 2022),
];

const LRC: &str = "[ti:夜の灯台]\n[ar:SORAMICHI]\n\
[00:10.00]最終電車を見送ってから歩く\n\
[00:14.00]街灯の影が線路に伸びる夜\n\
[00:18.00]<00:18.00>「ただいま」<00:19.20>波に<00:19.80>言った\n\
[00:22.00]<00:22.00>遠く<00:22.60>灯台<00:23.40>が<00:23.80>夜空<00:24.60>を<00:25.00>照らした\n\
[00:26.00]窓に映るのは眠たげな海の色\n\
[00:30.00]ホームのベンチで風を聞いた\n\
[00:34.00]\n\
[00:36.00]改札を抜けた先に\n\
[00:40.00]潮風の香りが少しした\n\
[00:44.00]どこまでも続いていく砂浜\n\
[00:48.00]足あとを数えてたんだ\n";

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
                    let ctx = ui.ctx().clone();
                    eframe::App::logic(app, &ctx, &mut frame);
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
        a.ui.toasts.push(crate::presentation::ui::widgets::ToastKind::Success, "「夜の灯台」をキューに追加しました", f32::INFINITY);
    });
    shoot(&mut h, &format!("library_queue_{suffix}"));

    with_app(&mut h, |a| {
        a.ui.toasts = Default::default();
        a.ui.right_panel = Some(RightPanel::Lyrics);
        a.ui.library_query = "soramichi".into();
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

    with_app(&mut h, |a| a.open_artist("朝霧航太".into()));
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

fn click_at(h: &mut H<'_>, pos: egui::Pos2, button: egui::PointerButton) {
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
}

/// Dialogs, collation / default order, optional columns, links, the queue page, toasts with
/// actions and the no-device indicator (`APP_SCREENS_DIR` as above).
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_polish() {
    for dark in [true, false] {
        let fx = fixture();
        let suffix = if dark { "dark" } else { "light" };
        let mut h = start(&fx, dark, Language::Ja);

        with_app(&mut h, |a| {
            a.navigate(Page::Library);
            a.ui.right_panel = None;
        });
        shoot(&mut h, &format!("polish_library_default_order_{suffix}"));

        // Hover on the first row's artist link.
        h.hover_at(egui::pos2(775.0, 155.0));
        shoot(&mut h, &format!("polish_library_artist_link_hover_{suffix}"));
        h.remove_cursor();

        // Header context menu: optional columns.
        click_at(&mut h, egui::pos2(420.0, 115.0), egui::PointerButton::Secondary);
        shoot(&mut h, &format!("polish_library_column_menu_{suffix}"));
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        with_app(&mut h, |a| a.ui.library_columns = vec![crate::presentation::ui::views::Column::Year, crate::presentation::ui::views::Column::Genre]);
        shoot(&mut h, &format!("polish_library_year_genre_{suffix}"));
        with_app(&mut h, |a| a.ui.library_columns.clear());

        with_app(&mut h, |a| a.ui.modal = Some(super::Modal::RenamePlaylist { id: "p2".into(), name: "作業用 BGM".into() }));
        shoot(&mut h, &format!("polish_dialog_rename_{suffix}"));
        with_app(&mut h, |a| a.ui.modal = Some(super::Modal::DeletePlaylist { id: "p3".into(), name: "Night Drive".into() }));
        shoot(&mut h, &format!("polish_dialog_delete_{suffix}"));
        with_app(&mut h, |a| {
            let name = a.suggest_playlist_name(a.tr().t("playlist.from_queue"));
            a.ui.modal = Some(super::Modal::CreatePlaylist { name, tracks: a.queue.clone() });
        });
        shoot(&mut h, &format!("polish_dialog_save_queue_{suffix}"));
        with_app(&mut h, |a| a.ui.modal = None);

        with_app(&mut h, |a| a.navigate(Page::Artists));
        shoot(&mut h, &format!("polish_artists_{suffix}"));

        // Album card: hover the artist link of the first card.
        with_app(&mut h, |a| a.navigate(Page::Albums));
        h.run_steps(2);
        h.hover_at(egui::pos2(270.0, 0.0));
        h.run_steps(1);
        h.hover_at(egui::pos2(262.0, 321.0));
        shoot(&mut h, &format!("polish_album_card_artist_hover_{suffix}"));
        h.remove_cursor();

        with_app(&mut h, |a| a.navigate(Page::Queue));
        shoot(&mut h, &format!("polish_queue_page_{suffix}"));
        h.hover_at(egui::pos2(700.0, 352.0));
        shoot(&mut h, &format!("polish_queue_page_row_hover_{suffix}"));
        h.remove_cursor();

        with_app(&mut h, |a| {
            let tr = a.tr();
            // Sticky: the harness advances time in large steps.
            a.ui.toasts.push_with_action(
                crate::presentation::ui::widgets::ToastKind::Success,
                tr.f("toast.enqueued_one", &[&"海鳴り"]),
                f32::INFINITY,
                tr.t("toast.show_queue"),
                crate::presentation::ui::UiAction::Navigate(Page::Queue),
            );
            a.navigate(Page::Library);
            a.no_output_device = true;
        });
        shoot(&mut h, &format!("polish_toast_action_no_device_{suffix}"));
        with_app(&mut h, |a| {
            a.ui.toasts = Default::default();
            a.no_output_device = false;
        });

        // Narrower window with the queue panel open.
        h.set_size(Vec2::new(1000.0, 700.0));
        with_app(&mut h, |a| {
            a.ui.right_panel = Some(RightPanel::Queue);
            a.open_playlist("p2".into());
        });
        shoot(&mut h, &format!("polish_playlist_detail_1000_panel_{suffix}"));
        with_app(&mut h, |a| a.navigate(Page::Artists));
        shoot(&mut h, &format!("polish_artists_1000_panel_{suffix}"));
        with_app(&mut h, |a| {
            let key = a.library.album_key_of(Path::new(&fx.paths[3])).unwrap();
            a.open_album(key);
        });
        shoot(&mut h, &format!("polish_album_detail_1000_panel_{suffix}"));
        with_app(&mut h, |a| a.navigate(Page::Queue));
        shoot(&mut h, &format!("polish_queue_page_1000_panel_{suffix}"));
        with_app(&mut h, |a| {
            a.ui.right_panel = None;
            a.ui.album_card_size = 260.0;
            a.navigate(Page::Albums);
        });
        shoot(&mut h, &format!("polish_albums_large_cards_1000_{suffix}"));
    }
}

/// Now Playing layouts: wide / stacked by the space the page actually gets (right panel
/// open or closed), the enlarged artwork overlay, and the sharper texture at 2x.
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_now_playing_layouts() {
    let fx = fixture();
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        let mut h = start(&fx, dark, Language::Ja);
        for (w, ht) in [(1280.0, 800.0), (1000.0, 700.0)] {
            h.set_size(Vec2::new(w, ht));
            for (panel, tag) in [(None, ""), (Some(RightPanel::Queue), "_queue"), (Some(RightPanel::Lyrics), "_lyrics")] {
                if panel == Some(RightPanel::Lyrics) && !dark {
                    continue;
                }
                with_app(&mut h, |a| {
                    a.navigate(Page::NowPlaying);
                    a.ui.right_panel = panel;
                });
                shoot(&mut h, &format!("np_{w}x{ht}{tag}_{suffix}"));
                let layout = h.state().as_ref().unwrap().ui.now.layout.expect("layout");
                // Nothing overlaps: lyrics start right of / below the cover and its labels.
                if layout.wide {
                    assert!(layout.lyrics.left() >= layout.art.right() + 40.0);
                    assert!(layout.lyrics.width() >= 359.0, "{w}x{ht}{tag}: {layout:?}");
                } else {
                    assert!(layout.lyrics.top() >= layout.info.bottom() + 20.0, "{w}x{ht}{tag}: {layout:?}");
                }
                assert!(layout.lyrics.right() <= layout.inner.right() + 0.5);
            }
        }
        h.set_size(SIZE);
        with_app(&mut h, |a| {
            a.ui.right_panel = None;
            a.ui.now.overlay_open = true;
        });
        shoot(&mut h, &format!("np_overlay_{suffix}"));
        with_app(&mut h, |a| a.ui.now.overlay_open = false);
        if dark {
            h.set_size(Vec2::new(1920.0, 1080.0));
            shoot(&mut h, "np_1920x1080_dark");
            h.set_size(Vec2::new(860.0, 640.0));
            with_app(&mut h, |a| a.ui.sidebar.collapsed = true);
            shoot(&mut h, "np_860x640_collapsed_dark");
            h.set_size(Vec2::new(640.0, 720.0));
            shoot(&mut h, "np_640x720_collapsed_dark");
            with_app(&mut h, |a| a.ui.right_panel = Some(RightPanel::Queue));
            h.set_size(Vec2::new(900.0, 600.0));
            shoot(&mut h, "np_900x600_collapsed_queue_dark");
        }
    }
}

/// 2x with a large cover: the enlarged view asks for (and gets) a sharper texture, which is
/// freed after leaving the page.
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_now_playing_hires_2x() {
    let fx = fixture();
    let folder = Path::new(&fx.paths[0]).parent().unwrap().to_path_buf();
    save_png(&cover_image(0, 1800), &folder.join("cover.png"));
    let mut h: H<'_> = Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(2.0)
        .with_theme(egui::Theme::Dark)
        .build_ui_state(
            |ui, app: &mut Option<GuiApp>| {
                if let Some(app) = app.as_mut() {
                    let mut frame = eframe::Frame::_new_kittest();
                    let ctx = ui.ctx().clone();
                    eframe::App::logic(app, &ctx, &mut frame);
                    eframe::App::ui(app, ui, &mut frame);
                }
            },
            None,
        );
    let ctx = h.ctx.clone();
    *h.state_mut() = Some(new_app(&ctx, &fx, true, Language::Ja));
    pump(&mut h, 20.0, |a| a.ui.fonts_rx.is_none() && a.library.loaded);
    with_app(&mut h, |a| {
        stage(a, &fx);
        a.ui.toasts = Default::default();
        a.navigate(Page::NowPlaying);
    });
    pump(&mut h, 5.0, |a| a.play_artwork_handle.is_some());
    h.run_steps(2);
    with_app(&mut h, |a| a.ui.now.overlay_open = true);
    pump(&mut h, 5.0, |a| a.artwork.hires_texture_size().is_some());
    shoot(&mut h, "np_overlay_2x_dark");
    let size = h.state().as_ref().unwrap().artwork.hires_texture_size();
    assert_eq!(size, Some([1536, 1536]), "sharper cover at 2x");
    with_app(&mut h, |a| a.navigate(Page::Library));
    for _ in 0..40 {
        h.run_steps(1);
    }
    assert_eq!(h.state().as_ref().unwrap().artwork.hires_texture_size(), None, "freed after leaving");
}

/// Responsive layout of Settings / Folders (right panel open and closed, two window sizes,
/// plus a cramped one) and the EQ popover at normal and narrow widths, dark and light.
/// Long device / folder names exercise the ellipses.
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn responsive_settings_folders_eq() {
    let fx = fixture();
    let long_device = "スピーカー (Realtek(R) High Definition Audio) - Front Left/Right Analog Output #2";
    let long_dir = fx.db.parent().unwrap().join("music/Very Long Artist Name Collection/Disc 01 (Remastered Deluxe Edition 2024)");
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        let mut h = start(&fx, dark, Language::Ja);
        with_app(&mut h, |a| {
            a.preferred_output_device = Some(long_device.into());
            a.active_output_device = Some(long_device.into());
            a.device_names = vec![long_device.into(), "Headphones (USB Audio)".into()];
            a.scan_targets.push(long_dir.to_string_lossy().into_owned());
            a.scan_lrc_targets.push("/mnt/c/Users/someone/Music/Lyrics/Japanese/Anime Songs/2020-2024".into());
            a.scan_excludes.push("node_modules".into());
        });
        for (w, ht) in [(1280.0, 800.0), (1000.0, 700.0), (860.0, 640.0)] {
            h.set_size(Vec2::new(w, ht));
            for (page, name) in [(Page::Settings, "settings"), (Page::Folders, "folders")] {
                for (panel, pname) in [(None, "nopanel"), (Some(RightPanel::Queue), "panel")] {
                    if w < 900.0 && panel.is_none() {
                        continue;
                    }
                    with_app(&mut h, |a| {
                        a.ui.right_panel = panel;
                        a.navigate(page);
                    });
                    shoot(&mut h, &format!("{name}_{w}x{ht}_{pname}_{suffix}"));
                }
            }
        }
        // Whole pages (lower sections) with the panel open.
        h.set_size(Vec2::new(1000.0, 2300.0));
        with_app(&mut h, |a| {
            a.ui.right_panel = Some(RightPanel::Lyrics);
            a.navigate(Page::Settings);
            a.no_output_device = true;
        });
        shoot(&mut h, &format!("settings_1000x2300_panel_{suffix}"));
        h.set_size(Vec2::new(860.0, 1300.0));
        with_app(&mut h, |a| {
            a.no_output_device = false;
            a.navigate(Page::Folders);
        });
        shoot(&mut h, &format!("folders_860x1300_panel_{suffix}"));

        // EQ popover: normal, narrow (compressed columns) and very narrow (scrolls).
        with_app(&mut h, |a| {
            a.ui.right_panel = None;
            a.navigate(Page::Library);
            a.ui.eq_open = true;
        });
        for (w, ht, collapsed) in [(1280.0, 800.0, false), (560.0, 700.0, true), (400.0, 640.0, true), (360.0, 640.0, true)] {
            h.set_size(Vec2::new(w, ht));
            with_app(&mut h, |a| a.ui.sidebar.collapsed = collapsed);
            shoot(&mut h, &format!("eq_popover_{w}x{ht}_{suffix}"));
        }
    }
    // English labels are longer.
    let mut h = start(&fx, true, Language::En);
    h.set_size(Vec2::new(1000.0, 700.0));
    with_app(&mut h, |a| {
        a.ui.right_panel = Some(RightPanel::Queue);
        a.navigate(Page::Settings);
    });
    shoot(&mut h, "settings_en_1000x700_panel_dark");
    with_app(&mut h, |a| a.navigate(Page::Folders));
    shoot(&mut h, "folders_en_1000x700_panel_dark");
    with_app(&mut h, |a| {
        a.ui.right_panel = None;
        a.navigate(Page::Library);
        a.ui.eq_open = true;
    });
    h.set_size(Vec2::new(560.0, 700.0));
    shoot(&mut h, "eq_popover_en_560x700_dark");
}

/// Minimum window size (and slightly above) with 10 playlists, sidebar expanded and
/// collapsed: nav, scrolling playlists and the bottom block must not collide.
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_min_size() {
    let [min_w, min_h] = crate::presentation::ui::shell::MIN_WINDOW_SIZE;
    for dark in [true, false] {
        let fx = fixture();
        let suffix = if dark { "dark" } else { "light" };
        let mut h = start(&fx, dark, Language::Ja);
        with_app(&mut h, |a| {
            for i in 0..7 {
                a.playlists.push(PlaylistDto { id: format!("extra{i}"), name: format!("プレイリスト {}", i + 4), tracks: fx.paths[..3].to_vec() });
            }
            a.ui.right_panel = None;
            a.navigate(Page::Library);
        });
        for (w, hgt, tag) in [(min_w, min_h, "min"), (min_w + 60.0, min_h + 60.0, "above")] {
            h.set_size(Vec2::new(w, hgt));
            for collapsed in [false, true] {
                with_app(&mut h, |a| a.ui.sidebar.collapsed = collapsed);
                let side = if collapsed { "collapsed" } else { "expanded" };
                shoot(&mut h, &format!("minsize_{tag}_{side}_library_{suffix}"));
            }
        }
        // Every page at the minimum size, sidebar expanded and collapsed.
        h.set_size(Vec2::new(min_w, min_h));
        for collapsed in [false, true] {
            let side = if collapsed { "collapsed" } else { "expanded" };
            with_app(&mut h, |a| a.ui.sidebar.collapsed = collapsed);
            for (page, name) in [
                (Page::Queue, "queue"),
                (Page::Albums, "albums"),
                (Page::Artists, "artists"),
                (Page::Playlists, "playlists"),
                (Page::NowPlaying, "now_playing"),
                (Page::Folders, "folders"),
                (Page::Settings, "settings"),
            ] {
                with_app(&mut h, |a| a.navigate(page));
                shoot(&mut h, &format!("minsize_min_{side}_{name}_{suffix}"));
            }
            with_app(&mut h, |a| a.open_playlist("p2".into()));
            shoot(&mut h, &format!("minsize_min_{side}_playlist_detail_{suffix}"));
            with_app(&mut h, |a| {
                let key = a.library.album_key_of(Path::new(&fx.paths[3])).unwrap();
                a.open_album(key);
            });
            shoot(&mut h, &format!("minsize_min_{side}_album_detail_{suffix}"));
            with_app(&mut h, |a| a.open_artist("朝霧航太".into()));
            shoot(&mut h, &format!("minsize_min_{side}_artist_detail_{suffix}"));
        }
    }
}

/// Mixed JA / ZH-Hans / ZH-Hant / KO content: library rows, queue and Now Playing with a
/// Chinese title (per-element CJK fonts, item 13).
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_mixed_cjk() {
    const EXTRA: &[(&str, &str, &str, u64)] = &[
        ("林晓风 - 晴空", "林晓风", "海边的风", 269_000),
        ("我們的海", "海邊樂團", "遠方", 281_000),
        ("바다", "하늘빛", "Harbor", 214_000),
        ("远行", "陈小雨", "小雨", 230_000),
        ("说再见", "白鹭乐队", "夏天的72个车站", 223_000),
    ];
    for dark in [true, false] {
        let fx = fixture();
        let suffix = if dark { "dark" } else { "light" };
        let mut extra_paths = Vec::new();
        {
            let cache = MetaCache::new(&fx.db).unwrap();
            for (i, (title, artist, album, _)) in EXTRA.iter().enumerate() {
                let path = fx.db.parent().unwrap().join("music").join(format!("cjk{i}.flac")).to_string_lossy().into_owned();
                let cid = format!("cjk{i}");
                cache.upsert_track(&cid, Some(title), Some(artist), Some(artist), None, Some(album), None, 0).unwrap();
                cache.upsert_location(&cid, &path, None, None, 1, 0, 100 + i as i64, true, true).unwrap();
                extra_paths.push(path);
            }
        }
        let mut h = start(&fx, dark, Language::Ja);
        with_app(&mut h, |a| {
            let rows = Arc::make_mut(&mut a.library.rows);
            for row in rows.iter_mut() {
                if let Some(e) = EXTRA.iter().find(|e| e.0 == row.title) {
                    row.duration_ms = Some(e.3);
                }
            }
            a.library.rebuild();
            a.library.revision += 1;
            a.ui.right_panel = None;
            a.navigate(Page::Library);
            // Mixed rows first: search for nothing, sort by title descending puts CJK on top.
            a.ui.library_table.sort = crate::presentation::ui::SortSpec { column: crate::presentation::ui::SortColumn::Title, descending: true };
        });
        shoot(&mut h, &format!("cjk_library_{suffix}"));

        with_app(&mut h, |a| {
            let mut q = extra_paths.clone();
            q.extend(fx.paths[..6].iter().cloned());
            a.queue = q;
            a.current_queue_index = Some(0);
            a.on_track_changed(Some(0), extra_paths[0].clone());
            a.play_duration_ms = Some(EXTRA[0].3);
            a.play_position_ms = 30_000;
            a.navigate(Page::Queue);
        });
        shoot(&mut h, &format!("cjk_queue_{suffix}"));
        with_app(&mut h, |a| a.navigate(Page::NowPlaying));
        shoot(&mut h, &format!("cjk_now_playing_{suffix}"));
        with_app(&mut h, |a| {
            a.navigate(Page::Library);
            a.ui.right_panel = Some(RightPanel::Queue);
        });
        shoot(&mut h, &format!("cjk_library_queue_panel_{suffix}"));
    }
}

/// The lyric text-shift popover on Now Playing (`APP_SCREENS_DIR` as above).
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn real_app_lyrics_shift_popover() {
    for dark in [true, false] {
        let fx = fixture();
        let suffix = if dark { "dark" } else { "light" };
        let mut h = start(&fx, dark, Language::Ja);
        with_app(&mut h, |a| {
            a.navigate(Page::NowPlaying);
            a.ui.right_panel = None;
        });
        h.run_steps(2);
        with_app(&mut h, |a| a.set_lyrics_shift(-1, 0));
        h.run_steps(2);
        let pos = std::env::var("SHIFT_BTN")
            .ok()
            .and_then(|v| v.split_once(',').map(|(x, y)| egui::pos2(x.parse().unwrap(), y.parse().unwrap())))
            .unwrap_or(egui::pos2(968.0, 656.0));
        click_at(&mut h, pos, egui::PointerButton::Primary);
        h.run_steps(2);
        shoot(&mut h, &format!("lyrics_shift_popover_{suffix}"));
        h.key_press(egui::Key::Escape);
        // A track without lyrics: the "search online" button.
        with_app(&mut h, |a| {
            a.lyrics_candidates.clear();
            a.selected_lyrics_index = None;
            a.loaded_timeline = None;
            a.lyrics_revision += 1;
        });
        h.run_steps(2);
        shoot(&mut h, &format!("lyrics_search_online_{suffix}"));
        with_app(&mut h, |a| a.ui.right_panel = Some(crate::presentation::ui::RightPanel::Lyrics));
        with_app(&mut h, |a| a.navigate(Page::Library));
        h.run_steps(2);
        shoot(&mut h, &format!("lyrics_panel_search_online_{suffix}"));
    }
}

// ---------------------------------------------------------------------------------------
// Sound quality: Settings → 音質, the EQ popover's parametric tab and the signal path.
// ---------------------------------------------------------------------------------------

use lrc_player_application::infrastructure::audio::engine::{
    CrossfeedLevel, DeviceInfo, EngineSnapshot, EngineState, GainSource, ResamplerKind, SignalPath,
};

/// An AutoEq profile in the shape AutoEq publishes (`ParametricEQ.txt`).
const AUTOEQ_PROFILE: &str = "Preamp: -6.4 dB\n\
Filter 1: ON LSC Fc 105 Hz Gain 6.0 dB Q 0.70\n\
Filter 2: ON PK Fc 52 Hz Gain 1.1 dB Q 1.20\n\
Filter 3: ON PK Fc 180 Hz Gain -2.3 dB Q 0.53\n\
Filter 4: ON PK Fc 1365 Hz Gain -1.4 dB Q 1.90\n\
Filter 5: ON PK Fc 3062 Hz Gain -3.0 dB Q 3.62\n\
Filter 6: ON PK Fc 4655 Hz Gain 2.5 dB Q 2.35\n\
Filter 7: ON PK Fc 6164 Hz Gain -3.5 dB Q 4.51\n\
Filter 8: ON PK Fc 8063 Hz Gain 3.2 dB Q 2.52\n\
Filter 9: ON PK Fc 11000 Hz Gain -1.9 dB Q 1.43\n\
Filter 10: ON HSC Fc 10000 Hz Gain 2.5 dB Q 0.70\n";

fn snapshot(signal: SignalPath, path: &str) -> EngineSnapshot {
    EngineSnapshot {
        state: EngineState::Playing,
        path: Some(PathBuf::from(path)),
        position_ms: 24_100,
        duration_ms: Some(SONGS[0].3),
        device: Some(signal.output.clone()),
        underruns: 0,
        volume: signal.volume,
        muted: false,
        signal: Some(signal),
    }
}

/// FLAC 44.1/16 resampled to a shared 48 kHz device with every stage on.
fn processed_chain() -> SignalPath {
    SignalPath {
        codec: "flac".into(),
        source_rate: 44_100,
        source_bits: Some(16),
        source_channels: 2,
        resampled: true,
        resampler: Some(ResamplerKind::Fft),
        channels_mapped: false,
        output: DeviceInfo {
            name: "スピーカー (Realtek(R) High Definition Audio)".into(),
            sample_rate: 48_000,
            channels: 2,
            sample_format: "F32".into(),
            buffer_frames: Some(2400),
            follows_default: true,
            exclusive: false,
            dither: false,
        },
        eq: true,
        parametric_eq: true,
        crossfeed: Some(CrossfeedLevel::Medium),
        normalization_db: -6.3,
        normalization_source: GainSource::Hint,
        headroom_db: -4.2,
        volume: 0.6,
        volume_active: true,
        limiter_engaged: false,
        bit_perfect: false,
    }
}

/// FLAC 96/24 to an exclusive 24-bit USB DAC, everything off.
fn bit_perfect_chain(exclusive: bool) -> SignalPath {
    SignalPath {
        codec: "flac".into(),
        source_rate: 96_000,
        source_bits: Some(24),
        source_channels: 2,
        resampled: false,
        resampler: None,
        channels_mapped: false,
        output: DeviceInfo {
            name: "USB DAC (Topping E30)".into(),
            sample_rate: 96_000,
            channels: 2,
            sample_format: if exclusive { "I24".into() } else { "F32".into() },
            buffer_frames: None,
            follows_default: false,
            exclusive,
            dither: exclusive,
        },
        eq: false,
        parametric_eq: false,
        crossfeed: None,
        normalization_db: 0.0,
        normalization_source: GainSource::None,
        headroom_db: 0.0,
        volume: 1.0,
        volume_active: false,
        limiter_engaged: false,
        bit_perfect: true,
    }
}

/// `APP_SCREENS_DIR=<dir> cargo test -p lrc_player --lib app::screenshot_tests::sound_quality -- --ignored --test-threads=1`
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn sound_quality() {
    let fx = fixture();
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        let mut h = start(&fx, dark, Language::Ja);
        let path = fx.paths[0].clone();
        with_app(&mut h, |a| {
            a.engine_snapshot = Some(snapshot(processed_chain(), &path));
            a.config.crossfeed_enabled = true;
            a.config.normalization_preamp_db = -2.0;
            a.loudness_analysis_progress();
        });
        // Settings → 音質, scrolled into view.
        for (w, ht, panel) in [(1280.0, 800.0, None), (1280.0, 800.0, Some(RightPanel::Queue)), (1000.0, 700.0, Some(RightPanel::Queue))] {
            h.set_size(Vec2::new(w, ht));
            with_app(&mut h, |a| {
                a.ui.right_panel = panel;
                a.navigate(Page::Settings);
                a.ui.scroll_to_sound = 3;
            });
            let pname = if panel.is_some() { "panel" } else { "nopanel" };
            shoot(&mut h, &format!("sound_settings_{w}x{ht}_{pname}_{suffix}"));
        }
        // Lower part of the section (signal path) on a tall window.
        h.set_size(Vec2::new(1000.0, 1500.0));
        with_app(&mut h, |a| {
            a.navigate(Page::Settings);
            a.ui.scroll_to_sound = 3;
        });
        shoot(&mut h, &format!("sound_settings_1000x1500_panel_{suffix}"));

        // EQ popover, parametric tab with a loaded AutoEq profile.
        h.set_size(SIZE);
        with_app(&mut h, |a| {
            a.ui.right_panel = None;
            a.navigate(Page::Library);
            a.ui.toasts = Default::default();
            assert!(a.import_autoeq_text(AUTOEQ_PROFILE));
            a.ui.eq_tab = crate::presentation::ui::views::EqTab::Parametric;
            a.ui.eq_open = true;
        });
        shoot(&mut h, &format!("eq_parametric_1280x800_{suffix}"));
        for (w, ht, collapsed) in [(1000.0, 700.0, false), (560.0, 700.0, true), (400.0, 640.0, true)] {
            h.set_size(Vec2::new(w, ht));
            with_app(&mut h, |a| a.ui.sidebar.collapsed = collapsed);
            shoot(&mut h, &format!("eq_parametric_{w}x{ht}_{suffix}"));
        }
        h.set_size(SIZE);
        with_app(&mut h, |a| {
            a.ui.sidebar.collapsed = false;
            a.ui.eq_tab = crate::presentation::ui::views::EqTab::Graphic;
        });
        shoot(&mut h, &format!("eq_graphic_with_graph_{suffix}"));

        // Signal path popover: processed, untouched (shared) and bit-perfect (exclusive).
        with_app(&mut h, |a| {
            a.ui.eq_open = false;
            a.ui.toasts = Default::default();
            a.ui.signal_open = true;
        });
        shoot(&mut h, &format!("signal_processed_{suffix}"));
        with_app(&mut h, |a| a.engine_snapshot = Some(snapshot(bit_perfect_chain(false), &path)));
        shoot(&mut h, &format!("signal_untouched_{suffix}"));
        with_app(&mut h, |a| a.engine_snapshot = Some(snapshot(bit_perfect_chain(true), &path)));
        shoot(&mut h, &format!("signal_bit_perfect_{suffix}"));
        h.set_size(Vec2::new(760.0, 640.0));
        shoot(&mut h, &format!("signal_bit_perfect_760x640_{suffix}"));
    }
}

/// A synthetic IEM target (bass shelf, pinna gain, treble roll-off), absolute SPL.
fn autoeq_target_db(f: f32) -> f32 {
    let f = f as f64;
    let bass = 7.0 / (1.0 + (f / 110.0).powi(2));
    let pinna = 9.0 * (-(f / 2900.0).log2().powi(2) / (2.0 * 0.55)).exp();
    let roll = -7.0 / (1.0 + (9500.0 / f).powi(4));
    (75.0 + bass + pinna + roll) as f32
}

/// A synthetic DIY IEM measurement (as IEM_Measure exports it): less bass, a 6 kHz peak, a
/// dip at 3 kHz, a treble resonance; `skew` makes L and R differ a little.
fn autoeq_measured_db(f: f32, skew: f32) -> f32 {
    let x = f as f64;
    let g = |hz: f64, db: f64, w: f64| db * (-(x / hz).log2().powi(2) / (2.0 * w * w)).exp();
    let base = autoeq_target_db(f) as f64 + 22.0 - 5.0 / (1.0 + (x / 90.0).powi(2));
    (base + g(3100.0, -4.5, 0.25) + g(6000.0 * (1.0 + skew as f64 * 0.05), 7.0, 0.12) + g(12_000.0, -6.0, 0.15)
        + g(1300.0, 1.5 + skew as f64, 0.4)) as f32
}

fn autoeq_curve(f: impl Fn(f32) -> f32) -> lrc_player_application::infrastructure::audio::engine::FrequencyResponse {
    let freqs: Vec<f32> = (0..=480).map(|i| 20.0 * 1000f32.powf(i as f32 / 480.0)).collect();
    let db = freqs.iter().map(|&x| f(x)).collect();
    lrc_player_application::infrastructure::audio::engine::FrequencyResponse { freqs, db }
}

/// Runs the auto-create synchronously (worker + polling).
fn run_auto_eq(a: &mut GuiApp) {
    a.start_auto_eq(None);
    let deadline = Instant::now() + Duration::from_secs(30);
    while a.auto_eq_running() {
        a.poll_auto_eq();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    a.ui.toasts = Default::default();
}

/// EQ popover, parametric tab with auto-EQ curves: L+R with the 自動作成 section expanded
/// (before and after creating filters), collapsed, and L・R 別 (R selected) — at 1280x800,
/// 1000x700, 560x700 and 400x640, dark and light.
///
/// `APP_SCREENS_DIR=<dir> cargo test -p lrc_player --lib app::screenshot_tests::autoeq -- --ignored --test-threads=1`
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn autoeq_parametric() {
    use super::auto_eq::{CurveSlot, LoadedCurve};
    use crate::presentation::ui::views::EqTab;
    use lrc_player_application::infrastructure::audio::engine::{PeqChannelMode, PeqSide};
    let fx = fixture();
    let sizes = [(1280.0, 800.0, false), (1000.0, 700.0, false), (560.0, 700.0, true), (400.0, 640.0, true)];
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        let mut h = start(&fx, dark, Language::Ja);
        with_app(&mut h, |a| {
            a.ui.right_panel = None;
            a.navigate(Page::Library);
            a.ui.toasts = Default::default();
            a.ui.eq_tab = EqTab::Parametric;
            a.ui.eq_open = true;
            a.set_auto_eq_curve(CurveSlot::Target, LoadedCurve::new("IEM_target_harman2019.csv".into(), autoeq_curve(autoeq_target_db)));
            a.set_auto_eq_curve(CurveSlot::Source, LoadedCurve::new("DIY-IEM_L_2026-11-14.txt".into(), autoeq_curve(|f| autoeq_measured_db(f, 0.0))));
            a.auto_eq.expanded = true;
        });
        let shoot_sizes = |h: &mut H<'_>, name: &str| {
            for (w, ht, collapsed) in sizes {
                h.set_size(Vec2::new(w, ht));
                with_app(h, |a| a.ui.sidebar.collapsed = collapsed);
                shoot(h, &format!("{name}_{w}x{ht}_{suffix}"));
            }
        };
        // L+R, curves loaded, nothing created yet.
        shoot_sizes(&mut h, "autoeq_linked_loaded");
        // After 「フィルターを作成」.
        with_app(&mut h, run_auto_eq);
        shoot_sizes(&mut h, "autoeq_linked_created");
        // Section collapsed: the filter list gets the room.
        with_app(&mut h, |a| a.auto_eq.expanded = false);
        shoot_sizes(&mut h, "autoeq_linked_collapsed");
        // L・R 別 with separate L / R measurements, R selected, both fitted.
        with_app(&mut h, |a| {
            a.auto_eq.expanded = true;
            a.set_peq_channel_mode(PeqChannelMode::Separate);
            a.set_auto_eq_same_source(false);
            a.set_auto_eq_curve(CurveSlot::SourceRight, LoadedCurve::new("DIY-IEM_R_2026-11-14.txt".into(), autoeq_curve(|f| autoeq_measured_db(f, 1.0))));
            run_auto_eq(a);
            a.set_peq_edit_side(PeqSide::Right);
        });
        shoot_sizes(&mut h, "autoeq_separate_r");
        with_app(&mut h, |a| {
            a.auto_eq.expanded = false;
            a.set_peq_edit_side(PeqSide::Left);
        });
        shoot_sizes(&mut h, "autoeq_separate_l_collapsed");
    }
}

/// Settings → 歌詞 → 「歌詞のない曲をまとめて取得」: idle (with recently checked tracks), the
/// confirmation dialog, running (fake LRCLIB that stalls on the 8th track), stopping and the
/// last result, at 1280x800 and 1000x700 with the right panel, dark and light.
///
/// `APP_SCREENS_DIR=<dir> cargo test -p lrc_player --lib app::screenshot_tests::lyrics_batch -- --ignored --test-threads=1`
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn lyrics_batch_settings() {
    use lrc_player_application::infrastructure::lyrics::{LrclibQuery, LrclibResult};
    let sizes = [(1280.0, 800.0, None), (1000.0, 700.0, Some(RightPanel::Queue))];
    for dark in [true, false] {
        // Fresh files per theme: the run writes .lrc files.
        let fx = fixture();
        let suffix = if dark { "dark" } else { "light" };
        let mut h = start(&fx, dark, Language::Ja);
        with_app(&mut h, |a| {
            let misses = a.lyric_misses();
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let mut m = misses.lock().unwrap();
            for p in &fx.paths[15..18] {
                m.record(p, super::lyric_batch::MissKind::NotFound, now);
            }
        });
        let show = |h: &mut H<'_>, w: f32, ht: f32, panel: Option<RightPanel>| {
            h.set_size(Vec2::new(w, ht));
            with_app(h, |a| {
                a.ui.right_panel = panel;
                a.navigate(Page::Settings);
                a.ui.lyric_batch.scroll_into_view = 3;
            });
        };
        for (w, ht, panel) in sizes {
            show(&mut h, w, ht, panel);
            pump(&mut h, 10.0, |a| a.ui.lyric_batch.has_count());
            let pname = if panel.is_some() { "panel" } else { "nopanel" };
            shoot(&mut h, &format!("lyrics_batch_idle_{w}x{ht}_{pname}_{suffix}"));
        }
        with_app(&mut h, |a| a.ui.lyric_batch.confirm = true);
        shoot(&mut h, &format!("lyrics_batch_dialog_1000x700_{suffix}"));
        with_app(&mut h, |a| a.ui.lyric_batch.confirm = false);

        // Running: a few varied answers, then the fake server stalls until `release` drops.
        let (release, stall) = crossbeam_channel::bounded::<()>(0);
        let mut n = 0usize;
        let fetch: super::lyric_batch::FetchFn = Box::new(move |_q: &LrclibQuery| {
            n += 1;
            Ok(match n {
                1 | 4 => LrclibResult::Synced { lrc: "[00:01.00]la\n".into(), word_sync: false },
                2 | 5 | 6 => LrclibResult::NotFound,
                3 => LrclibResult::Instrumental,
                7 => LrclibResult::PlainOnly,
                _ => {
                    let _ = stall.recv();
                    LrclibResult::NotFound
                }
            })
        });
        let ctx = h.ctx.clone();
        with_app(&mut h, |a| a.start_lyrics_batch_with(&ctx, fetch, Duration::ZERO));
        pump(&mut h, 10.0, |a| a.lyrics_batch_progress().is_some_and(|(c, _)| c.done >= 7));
        for (w, ht, panel) in sizes {
            show(&mut h, w, ht, panel);
            let pname = if panel.is_some() { "panel" } else { "nopanel" };
            shoot(&mut h, &format!("lyrics_batch_running_{w}x{ht}_{pname}_{suffix}"));
        }
        with_app(&mut h, |a| a.cancel_lyrics_batch());
        shoot(&mut h, &format!("lyrics_batch_stopping_1000x700_{suffix}"));
        drop(release);
        pump(&mut h, 10.0, |a| a.lyrics_batch_progress().is_none());
        for (w, ht, panel) in sizes {
            show(&mut h, w, ht, panel);
            let pname = if panel.is_some() { "panel" } else { "nopanel" };
            shoot(&mut h, &format!("lyrics_batch_done_{w}x{ht}_{pname}_{suffix}"));
        }
    }
}

// ---------------------------------------------------------------------------------------
// Desktop lyrics
// ---------------------------------------------------------------------------------------

/// Fonts for widget-only shots (the app loads its own on a worker).
fn widget_fonts() -> &'static egui::FontDefinitions {
    static DEFS: std::sync::OnceLock<egui::FontDefinitions> = std::sync::OnceLock::new();
    DEFS.get_or_init(|| crate::presentation::ui::fonts::build_definitions(&[Path::new("/mnt/c/Windows/Fonts")]).0)
}

/// A stand-in desktop behind the overlay: a gradient wallpaper and a white "window".
fn paint_desktop(ui: &egui::Ui) {
    let r = ui.max_rect();
    let mut mesh = egui::epaint::Mesh::default();
    let (a, b) = (Color32::from_rgb(28, 60, 120), Color32::from_rgb(214, 120, 70));
    mesh.colored_vertex(r.left_top(), a);
    mesh.colored_vertex(r.right_top(), b);
    mesh.colored_vertex(r.right_bottom(), b);
    mesh.colored_vertex(r.left_bottom(), a);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
    let win = egui::Rect::from_min_size(r.min + egui::vec2(r.width() * 0.55, r.height() * 0.15), egui::vec2(r.width() * 0.5, r.height()));
    ui.painter().rect_filled(win, 6.0, Color32::from_rgb(246, 246, 248));
    for i in 0..6 {
        let y = win.top() + 14.0 + i as f32 * 16.0;
        ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(win.left() + 14.0, y), egui::vec2(win.width() * 0.6, 6.0)), 3.0, Color32::from_gray(200));
    }
}

#[derive(Clone, Copy)]
enum OverlayCase {
    Karaoke,
    Plain,
    Intro,
    Interlude,
    NoTrack,
    NoLyrics,
    Instrumental,
}

#[derive(Clone, Copy, PartialEq)]
enum OverlayArt {
    /// Setting off.
    Off,
    /// The synthetic cover.
    Cover,
    /// A nearly white cover (edge visibility on the light panel).
    Pale,
    /// Track without art: the placeholder tile.
    Placeholder,
}

struct OverlayShot {
    name: &'static str,
    case: OverlayCase,
    size: Vec2,
    opacity: f32,
    hover: bool,
    locked: bool,
    show_title: bool,
    art: OverlayArt,
    /// Opacity slider open in the hover bar.
    slider: bool,
}

fn pale_cover(size: usize) -> ColorImage {
    let mut px = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 / size as f32 - 0.5).powi(2) + (y as f32 / size as f32 - 0.55).powi(2)).sqrt();
            px.push(if d < 0.18 { Color32::from_rgb(200, 196, 190) } else { Color32::from_rgb(248, 247, 244) });
        }
    }
    ColorImage::new([size, size], px)
}

fn overlay_shot(dark: bool, shot: &OverlayShot) {
    use crate::presentation::ui::text::TextLang;
    use crate::presentation::ui::views::desktop_lyrics::{
        desktop_lyrics_view, DesktopLyricsContent, DesktopLyricsProps, DesktopLyricsViewState, OverlayLine,
    };
    let (case, opacity, hover, locked, show_title, art) = (shot.case, shot.opacity, shot.hover, shot.locked, shot.show_title, shot.art);
    let mut state = DesktopLyricsViewState::default();
    state.set_opacity_slider_open(shot.slider);
    let mut h = Harness::builder()
        .with_size(shot.size)
        .with_pixels_per_point(1.0)
        .with_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light })
        .build_ui(move |ui| {
            let ctx = ui.ctx().clone();
            let id = egui::Id::new("overlay_shot_fonts");
            if ctx.data(|d| d.get_temp::<bool>(id).is_none()) {
                crate::presentation::ui::fonts::install_definitions(&ctx, widget_fonts().clone());
                ctx.data_mut(|d| d.insert_temp(id, true));
            }
            let mode = if dark { theme::ThemeMode::Dark } else { theme::ThemeMode::Light };
            theme::sync(&ctx, mode, theme::Accent::Teal);
            paint_desktop(ui);
            let tex_id = egui::Id::new("overlay_shot_cover");
            let cover = ctx.data(|d| d.get_temp::<egui::TextureHandle>(tex_id)).unwrap_or_else(|| {
                let img = if art == OverlayArt::Pale { pale_cover(256) } else { cover_image(2, 256) };
                let t = ctx.load_texture("overlay_cover", img, egui::TextureOptions::LINEAR);
                ctx.data_mut(|d| d.insert_temp(tex_id, t.clone()));
                t
            });
            let karaoke = OverlayLine { text: "<00:22.00>遠く灯台が夜空を照らした", karaoke: true, highlighted_chars: 0.0 };
            let content = match case {
                OverlayCase::Karaoke => DesktopLyricsContent::Lyrics {
                    current: Some(OverlayLine { text: "遠く灯台が夜空を照らした", highlighted_chars: 5.4, ..karaoke }),
                    intro: false,
                    next: Some("窓に映るのは眠たげな海の色"),
                    line_key: 3,
                    lang: TextLang::Ja,
                },
                OverlayCase::Plain => DesktopLyricsContent::Lyrics {
                    current: Some(OverlayLine { text: "Counting the windows on the last train home", karaoke: false, highlighted_chars: 0.0 }),
                    intro: false,
                    next: Some("Harbor lights, one by one"),
                    line_key: 3,
                    lang: TextLang::Latin,
                },
                OverlayCase::Intro => DesktopLyricsContent::Lyrics { current: None, intro: true, next: Some("最終電車を見送ってから歩く"), line_key: 0, lang: TextLang::Ja },
                OverlayCase::Interlude => DesktopLyricsContent::Lyrics { current: None, intro: false, next: Some("改札を抜けた先に"), line_key: 7, lang: TextLang::Ja },
                OverlayCase::NoTrack => DesktopLyricsContent::NoTrack,
                OverlayCase::NoLyrics => DesktopLyricsContent::NoLyrics,
                OverlayCase::Instrumental => DesktopLyricsContent::Instrumental,
            };
            let props = DesktopLyricsProps {
                content,
                title: "夜の灯台",
                artist: "SORAMICHI",
                font_size: 32.0,
                opacity,
                show_next_line: true,
                show_title,
                locked,
                playing: true,
                has_track: !matches!(case, OverlayCase::NoTrack),
                force_controls: hover,
                artwork: matches!(art, OverlayArt::Cover | OverlayArt::Pale).then(|| crate::presentation::ui::widgets::Tex::from(&cover)),
                show_artwork: art != OverlayArt::Off,
                tr: crate::presentation::ui::Tr::JA,
            };
            let _ = desktop_lyrics_view(ui, &props, &mut state);
        });
    // The lock hint fades out after a few seconds of (harness) time.
    h.run_steps(if locked { 2 } else { 24 });
    let img = h.render().expect("render (needs a wgpu adapter)");
    let dir = out_dir().join("screens-overlay");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("overlay_{}_{}.png", shot.name, if dark { "dark" } else { "light" }));
    img.save(&path).expect("save png");
    eprintln!("wrote {}", path.display());
}

/// Desktop lyrics: the overlay content over a stand-in desktop (dark / light, karaoke
/// mid-line, hover controls, empty states, locked, transparent), the Settings subsection
/// and the transport button. PNGs in `$APP_SCREENS_DIR/screens-overlay/`.
#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn desktop_lyrics_screens() {
    let wide = super::desktop_lyrics::DEFAULT_SIZE;
    let shots = [
        OverlayShot { name: "karaoke", case: OverlayCase::Karaoke, size: wide, opacity: 0.72, hover: false, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "karaoke_hover", case: OverlayCase::Karaoke, size: wide, opacity: 0.72, hover: true, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "karaoke_title", case: OverlayCase::Karaoke, size: Vec2::new(640.0, 150.0), opacity: 0.72, hover: false, locked: false, show_title: true, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "karaoke_transparent", case: OverlayCase::Karaoke, size: wide, opacity: 0.0, hover: false, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "plain_long_narrow", case: OverlayCase::Plain, size: Vec2::new(380.0, 120.0), opacity: 0.72, hover: false, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "narrow_hover", case: OverlayCase::Karaoke, size: Vec2::new(300.0, 110.0), opacity: 0.72, hover: true, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "intro", case: OverlayCase::Intro, size: wide, opacity: 0.72, hover: false, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "interlude", case: OverlayCase::Interlude, size: wide, opacity: 0.72, hover: false, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "no_track", case: OverlayCase::NoTrack, size: wide, opacity: 0.72, hover: false, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "no_lyrics", case: OverlayCase::NoLyrics, size: wide, opacity: 0.72, hover: false, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "instrumental", case: OverlayCase::Instrumental, size: wide, opacity: 0.72, hover: false, locked: false, show_title: false, art: OverlayArt::Off, slider: false },
        OverlayShot { name: "locked", case: OverlayCase::Karaoke, size: wide, opacity: 0.72, hover: false, locked: true, show_title: false, art: OverlayArt::Off, slider: false },
    ];
    let art = |name, case, size, opacity, hover, art, slider| OverlayShot { name, case, size, opacity, hover, locked: false, show_title: false, art, slider };
    let art_shots = [
        art("art_karaoke", OverlayCase::Karaoke, wide, 0.72, false, OverlayArt::Cover, false),
        art("art_karaoke_hover", OverlayCase::Karaoke, wide, 0.72, true, OverlayArt::Cover, false),
        art("art_opacity_slider", OverlayCase::Karaoke, wide, 0.35, true, OverlayArt::Cover, true),
        art("art_transparent", OverlayCase::Karaoke, wide, 0.0, false, OverlayArt::Cover, false),
        art("art_pale_cover", OverlayCase::Karaoke, wide, 0.72, false, OverlayArt::Pale, false),
        art("art_placeholder", OverlayCase::Karaoke, wide, 0.72, false, OverlayArt::Placeholder, false),
        art("art_placeholder_transparent", OverlayCase::Karaoke, wide, 0.0, false, OverlayArt::Placeholder, false),
        art("art_no_lyrics", OverlayCase::NoLyrics, wide, 0.72, false, OverlayArt::Cover, false),
        art("art_instrumental", OverlayCase::Instrumental, wide, 0.72, false, OverlayArt::Cover, false),
        art("art_plain_430", OverlayCase::Plain, Vec2::new(430.0, 120.0), 0.72, false, OverlayArt::Cover, false),
        art("art_narrow_300_no_art", OverlayCase::Karaoke, Vec2::new(300.0, 110.0), 0.72, true, OverlayArt::Cover, false),
        art("art_min_size_slider", OverlayCase::Karaoke, super::desktop_lyrics::MIN_SIZE, 0.5, true, OverlayArt::Cover, true),
        art("art_430_hover", OverlayCase::Karaoke, Vec2::new(430.0, 144.0), 0.72, true, OverlayArt::Cover, false),
        art("art_tall", OverlayCase::Karaoke, Vec2::new(720.0, 260.0), 0.72, false, OverlayArt::Cover, false),
    ];
    for dark in [true, false] {
        for shot in shots.iter().chain(&art_shots) {
            overlay_shot(dark, shot);
        }
    }

    // The real app: Settings subsection and the transport button (active / locked).
    for dark in [true, false] {
        let fx = fixture();
        let suffix = if dark { "dark" } else { "light" };
        let mut h = start(&fx, dark, Language::Ja);
        // Register the overlay viewport without embedding it into the main screenshot.
        h.ctx.set_embed_viewports(false);
        let dir = "screens-overlay";
        std::fs::create_dir_all(out_dir().join(dir)).unwrap();
        with_app(&mut h, |a| {
            a.config.desktop_lyrics.enabled = true;
            a.navigate(Page::Library);
            a.ui.right_panel = Some(RightPanel::Lyrics);
        });
        shoot(&mut h, &format!("{dir}/transport_desktop_lyrics_on_{suffix}"));
        with_app(&mut h, |a| a.config.desktop_lyrics.locked = true);
        shoot(&mut h, &format!("{dir}/transport_desktop_lyrics_locked_{suffix}"));
        // 3× zoom of the transport's right-hand toggles.
        h.run_steps(2);
        let img = h.render().expect("render");
        let (w, ht) = (img.width(), img.height());
        let full = image::RgbaImage::from_raw(w, ht, img.into_raw()).unwrap();
        let crop = image::imageops::crop_imm(&full, w - 340, ht - 80, 340, 80).to_image();
        let zoom = image::imageops::resize(&crop, 1020, 240, image::imageops::FilterType::Nearest);
        zoom.save(out_dir().join(format!("{dir}/transport_desktop_lyrics_locked_zoom_{suffix}.png"))).unwrap();
        h.set_size(Vec2::new(760.0, 640.0));
        shoot(&mut h, &format!("{dir}/transport_desktop_lyrics_760_{suffix}"));
        h.set_size(Vec2::new(745.0, 640.0));
        shoot(&mut h, &format!("{dir}/transport_desktop_lyrics_745_{suffix}"));
        h.set_size(Vec2::new(1280.0, 3000.0));
        with_app(&mut h, |a| {
            a.ui.right_panel = None;
            a.navigate(Page::Settings);
        });
        shoot(&mut h, &format!("{dir}/settings_desktop_lyrics_{suffix}"));
    }
}

