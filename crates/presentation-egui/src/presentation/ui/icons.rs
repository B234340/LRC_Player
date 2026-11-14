//! Vector icons painted with the egui painter (crisp at any DPI, no glyph coverage issues).
//!
//! Every icon is designed on a 24×24 grid and scaled into the target rect.

use std::f32::consts::{PI, TAU};

use eframe::egui::{
    self, epaint::Mesh, pos2, vec2, Color32, CursorIcon, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, Ui, Vec2,
};

use super::theme::{self, Palette};

/// Available icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Play,
    Pause,
    Previous,
    Next,
    Shuffle,
    Repeat,
    RepeatOne,
    Volume0,
    Volume1,
    Volume2,
    Volume3,
    Mute,
    Search,
    Lyrics,
    Queue,
    Eq,
    Settings,
    Library,
    Album,
    Artist,
    Playlist,
    Folder,
    Plus,
    Minus,
    Close,
    More,
    MoreVertical,
    Heart,
    HeartFilled,
    Note,
    NowPlaying,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    Check,
    Trash,
    Refresh,
    Grip,
    SidebarToggle,
    Save,
    Info,
    Warning,
    Error,
    Success,
    Speaker,
    TextSmaller,
    TextLarger,
    Undo,
    ArrowDownToLine,
    /// Pencil (rename / edit).
    Edit,
    /// Waveform (signal path / sound quality).
    Waveform,
}

impl Icon {
    /// Volume icon for a linear gain / mute state.
    pub fn for_volume(gain: f32, muted: bool) -> Icon {
        if muted {
            Icon::Mute
        } else if gain <= 0.0001 {
            Icon::Volume0
        } else if gain < 0.12 {
            Icon::Volume1
        } else if gain < 0.45 {
            Icon::Volume2
        } else {
            Icon::Volume3
        }
    }
}

/// Maps the 24×24 design grid into a rect.
struct Grid {
    origin: Pos2,
    scale: f32,
}

impl Grid {
    fn new(rect: Rect) -> Self {
        let side = rect.width().min(rect.height());
        let origin = rect.center() - Vec2::splat(side / 2.0);
        Self { origin, scale: side / 24.0 }
    }
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.scale
    }
    fn s(&self, v: f32) -> f32 {
        v * self.scale
    }
    fn pts(&self, pts: &[(f32, f32)]) -> Vec<Pos2> {
        pts.iter().map(|&(x, y)| self.p(x, y)).collect()
    }
}

fn cubic(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let v = p0.to_vec2() * (u * u * u)
                + p1.to_vec2() * (3.0 * u * u * t)
                + p2.to_vec2() * (3.0 * u * t * t)
                + p3.to_vec2() * (t * t * t);
            v.to_pos2()
        })
        .collect()
}

