//! Collapsible navigation sidebar (icons + labels, active highlight, playlists section).

use eframe::egui::{
    self, pos2, vec2, Align, Color32, CursorIcon, FontId, Layout, Rect, Response, Sense, Stroke,
    Ui, UiBuilder, Vec2,
};

use crate::presentation::ui::{
    actions::{Page, UiAction},
    i18n::Tr,
    text,
    icons::{icon_button, paint_icon, Icon},
    theme::{self, metrics, Palette},
    views::track_table::TrackDragPayload,
};

/// One playlist entry shown under the navigation.
#[derive(Clone, Copy, Debug)]
pub struct PlaylistEntry<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub track_count: usize,
}

pub struct SidebarProps<'a> {
    pub active: Page,
    /// Highlight a playlist instead of a page.
    pub active_playlist: Option<&'a str>,
    pub playlists: &'a [PlaylistEntry<'a>],
    /// `Some(progress 0..1)` (or `Some(NAN)` for indeterminate) while a scan runs.
    pub scanning: Option<f32>,
    pub tr: Tr,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidebarState {
    pub collapsed: bool,
}

impl SidebarState {
    /// Width the caller should give the sidebar panel.
    pub fn width(&self) -> f32 {
        if self.collapsed {
            metrics::SIDEBAR_COLLAPSED
        } else {
            metrics::SIDEBAR_EXPANDED
        }
    }
}

fn page_icon(page: Page) -> Icon {
    match page {
        Page::NowPlaying => Icon::NowPlaying,
        Page::Library => Icon::Library,
        Page::Albums => Icon::Album,
        Page::Artists => Icon::Artist,
        Page::Playlists => Icon::Playlist,
        Page::Folders => Icon::Folder,
        Page::Queue => Icon::Queue,
        Page::Settings => Icon::Settings,
    }
}

pub fn page_label_key(page: Page) -> &'static str {
    match page {
        Page::NowPlaying => "nav.now_playing",
        Page::Library => "nav.library",
        Page::Albums => "nav.albums",
        Page::Artists => "nav.artists",
        Page::Playlists => "nav.playlists",
        Page::Folders => "nav.folders",
        Page::Queue => "nav.queue",
        Page::Settings => "nav.settings",
    }
}

// ---------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------

/// Height of a navigation / playlist row and the gap between rows.
pub const NAV_ROW_H: f32 = 36.0;
pub const NAV_ROW_GAP: f32 = 2.0;
const TOP_PAD: f32 = 14.0;
const BRAND_H: f32 = 32.0;
const BRAND_GAP: f32 = 18.0;
/// Space between the main nav and the playlist section.
const SECTION_GAP: f32 = 14.0;
/// "プレイリスト" heading (expanded) incl. its spacing; the collapsed divider is shorter.
const PLAYLIST_HEADER_H: f32 = 18.0;
const PLAYLIST_DIVIDER_H: f32 = 9.0;
/// Gap above the bottom block and margin below it.
const BOTTOM_GAP: f32 = 8.0;
const BOTTOM_PAD: f32 = 10.0;
/// Sidebar toggle button (18 px icon + padding).
const TOGGLE_H: f32 = 34.0;
/// Scan indicator row (only while scanning).
const SCAN_ROW_H: f32 = 30.0;
/// Playlist rows guaranteed to be visible at the minimum window height.
pub const MIN_PLAYLIST_ROWS: usize = 2;

const fn rows_h(n: usize) -> f32 {
    if n == 0 {
        0.0
    } else {
        n as f32 * NAV_ROW_H + (n - 1) as f32 * NAV_ROW_GAP
    }
}

/// Height of the fixed bottom block (settings + toggle, plus the scan row while scanning).
pub const fn bottom_block_h(scanning: bool) -> f32 {
    NAV_ROW_H + NAV_ROW_GAP + TOGGLE_H + if scanning { SCAN_ROW_H } else { 0.0 }
}

