//! Screenshot gallery for the `ui` components (headless wgpu via egui_kittest).
//!
//! Needs a Vulkan/GL adapter (lavapipe works). Ignored by default; run with:
//!
//! ```text
//! UI_SCREENS_DIR=/some/dir cargo test -p lrc_player --lib ui::gallery_tests -- --ignored --test-threads=1
//! ```
//!
//! PNGs go to `$UI_SCREENS_DIR` (default: `target/ui-screens`). Japanese text uses a system
//! Japanese font when one is found (also searched in `/mnt/c/Windows/Fonts` under WSL).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use eframe::egui::{
    self, vec2, CentralPanel, Color32, ColorImage, Context, FontDefinitions, Panel, TextureHandle,
    TextureOptions, Ui, Vec2,
};
use egui_kittest::Harness;

use super::{
    actions::{EqSettings, Page, RepeatMode, RightPanel},
    fonts,
    i18n::Tr,
    icons::{icon_button, paint_icon, Icon},
    shell::{self, sidebar, transport_bar, PlaylistEntry, SidebarProps, SidebarState, TransportProps},
    theme::{self, Accent, ThemeMode},
    views::{
        album_grid, eq_panel, lyrics_view, make_backdrop, now_playing, queue_panel, settings_row,
        settings_section, track_table, AlbumGridProps, AlbumItem, Column, EqPanelProps, IndexMode,
        LyricLine, LyricsProps, LyricsViewState, NowPlayingProps, NowPlayingState, QueueItem,
        QueueProps, TrackRow, TrackTableProps, TrackTableState,
    },
    widgets::{
        self, button, empty_state, search_box, section_header, seek_bar, segmented, toggle_switch,
        volume_control, ButtonKind, Tex, Toasts,
    },
};

// ---------------------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------------------

fn out_dir() -> PathBuf {
    let dir = std::env::var_os("UI_SCREENS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-screens"));
    std::fs::create_dir_all(&dir).expect("create screenshot dir");
    dir
}

fn font_defs() -> &'static FontDefinitions {
    static DEFS: OnceLock<FontDefinitions> = OnceLock::new();
    DEFS.get_or_init(|| {
        let wsl = Path::new("/mnt/c/Windows/Fonts");
        let (defs, report) = fonts::build_definitions(&[wsl]);
        eprintln!("fonts: {report:?}");
        defs
    })
}

fn setup(ctx: &Context, dark: bool) {
    let first = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("gallery_setup")).is_none());
    if first {
        fonts::install_definitions(ctx, font_defs().clone());
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("gallery_setup"), true));
    }
    theme::sync(ctx, if dark { ThemeMode::Dark } else { ThemeMode::Light }, Accent::Teal);
}

fn shoot<S>(harness: &mut Harness<'_, S>, name: &str) {
    harness.run_steps(6);
    let img = harness.render().expect("render (needs wgpu adapter)");
    let path = out_dir().join(format!("{name}.png"));
    img.save(&path).expect("save png");
    eprintln!("wrote {}", path.display());
}

fn harness_for<'a, S>(size: Vec2, dark: bool, state: S, mut app: impl FnMut(&mut Ui, &mut S) + 'a) -> Harness<'a, S> {
    Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light })
        .build_ui_state(
            move |ui, s| {
                setup(ui.ctx(), dark);
                let pal = theme::palette(ui);
                ui.painter().rect_filled(ui.max_rect(), 0.0, pal.bg);
                app(ui, s)
            },
            state,
        )
}

// ---------------------------------------------------------------------------------------
// Mock data
// ---------------------------------------------------------------------------------------

struct Mock {
    rows: Vec<TrackRow>,
    albums: Vec<AlbumItem>,
    covers: Vec<Option<TextureHandle>>,
    backdrop: Option<TextureHandle>,
}

