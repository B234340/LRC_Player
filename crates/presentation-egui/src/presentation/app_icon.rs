//! The application icon: drawn with egui (same visual language as the sidebar logo), rendered
//! to PNG/ICO by an ignored test (`render_app_icon`) into `assets/icon/`, and embedded for the
//! window / taskbar icon at runtime ([`icon_data`]).

use eframe::egui::{self, epaint::Vertex, pos2, vec2, Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke};

/// The 256 px icon, embedded for the window icon.
const ICON_256: &[u8] = include_bytes!("../../../../assets/icon/icon_256.png");

/// Window / taskbar icon (decoded from the embedded PNG).
pub fn icon_data() -> Option<egui::IconData> {
    let img = image::load_from_memory(ICON_256).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

const TOP: Color32 = Color32::from_rgb(0x3A, 0xD6, 0xC4);
const BOTTOM: Color32 = Color32::from_rgb(0x0B, 0x84, 0x78);

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Rounded-rectangle outline (corner arcs with `seg` points each).
fn rounded_outline(r: Rect, radius: f32, seg: usize) -> Vec<Pos2> {
    let mut pts = Vec::with_capacity(seg * 4 + 4);
    let corners = [
        (pos2(r.right() - radius, r.top() + radius), -90.0f32),
        (pos2(r.right() - radius, r.bottom() - radius), 0.0),
        (pos2(r.left() + radius, r.bottom() - radius), 90.0),
        (pos2(r.left() + radius, r.top() + radius), 180.0),
    ];
    for (c, start) in corners {
        for i in 0..=seg {
            let a = (start + 90.0 * i as f32 / seg as f32).to_radians();
            pts.push(c + vec2(a.cos(), a.sin()) * radius);
        }
    }
    pts
}

/// Paints the icon into `rect` (square). `detailed`: lyric lines next to the note (large
/// sizes); otherwise just a bigger note (16–32 px stay legible).
pub fn paint_app_icon(painter: &Painter, rect: Rect, detailed: bool) {
    let s = rect.width();
    let tile = rect.shrink(s * 0.055);
    let outline = rounded_outline(tile, tile.width() * 0.23, 24);
    // Vertical gradient fill (fan from the centre).
    let mut mesh = Mesh::default();
    let color_at = |p: Pos2| lerp(TOP, BOTTOM, ((p.y - tile.top()) / tile.height()).clamp(0.0, 1.0));
    mesh.vertices.push(Vertex { pos: tile.center(), uv: egui::epaint::WHITE_UV, color: color_at(tile.center()) });
    for p in &outline {
        mesh.vertices.push(Vertex { pos: *p, uv: egui::epaint::WHITE_UV, color: color_at(*p) });
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.indices.extend_from_slice(&[0, 1 + i, 1 + (i + 1) % n]);
    }
    painter.add(Shape::mesh(mesh));
    // Soft highlight along the top edge.
    painter.add(Shape::closed_line(outline, Stroke::new(s * 0.006, Color32::from_white_alpha(40))));

    let white = Color32::WHITE;
    let u = tile.width();
    let at = |x: f32, y: f32| pos2(tile.left() + x * u, tile.top() + y * u);
    // Note geometry (two beamed eighth notes), in tile units.
    let (scale, dx, dy) = if detailed { (0.95, -0.035, 0.0) } else { (1.32, 0.07, -0.01) };
    let p = |x: f32, y: f32| {
        let (cx, cy) = (0.36, 0.5);
        at(cx + (x - cx) * scale + dx, cy + (y - cy) * scale + dy)
    };
    let head_r = 0.085 * u * scale;
    let stem = Stroke::new(0.05 * u * scale, white);
    let heads = [(0.235, 0.70), (0.475, 0.645)];
    for (x, y) in heads {
        painter.add(egui::epaint::EllipseShape::filled(p(x, y), vec2(head_r * 1.2, head_r * 0.95), white));
    }
    let stem_x = [0.235 + 0.075, 0.475 + 0.075];
    painter.line_segment([p(stem_x[0], 0.69), p(stem_x[0], 0.30)], stem);
    painter.line_segment([p(stem_x[1], 0.635), p(stem_x[1], 0.245)], stem);
    let beam = vec![p(stem_x[0] - 0.025, 0.275), p(stem_x[1] + 0.025, 0.22), p(stem_x[1] + 0.025, 0.33), p(stem_x[0] - 0.025, 0.385)];
    painter.add(Shape::convex_polygon(beam, white, Stroke::NONE));

    if detailed {
        // Lyric lines; the first one half "sung" (karaoke fill).
        let bar = |x0: f32, x1: f32, y: f32, c: Color32| {
            let r = Rect::from_min_max(at(x0, y - 0.036), at(x1, y + 0.036));
            painter.rect_filled(r, r.height() / 2.0, c);
        };
        let faint = Color32::from_white_alpha(115);
        bar(0.62, 0.87, 0.42, faint);
        bar(0.62, 0.76, 0.42, white);
        bar(0.62, 0.82, 0.575, faint);
        bar(0.62, 0.87, 0.73, faint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icon_decodes() {
        let icon = icon_data().expect("embedded icon");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        // Transparent corner, opaque centre.
        assert_eq!(icon.rgba[3], 0);
        let c = (128 * 256 + 128) * 4;
        assert_eq!(icon.rgba[c + 3], 255);
    }

    /// Renders the icon PNGs and the Windows .ico into `assets/icon/`:
    /// `cargo test -p lrc_player --lib app_icon::tests::render_app_icon -- --ignored`.
    #[test]
    #[ignore]
    fn render_app_icon() {
        use egui_kittest::Harness;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icon");
        std::fs::create_dir_all(&dir).unwrap();
        let render = |size: u32, detailed: bool| -> image::RgbaImage {
            // Drawn at 4x and downsampled for smooth edges.
            let big = size * 4;
            let mut h = Harness::builder()
                .with_size(vec2(big as f32, big as f32))
                .with_pixels_per_point(1.0)
                .build_ui(move |ui| {
                    let painter = ui.ctx().layer_painter(egui::LayerId::background());
                    paint_app_icon(&painter, Rect::from_min_size(Pos2::ZERO, vec2(big as f32, big as f32)), detailed);
                });
            h.run();
            let img = h.render().expect("render (needs a wgpu adapter)");
            // (kittest uses another `image` version: go through raw bytes.)
            let (w, ht) = (img.width(), img.height());
            let img = image::RgbaImage::from_raw(w, ht, img.into_raw()).unwrap();
            image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3)
        };
        let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256, 512];
        let mut ico_entries = Vec::new();
        for &size in &sizes {
            let img = render(size, size >= 40);
            let path = dir.join(format!("icon_{size}.png"));
            img.save(&path).unwrap();
            if size <= 256 {
                ico_entries.push((size, std::fs::read(&path).unwrap()));
            }
        }
        // ICO with PNG-compressed entries (Vista+).
        let mut ico = Vec::new();
        ico.extend_from_slice(&[0, 0, 1, 0]);
        ico.extend_from_slice(&(ico_entries.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * ico_entries.len() as u32;
        for (size, data) in &ico_entries {
            let dim = if *size >= 256 { 0 } else { *size as u8 };
            ico.extend_from_slice(&[dim, dim, 0, 0, 1, 0, 32, 0]);
            ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
            ico.extend_from_slice(&offset.to_le_bytes());
            offset += data.len() as u32;
        }
        for (_, data) in &ico_entries {
            ico.extend_from_slice(data);
        }
        std::fs::write(dir.join("lrc_player.ico"), ico).unwrap();
    }
}