/// Smallest sidebar height at which the brand, the whole main nav, the playlist heading
/// with [`MIN_PLAYLIST_ROWS`] rows and the bottom block fit without overlapping:
/// `14 + 32 + 18 + nav(7 × 36 + 6 × 2 = 264) + 14 + 18 + rows(2 × 36 + 2 = 74) + 8 + bottom(36 + 2 + 34 = 72) + 10 = 524`.
pub const SIDEBAR_MIN_HEIGHT: f32 = TOP_PAD
    + BRAND_H
    + BRAND_GAP
    + rows_h(Page::MAIN.len())
    + SECTION_GAP
    + PLAYLIST_HEADER_H
    + rows_h(MIN_PLAYLIST_ROWS)
    + BOTTOM_GAP
    + bottom_block_h(false)
    + BOTTOM_PAD;

/// Regions of the sidebar. The bottom block is placed first, the playlist section gets
/// whatever height is left (down to 0), so nothing can overlap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SidebarLayout {
    pub brand: Rect,
    pub nav: Rect,
    /// Playlist heading / divider, when at least one row fits under it.
    pub playlist_header: Option<Rect>,
    /// Scrollable playlist rows (zero height when nothing fits).
    pub playlists: Rect,
    pub bottom: Rect,
}

pub fn sidebar_layout(full: Rect, collapsed: bool, scanning: bool, has_playlists: bool) -> SidebarLayout {
    let pad = if collapsed { 10.0 } else { 12.0 };
    let (left, right) = (full.left() + pad, full.right() - pad);
    let brand = Rect::from_min_max(pos2(left, full.top() + TOP_PAD), pos2(right, full.top() + TOP_PAD + BRAND_H));
    let bottom_top = (full.bottom() - BOTTOM_PAD - bottom_block_h(scanning)).max(brand.bottom());
    let bottom = Rect::from_min_max(pos2(left, bottom_top), pos2(right, full.bottom() - BOTTOM_PAD).max(pos2(right, bottom_top)));
    let nav_top = brand.bottom() + BRAND_GAP;
    // The nav never extends into the bottom block (it is clipped if the window is smaller
    // than the minimum size).
    let nav_bottom = (nav_top + rows_h(Page::MAIN.len())).min(bottom.top() - BOTTOM_GAP).max(nav_top);
    let nav = Rect::from_min_max(pos2(left, nav_top), pos2(right, nav_bottom));
    let section_bottom = (bottom.top() - BOTTOM_GAP).max(nav.bottom());
    let section_top = (nav.bottom() + SECTION_GAP).min(section_bottom);
    let header_h = if collapsed { PLAYLIST_DIVIDER_H } else { PLAYLIST_HEADER_H };
    let fits_one_row = section_bottom - section_top >= header_h + NAV_ROW_H;
    let (playlist_header, list_top) = if has_playlists && fits_one_row {
        (Some(Rect::from_min_max(pos2(left, section_top), pos2(right, section_top + header_h))), section_top + header_h)
    } else {
        (None, section_bottom)
    };
    let playlists = Rect::from_min_max(pos2(left, list_top), pos2(right, section_bottom));
    SidebarLayout { brand, nav, playlist_header, playlists, bottom }
}

fn nav_id(page: Page) -> egui::Id {
    egui::Id::new(("sidebar_nav", page_label_key(page)))
}

fn playlist_id(id: &str) -> egui::Id {
    egui::Id::new(("sidebar_playlist", id))
}

/// Where the toggle button was drawn last (for layout tests).
fn toggle_rect_id() -> egui::Id {
    egui::Id::new("sidebar_toggle_rect")
}

