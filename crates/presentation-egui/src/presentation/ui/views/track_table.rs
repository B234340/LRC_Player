//! Virtualized, sortable, resizable track table.
//!
//! Only visible rows are laid out (`ScrollArea::show_rows`), so 20k+ rows stay smooth.
//! Filtering/sorting works on index vectors and is cached in [`TrackTableState`]; it is
//! recomputed only when `rows_revision`, the query or the sort change.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Align, Color32, CursorIcon, EventFilter, FontId, Id, Key,
    LayerId, Modifiers, Order, Rect, Response, ScrollArea, Sense, Stroke, Ui, Vec2,
};

use super::search::{build_keys, filter_and_sort_with_keys, RowKeys};
use crate::presentation::ui::{
    actions::{SortColumn, SortSpec, UiAction},
    format::format_duration_opt,
    i18n::Tr,
    icons::{paint_icon, Icon},
    shell::PlaylistEntry,
    text,
    theme::{self, Palette},
    widgets::empty_state,
};

/// One track as shown in the table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackRow {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<i32>,
    pub genre: String,
}

/// Table columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Column {
    Index,
    Title,
    Artist,
    Album,
    Year,
    Genre,
    Duration,
}

impl Column {
    pub const DEFAULT: [Column; 5] = [Column::Index, Column::Title, Column::Artist, Column::Album, Column::Duration];

    fn slot(self) -> usize {
        self as usize
    }

    fn sort(self) -> SortColumn {
        match self {
            Column::Index => SortColumn::Index,
            Column::Title => SortColumn::Title,
            Column::Artist => SortColumn::Artist,
            Column::Album => SortColumn::Album,
            Column::Year => SortColumn::Year,
            Column::Genre => SortColumn::Genre,
            Column::Duration => SortColumn::Duration,
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Column::Index => "table.number",
            Column::Title => "table.title",
            Column::Artist => "table.artist",
            Column::Album => "table.album",
            Column::Year => "table.year",
            Column::Genre => "table.genre",
            Column::Duration => "table.duration",
        }
    }

    fn default_width(self) -> f32 {
        match self {
            Column::Index => 48.0,
            Column::Title => 0.0, // flexible
            Column::Artist => 200.0,
            Column::Album => 220.0,
            Column::Year => 64.0,
            Column::Genre => 120.0,
            Column::Duration => 72.0,
        }
    }

    fn min_width(self) -> f32 {
        match self {
            Column::Index => 36.0,
            Column::Title => 160.0,
            Column::Year => 48.0,
            Column::Duration => 56.0,
            _ => 110.0,
        }
    }

    fn resizable(self) -> bool {
        !matches!(self, Column::Index | Column::Title | Column::Duration)
    }

    fn right_aligned(self) -> bool {
        matches!(self, Column::Duration | Column::Index | Column::Year)
    }
}

/// What the `#` column shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IndexMode {
    /// 1-based position in the current view.
    #[default]
    Position,
    /// The track number tag (album pages).
    TrackNumber,
}

/// Which cell texts act as links. Only the painted text is clickable (pointer cursor,
/// accent color and underline on hover); the rest of the cell still selects the row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableLinks {
    /// Title plays the view from that row (like a double-click).
    pub title: bool,
    /// Artist opens the artist page (`UiAction::ShowArtist`).
    pub artist: bool,
    /// Album opens the album page (`UiAction::ShowAlbumOfTrack`).
    pub album: bool,
}

impl TableLinks {
    pub const ALL: TableLinks = TableLinks { title: true, artist: true, album: true };
    pub const NONE: TableLinks = TableLinks { title: false, artist: false, album: false };

    pub fn enabled(self, c: Column) -> bool {
        match c {
            Column::Title => self.title,
            Column::Artist => self.artist,
            Column::Album => self.album,
            _ => false,
        }
    }
}

/// Rect of a cell's text as painted: left-aligned (or right-aligned) with [`CELL_PAD`],
/// vertically centered.
pub(crate) fn cell_text_rect(cell: Rect, text: Vec2, right: bool) -> Rect {
    let x = if right { cell.right() - CELL_PAD - text.x } else { cell.left() + CELL_PAD };
    Rect::from_min_size(pos2(x, cell.center().y - text.y / 2.0), text)
}

/// The link (if any) whose text is under `pos`. Link rects get 2 px of vertical slack so
/// the underline is part of the target.
pub(crate) fn hit_link(links: &[(Column, Rect)], pos: egui::Pos2) -> Option<Column> {
    links.iter().find(|(_, r)| r.expand2(vec2(0.0, 2.0)).contains(pos)).map(|(c, _)| *c)
}

/// Payload carried while dragging rows (to reorder, or onto a sidebar playlist).
#[derive(Clone, Debug)]
pub struct TrackDragPayload {
    pub source: Id,
    /// Source row indices, in view order.
    pub rows: Vec<usize>,
    pub paths: Vec<PathBuf>,
    /// Title of the first dragged row (for the drag ghost).
    pub label: String,
}