fn arc(center: Pos2, r: f32, a0: f32, a1: f32, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

/// Open polyline with round caps.
fn polyline(painter: &Painter, pts: Vec<Pos2>, stroke: Stroke) {
    if pts.len() < 2 {
        return;
    }
    let r = stroke.width * 0.5;
    painter.circle_filled(pts[0], r, stroke.color);
    painter.circle_filled(*pts.last().unwrap(), r, stroke.color);
    if pts.len() > 2 {
        // round joins for sharp corners
        for p in &pts[1..pts.len() - 1] {
            painter.circle_filled(*p, r * 0.98, stroke.color);
        }
    }
    painter.add(Shape::line(pts, stroke));
}

fn seg(painter: &Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    polyline(painter, vec![a, b], stroke);
}

/// Fill a star-shaped polygon (visible from `center`) with a triangle fan.
fn fill_fan(painter: &Painter, center: Pos2, outline: &[Pos2], color: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    for p in outline {
        mesh.colored_vertex(*p, color);
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
    // anti-aliased edge
    painter.add(Shape::closed_line(outline.to_vec(), Stroke::new(0.8, color)));
}

fn heart_outline(g: &Grid) -> Vec<Pos2> {
    // Parametric heart, scaled into roughly x 3..21, y 4..20.
    (0..64)
        .map(|i| {
            let t = i as f32 / 64.0 * TAU;
            let x = 16.0 * t.sin().powi(3);
            let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            g.p(12.0 + x * 0.56, 11.2 - y * 0.56)
        })
        .collect()
}

/// Paint `icon` centered in `rect` (uses the shorter side) with `color`.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let g = Grid::new(rect);
    let w = (g.s(1.75)).max(1.1);
    let st = Stroke::new(w, color);
    let fill = |pts: &[(f32, f32)]| {
        painter.add(Shape::convex_polygon(g.pts(pts), color, Stroke::new(0.6, color)));
    };
    let rrect = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        painter.rect_filled(Rect::from_min_max(g.p(x0, y0), g.p(x1, y1)), g.s(r), color);
    };

    match icon {
        Icon::Play => fill(&[(7.5, 4.8), (19.5, 12.0), (7.5, 19.2)]),
        Icon::Pause => {
            rrect(6.5, 5.0, 10.0, 19.0, 1.2);
            rrect(14.0, 5.0, 17.5, 19.0, 1.2);
        }
        Icon::Previous => {
            rrect(5.0, 5.5, 7.6, 18.5, 1.0);
            fill(&[(19.0, 5.5), (8.6, 12.0), (19.0, 18.5)]);
        }
        Icon::Next => {
            rrect(16.4, 5.5, 19.0, 18.5, 1.0);
            fill(&[(5.0, 5.5), (15.4, 12.0), (5.0, 18.5)]);
        }
        Icon::Shuffle => {
            let a = cubic(g.p(3.5, 7.0), g.p(10.0, 7.0), g.p(12.0, 17.0), g.p(19.0, 17.0), 16);
            let b = cubic(g.p(3.5, 17.0), g.p(10.0, 17.0), g.p(12.0, 7.0), g.p(19.0, 7.0), 16);
            polyline(painter, a, st);
            polyline(painter, b, st);
            polyline(painter, g.pts(&[(16.5, 4.0), (19.8, 7.0), (16.5, 10.0)]), st);
            polyline(painter, g.pts(&[(16.5, 14.0), (19.8, 17.0), (16.5, 20.0)]), st);
        }
        Icon::Repeat | Icon::RepeatOne => {
            // top: left-bottom corner up to the right with an arrow
            let mut top = vec![g.p(4.5, 12.5)];
            top.extend(cubic(g.p(4.5, 10.0), g.p(4.5, 7.0), g.p(5.5, 7.0), g.p(8.0, 7.0), 8));
            top.push(g.p(19.0, 7.0));
            polyline(painter, top, st);
            polyline(painter, g.pts(&[(16.0, 4.0), (19.2, 7.0), (16.0, 10.0)]), st);
            let mut bottom = vec![g.p(19.5, 11.5)];
            bottom.extend(cubic(g.p(19.5, 14.0), g.p(19.5, 17.0), g.p(18.5, 17.0), g.p(16.0, 17.0), 8));
            bottom.push(g.p(5.0, 17.0));
            polyline(painter, bottom, st);
            polyline(painter, g.pts(&[(8.0, 14.0), (4.8, 17.0), (8.0, 20.0)]), st);
            if icon == Icon::RepeatOne {
                let thin = Stroke::new(w * 0.85, color);
                polyline(painter, g.pts(&[(10.8, 10.6), (12.3, 9.6), (12.3, 14.4)]), thin);
            }
        }
        Icon::Volume0 | Icon::Volume1 | Icon::Volume2 | Icon::Volume3 | Icon::Mute | Icon::Speaker => {
            fill(&[(3.5, 9.3), (7.5, 9.3), (12.0, 5.5), (12.0, 18.5), (7.5, 14.7), (3.5, 14.7)]);
            let c = g.p(12.0, 12.0);
            let waves = match icon {
                Icon::Volume1 => 1,
                Icon::Volume2 => 2,
                Icon::Volume3 | Icon::Speaker => 3,
                _ => 0,
            };
            for i in 0..waves {
                let r = g.s(3.6 + 3.0 * i as f32);
                let span = 0.85 - 0.08 * i as f32;
                polyline(painter, arc(c, r, -span, span, 12), st);
            }
            if icon == Icon::Mute {
                seg(painter, g.p(15.5, 9.0), g.p(21.0, 15.0), st);
                seg(painter, g.p(21.0, 9.0), g.p(15.5, 15.0), st);
            }
        }
        Icon::Search => {
            painter.circle_stroke(g.p(10.5, 10.5), g.s(6.0), st);
            seg(painter, g.p(15.0, 15.0), g.p(20.0, 20.0), st);
        }
        Icon::Lyrics => {
            // microphone: capsule, cradle, stand
            painter.rect_stroke(
                Rect::from_min_max(g.p(9.0, 2.8), g.p(15.0, 13.6)),
                g.s(3.0),
                st,
                egui::StrokeKind::Middle,
            );
            polyline(painter, arc(g.p(12.0, 10.6), g.s(6.2), 0.0, PI, 16), st);
            seg(painter, g.p(12.0, 16.8), g.p(12.0, 20.6), st);
            seg(painter, g.p(8.6, 20.6), g.p(15.4, 20.6), st);
        }
        Icon::Queue => {
            seg(painter, g.p(4.0, 6.5), g.p(19.5, 6.5), st);
            seg(painter, g.p(4.0, 11.5), g.p(19.5, 11.5), st);
            seg(painter, g.p(4.0, 16.5), g.p(11.5, 16.5), st);
            fill(&[(14.5, 13.8), (20.5, 17.0), (14.5, 20.2)]);
        }
        Icon::Eq => {
            for (x, knob) in [(6.0, 14.5), (12.0, 8.0), (18.0, 16.0)] {
                seg(painter, g.p(x, 4.0), g.p(x, 20.0), Stroke::new(w * 0.8, color));
                painter.circle_filled(g.p(x, knob), g.s(2.6), color);
            }
        }
        Icon::Settings => {
            let c = g.p(12.0, 12.0);
            let teeth = 8;
            let mut outline = Vec::new();
            for i in 0..teeth {
                let a = i as f32 / teeth as f32 * TAU;
                let step = TAU / teeth as f32;
                for (da, r) in [(-0.30, 7.0), (-0.17, 9.3), (0.17, 9.3), (0.30, 7.0)] {
                    let aa = a + da * step * 1.6;
                    outline.push(c + vec2(aa.cos(), aa.sin()) * g.s(r));
                }
            }
            painter.add(Shape::closed_line(outline, st));
            painter.circle_stroke(c, g.s(3.0), st);
        }
        Icon::Library => {
            seg(painter, g.p(5.0, 4.5), g.p(5.0, 19.5), st);
            seg(painter, g.p(9.5, 4.5), g.p(9.5, 19.5), st);
            polyline(
                painter,
                g.pts(&[(13.3, 5.5), (16.8, 4.6), (20.3, 18.5), (16.8, 19.4), (13.3, 5.5)]),
                st,
            );
        }
        Icon::Album => {
            painter.circle_stroke(g.p(12.0, 12.0), g.s(8.5), st);
            painter.circle_filled(g.p(12.0, 12.0), g.s(2.2), color);
        }
        Icon::Artist => {
            painter.circle_stroke(g.p(12.0, 8.0), g.s(3.8), st);
            polyline(
                painter,
                cubic(g.p(4.5, 20.0), g.p(5.0, 13.0), g.p(19.0, 13.0), g.p(19.5, 20.0), 16),
                st,
            );
        }
        Icon::Playlist => {
            seg(painter, g.p(3.5, 6.0), g.p(15.5, 6.0), st);
            seg(painter, g.p(3.5, 11.0), g.p(15.5, 11.0), st);
            seg(painter, g.p(3.5, 16.0), g.p(9.5, 16.0), st);
            painter.circle_filled(g.p(15.5, 18.0), g.s(2.7), color);
            seg(painter, g.p(17.8, 18.0), g.p(17.8, 9.5), st);
            seg(painter, g.p(17.8, 9.5), g.p(21.0, 10.8), st);
        }
        Icon::Folder => {
            polyline(
                painter,
                g.pts(&[
                    (3.5, 18.5),
                    (3.5, 6.0),
                    (9.5, 6.0),
                    (11.5, 8.5),
                    (20.5, 8.5),
                    (20.5, 18.5),
                    (3.5, 18.5),
                ]),
                st,
            );
        }
        Icon::Plus => {
            seg(painter, g.p(12.0, 5.0), g.p(12.0, 19.0), st);
            seg(painter, g.p(5.0, 12.0), g.p(19.0, 12.0), st);
        }
        Icon::Minus => seg(painter, g.p(5.0, 12.0), g.p(19.0, 12.0), st),
        Icon::Close => {
            seg(painter, g.p(6.5, 6.5), g.p(17.5, 17.5), st);
            seg(painter, g.p(17.5, 6.5), g.p(6.5, 17.5), st);
        }
        Icon::More => {
            for x in [6.0, 12.0, 18.0] {
                painter.circle_filled(g.p(x, 12.0), g.s(1.7), color);
            }
        }
        Icon::MoreVertical => {
            for y in [6.0, 12.0, 18.0] {
                painter.circle_filled(g.p(12.0, y), g.s(1.7), color);
            }
        }
        Icon::Heart => {
            // Two open halves so the cusps get round caps instead of miter spikes.
            let o = heart_outline(&g);
            let half = o.len() / 2;
            let mut right: Vec<Pos2> = o[..=half].to_vec();
            let mut left: Vec<Pos2> = o[half..].to_vec();
            left.push(o[0]);
            right.dedup();
            left.dedup();
            painter.add(Shape::line(right, st));
            painter.add(Shape::line(left, st));
            painter.circle_filled(o[0], w * 0.5, color);
            painter.circle_filled(o[half], w * 0.5, color);
        }
        Icon::HeartFilled => {
            let o = heart_outline(&g);
            fill_fan(painter, g.p(12.0, 12.0), &o, color);
        }
        Icon::Note => {
            painter.circle_filled(g.p(7.3, 17.2), g.s(2.8), color);
            painter.circle_filled(g.p(17.0, 15.2), g.s(2.8), color);
            seg(painter, g.p(9.7, 17.2), g.p(9.7, 6.4), st);
            seg(painter, g.p(19.4, 15.2), g.p(19.4, 4.4), st);
            fill(&[(9.7, 5.6), (19.4, 3.6), (19.4, 6.8), (9.7, 8.8)]);
        }
        Icon::NowPlaying => {
            painter.circle_stroke(g.p(12.0, 12.0), g.s(8.8), st);
            fill(&[(10.0, 8.0), (16.2, 12.0), (10.0, 16.0)]);
        }
        Icon::ChevronLeft => polyline(painter, g.pts(&[(14.5, 5.5), (8.0, 12.0), (14.5, 18.5)]), st),
        Icon::ChevronRight => polyline(painter, g.pts(&[(9.5, 5.5), (16.0, 12.0), (9.5, 18.5)]), st),
        Icon::ChevronDown => polyline(painter, g.pts(&[(5.5, 9.0), (12.0, 15.5), (18.5, 9.0)]), st),
        Icon::ChevronUp => polyline(painter, g.pts(&[(5.5, 15.0), (12.0, 8.5), (18.5, 15.0)]), st),
        Icon::Check => polyline(painter, g.pts(&[(5.0, 12.5), (10.0, 17.5), (19.0, 7.0)]), st),
        Icon::Trash => {
            seg(painter, g.p(4.0, 6.5), g.p(20.0, 6.5), st);
            polyline(painter, g.pts(&[(9.0, 6.5), (9.5, 4.0), (14.5, 4.0), (15.0, 6.5)]), st);
            polyline(painter, g.pts(&[(6.0, 6.5), (7.0, 20.0), (17.0, 20.0), (18.0, 6.5)]), st);
            seg(painter, g.p(10.0, 10.0), g.p(10.3, 16.5), Stroke::new(w * 0.8, color));
            seg(painter, g.p(14.0, 10.0), g.p(13.7, 16.5), Stroke::new(w * 0.8, color));
        }
        Icon::Waveform => {
            // One sine period between two short stems (a signal passing through).
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = i as f32 / 24.0;
                    g.p(5.0 + 14.0 * t, 12.0 - 5.5 * (t * TAU).sin())
                })
                .collect();
            polyline(painter, pts, st);
            seg(painter, g.p(2.5, 12.0), g.p(5.0, 12.0), st);
            seg(painter, g.p(19.0, 12.0), g.p(21.5, 12.0), st);
        }
        Icon::Edit => {
            polyline(painter, g.pts(&[(5.0, 19.0), (5.6, 15.4), (15.8, 5.2), (18.8, 8.2), (8.6, 18.4), (5.0, 19.0)]), st);
            seg(painter, g.p(13.6, 7.4), g.p(16.6, 10.4), st);
            seg(painter, g.p(12.0, 20.0), g.p(19.5, 20.0), st);
        }
        Icon::Refresh => {
            polyline(painter, arc(g.p(12.0, 12.0), g.s(7.5), (-40.0f32).to_radians(), 255.0f32.to_radians(), 28), st);
            fill(&[(10.4, 1.2), (15.0, 4.7), (10.4, 8.2)]);
        }
        Icon::Grip => {
            for y in [7.0, 12.0, 17.0] {
                for x in [9.0, 15.0] {
                    painter.circle_filled(g.p(x, y), g.s(1.5), color);
                }
            }
        }
        Icon::SidebarToggle => {
            painter.rect_stroke(
                Rect::from_min_max(g.p(3.5, 4.5), g.p(20.5, 19.5)),
                g.s(2.5),
                st,
                egui::StrokeKind::Middle,
            );
            seg(painter, g.p(9.5, 4.5), g.p(9.5, 19.5), st);
        }
        Icon::Save => {
            polyline(painter, g.pts(&[(12.0, 3.5), (12.0, 14.5)]), st);
            polyline(painter, g.pts(&[(7.5, 10.5), (12.0, 15.0), (16.5, 10.5)]), st);
            polyline(painter, g.pts(&[(4.5, 15.5), (4.5, 20.0), (19.5, 20.0), (19.5, 15.5)]), st);
        }
        Icon::Info | Icon::Warning | Icon::Error | Icon::Success => {
            painter.circle_filled(g.p(12.0, 12.0), g.s(9.5), color);
            let ink = theme::on_color(color);
            let st2 = Stroke::new(w * 1.05, ink);
            match icon {
                Icon::Info => {
                    seg(painter, g.p(12.0, 11.0), g.p(12.0, 16.5), st2);
                    painter.circle_filled(g.p(12.0, 7.6), g.s(1.3), ink);
                }
                Icon::Warning | Icon::Error => {
                    seg(painter, g.p(12.0, 6.8), g.p(12.0, 12.8), st2);
                    painter.circle_filled(g.p(12.0, 16.6), g.s(1.3), ink);
                }
                _ => polyline(painter, g.pts(&[(7.5, 12.3), (10.8, 15.5), (16.5, 9.0)]), st2),
            }
        }
        Icon::TextSmaller | Icon::TextLarger => {
            // "A" glyph + small minus/plus
            let s = if icon == Icon::TextLarger { 1.0 } else { 0.8 };
            let base = 19.0;
            let top = base - 13.0 * s;
            let half = 5.5 * s;
            let cx = 10.0;
            polyline(painter, g.pts(&[(cx - half, base), (cx, top), (cx + half, base)]), st);
            seg(painter, g.p(cx - half * 0.55, base - 5.0 * s), g.p(cx + half * 0.55, base - 5.0 * s), st);
            seg(painter, g.p(17.0, 7.0), g.p(22.0, 7.0), Stroke::new(w * 0.85, color));
            if icon == Icon::TextLarger {
                seg(painter, g.p(19.5, 4.5), g.p(19.5, 9.5), Stroke::new(w * 0.85, color));
            }
        }
        Icon::Undo => {
            polyline(painter, arc(g.p(12.5, 13.0), g.s(6.5), -PI * 0.95, PI * 0.55, 24), st);
            polyline(painter, g.pts(&[(5.5, 6.5), (6.0, 12.0), (11.0, 10.5)]), st);
        }
        Icon::ArrowDownToLine => {
            seg(painter, g.p(12.0, 4.0), g.p(12.0, 15.0), st);
            polyline(painter, g.pts(&[(7.0, 10.5), (12.0, 15.5), (17.0, 10.5)]), st);
            seg(painter, g.p(5.5, 19.5), g.p(18.5, 19.5), st);
        }
    }
}

