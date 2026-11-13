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

        // Hover on the first row's artist link (Dua Lipa).
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
                tr.f("toast.enqueued_one", &[&"群青"]),
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
            with_app(&mut h, |a| a.open_artist("米津玄師".into()));
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
        ("周杰伦 - 晴天", "周杰伦", "叶惠美", 269_000),
        ("我們的愛", "飛兒樂團", "無限", 281_000),
        ("사랑", "아이유", "Palette", 214_000),
        ("优里", "周深", "深深", 230_000),
        ("说爱你", "蔡依林", "看我72变", 223_000),
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