pub struct TrackTableProps<'a> {
    pub id_salt: &'a str,
    pub rows: &'a [TrackRow],
    /// Bump whenever `rows` changes (cache key).
    pub rows_revision: u64,
    pub query: &'a str,
    pub columns: &'a [Column],
    pub now_playing: Option<&'a Path>,
    pub playing: bool,
    pub index_mode: IndexMode,
    /// Allow drag-reordering (only active with Index sort and no query).
    pub reorderable: bool,
    /// Playlists for the "Add to playlist" submenu.
    pub playlists: &'a [PlaylistEntry<'a>],
    /// Offer "Remove from playlist" (context menu, Delete key) emitting `RemoveRows`.
    pub removable: bool,
    /// Columns the header context menu can show / hide (emits `SetColumnVisible`); empty =
    /// no header menu. A column is shown when it is in `columns`.
    pub optional_columns: &'a [Column],
    /// Cell texts that act as links.
    pub links: TableLinks,
    pub tr: Tr,
}

/// Cached view (filtered + sorted indices).
#[derive(Clone, Debug, Default)]
struct ViewCache {
    revision: u64,
    len: usize,
    query: String,
    sort: SortSpec,
    view: Vec<usize>,
    valid: bool,
}

#[derive(Clone, Debug)]
pub struct TrackTableState {
    pub sort: SortSpec,
    /// Selected source-row indices.
    pub selected: HashSet<usize>,
    anchor: Option<usize>,
    /// Keyboard cursor (view position).
    cursor: Option<usize>,
    widths: [f32; 7],
    keys: Vec<RowKeys>,
    keys_revision: Option<(u64, usize)>,
    cache: ViewCache,
    scroll_to_cursor: bool,
    last_offset: f32,
    last_viewport_h: f32,
}

impl Default for TrackTableState {
    fn default() -> Self {
        let mut widths = [0.0; 7];
        for c in [Column::Index, Column::Title, Column::Artist, Column::Album, Column::Year, Column::Genre, Column::Duration] {
            widths[c.slot()] = c.default_width();
        }
        Self {
            sort: SortSpec::default(),
            selected: HashSet::new(),
            anchor: None,
            cursor: None,
            widths,
            keys: Vec::new(),
            keys_revision: None,
            cache: ViewCache::default(),
            scroll_to_cursor: false,
            last_offset: 0.0,
            last_viewport_h: 0.0,
        }
    }
}

impl TrackTableState {
    /// Filtered + sorted source-row indices (cached).
    pub fn view(&mut self, rows: &[TrackRow], revision: u64, query: &str) -> &[usize] {
        let rev = (revision, rows.len());
        if self.keys_revision != Some(rev) {
            self.keys = build_keys(rows);
            self.keys_revision = Some(rev);
            self.cache.valid = false;
            self.selected.clear();
            self.anchor = None;
            self.cursor = None;
        }
        let c = &self.cache;
        if !c.valid || c.revision != revision || c.len != rows.len() || c.query != query || c.sort != self.sort {
            self.cache = ViewCache {
                revision,
                len: rows.len(),
                query: query.to_owned(),
                sort: self.sort,
                view: filter_and_sort_with_keys(rows, &self.keys, query, self.sort),
                valid: true,
            };
        }
        &self.cache.view
    }

    pub fn column_width(&self, c: Column) -> f32 {
        self.widths[c.slot()]
    }

    pub fn set_column_width(&mut self, c: Column, w: f32) {
        self.widths[c.slot()] = w.max(c.min_width());
    }

    /// Selected paths in view order.
    pub fn selected_paths(&self, rows: &[TrackRow]) -> Vec<PathBuf> {
        self.cache
            .view
            .iter()
            .filter(|i| self.selected.contains(i))
            .map(|&i| rows[i].path.clone())
            .collect()
    }

    /// Select the row with `path` and scroll it into view (e.g. "show now playing").
    pub fn reveal(&mut self, rows: &[TrackRow], path: &Path) {
        if let Some(pos) = self.cache.view.iter().position(|&i| rows[i].path == path) {
            let row = self.cache.view[pos];
            self.selected.clear();
            self.selected.insert(row);
            self.anchor = Some(pos);
            self.cursor = Some(pos);
            self.scroll_to_cursor = true;
        }
    }
}

const HEADER_H: f32 = 32.0;
const ROW_H: f32 = theme::metrics::ROW_HEIGHT;
const CELL_PAD: f32 = 10.0;
/// Width reserved at the end of the title cell for the hover ⋯ button.
const MORE_SLOT: f32 = 28.0;