const SONGS: &[(&str, &str, &str, u64)] = &[
    ("夜に駆ける", "YOASOBI", "THE BOOK", 261_000),
    ("アイドル", "YOASOBI", "THE BOOK 3", 213_000),
    ("群青", "YOASOBI", "THE BOOK", 249_000),
    ("Lemon", "米津玄師", "STRAY SHEEP", 255_000),
    ("感電", "米津玄師", "STRAY SHEEP", 268_000),
    ("馬と鹿", "米津玄師", "STRAY SHEEP", 249_000),
    ("Pretender", "Official髭男dism", "Traveler", 326_000),
    ("Subtitle", "Official髭男dism", "Subtitle", 305_000),
    ("紅蓮華", "LiSA", "LEO-NiNE", 239_000),
    ("炎", "LiSA", "LEO-NiNE", 275_000),
    ("Blinding Lights", "The Weeknd", "After Hours", 200_000),
    ("Save Your Tears", "The Weeknd", "After Hours", 215_000),
    ("Levitating", "Dua Lipa", "Future Nostalgia", 203_000),
    ("Don't Start Now", "Dua Lipa", "Future Nostalgia", 183_000),
    ("ベテルギウス", "優里", "壱", 271_000),
    ("ドライフラワー", "優里", "壱", 285_000),
    ("怪獣の花唄", "Vaundy", "strobo", 223_000),
    ("踊り子", "Vaundy", "strobo", 208_000),
    ("Mixed Nuts", "Official髭男dism", "Mixed Nuts", 213_000),
    ("Bling-Bang-Bang-Born", "Creepy Nuts", "Bling-Bang-Bang-Born", 168_000),
    ("Idol (English Version)", "YOASOBI", "THE BOOK 3", 213_000),
    ("Shinunoga E-Wa", "Fujii Kaze", "HELP EVER HURT NEVER", 186_000),
    ("きらり", "藤井 風", "LOVE ALL SERVE ALL", 205_000),
    ("花", "藤井 風", "LOVE ALL SERVE ALL", 216_000),
];

fn mock_rows(n: usize) -> Vec<TrackRow> {
    (0..n)
        .map(|i| {
            let (t, a, al, d) = SONGS[i % SONGS.len()];
            let title = if i < SONGS.len() { t.to_owned() } else { format!("{t} ({})", i / SONGS.len() + 1) };
            TrackRow {
                path: PathBuf::from(format!("/music/{a}/{al}/{i:05}.flac")),
                title,
                artist: a.into(),
                album: al.into(),
                duration_ms: Some(d),
                track_no: Some((i % 12) as u32 + 1),
                disc_no: None,
                year: Some(2018 + (i % 7) as i32),
                genre: if i % 3 == 0 { "J-Pop".into() } else { "Pop".into() },
            }
        })
        .collect()
}

fn mock_albums() -> Vec<AlbumItem> {
    let mut v: Vec<AlbumItem> = Vec::new();
    for (_, a, al, _) in SONGS {
        if !v.iter().any(|x| x.title == *al) {
            v.push(AlbumItem {
                key: format!("{a}/{al}"),
                title: (*al).into(),
                artist: (*a).into(),
                year: Some(2019 + (v.len() % 5) as i32),
                track_count: 8 + v.len() % 6,
                album_artist: false,
            });
        }
    }
    v
}

/// Synthetic "cover art": a few blended color fields, deterministic per seed.
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
            let mut c = theme::blend(base, glow, ring * ring);
            if seed.is_multiple_of(3) && ((fx * 10.0) as i32 + (fy * 10.0) as i32) % 7 == 0 {
                c = theme::blend(c, Color32::WHITE, 0.08);
            }
            px.push(c);
        }
    }
    ColorImage::new([size, size], px)
}

impl Mock {
    fn new(ctx: &Context, n_rows: usize) -> Self {
        let albums = mock_albums();
        let covers = (0..albums.len())
            .map(|i| {
                // leave a couple without art to show the placeholder
                (i % 5 != 3).then(|| ctx.load_texture(format!("cover{i}"), cover_image(i, 160), TextureOptions::LINEAR))
            })
            .collect();
        let backdrop = Some(ctx.load_texture("backdrop", make_backdrop(&cover_image(0, 400)), TextureOptions::LINEAR));
        Self { rows: mock_rows(n_rows), albums, covers, backdrop }
    }

    fn cover(&self, album_title: &str) -> Option<Tex> {
        let i = self.albums.iter().position(|a| a.title == album_title)?;
        self.covers[i].as_ref().map(Tex::from)
    }
}