/// Square icon button with hover/press feedback, pointer cursor and tooltip.
///
/// `size` is the icon size; the hit target adds padding around it. `active` tints the icon
/// with the accent and draws a small dot beneath it.
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32, active: bool, tooltip: &str) -> Response {
    icon_button_ex(ui, icon, size, IconButtonStyle { active, ..Default::default() }, tooltip)
}

/// Extra knobs for [`icon_button_ex`].
#[derive(Clone, Copy, Debug)]
pub struct IconButtonStyle {
    pub active: bool,
    /// Show an accent dot under the icon when active.
    pub active_dot: bool,
    /// Override the resting icon color.
    pub color: Option<Color32>,
    /// Hover fill override (e.g. translucent white on artwork).
    pub hover_fill: Option<Color32>,
    pub enabled: bool,
    /// Extra padding around the icon (each side).
    pub padding: Option<f32>,
}

impl Default for IconButtonStyle {
    fn default() -> Self {
        Self {
            active: false,
            active_dot: true,
            color: None,
            hover_fill: None,
            enabled: true,
            padding: None,
        }
    }
}

pub fn icon_button_ex(
    ui: &mut Ui,
    icon: Icon,
    size: f32,
    style: IconButtonStyle,
    tooltip: &str,
) -> Response {
    let pal = theme::palette(ui);
    let pad = style.padding.unwrap_or((size * 0.4).clamp(5.0, 10.0));
    let side = size + pad * 2.0;
    let sense = if style.enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(side), sense);
    if style.enabled {
        resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    }
    if !tooltip.is_empty() {
        resp = resp.on_hover_text(tooltip);
    }
    if ui.is_rect_visible(rect) {
        paint_icon_button(ui, &resp, rect, icon, size, style, &pal);
    }
    resp
}

