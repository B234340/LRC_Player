//! Play queue panel: now-playing indicator, drag-to-reorder, remove, clear, save.

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Color32, CursorIcon, FontId, Id, LayerId, Order, Rect,
    ScrollArea, Sense, Stroke, Ui, Vec2,
};

use super::track_table::TrackDragPayload;
use crate::presentation::ui::{
    actions::UiAction,
    format::{format_count, format_duration_opt, format_total},
    i18n::Tr,
    icons::{icon_button_ex, paint_icon, Icon, IconButtonStyle},
    theme,
    widgets::{button, empty_state, paint_artwork, ButtonKind, Tex},
};

/// One queue entry.
#[derive(Clone, Copy, Debug)]
pub struct QueueItem<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub duration_ms: Option<u64>,
    pub artwork: Option<Tex>,
    /// Placeholder seed (album name).
    pub seed: &'a str,
}

pub struct QueueProps<'a> {
    pub items: &'a [QueueItem<'a>],
    pub current: Option<usize>,
    pub playing: bool,
    pub tr: Tr,
}

#[derive(Clone, Copy, Debug)]
struct QueueDrag {
    from: usize,
}

const ROW_H: f32 = 52.0;

/// Draw the queue panel filling `ui`.
pub fn queue_panel(ui: &mut Ui, props: &QueueProps<'_>) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let mut actions = Vec::new();
    let id = Id::new("queue_panel");

    // ---- header ----
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(theme::title(ui, tr.t("queue.title")).color(pal.text));
            let total: u64 = props.items.iter().filter_map(|i| i.duration_ms).sum();
            let sub = format!("{} {} · {}", format_count(props.items.len()), tr.t("table.tracks_count"), format_total(total));
            ui.label(theme::caption(ui, sub));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(!props.items.is_empty(), |ui| {
                if button(ui, ButtonKind::Ghost, None, tr.t("queue.clear")).clicked() {
                    actions.push(UiAction::QueueClear);
                }
                let save = icon_button_ex(ui, Icon::Playlist, 16.0, IconButtonStyle { enabled: !props.items.is_empty(), ..Default::default() }, tr.t("queue.save_as_playlist"));
                if save.clicked() {
                    actions.push(UiAction::QueueSaveAsPlaylist);
                }
            });
        });
    });
    ui.add_space(6.0);

    if props.items.is_empty() {
        actions.extend(queue_empty(ui, id, tr));
        return actions;
    }

    actions.extend(queue_rows(ui, id, props, 0));
    actions
}

/// Fixed slots of a queue row (independent of hover, so hovering never moves anything).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowSlots {
    pub art: Rect,
    pub text: Rect,
    /// Remove button (shown on hover over the duration).
    pub remove: Rect,
    pub grip: Rect,
}

/// Trailing area reserved for duration / grip + remove.
const TRAILING_W: f32 = 74.0;

pub(crate) fn row_slots(rect: Rect) -> RowSlots {
    let bg = rect.shrink2(vec2(0.0, 2.0));
    let art = Rect::from_min_size(pos2(bg.left() + 8.0, bg.center().y - 18.0), Vec2::splat(36.0));
    let text_left = art.right() + 10.0;
    let text = Rect::from_min_max(pos2(text_left, bg.top()), pos2((bg.right() - TRAILING_W).max(text_left + 20.0), bg.bottom()));
    RowSlots {
        art,
        text,
        remove: Rect::from_center_size(pos2(bg.right() - 20.0, bg.center().y), Vec2::splat(28.0)),
        grip: Rect::from_center_size(pos2(bg.right() - 50.0, bg.center().y), Vec2::splat(14.0)),
    }
}

/// Paint the drop indicator line at `y` across `rect`.
fn insertion_line(ui: &Ui, rect: Rect, y: f32, color: Color32) {
    ui.painter().hline(rect.x_range().shrink(6.0), y, Stroke::new(2.0, color));
    ui.painter().circle_filled(pos2(rect.left() + 6.0, y), 3.0, color);
}

