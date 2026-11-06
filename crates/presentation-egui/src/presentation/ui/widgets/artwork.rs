//! Rounded artwork image with a tinted-gradient + music-note placeholder.

use std::hash::{Hash, Hasher};

use eframe::egui::{
    self, epaint::Mesh, pos2, vec2, Color32, Painter, Rect, Response, Sense,
    Shape, TextureHandle, TextureId, Ui, Vec2,
};

use crate::presentation::ui::{icons, theme};

/// A texture plus its pixel size (needed for cover-fit cropping).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tex {
    pub id: TextureId,
    pub size: Vec2,
}

impl From<&TextureHandle> for Tex {
    fn from(t: &TextureHandle) -> Self {
        Tex { id: t.id(), size: t.size_vec2() }
    }
}

/// UV rect that crops a texture of `tex_size` to fill `target` (object-fit: cover).
pub fn cover_uv(tex_size: Vec2, target: Vec2) -> Rect {
    if tex_size.x <= 0.0 || tex_size.y <= 0.0 || target.x <= 0.0 || target.y <= 0.0 {
        return Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    }
    let tex_aspect = tex_size.x / tex_size.y;
    let target_aspect = target.x / target.y;
    if tex_aspect > target_aspect {
        let w = target_aspect / tex_aspect;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = tex_aspect / target_aspect;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

/// Allocate and paint a square artwork of `size`.
pub fn artwork(ui: &mut Ui, tex: Option<Tex>, size: f32, rounding: u8, seed: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if ui.is_rect_visible(rect) {
        paint_artwork(ui, rect, tex, rounding, seed);
    }
    resp
}

/// Paint artwork (or the placeholder) into `rect`.
pub fn paint_artwork(ui: &Ui, rect: Rect, tex: Option<Tex>, rounding: u8, seed: &str) {
    match tex {
        Some(t) => {
            egui::Image::new((t.id, rect.size()))
                .uv(cover_uv(t.size, rect.size()))
                .corner_radius(rounding)
                .paint_at(ui, rect);
        }
        None => paint_placeholder(ui.painter(), rect, rounding, seed, theme::palette(ui).dark),
    }
}

/// Stable hue (0..1) for a string.
pub fn seed_hue(seed: &str) -> f32 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut h);
    (h.finish() % 360) as f32 / 360.0
}

/// Gradient placeholder with a note icon.
pub fn paint_placeholder(painter: &Painter, rect: Rect, rounding: u8, seed: &str, dark: bool) {
    let hue = seed_hue(seed);
    let (s, v) = if dark { (0.42, 0.42) } else { (0.30, 0.86) };
    let c0: Color32 = egui::ecolor::Hsva::new(hue, s, v, 1.0).into();
    let c1: Color32 = egui::ecolor::Hsva::new((hue + 0.09) % 1.0, s * 0.9, v * if dark { 0.55 } else { 0.78 }, 1.0).into();
    gradient_rounded_rect(painter, rect, rounding as f32, c0, c1);
    let icon_size = rect.width().min(rect.height()) * 0.38;
    let ink = if dark { Color32::from_white_alpha(120) } else { Color32::from_white_alpha(200) };
    icons::paint_icon(painter, Rect::from_center_size(rect.center(), Vec2::splat(icon_size)), icons::Icon::Note, ink);
}

/// Diagonal (top-left → bottom-right) gradient in a rounded rect, as a triangle fan.
pub fn gradient_rounded_rect(painter: &Painter, rect: Rect, radius: f32, c0: Color32, c1: Color32) {
    let diag = (rect.width() + rect.height()).max(1.0);
    gradient_rounded_rect_with(painter, rect, radius, |p| {
        let t = ((p.x - rect.left()) + (p.y - rect.top())) / diag;
        theme::blend(c0, c1, t)
    });
}

/// Rounded rect filled with a per-vertex color function (triangle fan, convex outline).
pub fn gradient_rounded_rect_with(painter: &Painter, rect: Rect, radius: f32, color_at: impl Fn(egui::Pos2) -> Color32) {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let mut outline = Vec::with_capacity(4 * 9);
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -90.0f32),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), 90.0),
        (pos2(rect.left() + r, rect.top() + r), 180.0),
    ];
    let steps = if r > 0.5 { 8 } else { 0 };
    for (c, start) in corners {
        for i in 0..=steps {
            let a = (start + 90.0 * i as f32 / steps.max(1) as f32).to_radians();
            outline.push(c + vec2(a.cos(), a.sin()) * r);
        }
    }
    // Interior vertices along the vertical axis keep vertical gradients linear.
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.center(), color_at(rect.center()));
    for p in &outline {
        mesh.colored_vertex(*p, color_at(*p));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
}

/// Soft drop shadow for elevated artwork.
pub fn paint_shadow(painter: &Painter, rect: Rect, rounding: u8, blur: u8, alpha: u8) {
    let shadow = egui::epaint::Shadow {
        offset: [0, (blur / 3) as i8],
        blur,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
    };
    painter.add(shadow.as_shape(rect, rounding));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_uv_crops_center() {
        let uv = cover_uv(vec2(200.0, 100.0), vec2(50.0, 50.0));
        assert!((uv.min.x - 0.25).abs() < 1e-6 && (uv.max.x - 0.75).abs() < 1e-6);
        assert_eq!(uv.min.y, 0.0);
        let uv = cover_uv(vec2(100.0, 100.0), vec2(50.0, 50.0));
        assert_eq!(uv, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
    }
}