/// Layout of visible columns: (column, x offset, width).
///
/// When space is short, resizable columns shrink proportionally towards their minimum
/// width first; only then are optional columns dropped (Genre, Year, Album, Artist).
pub(crate) fn layout_columns(state: &TrackTableState, columns: &[Column], width: f32) -> Vec<(Column, f32, f32)> {
    let title_need = Column::Title.min_width() + 40.0;
    let mut cols: Vec<Column> = columns.to_vec();
    let mut widths: Vec<f32>;
    loop {
        widths = cols.iter().map(|c| if *c == Column::Title { 0.0 } else { state.column_width(*c) }).collect();
        let fixed: f32 = widths.iter().sum();
        let deficit = fixed + title_need - width;
        if deficit > 0.0 {
            let shrinkable: f32 = cols
                .iter()
                .zip(&widths)
                .filter(|(c, _)| c.resizable())
                .map(|(c, w)| (w - c.min_width()).max(0.0))
                .sum();
            if shrinkable > 0.0 {
                let k = (deficit / shrinkable).min(1.0);
                for (c, w) in cols.iter().zip(widths.iter_mut()) {
                    if c.resizable() {
                        *w -= (*w - c.min_width()).max(0.0) * k;
                    }
                }
            }
        }
        let fixed: f32 = widths.iter().sum();
        if width - fixed >= title_need {
            break;
        }
        let Some(drop) = [Column::Genre, Column::Year, Column::Album, Column::Artist]
            .into_iter()
            .find(|d| cols.contains(d))
        else {
            break;
        };
        cols.retain(|c| *c != drop);
    }
    let fixed: f32 = widths.iter().sum();
    let title_w = (width - fixed).max(Column::Title.min_width());
    let mut x = 0.0;
    cols.into_iter()
        .zip(widths)
        .map(|(c, w)| {
            let w = if c == Column::Title { title_w } else { w };
            let out = (c, x, w);
            x += w;
            out
        })
        .collect()
}

fn cell_job(text: &str, font: FontId, color: Color32, width: f32, right: bool) -> LayoutJob {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(8.0));
    if right {
        job.halign = Align::RIGHT;
    }
    job
}

/// Draw the table. Returns user actions.
pub fn track_table(ui: &mut Ui, props: &TrackTableProps<'_>, state: &mut TrackTableState) -> Vec<UiAction> {
    let tr = props.tr;
    let mut actions = Vec::new();
    let table_id = ui.make_persistent_id(("track_table", props.id_salt));

    if props.rows.is_empty() {
        if empty_state(ui, Icon::Library, tr.t("library.empty"), tr.t("library.empty_hint"), Some((Icon::Plus, tr.t("library.add_folder")))) {
            actions.push(UiAction::AddFolder);
        }
        return actions;
    }

    state.view(props.rows, props.rows_revision, props.query);
    // Borrow the cached view without copying it every frame.
    let view = std::mem::take(&mut state.cache.view);
    let actions = table_body(ui, props, state, &view, table_id, actions);
    state.cache.view = view;
    actions
}