/// Rows of the queue from index `first` on (the queue page lists only "up next"):
/// double-click plays, hover shows remove, drag reorders, and tracks dragged from any
/// track table can be dropped between rows (`QueueInsert`) or below the list (append).
pub fn queue_rows(ui: &mut Ui, id: Id, props: &QueueProps<'_>, first: usize) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let mut actions = Vec::new();
    let width = ui.available_width();
    let mut drop: Option<(usize, usize)> = None;
    let mut insert: Option<(Vec<std::path::PathBuf>, usize)> = None;
    let count = props.items.len().saturating_sub(first);
    let out = ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, count, |ui, range| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            for row in range {
                let i = first + row;
                let item = &props.items[i];
                // Fixed-size row with a stable id; everything hover-dependent below only
                // changes what is painted, never the layout.
                let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
                let resp = ui.interact(rect, id.with(("row", i)), Sense::click_and_drag());
                let is_cur = props.current == Some(i);
                let played = props.current.is_some_and(|c| i < c);
                // Geometric hover: the remove button on top of the row must not un-hover it.
                let hovered = resp.dragged() || (ui.rect_contains_pointer(rect) && !egui::DragAndDrop::has_any_payload(ui.ctx()));
                let painter = ui.painter();
                let bg = rect.shrink2(vec2(0.0, 2.0));
                if is_cur {
                    painter.rect_filled(bg, 8.0, pal.accent_soft);
                } else if hovered {
                    painter.rect_filled(bg, 8.0, pal.hover);
                }
                let slots = row_slots(rect);
                let art = slots.art;
                paint_artwork(ui, art, item.artwork, 5, item.seed);
                if is_cur {
                    painter.rect_filled(art, 5.0, Color32::from_black_alpha(110));
                    let icon = if props.playing { Icon::Volume2 } else { Icon::Pause };
                    paint_icon(painter, art.shrink(10.0), icon, Color32::WHITE);
                } else if hovered {
                    painter.rect_filled(art, 5.0, Color32::from_black_alpha(90));
                    paint_icon(painter, art.shrink(10.0), Icon::Play, Color32::WHITE);
                }
                // text (truncation width does not depend on hover)
                let text_left = slots.text.left();
                let tw = slots.text.width();
                let alpha = if played { 0.55 } else { 1.0 };
                let title_c = theme::with_alpha(if is_cur { pal.accent_fg } else { pal.text }, alpha);
                let font = if is_cur { theme::bold_font(ui.ctx(), 13.0) } else { FontId::proportional(13.0) };
                let mut j = LayoutJob::simple_singleline(item.title.to_owned(), font, title_c);
                j.wrap = egui::text::TextWrapping::truncate_at_width(tw);
                let g = painter.layout_job(j);
                painter.galley(pos2(text_left, bg.center().y - g.size().y - 1.0), g, title_c);
                let sub_c = theme::with_alpha(pal.text_muted, alpha);
                let mut j = LayoutJob::simple_singleline(item.artist.to_owned(), FontId::proportional(12.0), sub_c);
                j.wrap = egui::text::TextWrapping::truncate_at_width(tw);
                let g = painter.layout_job(j);
                painter.galley(pos2(text_left, bg.center().y + 2.0), g, sub_c);
                // Trailing slot: duration at rest; grip + remove button on hover (overlay
                // widgets via `interact`, which never touches the layout cursor).
                if hovered {
                    let r = slots.remove;
                    let rm = ui
                        .interact(r, id.with(("remove", i)), Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(tr.t("queue.remove"));
                    if rm.hovered() {
                        ui.painter().rect_filled(r, 6.0, pal.pressed);
                    }
                    paint_icon(ui.painter(), r.shrink(8.0), Icon::Close, if rm.hovered() { pal.text } else { pal.text_muted });
                    if rm.clicked() {
                        actions.push(UiAction::QueueRemove(i));
                    }
                    paint_icon(ui.painter(), slots.grip, Icon::Grip, pal.text_faint);
                } else {
                    let d = format_duration_opt(item.duration_ms);
                    let g = ui.painter().layout_no_wrap(d, FontId::proportional(12.0), sub_c);
                    ui.painter().galley(pos2(bg.right() - 10.0 - g.size().x, bg.center().y - g.size().y / 2.0), g, sub_c);
                }

                if resp.double_clicked() {
                    actions.push(UiAction::QueuePlayIndex(i));
                }
                resp.dnd_set_drag_payload(QueueDrag { from: i });
                if resp.dragged() {
                    resp.clone().on_hover_cursor(CursorIcon::Grabbing);
                }
                let below = ui.input(|inp| inp.pointer.hover_pos()).is_some_and(|h| h.y > rect.center().y);
                let y = if below { rect.bottom() } else { rect.top() };
                if let Some(p) = resp.dnd_hover_payload::<QueueDrag>() {
                    if p.from != i {
                        insertion_line(ui, rect, y, pal.accent);
                    }
                }
                if let Some(p) = resp.dnd_release_payload::<QueueDrag>() {
                    drop = Some((p.from, i + usize::from(below)));
                }
                if resp.dnd_hover_payload::<TrackDragPayload>().is_some() {
                    insertion_line(ui, rect, y, pal.accent);
                }
                if let Some(p) = resp.dnd_release_payload::<TrackDragPayload>() {
                    insert = Some((p.paths.clone(), i + usize::from(below)));
                }
            }
        });
    // Below the last row: append.
    let content_bottom = out.inner_rect.top() + (out.content_size.y - out.state.offset.y);
    let rest = Rect::from_min_max(pos2(out.inner_rect.left(), content_bottom.max(out.inner_rect.top())), out.inner_rect.max);
    if rest.height() > 4.0 {
        let r = ui.interact(rest, id.with("drop_end"), Sense::hover());
        if r.dnd_hover_payload::<TrackDragPayload>().is_some() {
            insertion_line(ui, rest, rest.top() + 1.0, pal.accent);
        }
        if let Some(p) = r.dnd_release_payload::<TrackDragPayload>() {
            insert = Some((p.paths.clone(), props.items.len()));
        }
    }
    if let Some((from, to)) = drop {
        if from != to && from + 1 != to {
            actions.push(UiAction::QueueReorder { from, to });
        }
    }
    if let Some((paths, at)) = insert {
        actions.push(UiAction::QueueInsert { paths, at });
    }
    // ghost
    if let Some(p) = egui::DragAndDrop::payload::<QueueDrag>(ui.ctx()) {
        if let (Some(pos), Some(item)) = (ui.ctx().pointer_interact_pos(), props.items.get(p.from)) {
            let painter = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, id.with("ghost")));
            let mut j = LayoutJob::simple_singleline(item.title.to_owned(), FontId::proportional(13.0), pal.text);
            j.wrap = egui::text::TextWrapping::truncate_at_width(220.0);
            let g = painter.layout_job(j);
            let r = Rect::from_min_size(pos + vec2(12.0, 8.0), vec2(g.size().x + 20.0, 28.0));
            painter.add(egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: pal.shadow }.as_shape(r, 8));
            painter.rect(r, 8.0, pal.elevated, Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
            painter.galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, pal.text);
        }
    }
    actions
}