fn mock_lyrics() -> Vec<LyricLine<'static>> {
    let texts = [
        ("沈むように溶けてゆくように", false),
        ("二人だけの空が広がる夜に", false),
        ("「さよなら」だけだった", true),
        ("その一言で全てが分かった", true),
        ("日が沈み出した空と君の姿", false),
        ("フェンス越しに重なっていた", false),
        ("", false),
        ("初めて会った日から", false),
        ("僕の心の全てを奪った", false),
        ("どこか儚い空気を纏う君は", false),
        ("寂しい目をしてたんだ", false),
    ];
    texts
        .iter()
        .enumerate()
        .map(|(i, (t, k))| LyricLine { text: t, start_ms: 10_000 + i as u64 * 4_000, karaoke: *k })
        .collect()
}

struct ShellState {
    mock: Option<Mock>,
    sidebar: SidebarState,
    table: TrackTableState,
    query: String,
    now: NowPlayingState,
    lyrics: LyricsViewState,
    toasts: Toasts,
}

impl ShellState {
    fn new(collapsed: bool) -> Self {
        let mut toasts = Toasts::new();
        toasts.success("「夜に駆ける」をキューに追加しました");
        Self {
            mock: None,
            sidebar: SidebarState { collapsed },
            table: TrackTableState::default(),
            query: String::new(),
            now: NowPlayingState::default(),
            lyrics: LyricsViewState::default(),
            toasts,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MockPage {
    Library,
    Albums,
    NowPlaying,
    Settings,
}

fn mock_shell(ui: &mut Ui, s: &mut ShellState, page: MockPage, right: Option<RightPanel>, tr: Tr, rows: usize) {
    let ctx = ui.ctx().clone();
    if s.mock.is_none() {
        s.mock = Some(Mock::new(&ctx, rows));
    }
    let mock = s.mock.as_ref().unwrap();
    let pal = theme::palette(ui);
    let now_row = &mock.rows[0];
    let playlists = [
        PlaylistEntry { id: "p1", name: "お気に入り", track_count: 42 },
        PlaylistEntry { id: "p2", name: "作業用 BGM", track_count: 120 },
        PlaylistEntry { id: "p3", name: "Night Drive", track_count: 18 },
    ];

    Panel::bottom("transport")
        .exact_size(shell::transport::HEIGHT)
        .frame(shell::chrome_frame(&pal))
        .show_separator_line(false)
        .show(ui, |ui| {
            transport_bar(
                ui,
                &TransportProps {
                    has_track: true,
                    title: &now_row.title,
                    artist: &now_row.artist,
                    artwork: mock.cover(&now_row.album),
                    artwork_seed: &now_row.album,
                    playing: true,
                    position_ms: 18_400,
                    duration_ms: now_row.duration_ms.unwrap_or(0),
                    shuffle: true,
                    repeat: RepeatMode::All,
                    volume: 0.35,
                    muted: false,
                    right_panel: right,
                    eq_open: false,
                    eq_enabled: true,
                    queue_len: 12,
                    no_output: false,
                    favorite: Some(false),
                    tr,
                },
            );
        });
    Panel::left("sidebar")
        .exact_size(s.sidebar.width())
        .frame(shell::chrome_frame(&pal))
        .show_separator_line(false)
        .resizable(false)
        .show(ui, |ui| {
            let active = match page {
                MockPage::Library => Page::Library,
                MockPage::Albums => Page::Albums,
                MockPage::NowPlaying => Page::NowPlaying,
                MockPage::Settings => Page::Settings,
            };
            sidebar(
                ui,
                &SidebarProps { active, active_playlist: None, playlists: &playlists, scanning: Some(0.62), tr },
                &mut s.sidebar,
            );
        });
    if let Some(rp) = right {
        Panel::right("right")
            .exact_size(theme::metrics::RIGHT_PANEL_WIDTH)
            .frame(shell::right_panel_frame(&pal))
            .show_separator_line(false)
            .resizable(false)
            .show(ui, |ui| match rp {
                RightPanel::Queue => {
                    let items: Vec<QueueItem> = mock.rows[..14]
                        .iter()
                        .map(|r| QueueItem { title: &r.title, artist: &r.artist, duration_ms: r.duration_ms, artwork: mock.cover(&r.album), seed: &r.album })
                        .collect();
                    queue_panel(ui, &QueueProps { items: &items, current: Some(2), playing: true, tr });
                }
                RightPanel::Lyrics => {
                    let lines = mock_lyrics();
                    lyrics_view(
                        ui,
                        &LyricsProps {
                            lines: &lines,
                            revision: 1,
                            current: Some(3),
                            highlighted_chars: 6.4,
                            offset_ms: 300,
                            font_size: 22.0,
                            align: egui::Align::Min,
                            on_backdrop: false,
                            show_controls: true,
                            can_save_offset: true,
                            tr,
                        },
                        &mut s.lyrics,
                    );
                }
            });
    }
    let central = if page == MockPage::NowPlaying {
        egui::Frame::new().fill(pal.bg).inner_margin(egui::Margin { left: 0, right: 8, top: 8, bottom: 8 })
    } else {
        shell::central_frame(&pal)
    };
    CentralPanel::default().frame(central).show(ui, |ui| match page {
        MockPage::NowPlaying => {
            let lines = mock_lyrics();
            let rect = ui.available_rect_before_wrap();
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            child.set_clip_rect(rect);
            // rounded corners for the backdrop card
            now_playing(
                &mut child,
                &NowPlayingProps {
                    has_track: true,
                    title: &now_row.title,
                    artist: &now_row.artist,
                    album: &now_row.album,
                    artwork: mock.cover(&now_row.album),
                    backdrop: mock.backdrop.as_ref().map(Tex::from),
                    seed: &now_row.album,
                    lyrics: Some(LyricsProps {
                        lines: &lines,
                        revision: 1,
                        current: Some(3),
                        highlighted_chars: 6.4,
                        offset_ms: 0,
                        font_size: 30.0,
                        align: egui::Align::Min,
                        on_backdrop: true,
                        show_controls: true,
                        can_save_offset: true,
                        tr,
                    }),
                    favorite: Some(true),
                    tr,
                },
                &mut s.now,
            );
        }
        _ => {
            shell::page_frame(&pal).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                match page {
                    MockPage::Library => {
                        let count = format!("{} {}", super::format::format_count(mock.rows.len()), tr.t("table.tracks_count"));
                        section_header(ui, tr.t("nav.library"), Some(&count), |ui| {
                            search_box(ui, egui::Id::new("lib_search"), &mut s.query, 300.0, false, &tr);
                        });
                        ui.add_space(12.0);
                        track_table(
                            ui,
                            &TrackTableProps {
                                id_salt: "library",
                                rows: &mock.rows,
                                rows_revision: 1,
                                query: &s.query,
                                columns: &Column::DEFAULT,
                                now_playing: Some(&mock.rows[0].path),
                                playing: true,
                                index_mode: IndexMode::Position,
                                reorderable: false,
                                playlists: &playlists,
                                removable: false,
                                optional_columns: &[Column::Year, Column::Genre],
                                links: crate::presentation::ui::views::TableLinks::ALL,
                                favorites: None,
                                tr,
                            },
                            &mut s.table,
                        );
                    }
                    MockPage::Albums => {
                        let sub = format!("{} {}", mock.albums.len(), tr.t("nav.albums"));
                        section_header(ui, tr.t("nav.albums"), Some(&sub), |_| {});
                        ui.add_space(16.0);
                        let covers = &mock.covers;
                        let albums = &mock.albums;
                        album_grid(
                            ui,
                            &AlbumGridProps { id_salt: "albums", albums, now_playing: Some(albums[0].key.as_str()), card_size: crate::presentation::ui::views::album_grid::ALBUM_CARD_DEFAULT, tr },
                            &mut |key| {
                                let i = albums.iter().position(|a| a.key == key)?;
                                covers[i].as_ref().map(Tex::from)
                            },
                        );
                    }
                    MockPage::Settings => settings_page(ui, tr),
                    MockPage::NowPlaying => {}
                }
            });
        }
    });
    s.toasts.show(&ctx, shell::transport::HEIGHT + 8.0, &tr);
}

fn settings_page(ui: &mut Ui, tr: Tr) {
    section_header(ui, tr.t("nav.settings"), None, |_| {});
    ui.add_space(16.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(720.0);
        settings_section(ui, tr.t("settings.appearance"), None, |ui| {
            settings_row(ui, true, tr.t("settings.theme"), None, |ui| {
                segmented(
                    ui,
                    egui::Id::new("theme_seg"),
                    &[tr.t("settings.theme_light"), tr.t("settings.theme_dark"), tr.t("settings.theme_system")],
                    2,
                );
            });
            settings_row(ui, false, tr.t("settings.accent"), None, |ui| {
                let pal = theme::palette(ui);
                for a in Accent::ALL.iter().rev() {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
                    ui.painter().circle_filled(r.center(), 10.0, a.color(pal.dark));
                    if *a == Accent::Teal {
                        ui.painter().circle_stroke(r.center(), 12.5, egui::Stroke::new(2.0, pal.text));
                    }
                }
            });
            settings_row(ui, false, "カラオケ表示", Some("文字単位のタイミングがある歌詞を左から塗りつぶします"), |ui| {
                let mut on = true;
                toggle_switch(ui, &mut on);
            });
        });
        settings_section(ui, "再生", Some("出力デバイスと音質"), |ui| {
            settings_row(ui, true, "出力デバイス", None, |ui| {
                egui::ComboBox::from_id_salt("dev").selected_text("既定のデバイス").width(220.0).show_ui(ui, |_| {});
            });
            settings_row(ui, false, "リサンプラー品質", Some("高いほど CPU 負荷が増えます"), |ui| {
                segmented(ui, egui::Id::new("rs"), &["低", "標準", "高"], 1);
            });
        });
    });
}