fn table_body(
    ui: &mut Ui,
    props: &TrackTableProps<'_>,
    state: &mut TrackTableState,
    view: &[usize],
    table_id: Id,
    mut actions: Vec<UiAction>,
) -> Vec<UiAction> {
    let pal = theme::palette(ui);
    let tr = props.tr;
    let width = ui.available_width();
    let cols = layout_columns(state, props.columns, width);
    let reorder_enabled = props.reorderable && props.query.trim().is_empty() && state.sort.column == SortColumn::Index;

    // ---------------- header ----------------
    let (header_rect, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
    let mut resize: Option<(Column, f32)> = None;
    let header_hovered = ui.rect_contains_pointer(header_rect);
    for (c, x, w) in &cols {
        let r = Rect::from_min_size(header_rect.min + vec2(*x, 0.0), vec2(*w, HEADER_H));
        let resp = ui
            .interact(r, table_id.with(("hdr", c.slot())), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let sorted = state.sort.column == c.sort() && *c != Column::Index;
        let hovered = resp.hovered();
        if hovered {
            ui.painter().rect_filled(r.shrink2(vec2(2.0, 4.0)), 6.0, theme::with_alpha(pal.hover, 0.8));
        }
        let color = if sorted || hovered { pal.text } else { pal.text_muted };
        let font = if sorted { theme::bold_font(ui.ctx(), 12.0) } else { FontId::proportional(12.0) };
        let arrow_w = if sorted { 14.0 } else { 0.0 };
        let job = cell_job(tr.t(c.label_key()), font, color, w - CELL_PAD * 2.0 - arrow_w, false);
        let g = ui.painter().layout_job(job);
        let tx = if c.right_aligned() { r.right() - CELL_PAD - g.size().x - arrow_w } else { r.left() + CELL_PAD };
        let gy = r.center().y - g.size().y / 2.0;
        let gw = g.size().x;
        ui.painter().galley(pos2(tx, gy), g, color);
        if sorted {
            let icon = if state.sort.descending { Icon::ChevronDown } else { Icon::ChevronUp };
            paint_icon(ui.painter(), Rect::from_center_size(pos2(tx + gw + 9.0, r.center().y), Vec2::splat(11.0)), icon, pal.accent_fg);
        }
        if !props.optional_columns.is_empty() {
            resp.context_menu(|ui| column_menu(ui, props, &mut actions));
        }
        if resp.clicked() {
            let col = c.sort();
            state.sort = if *c == Column::Index {
                SortSpec::default()
            } else if state.sort.column != col {
                SortSpec { column: col, descending: false }
            } else if !state.sort.descending {
                SortSpec { column: col, descending: true }
            } else {
                SortSpec::default()
            };
            actions.push(UiAction::Sort(state.sort));
        }
        // resize handle on the right edge
        if c.resizable() {
            let handle = Rect::from_center_size(pos2(r.right(), r.center().y), vec2(8.0, HEADER_H - 8.0));
            let h = ui
                .interact(handle, table_id.with(("resize", c.slot())), Sense::drag())
                .on_hover_cursor(CursorIcon::ResizeHorizontal);
            let active = h.hovered() || h.dragged();
            if active || header_hovered {
                ui.painter().vline(
                    r.right(),
                    handle.y_range().shrink(4.0),
                    Stroke::new(if active { 2.0 } else { 1.0 }, if active { pal.accent } else { pal.border_strong }),
                );
            }
            if h.dragged() {
                resize = Some((*c, w + h.drag_delta().x));
            }
        }
    }
    if let Some((c, w)) = resize {
        state.set_column_width(c, w);
    }
    ui.painter().hline(header_rect.x_range(), header_rect.bottom() - 0.5, Stroke::new(1.0, pal.border));

    if view.is_empty() {
        ui.add_space(24.0);
        empty_state(ui, Icon::Search, tr.t("table.no_results"), tr.t("table.no_results_hint"), None);
        return actions;
    }

    // ---------------- keyboard ----------------
    let body_rect = Rect::from_min_size(ui.cursor().min, ui.available_size());
    let focus_resp = ui.interact(body_rect, table_id.with("focus"), Sense::focusable_noninteractive());
    let has_focus = focus_resp.has_focus();
    if has_focus {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                focus_resp.id,
                EventFilter { vertical_arrows: true, horizontal_arrows: false, tab: false, escape: false },
            )
        });
        handle_keys(ui, props, state, view, &mut actions);
    }

    // ---------------- rows ----------------
    let mut scroll = ScrollArea::vertical().id_salt(table_id.with("scroll")).auto_shrink([false, false]);
    if state.scroll_to_cursor {
        if let Some(cur) = state.cursor {
            let top = cur as f32 * ROW_H;
            let vh = state.last_viewport_h.max(ROW_H);
            if top < state.last_offset {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top + ROW_H > state.last_offset + vh {
                scroll = scroll.vertical_scroll_offset(top + ROW_H - vh);
            }
        }
        state.scroll_to_cursor = false;
    }

    let mut drop: Option<(usize, usize)> = None;
    let dragging = egui::DragAndDrop::has_any_payload(ui.ctx());
    let out = scroll.show_rows(ui, ROW_H, view.len(), |ui, range| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        for pos in range {
            let row_idx = view[pos];
            let row = &props.rows[row_idx];
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click_and_drag());
            let is_now = props.now_playing.is_some_and(|p| p == row.path);
            let selected = state.selected.contains(&row_idx);
            let play_view = |start: usize| UiAction::PlayTracks {
                paths: view.iter().map(|&i| props.rows[i].path.clone()).collect(),
                start,
            };

            // Hover affordances: ▶ in the # cell, ⋯ at the end of the title cell.
            let more_id = table_id.with(("more", row_idx));
            let more_open = egui::Popup::is_id_open(ui.ctx(), more_id.with("popup"));
            let row_hovered = !dragging && ui.rect_contains_pointer(rect);
            let buttons = row_hovered || more_open;
            let hover = if row_hovered { ui.ctx().pointer_hover_pos() } else { None };
            let links = paint_row(ui, &pal, props, &cols, rect, &resp, row, pos, is_now, selected, has_focus && state.cursor == Some(pos), hover, buttons);
            let hovered_link = hover.and_then(|p| hit_link(&links, p));
            let resp = match hovered_link {
                Some(c) => {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    let tip = match c {
                        Column::Title => tr.t("table.click_to_play"),
                        Column::Artist => tr.t("menu.show_artist"),
                        _ => tr.t("menu.show_album"),
                    };
                    resp.on_hover_text_at_pointer(tip)
                }
                None => resp,
            };
            if buttons {
                if let Some((_, x, w)) = cols.iter().find(|c| c.0 == Column::Index) {
                    let cell = Rect::from_min_size(pos2(rect.left() + x, rect.top()), vec2(*w, ROW_H));
                    let r = Rect::from_center_size(pos2(cell.right() - CELL_PAD - 7.0, cell.center().y), Vec2::splat(24.0));
                    let play = ui
                        .interact(r, table_id.with(("play", row_idx)), Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(tr.t("table.play_from_here"));
                    if play.clicked() {
                        actions.push(play_view(pos));
                    }
                }
                if let Some((_, x, w)) = cols.iter().find(|c| c.0 == Column::Title) {
                    let r = Rect::from_center_size(pos2(rect.left() + x + w - 16.0, rect.center().y), Vec2::splat(24.0));
                    let more = ui
                        .interact(r, more_id, Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand);
                    let hot = more.hovered() || more_open;
                    if hot {
                        ui.painter().rect_filled(r, 6.0, if selected { pal.hover } else { pal.pressed });
                    }
                    paint_icon(ui.painter(), r.shrink(5.0), Icon::More, if hot { pal.text } else { pal.text_muted });
                    let more = if more_open { more } else { more.on_hover_text(tr.t("table.more")) };
                    if more.clicked() && !selected {
                        state.selected.clear();
                        state.selected.insert(row_idx);
                        state.anchor = Some(pos);
                        state.cursor = Some(pos);
                    }
                    egui::Popup::menu(&more).show(|ui| {
                        context_menu(ui, props, state, view, row, &mut actions);
                    });
                }
            }

            if resp.clicked() {
                focus_resp.request_focus();
                let mods = ui.input(|i| i.modifiers);
                click_select(state, view, pos, mods);
                let link = if mods.shift || mods.command {
                    None
                } else {
                    resp.interact_pointer_pos().and_then(|p| hit_link(&links, p))
                };
                match link {
                    // The second click of a double-click on a title is not a second play.
                    Some(Column::Title) if !resp.double_clicked() => actions.push(play_view(pos)),
                    Some(Column::Artist) => actions.push(UiAction::ShowArtist(row.artist.clone())),
                    Some(Column::Album) => actions.push(UiAction::ShowAlbumOfTrack(row.path.clone())),
                    _ => {}
                }
            }
            if resp.double_clicked() {
                let on_link = resp.interact_pointer_pos().and_then(|p| hit_link(&links, p)).is_some();
                if !on_link {
                    actions.push(play_view(pos));
                }
            }
            if resp.secondary_clicked() && !selected {
                state.selected.clear();
                state.selected.insert(row_idx);
                state.anchor = Some(pos);
                state.cursor = Some(pos);
            }
            resp.context_menu(|ui| {
                context_menu(ui, props, state, view, row, &mut actions);
            });

            // drag source
            if resp.drag_started() && !state.selected.contains(&row_idx) {
                state.selected.clear();
                state.selected.insert(row_idx);
                state.anchor = Some(pos);
            }
            if resp.dragged() {
                let rows: Vec<usize> = view.iter().copied().filter(|i| state.selected.contains(i)).collect();
                let paths = rows.iter().map(|&i| props.rows[i].path.clone()).collect();
                resp.dnd_set_drag_payload(TrackDragPayload {
                    source: table_id,
                    rows,
                    paths,
                    label: row.title.clone(),
                });
            }
            // drop target (reorder)
            if reorder_enabled {
                if let Some(p) = resp.dnd_hover_payload::<TrackDragPayload>() {
                    if p.source == table_id {
                        let below = ui.input(|i| i.pointer.hover_pos()).is_some_and(|h| h.y > rect.center().y);
                        let y = if below { rect.bottom() } else { rect.top() };
                        ui.painter().hline(rect.x_range().shrink(4.0), y, Stroke::new(2.0, pal.accent));
                        ui.painter().circle_filled(pos2(rect.left() + 4.0, y), 3.0, pal.accent);
                    }
                }
                if let Some(p) = resp.dnd_release_payload::<TrackDragPayload>() {
                    if p.source == table_id {
                        if let Some(&from) = p.rows.first() {
                            let below = ui.input(|i| i.pointer.hover_pos()).is_some_and(|h| h.y > rect.center().y);
                            let to = row_idx + usize::from(below);
                            drop = Some((from, to));
                        }
                    }
                }
            }
        }
    });
    state.last_offset = out.state.offset.y;
    state.last_viewport_h = out.inner_rect.height();
    if let Some((from, to)) = drop {
        if from != to && from + 1 != to {
            actions.push(UiAction::Reorder { from, to });
        }
    }

    // drag ghost
    if let Some(p) = egui::DragAndDrop::payload::<TrackDragPayload>(ui.ctx()) {
        if p.source == table_id {
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                drag_ghost(ui, &pal, pos, &p.label, p.rows.len());
            }
        }
    }
    actions
}