fn paint_icon_button(
    ui: &Ui,
    resp: &Response,
    rect: Rect,
    icon: Icon,
    size: f32,
    style: IconButtonStyle,
    pal: &Palette,
) {
    let hovered = style.enabled && resp.hovered();
    let pressed = style.enabled && resp.is_pointer_button_down_on();
    let t = ui.ctx().animate_bool_with_time(resp.id.with("hover"), hovered, 0.1);
    let painter = ui.painter();
    let hover_fill = style.hover_fill.unwrap_or(pal.hover);
    if t > 0.0 {
        let fill = if pressed { pal.pressed } else { hover_fill };
        painter.rect_filled(rect, rect.height() * 0.5, theme::with_alpha(fill, t));
    }
    let base = style.color.unwrap_or(pal.text_muted);
    let mut color = if style.active {
        pal.accent_fg
    } else if hovered {
        style.color.map(|c| theme::with_alpha(c, 1.0)).unwrap_or(pal.text)
    } else {
        base
    };
    if !style.enabled {
        color = theme::with_alpha(color, 0.35);
    }
    let scale = if pressed { 0.9 } else { 1.0 };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(size * scale));
    paint_icon(painter, icon_rect, icon, color);
    if style.active && style.active_dot {
        painter.circle_filled(
            pos2(rect.center().x, rect.bottom() - 2.5),
            1.6,
            pal.accent_fg,
        );
    }
}

/// Large round accent button used for play/pause.
pub fn play_button(ui: &mut Ui, playing: bool, diameter: f32, tooltip: &str) -> Response {
    let pal = theme::palette(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tooltip);
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let pressed = resp.is_pointer_button_down_on();
        let t = ui.ctx().animate_bool_with_time(resp.id.with("hover"), hovered, 0.1);
        let r = diameter * 0.5 * if pressed { 0.94 } else { 1.0 + 0.04 * t };
        // Neutral (text-colored) disc reads as "primary" without competing with the accent.
        ui.painter().circle_filled(rect.center(), r, pal.text);
        let ink = if pal.dark { pal.bg } else { pal.surface };
        let icon = if playing { Icon::Pause } else { Icon::Play };
        let icon_size = diameter * 0.46;
        let offset = if playing { 0.0 } else { diameter * 0.03 };
        paint_icon(
            ui.painter(),
            Rect::from_center_size(rect.center() + vec2(offset, 0.0), Vec2::splat(icon_size)),
            icon,
            ink,
        );
    }
    resp
}