// ---------------------------------------------------------------------------------------
// Tests (screenshots)
// ---------------------------------------------------------------------------------------

const SHELL: Vec2 = Vec2::new(1280.0, 800.0);

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn shell_library_dark() {
    let mut h = harness_for(SHELL, true, ShellState::new(false), |ui, s| {
        mock_shell(ui, s, MockPage::Library, Some(RightPanel::Queue), Tr::JA, 2_000)
    });
    h.run_steps(3);
    // hover a row to show the hover state
    h.hover_at(egui::pos2(520.0, 205.0));
    shoot(&mut h, "shell_library_dark");
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn shell_library_light() {
    let mut h = harness_for(SHELL, false, ShellState::new(false), |ui, s| {
        mock_shell(ui, s, MockPage::Library, Some(RightPanel::Queue), Tr::JA, 2_000)
    });
    shoot(&mut h, "shell_library_light");
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn shell_library_english_20k() {
    let mut h = harness_for(SHELL, true, ShellState::new(true), |ui, s| {
        mock_shell(ui, s, MockPage::Library, Some(RightPanel::Lyrics), Tr::EN, 20_000)
    });
    shoot(&mut h, "shell_library_en_collapsed_lyrics");
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn shell_albums() {
    for dark in [true, false] {
        let mut h = harness_for(SHELL, dark, ShellState::new(false), |ui, s| {
            mock_shell(ui, s, MockPage::Albums, None, Tr::JA, 200)
        });
        h.run_steps(3);
        h.hover_at(egui::pos2(420.0, 200.0));
        shoot(&mut h, if dark { "shell_albums_dark" } else { "shell_albums_light" });
    }
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn shell_now_playing() {
    for (dark, size, name) in [
        (true, SHELL, "now_playing_dark"),
        (false, SHELL, "now_playing_light_theme"),
        (true, vec2(760.0, 720.0), "now_playing_narrow"),
    ] {
        let mut h = harness_for(size, dark, ShellState::new(size.x < 900.0), |ui, s| {
            mock_shell(ui, s, MockPage::NowPlaying, None, Tr::JA, 50)
        });
        shoot(&mut h, name);
    }
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn shell_settings() {
    for dark in [true, false] {
        let mut h = harness_for(SHELL, dark, ShellState::new(false), |ui, s| {
            mock_shell(ui, s, MockPage::Settings, None, Tr::JA, 50)
        });
        shoot(&mut h, if dark { "shell_settings_dark" } else { "shell_settings_light" });
    }
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn transport_widths() {
    for w in [560.0, 760.0, 1100.0] {
        for dark in [true, false] {
            let mut h = harness_for(vec2(w, 76.0), dark, (), move |ui, _| {
                transport_bar(
                    ui,
                    &TransportProps {
                        has_track: true,
                        title: "とても長いタイトルの曲名がここに入ります（Remastered 2024 Edition）",
                        artist: "アーティスト名 feat. ゲストボーカル",
                        artwork: None,
                        artwork_seed: "seed",
                        playing: false,
                        position_ms: 65_000,
                        duration_ms: 3_725_000,
                        shuffle: false,
                        repeat: RepeatMode::One,
                        volume: 0.0,
                        muted: true,
                        right_panel: Some(RightPanel::Lyrics),
                        eq_open: true,
                        eq_enabled: false,
                        queue_len: 12,
                        no_output: false,
                        favorite: Some(false),
                        tr: Tr::JA,
                    },
                );
            });
            shoot(&mut h, &format!("transport_{}_{}", w as u32, if dark { "dark" } else { "light" }));
        }
    }
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn components() {
    for dark in [true, false] {
        let mut h = harness_for(vec2(960.0, 900.0), dark, (String::from("夜に"), Toasts::new(), false), |ui, s| {
            let tr = Tr::JA;
            if s.2 {
                // keep toasts alive
            } else {
                s.1.info("ライブラリのスキャンが完了しました（1,204 曲）");
                s.1.warning("歌詞ファイルの文字コードを推定しました (Shift_JIS)");
                s.1.error("出力デバイスを開けませんでした: デバイスが見つかりません");
                s.2 = true;
            }
            components_page(ui, &mut s.0, tr);
            s.1.show(ui.ctx(), 8.0, &tr);
        });
        h.run_steps(2);
        h.hover_at(egui::pos2(260.0, 230.0));
        shoot(&mut h, if dark { "components_dark" } else { "components_light" });
    }
}

fn components_page(ui: &mut Ui, query: &mut String, tr: Tr) {
    let pal = theme::palette(ui);
    egui::Frame::new().inner_margin(24).show(ui, |ui| {
        ui.label(theme::heading(ui, "Icons").color(pal.text));
        ui.add_space(8.0);
        let all = ALL_ICONS;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (i, icon) in all.iter().enumerate() {
                icon_button(ui, *icon, 20.0, i % 9 == 4, &format!("{icon:?}"));
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for s in [12.0, 16.0, 24.0, 32.0, 48.0] {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(s), egui::Sense::hover());
                paint_icon(ui.painter(), r, Icon::Shuffle, pal.text);
                let (r, _) = ui.allocate_exact_size(Vec2::splat(s), egui::Sense::hover());
                paint_icon(ui.painter(), r, Icon::Settings, pal.text);
            }
            ui.add_space(12.0);
            super::icons::play_button(ui, false, 40.0, "Play");
            super::icons::play_button(ui, true, 40.0, "Pause");
        });
        ui.add_space(16.0);

        ui.label(theme::heading(ui, "Buttons & inputs").color(pal.text));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            button(ui, ButtonKind::Primary, Some(Icon::Play), "すべて再生");
            button(ui, ButtonKind::Secondary, Some(Icon::Shuffle), "シャッフル");
            button(ui, ButtonKind::Ghost, Some(Icon::Plus), "フォルダーを追加");
            button(ui, ButtonKind::Danger, Some(Icon::Trash), "ライブラリを消去");
            let mut on = true;
            toggle_switch(ui, &mut on);
            let mut off = false;
            toggle_switch(ui, &mut off);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            search_box(ui, egui::Id::new("s1"), query, 280.0, false, &tr);
            let mut empty = String::new();
            search_box(ui, egui::Id::new("s2"), &mut empty, 240.0, false, &tr);
            segmented(ui, egui::Id::new("seg"), &["曲", "アルバム", "アーティスト"], 1);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(theme::caption(ui, "1:02"));
            seek_bar(ui, egui::Id::new("seek_demo"), 62_000, 240_000, 320.0, false);
            ui.label(theme::caption(ui, "4:00"));
            ui.add_space(24.0);
            volume_control(ui, egui::Id::new("vol_demo"), widgets::ui_to_gain(0.7), false, 110.0, &tr);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (i, s) in [96.0, 64.0, 40.0].iter().enumerate() {
                widgets::artwork(ui, None, *s, if *s > 60.0 { 10 } else { 6 }, &format!("seed{i}"));
            }
            widgets::badge(ui, "128", pal.accent_soft, pal.accent_fg);
        });
        ui.add_space(16.0);
        ui.columns(2, |cols| {
            widgets::card(&mut cols[0], |ui| {
                ui.set_height(170.0);
                empty_state(ui, Icon::Queue, tr.t("queue.empty"), tr.t("queue.empty_hint"), Some((Icon::Library, tr.t("nav.library"))));
            });
            widgets::card(&mut cols[1], |ui| {
                let mut eq = EqSettings { enabled: true, preamp_db: -3.0, bands_db: [4.5, 3.5, 2.0, -0.5, -1.5, -0.5, 1.5, 3.0, 4.0, 4.5] };
                for a in eq_panel(ui, &EqPanelProps { settings: eq, tr }) {
                    if let super::UiAction::SetEq(s) = a {
                        eq = s;
                    }
                }
                let _ = eq;
            });
        });
    });
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn eq_popover() {
    for dark in [true, false] {
        let mut h = harness_for(vec2(560.0, 360.0), dark, (), |ui, _| {
            let pal = theme::palette(ui);
            egui::Frame::new().inner_margin(16).show(ui, |ui| {
                theme_card(ui, &pal, |ui| {
                    eq_panel(
                        ui,
                        &EqPanelProps {
                            settings: EqSettings { enabled: true, preamp_db: -2.0, bands_db: [-1.0, 1.0, 2.5, 3.5, 2.5, 0.0, -1.0, -1.0, 0.5, 1.0] },
                            tr: Tr::JA,
                        },
                    );
                });
            });
        });
        shoot(&mut h, if dark { "eq_dark" } else { "eq_light" });
    }
}

fn theme_card(ui: &mut Ui, pal: &theme::Palette, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(pal.elevated)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(theme::radius::LG)
        .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 28, spread: 0, color: pal.shadow })
        .inner_margin(16)
        .show(ui, |ui| add(ui));
}

#[test]
#[ignore = "renders PNGs with wgpu; run manually"]
fn icon_sheet_zoomed() {
    for dark in [true, false] {
        let mut h = Harness::builder()
            .with_size(vec2(520.0, 150.0))
            .with_pixels_per_point(2.5)
            .build_ui(move |ui| {
                setup(ui.ctx(), dark);
                let pal = theme::palette(ui);
                ui.painter().rect_filled(ui.max_rect(), 0.0, pal.surface);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for icon in ALL_ICONS {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::hover());
                        paint_icon(ui.painter(), r, *icon, pal.text);
                    }
                });
            });
        shoot(&mut h, if dark { "icons_zoom_dark" } else { "icons_zoom_light" });
    }
}

const ALL_ICONS: &[Icon] = &[
    Icon::Play, Icon::Pause, Icon::Previous, Icon::Next, Icon::Shuffle, Icon::Repeat, Icon::RepeatOne,
    Icon::Volume0, Icon::Volume1, Icon::Volume2, Icon::Volume3, Icon::Mute, Icon::Search, Icon::Lyrics,
    Icon::Queue, Icon::Eq, Icon::Settings, Icon::Library, Icon::Album, Icon::Artist, Icon::Playlist,
    Icon::Folder, Icon::Plus, Icon::Minus, Icon::Close, Icon::More, Icon::MoreVertical, Icon::Heart,
    Icon::HeartFilled, Icon::Note, Icon::NowPlaying, Icon::ChevronLeft, Icon::ChevronRight,
    Icon::ChevronDown, Icon::ChevronUp, Icon::Check, Icon::Trash, Icon::Refresh, Icon::Grip,
    Icon::SidebarToggle, Icon::Save, Icon::Info, Icon::Warning, Icon::Error, Icon::Success,
    Icon::TextSmaller, Icon::TextLarger, Icon::Undo, Icon::ArrowDownToLine, Icon::Speaker, Icon::Edit,
];