#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &Ui,
    pal: &Palette,
    props: &TrackTableProps<'_>,
    cols: &[(Column, f32, f32)],
    rect: Rect,
    resp: &Response,
    row: &TrackRow,
    pos: usize,
    is_now: bool,
    selected: bool,
    cursor: bool,
    hover: Option<egui::Pos2>,
    buttons: bool,
) -> Vec<(Column, Rect)> {
    let mut links = Vec::new();
    if !ui.is_rect_visible(rect) {
        return links;
    }
    let painter = ui.painter();
    let hovered = resp.hovered() || buttons;
    let bg = rect.shrink2(vec2(2.0, 1.0));
    if selected {
        painter.rect_filled(bg, 6.0, pal.selected);
    } else if hovered {
        painter.rect_filled(bg, 6.0, pal.hover);
    }
    if cursor {
        painter.rect_stroke(bg, 6.0, Stroke::new(1.0, theme::with_alpha(pal.accent, 0.6)), egui::StrokeKind::Inside);
    }
    if is_now {
        painter.rect_filled(Rect::from_min_size(bg.min + vec2(0.0, 7.0), vec2(3.0, bg.height() - 14.0)), 1.5, pal.accent);
    }
    let body = FontId::proportional(theme::size::BODY);
    for (c, x, w) in cols {
        let cell = Rect::from_min_size(pos2(rect.left() + x, rect.top()), vec2(*w, rect.height()));
        let inner_w = w - CELL_PAD * 2.0;
        let (text, color, font): (String, Color32, FontId) = match c {
            Column::Index => {
                if buttons {
                    let ir = Rect::from_center_size(pos2(cell.right() - CELL_PAD - 7.0, cell.center().y), Vec2::splat(13.0));
                    paint_icon(painter, ir, Icon::Play, if is_now { pal.accent_fg } else { pal.text });
                    continue;
                }
                if is_now {
                    let icon = if props.playing { Icon::Volume2 } else { Icon::Speaker };
                    let ir = Rect::from_center_size(pos2(cell.right() - CELL_PAD - 7.0, cell.center().y), Vec2::splat(14.0));
                    paint_icon(painter, ir, icon, pal.accent_fg);
                    continue;
                }
                let n = match props.index_mode {
                    IndexMode::Position => Some(pos as u32 + 1),
                    IndexMode::TrackNumber => row.track_no,
                };
                (n.map(|n| n.to_string()).unwrap_or_default(), pal.text_faint, FontId::proportional(12.5))
            }
            Column::Title => (
                row.title.clone(),
                if is_now { pal.accent_fg } else { pal.text },
                // User content: each cell in the font of its own language.
                text::font_id(&row.title, theme::size::BODY, is_now),
            ),
            Column::Artist => (row.artist.clone(), pal.text_muted, text::font_id(&row.artist, theme::size::BODY, false)),
            Column::Album => (row.album.clone(), pal.text_muted, text::font_id(&row.album, theme::size::BODY, false)),
            Column::Year => (row.year.map(|y| y.to_string()).unwrap_or_default(), pal.text_muted, body.clone()),
            Column::Genre => (row.genre.clone(), pal.text_muted, text::font_id(&row.genre, theme::size::BODY, false)),
            Column::Duration => (format_duration_opt(row.duration_ms), pal.text_muted, FontId::proportional(12.5)),
        };
        if text.is_empty() {
            continue;
        }
        // The end of the title cell is reserved for the ⋯ button (always, so the text never
        // re-truncates when the row is hovered).
        let inner_w = if *c == Column::Title { inner_w - MORE_SLOT } else { inner_w };
        let job = cell_job(&text, font, Color32::PLACEHOLDER, inner_w, false);
        let g = painter.layout_job(job);
        let text_rect = cell_text_rect(cell, g.size(), c.right_aligned());
        let mut color = color;
        if props.links.enabled(*c) {
            if hover.is_some_and(|p| hit_link(&[(*c, text_rect)], p).is_some()) {
                color = pal.accent_fg;
                painter.hline(text_rect.x_range(), text_rect.bottom() - 1.0, Stroke::new(1.0, color));
            }
            links.push((*c, text_rect));
        }
        painter.galley(text_rect.min, g, color);
    }
    links
}

