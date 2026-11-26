//! Live vector drawing of display-list items on the canvas: annotations
//! (dimensions, callouts, free graphics) are drawn every frame from their
//! current geometry instead of cached textures, so they move with their
//! view during a drag. Geometry, stroke widths and text placement are the
//! renderer's (`kdraft_render`), so the canvas matches the export: text
//! uses the bundled IBM Plex Sans through egui at the em size the font
//! metrics give for the display list's cap height.

use eframe::egui::{self, Color32, Painter, Pos2, Shape, Stroke, Vec2};
use kdraft_board::geom::{Affine, Point};
use kdraft_render::dl::{Cmd, Item, Path, TextRun};
use kdraft_render::Fonts;

/// Largest text size drawn, points (egui rasterises glyphs into its atlas;
/// beyond this the canvas is zoomed far into one label).
pub const MAX_TEXT_PX: f32 = 600.0;
/// Thinnest stroke drawn, points (as the raster back end's one pixel).
pub const MIN_STROKE_PX: f32 = 1.0;

fn color(c: kdraft_render::dl::Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// Subpaths flattened to polylines (sheet coordinates) with closed flags.
pub fn flatten(p: &Path, m: &Affine) -> Vec<(Vec<Point>, bool)> {
    let mut out = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    let mut last = Point::ORIGIN;
    let flush = |cur: &mut Vec<Point>, out: &mut Vec<(Vec<Point>, bool)>, closed: bool| {
        if cur.len() >= 2 {
            out.push((std::mem::take(cur), closed));
        } else {
            cur.clear();
        }
    };
    for c in &p.cmds {
        match c {
            Cmd::Move(q) => {
                flush(&mut cur, &mut out, false);
                cur.push(m.apply(*q));
                last = *q;
            }
            Cmd::Line(q) => {
                if cur.is_empty() {
                    cur.push(m.apply(last));
                }
                cur.push(m.apply(*q));
                last = *q;
            }
            Cmd::Cubic(a, b, q) => {
                if cur.is_empty() {
                    cur.push(m.apply(last));
                }
                let p0 = last;
                for k in 1..=12 {
                    let t = f64::from(k) / 12.0;
                    let u = 1.0 - t;
                    let x = u * u * u * p0.x
                        + 3.0 * u * u * t * a.x
                        + 3.0 * u * t * t * b.x
                        + t * t * t * q.x;
                    let y = u * u * u * p0.y
                        + 3.0 * u * u * t * a.y
                        + 3.0 * u * t * t * b.y
                        + t * t * t * q.y;
                    cur.push(m.apply(Point::new(x, y)));
                }
                last = *q;
            }
            Cmd::Close => flush(&mut cur, &mut out, true),
        }
    }
    flush(&mut cur, &mut out, false);
    out
}

/// Draws display-list `items` (sheet mm, transformed by `m`) with
/// `to_screen` at `zoom` screen points per mm. `tint` replaces every
/// colour (selection highlight).
pub fn paint(
    painter: &Painter,
    items: &[Item],
    m: &Affine,
    to_screen: &dyn Fn(Point) -> Pos2,
    zoom: f64,
    fonts: &Fonts,
    tint: Option<Color32>,
) {
    for it in items {
        match it {
            Item::Path(p) => {
                let subs = flatten(&p.path, m);
                if let Some(f) = &p.fill {
                    let c = tint.unwrap_or(color(f.color));
                    for (pts, _) in subs.iter().filter(|(v, _)| v.len() >= 3) {
                        let v: Vec<Pos2> = pts.iter().map(|q| to_screen(*q)).collect();
                        fill_polygon(painter, v, c);
                    }
                }
                if let Some(s) = &p.stroke {
                    let w = ((s.width * zoom) as f32).max(MIN_STROKE_PX);
                    let st = Stroke::new(w, tint.unwrap_or(color(s.color)));
                    for (pts, closed) in &subs {
                        let mut v: Vec<Pos2> = pts.iter().map(|q| to_screen(*q)).collect();
                        if *closed {
                            if let Some(f) = v.first().copied() {
                                v.push(f);
                            }
                        }
                        match &s.dash {
                            Some(d) if d.len() >= 2 => {
                                // Dots (near-zero dashes) as round-ish
                                // squares of the line width.
                                let on: Vec<f32> = d
                                    .iter()
                                    .step_by(2)
                                    .map(|x| ((x * zoom) as f32).max(w))
                                    .collect();
                                let off: Vec<f32> = d
                                    .iter()
                                    .skip(1)
                                    .step_by(2)
                                    .map(|x| ((x * zoom) as f32).max(1.0))
                                    .collect();
                                painter
                                    .extend(Shape::dashed_line_with_offset(&v, st, &on, &off, 0.0));
                            }
                            _ => {
                                painter.add(Shape::line(v, st));
                            }
                        }
                    }
                }
            }
            Item::Text(t) => text(painter, t, m, to_screen, zoom, fonts, tint),
            // Pictures are drawn from item textures, not live; an outline
            // here marks where one goes.
            Item::Image(i) => {
                let r = egui::Rect::from_two_pos(
                    to_screen(m.apply(i.rect.min)),
                    to_screen(m.apply(i.rect.max)),
                );
                let c = tint.unwrap_or(Color32::from_gray(160));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, c), egui::StrokeKind::Middle);
            }
            Item::Group(g) => {
                let inner = g.transform.then(m);
                paint(painter, &g.items, &inner, to_screen, zoom, fonts, tint);
            }
        }
    }
}

