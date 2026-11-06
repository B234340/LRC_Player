//! Play queue panel: now-playing indicator, drag-to-reorder, remove, clear, save.

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Color32, CursorIcon, FontId, Id, LayerId, Order, Rect,
    ScrollArea, Sense, Stroke, Ui, Vec2,
};

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
        empty_state(ui, Icon::Queue, tr.t("queue.empty"), tr.t("queue.empty_hint"), None);
        return actions;
    }

    let width = ui.available_width();
    let mut drop: Option<(usize, usize)> = None;
    ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, props.items.len(), |ui, range| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            for i in range {
                let item = &props.items[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click_and_drag());
                let is_cur = props.current == Some(i);
                let played = props.current.is_some_and(|c| i < c);
                let hovered = resp.hovered() || resp.dragged();
                let painter = ui.painter();
                let bg = rect.shrink2(vec2(0.0, 2.0));
                if is_cur {
                    painter.rect_filled(bg, 8.0, pal.accent_soft);
                } else if hovered {
                    painter.rect_filled(bg, 8.0, pal.hover);
                }
                // artwork / grip
                let art = Rect::from_min_size(pos2(bg.left() + 8.0, bg.center().y - 18.0), Vec2::splat(36.0));
                paint_artwork(ui, art, item.artwork, 5, item.seed);
                if is_cur {
                    painter.rect_filled(art, 5.0, Color32::from_black_alpha(110));
                    let icon = if props.playing { Icon::Volume2 } else { Icon::Pause };
                    paint_icon(painter, art.shrink(10.0), icon, Color32::WHITE);
                } else if hovered {
                    painter.rect_filled(art, 5.0, Color32::from_black_alpha(90));
                    paint_icon(painter, art.shrink(10.0), Icon::Play, Color32::WHITE);
                }
                // text
                let text_left = art.right() + 10.0;
                let right_w = if hovered { 70.0 } else { 46.0 };
                let tw = (bg.right() - right_w - text_left).max(20.0);
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
                // duration or remove button
                if hovered {
                    let r = Rect::from_center_size(pos2(bg.right() - 20.0, bg.center().y), Vec2::splat(28.0));
                    let rm = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                            icon_button_ex(ui, Icon::Close, 12.0, IconButtonStyle { padding: Some(8.0), ..Default::default() }, tr.t("queue.remove"))
                        })
                        .inner;
                    if rm.clicked() {
                        actions.push(UiAction::QueueRemove(i));
                    }
                    paint_icon(ui.painter(), Rect::from_center_size(pos2(bg.right() - 50.0, bg.center().y), Vec2::splat(14.0)), Icon::Grip, pal.text_faint);
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
                if let Some(p) = resp.dnd_hover_payload::<QueueDrag>() {
                    let below = ui.input(|inp| inp.pointer.hover_pos()).is_some_and(|h| h.y > rect.center().y);
                    if p.from != i {
                        let y = if below { rect.bottom() } else { rect.top() };
                        ui.painter().hline(rect.x_range().shrink(6.0), y, Stroke::new(2.0, pal.accent));
                        ui.painter().circle_filled(pos2(rect.left() + 6.0, y), 3.0, pal.accent);
                    }
                }
                if let Some(p) = resp.dnd_release_payload::<QueueDrag>() {
                    let below = ui.input(|inp| inp.pointer.hover_pos()).is_some_and(|h| h.y > rect.center().y);
                    drop = Some((p.from, i + usize::from(below)));
                }
            }
        });
    if let Some((from, to)) = drop {
        if from != to && from + 1 != to {
            actions.push(UiAction::QueueReorder { from, to });
        }
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