fn click_select(state: &mut TrackTableState, view: &[usize], pos: usize, mods: Modifiers) {
    let row = view[pos];
    if mods.shift {
        let anchor = state.anchor.unwrap_or(pos).min(view.len() - 1);
        let (a, b) = if anchor <= pos { (anchor, pos) } else { (pos, anchor) };
        if !mods.command {
            state.selected.clear();
        }
        state.selected.extend(view[a..=b].iter().copied());
    } else if mods.command {
        if !state.selected.remove(&row) {
            state.selected.insert(row);
        }
        state.anchor = Some(pos);
    } else {
        state.selected.clear();
        state.selected.insert(row);
        state.anchor = Some(pos);
    }
    state.cursor = Some(pos);
}

fn handle_keys(ui: &Ui, props: &TrackTableProps<'_>, state: &mut TrackTableState, view: &[usize], actions: &mut Vec<UiAction>) {
    let n = view.len();
    if n == 0 {
        return;
    }
    let (down, up, enter, home, end, all, pgdn, pgup, mods, delete) = ui.input_mut(|i| {
        (
            i.count_and_consume_key(Modifiers::NONE, Key::ArrowDown) + i.count_and_consume_key(Modifiers::SHIFT, Key::ArrowDown),
            i.count_and_consume_key(Modifiers::NONE, Key::ArrowUp) + i.count_and_consume_key(Modifiers::SHIFT, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Home),
            i.consume_key(Modifiers::NONE, Key::End),
            i.consume_key(Modifiers::COMMAND, Key::A),
            i.consume_key(Modifiers::NONE, Key::PageDown),
            i.consume_key(Modifiers::NONE, Key::PageUp),
            i.modifiers,
            props.removable && i.consume_key(Modifiers::NONE, Key::Delete),
        )
    });
    let page = ((state.last_viewport_h / ROW_H) as usize).saturating_sub(1).max(1);
    let cur = state.cursor.unwrap_or(0).min(n - 1);
    let mut next = None;
    if down > 0 {
        next = Some(if state.cursor.is_none() { 0 } else { (cur + down).min(n - 1) });
    }
    if up > 0 {
        next = Some(cur.saturating_sub(up));
    }
    if pgdn {
        next = Some((cur + page).min(n - 1));
    }
    if pgup {
        next = Some(cur.saturating_sub(page));
    }
    if home {
        next = Some(0);
    }
    if end {
        next = Some(n - 1);
    }
    if let Some(pos) = next {
        click_select(state, view, pos, Modifiers { shift: mods.shift, ..Default::default() });
        if !mods.shift {
            state.anchor = Some(pos);
        }
        state.scroll_to_cursor = true;
    }
    if all {
        state.selected = view.iter().copied().collect();
    }
    if delete && !state.selected.is_empty() {
        let mut rows: Vec<usize> = state.selected.iter().copied().collect();
        rows.sort_unstable();
        actions.push(UiAction::RemoveRows(rows));
    }
    if enter {
        if let Some(pos) = state.cursor {
            actions.push(UiAction::PlayTracks {
                paths: view.iter().map(|&i| props.rows[i].path.clone()).collect(),
                start: pos,
            });
        }
    }
}