/// Empty queue: hint, and the whole area accepts tracks dragged from a table.
pub fn queue_empty(ui: &mut Ui, id: Id, tr: Tr) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let rect = ui.available_rect_before_wrap();
    let r = ui.interact(rect, id.with("drop_empty"), Sense::hover());
    let hovering = r.dnd_hover_payload::<TrackDragPayload>().is_some();
    let mut actions = Vec::new();
    if let Some(p) = r.dnd_release_payload::<TrackDragPayload>() {
        actions.push(UiAction::QueueInsert { paths: p.paths.clone(), at: 0 });
    }
    if hovering {
        ui.painter().rect_stroke(rect.shrink(4.0), 12.0, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
    }
    empty_state(ui, Icon::Queue, tr.t("queue.empty"), tr.t("queue.empty_hint"), None);
    actions
}

/// Map a `QueueReorder { from, to }` (insertion index before removal) onto a vector.
pub fn apply_reorder<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from >= items.len() {
        return;
    }
    let item = items.remove(from);
    let to = if to > from { to - 1 } else { to };
    items.insert(to.min(items.len()), item);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hovering a row only changes paint: row rects stay identical, and the slots (art,
    /// text width, trailing area) do not depend on hover.
    #[test]
    fn hover_does_not_move_rows() {
        use egui_kittest::Harness;
        let titles = ["One", "Two", "Three"];
        let read_rows = |h: &Harness<'_, ()>| -> Vec<Rect> {
            let id = Id::new("queue_panel");
            (0..3).map(|i| h.ctx.read_response(id.with(("row", i))).expect("row").rect).collect()
        };
        let mut h = Harness::builder().with_size(vec2(360.0, 400.0)).build_ui(move |ui| {
            let items: Vec<QueueItem> = titles
                .iter()
                .map(|t| QueueItem { title: t, artist: "A", duration_ms: Some(200_000), artwork: None, seed: t })
                .collect();
            let _ = queue_panel(ui, &QueueProps { items: &items, current: Some(0), playing: true, tr: Tr::JA });
        });
        h.run_steps(3);
        let rest = read_rows(&h);
        h.hover_at(rest[1].center());
        for _ in 0..5 {
            h.run_steps(1);
            assert_eq!(read_rows(&h), rest, "rows moved while hovering");
        }
        let s = row_slots(rest[1]);
        assert!(s.text.right() <= s.grip.left(), "text never overlaps the trailing slot");
    }

    #[test]
    fn reorder_semantics() {
        let mut v = vec![0, 1, 2, 3, 4];
        apply_reorder(&mut v, 0, 3); // drop 0 before index 3
        assert_eq!(v, vec![1, 2, 0, 3, 4]);
        let mut v = vec![0, 1, 2, 3, 4];
        apply_reorder(&mut v, 4, 0);
        assert_eq!(v, vec![4, 0, 1, 2, 3]);
        let mut v = vec![0, 1, 2];
        apply_reorder(&mut v, 0, 3); // to end
        assert_eq!(v, vec![1, 2, 0]);
    }
}
