//! Collapsible navigation sidebar (icons + labels, active highlight, playlists section).

use eframe::egui::{
    self, pos2, vec2, Align, Color32, CursorIcon, FontId, Layout, Rect, Response, Sense, Stroke,
    Ui, UiBuilder, Vec2,
};

use crate::presentation::ui::{
    actions::{Page, UiAction},
    i18n::Tr,
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
        Page::Settings => "nav.settings",
    }
}

/// Draw the sidebar filling `ui`'s max rect. Size the containing panel with
/// [`SidebarState::width`].
pub fn sidebar(ui: &mut Ui, props: &SidebarProps<'_>, state: &mut SidebarState) -> Option<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let full = ui.max_rect();
    let collapsed = state.collapsed;
    let pad = if collapsed { 10.0 } else { 12.0 };
    let mut action = None;

    // ---- brand ----
    let brand = Rect::from_min_size(full.min + vec2(pad, 14.0), vec2(full.width() - pad * 2.0, 32.0));
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

    // ---- bottom area (settings + collapse) ----
    let bottom_h = 36.0 * 2.0 + 10.0 + if props.scanning.is_some() { 30.0 } else { 0.0 };
    let bottom = Rect::from_min_max(pos2(full.left() + pad, full.bottom() - bottom_h - 10.0), pos2(full.right() - pad, full.bottom() - 10.0));

    // ---- main nav ----
    let nav_top = brand.bottom() + 18.0;
    let nav_rect = Rect::from_min_max(pos2(full.left() + pad, nav_top), pos2(full.right() - pad, bottom.top() - 8.0));
    let mut nav_ui = ui.new_child(UiBuilder::new().max_rect(nav_rect).layout(Layout::top_down(Align::Min)));
    nav_ui.spacing_mut().item_spacing.y = 2.0;
    for page in Page::MAIN {
        let active = props.active == page && props.active_playlist.is_none();
        if nav_item(&mut nav_ui, &pal, page_icon(page), tr.t(page_label_key(page)), active, collapsed).clicked() {
            action = Some(UiAction::Navigate(page));
        }
    }

    // ---- playlists ----
    if !props.playlists.is_empty() {
        nav_ui.add_space(14.0);
        if !collapsed {
            nav_ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(tr.t("nav.playlists_section"))
                        .font(FontId::proportional(11.0))
                        .color(pal.text_faint),
                );
            });
            nav_ui.add_space(2.0);
        } else {
            let (r, _) = nav_ui.allocate_exact_size(vec2(nav_ui.available_width(), 9.0), Sense::hover());
            nav_ui.painter().hline(r.x_range().shrink(10.0), r.center().y, Stroke::new(1.0, pal.border));
        }
        egui::ScrollArea::vertical()
            .id_salt("sidebar_playlists")
            .auto_shrink([false, false])
            .show(&mut nav_ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for pl in props.playlists {
                    let active = props.active_playlist == Some(pl.id);
                    let resp = nav_item(ui, &pal, Icon::Playlist, pl.name, active, collapsed);
                    let resp = if collapsed { resp.on_hover_text(pl.name) } else { resp };
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

    // ---- bottom ----
    let mut bottom_ui = ui.new_child(UiBuilder::new().max_rect(bottom).layout(Layout::top_down(Align::Min)));
    bottom_ui.spacing_mut().item_spacing.y = 2.0;
    if let Some(progress) = props.scanning {
        bottom_ui.horizontal(|ui| {
            ui.set_height(26.0);
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
        bottom_ui.add_space(4.0);
    }
    let settings_active = props.active == Page::Settings && props.active_playlist.is_none();
    if nav_item(&mut bottom_ui, &pal, Icon::Settings, tr.t("nav.settings"), settings_active, collapsed).clicked() {
        action = Some(UiAction::Navigate(Page::Settings));
    }
    let tip = if collapsed { tr.t("nav.expand") } else { tr.t("nav.collapse") };
    bottom_ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(if collapsed { (ui.available_width() - 34.0) / 2.0 } else { 2.0 });
        if icon_button(ui, Icon::SidebarToggle, 18.0, false, tip).clicked() {
            state.collapsed = !state.collapsed;
            action = Some(UiAction::ToggleSidebar);
        }
    });

    action
}

/// A 36px navigation row.
fn nav_item(ui: &mut Ui, pal: &Palette, icon: Icon, label: &str, active: bool, collapsed: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
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
            let font = if active { theme::bold_font(ui.ctx(), theme::size::BODY) } else { FontId::proportional(theme::size::BODY) };
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