fn context_menu(
    ui: &mut Ui,
    props: &TrackTableProps<'_>,
    state: &TrackTableState,
    view: &[usize],
    row: &TrackRow,
    actions: &mut Vec<UiAction>,
) {
    let tr = props.tr;
    let paths: Vec<PathBuf> = {
        let p: Vec<PathBuf> = view
            .iter()
            .filter(|i| state.selected.contains(i))
            .map(|&i| props.rows[i].path.clone())
            .collect();
        if p.is_empty() { vec![row.path.clone()] } else { p }
    };
    let item = |ui: &mut Ui, icon: Icon, text: &str| -> bool { menu_item(ui, icon, text).clicked() };
    if item(ui, Icon::Play, tr.t("menu.play_replace")) {
        actions.push(UiAction::PlayTracks { paths: paths.clone(), start: 0 });
        ui.close();
    }
    if item(ui, Icon::Next, tr.t("menu.play_next")) {
        actions.push(UiAction::PlayNext(paths.clone()));
        ui.close();
    }
    if item(ui, Icon::Queue, tr.t("menu.add_to_queue_end")) {
        actions.push(UiAction::Enqueue(paths.clone()));
        ui.close();
    }
    ui.menu_button(tr.t("menu.add_to_playlist"), |ui| {
        if props.playlists.is_empty() {
            ui.label(theme::muted(ui, tr.t("menu.no_playlists")));
        }
        for pl in props.playlists {
            if menu_item_ex(ui, Icon::Playlist, pl.name, true).clicked() {
                actions.push(UiAction::AddToPlaylist { playlist_id: pl.id.to_owned(), paths: paths.clone() });
                ui.close();
            }
        }
    });
    ui.separator();
    if item(ui, Icon::Album, tr.t("menu.show_album")) {
        actions.push(UiAction::ShowAlbumOfTrack(row.path.clone()));
        ui.close();
    }
    if item(ui, Icon::Artist, tr.t("menu.show_artist")) {
        actions.push(UiAction::ShowArtist(row.artist.clone()));
        ui.close();
    }
    if item(ui, Icon::Folder, tr.t("menu.open_folder")) {
        actions.push(UiAction::OpenContainingFolder(row.path.clone()));
        ui.close();
    }
    if item(ui, Icon::Info, tr.t("menu.details")) {
        actions.push(UiAction::ShowDetails(row.path.clone()));
        ui.close();
    }
    if props.removable {
        ui.separator();
        if item(ui, Icon::Trash, tr.t("menu.remove_from_playlist")) {
            let mut rows: Vec<usize> = view.iter().copied().filter(|i| state.selected.contains(i)).collect();
            if rows.is_empty() {
                if let Some(i) = props.rows.iter().position(|r| std::ptr::eq(r, row)) {
                    rows.push(i);
                }
            }
            actions.push(UiAction::RemoveRows(rows));
            ui.close();
        }
    }
}

/// Header context menu: show / hide the optional columns.
fn column_menu(ui: &mut Ui, props: &TrackTableProps<'_>, actions: &mut Vec<UiAction>) {
    let tr = props.tr;
    ui.set_max_width(200.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(theme::caption(ui, tr.t("table.columns")));
    });
    for c in props.optional_columns {
        let visible = props.columns.contains(c);
        if check_item(ui, visible, tr.t(c.label_key())).clicked() {
            actions.push(UiAction::SetColumnVisible { column: *c, visible: !visible });
            ui.close();
        }
    }
}