/// Draw the sidebar filling `ui`'s max rect. Size the containing panel with
/// [`SidebarState::width`].
pub fn sidebar(ui: &mut Ui, props: &SidebarProps<'_>, state: &mut SidebarState) -> Option<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let full = ui.max_rect();
    let collapsed = state.collapsed;
    let mut action = None;
    let layout = sidebar_layout(full, collapsed, props.scanning.is_some(), !props.playlists.is_empty());

    // ---- brand ----
    let brand = layout.brand;
    {
        let logo = Rect::from_min_size(brand.min + vec2(if collapsed { (brand.width() - 30.0) / 2.0 } else { 6.0 }, 1.0), Vec2::splat(30.0));
        ui.painter().rect_filled(logo, 8.0, pal.accent);
        paint_icon(ui.painter(), logo.shrink(7.0), Icon::Note, pal.accent_text);
        if !collapsed {
            let g = ui.painter().layout_no_wrap(
                tr.t("app.name").to_owned(),
                theme::bold_font(ui.ctx(), 15.0),
                pal.text,
            );
            ui.painter().galley(pos2(logo.right() + 10.0, logo.center().y - g.size().y / 2.0), g, pal.text);
        }
    }

    // ---- main nav (fixed) ----
    let mut nav_ui = ui.new_child(UiBuilder::new().max_rect(layout.nav).layout(Layout::top_down(Align::Min)));
    nav_ui.set_clip_rect(layout.nav.expand2(vec2(4.0, 0.0)).intersect(ui.clip_rect()));
    nav_ui.spacing_mut().item_spacing.y = NAV_ROW_GAP;
    for page in Page::MAIN {
        let active = props.active == page && props.active_playlist.is_none();
        if nav_item(&mut nav_ui, nav_id(page), &pal, page_icon(page), tr.t(page_label_key(page)), active, collapsed, false).clicked() {
            action = Some(UiAction::Navigate(page));
        }
    }

    // ---- playlists (scrolls, shrinks to nothing) ----
    if let Some(header) = layout.playlist_header {
        if collapsed {
            ui.painter().hline(header.x_range().shrink(10.0), header.center().y, Stroke::new(1.0, pal.border));
        } else {
            let g = ui.painter().layout_no_wrap(tr.t("nav.playlists_section").to_owned(), FontId::proportional(11.0), pal.text_faint);
            ui.painter().galley(pos2(header.left() + 12.0, header.top() + (header.height() - 2.0 - g.size().y) / 2.0), g, pal.text_faint);
        }
    }
    if layout.playlists.height() >= 1.0 && !props.playlists.is_empty() {
        let mut list_ui = ui.new_child(UiBuilder::new().max_rect(layout.playlists).layout(Layout::top_down(Align::Min)));
        list_ui.set_clip_rect(layout.playlists.expand2(vec2(4.0, 0.0)).intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .id_salt("sidebar_playlists")
            .auto_shrink([false, false])
            .max_height(layout.playlists.height())
            .min_scrolled_height(0.0)
            .show(&mut list_ui, |ui| {
                ui.spacing_mut().item_spacing.y = NAV_ROW_GAP;
                for pl in props.playlists {
                    let active = props.active_playlist == Some(pl.id);
                    let resp = nav_item(ui, playlist_id(pl.id), &pal, Icon::Playlist, pl.name, active, collapsed, true);
                    if resp.clicked() {
                        action = Some(UiAction::OpenPlaylist(pl.id.to_owned()));
                    }
                    // Drop tracks from a table onto a playlist.
                    if resp.dnd_hover_payload::<TrackDragPayload>().is_some() {
                        ui.painter().rect_stroke(resp.rect, 8.0, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
                    }
                    if let Some(payload) = resp.dnd_release_payload::<TrackDragPayload>() {
                        action = Some(UiAction::AddToPlaylist {
                            playlist_id: pl.id.to_owned(),
                            paths: payload.paths.clone(),
                        });
                    }
                }
            });
    }

    // ---- bottom (fixed) ----
    let mut bottom_ui = ui.new_child(UiBuilder::new().max_rect(layout.bottom).layout(Layout::top_down(Align::Min)));
    bottom_ui.spacing_mut().item_spacing.y = NAV_ROW_GAP;
    if let Some(progress) = props.scanning {
        bottom_ui.horizontal(|ui| {
            ui.set_height(SCAN_ROW_H - 4.0);
            ui.add_space(if collapsed { 12.0 } else { 14.0 });
            ui.add(egui::Spinner::new().size(14.0).color(pal.accent_fg));
            if !collapsed {
                let text = if progress.is_finite() {
                    format!("{} {:.0}%", tr.t("nav.scanning"), progress * 100.0)
                } else {
                    tr.t("nav.scanning").to_owned()
                };
                ui.label(egui::RichText::new(text).font(FontId::proportional(12.0)).color(pal.text_muted));
            }
        });
        bottom_ui.add_space(4.0 - NAV_ROW_GAP);
    }
    let settings_active = props.active == Page::Settings && props.active_playlist.is_none();
    if nav_item(&mut bottom_ui, nav_id(Page::Settings), &pal, Icon::Settings, tr.t("nav.settings"), settings_active, collapsed, false).clicked() {
        action = Some(UiAction::Navigate(Page::Settings));
    }
    let tip = if collapsed { tr.t("nav.expand") } else { tr.t("nav.collapse") };
    bottom_ui.allocate_ui_with_layout(vec2(bottom_ui.available_width(), TOGGLE_H), Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(if collapsed { (ui.available_width() - TOGGLE_H) / 2.0 } else { 2.0 });
        let toggle = icon_button(ui, Icon::SidebarToggle, 18.0, false, tip);
        ui.ctx().data_mut(|d| d.insert_temp(toggle_rect_id(), toggle.rect));
        if toggle.clicked() {
            state.collapsed = !state.collapsed;
            action = Some(UiAction::ToggleSidebar);
        }
    });

    action
}

/// A 36px navigation row.
/// `user_content`: the label is a playlist name (drawn in the font of its language).
#[allow(clippy::too_many_arguments)]
fn nav_item(ui: &mut Ui, id: egui::Id, pal: &Palette, icon: Icon, label: &str, active: bool, collapsed: bool, user_content: bool) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), NAV_ROW_H), Sense::hover());
    // Stable ids (layout tests read them back).
    let resp = ui.interact(rect, id, Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let resp = if collapsed { resp.on_hover_text(label) } else { resp };
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.1);
        let painter = ui.painter();
        if active {
            painter.rect_filled(rect, 8.0, pal.accent_soft);
        } else if t > 0.0 {
            painter.rect_filled(rect, 8.0, theme::with_alpha(pal.hover, t));
        }
        let color = if active {
            pal.accent_fg
        } else {
            theme::blend(pal.text_muted, pal.text, t)
        };
        let icon_x = if collapsed { rect.center().x } else { rect.left() + 12.0 + 9.0 };
        paint_icon(painter, Rect::from_center_size(pos2(icon_x, rect.center().y), Vec2::splat(18.0)), icon, color);
        if !collapsed {
            let font = if user_content {
                text::font_id(label, theme::size::BODY, active)
            } else if active {
                theme::bold_font(ui.ctx(), theme::size::BODY)
            } else {
                FontId::proportional(theme::size::BODY)
            };
            let max_w = rect.width() - 52.0;
            let job = egui::text::LayoutJob {
                wrap: egui::text::TextWrapping::truncate_at_width(max_w),
                ..egui::text::LayoutJob::simple_singleline(label.to_owned(), font, if active { pal.text } else { color })
            };
            let g = ui.painter().layout_job(job);
            painter.galley(pos2(rect.left() + 42.0, rect.center().y - g.size().y / 2.0), g, Color32::PLACEHOLDER);
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_height_matches_documented_value() {
        assert_eq!(SIDEBAR_MIN_HEIGHT, 524.0);
        assert_eq!(crate::presentation::ui::shell::MIN_WINDOW_SIZE, [708.0, 616.0]);
    }

    #[test]
    fn layout_regions_never_overlap() {
        for collapsed in [false, true] {
            for scanning in [false, true] {
                for h in [200.0, 400.0, SIDEBAR_MIN_HEIGHT, SIDEBAR_MIN_HEIGHT + 60.0, 900.0] {
                    let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(216.0, h));
                    let l = sidebar_layout(full, collapsed, scanning, true);
                    assert!(l.nav.bottom() <= l.bottom.top(), "nav/bottom h={h}");
                    assert!(l.playlists.bottom() <= l.bottom.top(), "playlists/bottom h={h}");
                    assert!(l.playlists.top() >= l.nav.bottom(), "playlists/nav h={h}");
                    assert!(l.playlists.height() >= 0.0);
                    if let Some(hdr) = l.playlist_header {
                        assert!(hdr.bottom() <= l.playlists.top() + 0.01);
                        assert!(l.playlists.height() >= NAV_ROW_H, "heading only with a row under it");
                    }
                }
                // At the minimum height the full nav and two playlist rows fit.
                let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(216.0, SIDEBAR_MIN_HEIGHT));
                let l = sidebar_layout(full, collapsed, false, true);
                assert!((l.nav.height() - rows_h(Page::MAIN.len())).abs() < 0.01);
                assert!(l.playlists.height() >= rows_h(MIN_PLAYLIST_ROWS) - 0.01, "{:?}", l.playlists);
                assert!(l.playlist_header.is_some());
            }
        }
    }

    /// Render the real sidebar at the minimum height with 10 playlists: no nav or playlist
    /// row rect (as clipped for interaction) touches the settings row or the toggle.
    #[test]
    fn rendered_bottom_block_does_not_overlap_rows() {
        use egui_kittest::Harness;
        let ids: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
        for collapsed in [false, true] {
            let ids2 = ids.clone();
            let width = if collapsed { metrics::SIDEBAR_COLLAPSED } else { metrics::SIDEBAR_EXPANDED };
            let mut h = Harness::builder().with_size(vec2(width, SIDEBAR_MIN_HEIGHT)).build_ui(move |ui| {
                let entries: Vec<PlaylistEntry> = ids2.iter().map(|id| PlaylistEntry { id, name: id, track_count: 1 }).collect();
                let mut st = SidebarState { collapsed };
                let rect = ui.ctx().content_rect();
                let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
                sidebar(&mut child, &SidebarProps { active: Page::Library, active_playlist: None, playlists: &entries, scanning: None, tr: Tr::JA }, &mut st);
            });
            h.run_steps(3);
            let ctx = h.ctx.clone();
            let settings = ctx.read_response(nav_id(Page::Settings)).expect("settings").interact_rect;
            let toggle: Rect = ctx.data(|d| d.get_temp(toggle_rect_id())).expect("toggle");
            assert!(toggle.top() >= settings.bottom() - 0.01);
            assert!(toggle.bottom() <= SIDEBAR_MIN_HEIGHT + 0.01, "toggle inside the window");
            let mut visible_playlists = 0;
            let rows = Page::MAIN.iter().map(|p| nav_id(*p)).chain(ids.iter().map(|id| playlist_id(id)));
            for id in rows {
                let Some(r) = ctx.read_response(id) else { continue };
                let r = r.interact_rect;
                if r.height() <= 0.5 {
                    continue; // scrolled out of view
                }
                assert!(!r.intersects(settings.shrink(0.5)), "row {r:?} overlaps settings {settings:?}");
                assert!(!r.intersects(toggle.shrink(0.5)), "row {r:?} overlaps toggle {toggle:?}");
                if r.height() >= NAV_ROW_H - 0.5 && ids.iter().any(|p| playlist_id(p) == id) {
                    visible_playlists += 1;
                }
            }
            assert!(visible_playlists >= MIN_PLAYLIST_ROWS, "collapsed={collapsed}: {visible_playlists} playlist rows visible");
        }
    }
}