/// Fills a simple polygon: convex ones directly, others by ear
/// clipping (egui fills only convex polygons).
fn fill_polygon(painter: &Painter, v: Vec<Pos2>, c: Color32) {
    if is_convex(&v) {
        painter.add(Shape::convex_polygon(v, c, Stroke::NONE));
        return;
    }
    let mut mesh = egui::Mesh::default();
    for p in &v {
        mesh.colored_vertex(*p, c);
    }
    for [a, b, d] in triangulate(&v) {
        mesh.add_triangle(a as u32, b as u32, d as u32);
    }
    painter.add(Shape::mesh(mesh));
}

fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

pub fn is_convex(v: &[Pos2]) -> bool {
    let n = v.len();
    let mut sign = 0.0f32;
    for i in 0..n {
        let c = cross(v[i], v[(i + 1) % n], v[(i + 2) % n]);
        if c.abs() > 1e-6 {
            if sign != 0.0 && c.signum() != sign {
                return false;
            }
            sign = c.signum();
        }
    }
    true
}

/// Ear-clipping triangulation of a simple polygon (indices into `v`).
pub fn triangulate(v: &[Pos2]) -> Vec<[usize; 3]> {
    let n = v.len();
    if n < 3 {
        return Vec::new();
    }
    let area: f32 = (0..n)
        .map(|i| cross(Pos2::ZERO, v[i], v[(i + 1) % n]))
        .sum();
    let ccw = area > 0.0;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (a, b, c) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let cr = cross(v[a], v[b], v[c]);
            if (cr > 0.0) != ccw || cr.abs() < 1e-9 {
                continue;
            }
            let inside = idx.iter().any(|&p| {
                if p == a || p == b || p == c {
                    return false;
                }
                let d1 = cross(v[a], v[b], v[p]);
                let d2 = cross(v[b], v[c], v[p]);
                let d3 = cross(v[c], v[a], v[p]);
                if ccw {
                    d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0
                } else {
                    d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0
                }
            });
            if !inside {
                out.push([a, b, c]);
                idx.remove(k);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}

fn text(
    painter: &Painter,
    t: &TextRun,
    m: &Affine,
    to_screen: &dyn Fn(Point) -> Pos2,
    zoom: f64,
    fonts: &Fonts,
    tint: Option<Color32>,
) {
    if t.invisible || t.text.is_empty() {
        return;
    }
    let placed = kdraft_render::font::place(t, fonts);
    let em_px = (placed.em * zoom * m.mean_scale()) as f32;
    if !(1.0..=MAX_TEXT_PX).contains(&em_px) {
        return;
    }
    let family = if t.bold {
        crate::theme::bold()
    } else {
        egui::FontFamily::Proportional
    };
    let c = tint.unwrap_or(color(t.color));
    let galley = painter.layout_no_wrap(t.text.clone(), egui::FontId::new(em_px, family), c);
    let Some(row) = galley.rows.first() else {
        return;
    };
    let baseline = row.pos.y
        + row
            .row
            .glyphs
            .first()
            .map_or(galley.size().y * 0.8, |g| g.pos.y);
    // Baseline start on the sheet, and the text direction on screen.
    let full = placed.transform.then(m);
    let b = to_screen(full.apply(placed.origin));
    let e = to_screen(full.apply(Point::new(placed.origin.x + 1.0, placed.origin.y)));
    let dir = (e - b).normalized();
    let angle = dir.y.atan2(dir.x);
    // Up on screen, perpendicular to the text direction.
    let up = Vec2::new(dir.y, -dir.x);
    let top_left = b + up * baseline;
    painter.add(egui::epaint::TextShape::new(top_left, galley, c).with_angle(angle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangulates_concave_polygons() {
        let l = [
            Pos2::new(0.0, 0.0),
            Pos2::new(4.0, 0.0),
            Pos2::new(4.0, 1.0),
            Pos2::new(1.0, 1.0),
            Pos2::new(1.0, 4.0),
            Pos2::new(0.0, 4.0),
        ];
        assert!(!is_convex(&l));
        let t = triangulate(&l);
        assert_eq!(t.len(), 4);
        let area: f32 = t
            .iter()
            .map(|[a, b, c]| cross(l[*a], l[*b], l[*c]).abs() / 2.0)
            .sum();
        assert!((area - 7.0).abs() < 1e-4, "{area}");
        assert!(is_convex(&l[..3]));
    }

    #[test]
    fn flattens_lines_curves_and_closed_paths() {
        let mut p = Path::new();
        p.cmds = vec![
            Cmd::Move(Point::new(0.0, 0.0)),
            Cmd::Line(Point::new(1.0, 0.0)),
            Cmd::Line(Point::new(1.0, 1.0)),
            Cmd::Close,
            Cmd::Move(Point::new(5.0, 5.0)),
            Cmd::Cubic(
                Point::new(6.0, 5.0),
                Point::new(7.0, 6.0),
                Point::new(7.0, 7.0),
            ),
        ];
        let f = flatten(&p, &Affine::translate(10.0, 0.0));
        assert_eq!(f.len(), 2);
        assert!(f[0].1 && !f[1].1);
        assert_eq!(f[0].0[0], Point::new(10.0, 0.0));
        assert_eq!(f[1].0.len(), 13);
        assert!(f[1]
            .0
            .last()
            .unwrap()
            .approx_eq(Point::new(17.0, 7.0), 1e-12));
    }
}