/// Menu entry with a leading check mark.
pub fn check_item(ui: &mut Ui, checked: bool, text: &str) -> Response {
    let pal = theme::palette(ui);
    let width = ui.available_width().clamp(160.0, 240.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, pal.hover);
    }
    let color = if resp.hovered() { pal.text } else { theme::blend(pal.text, pal.text_muted, 0.25) };
    if checked {
        paint_icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(14.0)), Icon::Check, pal.accent_fg);
    }
    let g = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(theme::size::BODY), color);
    ui.painter().galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, color);
    resp
}

/// Menu entry with a leading icon.
pub fn menu_item(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    menu_item_ex(ui, icon, text, false)
}

/// [`menu_item`]; `user_content` draws the label in the font of its language (playlist
/// names, titles) instead of the UI font.
pub fn menu_item_ex(ui: &mut Ui, icon: Icon, label: &str, user_content: bool) -> Response {
    let text = label;
    let pal = theme::palette(ui);
    let width = ui.available_width().max(180.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, pal.hover);
    }
    let color = if resp.hovered() { pal.text } else { theme::blend(pal.text, pal.text_muted, 0.25) };
    paint_icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(15.0)), icon, pal.text_muted);
    let font = if user_content { text::font_id(text, theme::size::BODY, false) } else { FontId::proportional(theme::size::BODY) };
    let g = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    ui.painter().galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, color);
    resp
}

fn drag_ghost(ui: &Ui, pal: &Palette, pos: egui::Pos2, label: &str, count: usize) {
    let painter = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, Id::new("track_drag_ghost")));
    let text = if count > 1 { format!("{label}  +{}", count - 1) } else { label.to_owned() };
    let g = painter.layout_job(cell_job(&text, text::font_id(label, 13.0, false), pal.text, 260.0, false));
    let rect = Rect::from_min_size(pos + vec2(14.0, 10.0), vec2(g.size().x + 40.0, 30.0));
    painter.add(egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: pal.shadow }.as_shape(rect, 8));
    painter.rect(rect, 8.0, pal.elevated, Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
    paint_icon(&painter, Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(14.0)), Icon::Note, pal.accent_fg);
    painter.galley(pos2(rect.left() + 30.0, rect.center().y - g.size().y / 2.0), g, pal.text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_hit_testing_covers_only_the_text() {
        let cell = Rect::from_min_size(pos2(100.0, 50.0), vec2(200.0, ROW_H));
        let text = vec2(60.0, 16.0);
        let r = cell_text_rect(cell, text, false);
        assert_eq!(r.left(), 100.0 + CELL_PAD);
        assert!((r.center().y - cell.center().y).abs() < 0.01);
        let rr = cell_text_rect(cell, text, true);
        assert_eq!(rr.right(), cell.right() - CELL_PAD);

        let artist_cell = cell.translate(vec2(200.0, 0.0));
        let links = [(Column::Title, r), (Column::Artist, cell_text_rect(artist_cell, vec2(40.0, 16.0), false))];
        // on the title text
        assert_eq!(hit_link(&links, r.center()), Some(Column::Title));
        // just under the text (underline slack)
        assert_eq!(hit_link(&links, pos2(r.center().x, r.bottom() + 1.5)), Some(Column::Title));
        // empty space of the title cell after the text: no link (row selection)
        assert_eq!(hit_link(&links, pos2(r.right() + 20.0, r.center().y)), None);
        // the cell padding before the text is not a link either
        assert_eq!(hit_link(&links, pos2(cell.left() + 2.0, r.center().y)), None);
        // artist text
        assert_eq!(hit_link(&links, pos2(artist_cell.left() + CELL_PAD + 5.0, cell.center().y)), Some(Column::Artist));
        // above the row's text band
        assert_eq!(hit_link(&links, pos2(r.center().x, cell.top() + 1.0)), None);
        assert!(TableLinks::ALL.enabled(Column::Album));
        assert!(!TableLinks::ALL.enabled(Column::Duration));
        assert!(!TableLinks::NONE.enabled(Column::Title));
    }

    #[test]
    fn columns_shrink_before_dropping() {
        let st = TrackTableState::default();
        let wide = layout_columns(&st, &Column::DEFAULT, 1200.0);
        assert_eq!(wide.len(), 5);
        let total: f32 = wide.iter().map(|c| c.2).sum();
        assert!((total - 1200.0).abs() < 0.5);
        // narrower: album kept but shrunk
        let mid = layout_columns(&st, &Column::DEFAULT, 650.0);
        assert_eq!(mid.len(), 5, "{mid:?}");
        let album = mid.iter().find(|c| c.0 == Column::Album).unwrap();
        assert!(album.2 < Column::Album.default_width());
        // very narrow: optional columns dropped, title keeps its minimum
        let narrow = layout_columns(&st, &Column::DEFAULT, 380.0);
        assert!(!narrow.iter().any(|c| c.0 == Column::Album));
        let title = narrow.iter().find(|c| c.0 == Column::Title).unwrap();
        assert!(title.2 >= Column::Title.min_width());
    }
}
