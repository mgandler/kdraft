//! Raster back end on tiny-skia: display lists to RGBA pixmaps at a given
//! resolution. Fills (non-zero, even-odd), strokes with caps, joins and
//! dashes, group transforms and clip rectangles (as masks, so rotated
//! clips work), and text as glyph outlines of the bundled font. The GUI
//! canvas draws view and page content with it; `kdraft export --format
//! png` writes its output.

use std::collections::HashMap;

use kdraft_board::geom::{Affine, BBox, Point};
use tiny_skia::{
    FillRule, LineCap as TCap, LineJoin as TJoin, Mask, Paint, PathBuilder, Pixmap,
    Stroke as TStroke, StrokeDash, Transform,
};

use crate::dl::{Cmd, Color, Item, LineCap, LineJoin, PageList, Path, PathItem, TextRun};
use crate::error::RenderError;
use crate::font::{self, Fonts};

/// Largest pixmap this back end allocates, in pixels (about 400 MB RGBA).
pub const MAX_PIXELS: u64 = 100_000_000;

/// Thinnest stroke drawn, mm (as in the PDF back end).
const MIN_STROKE: f64 = 0.01;

/// Thinnest stroke on the device, pixels.
pub const MIN_DEVICE_STROKE_PX: f64 = 1.0;

/// What to draw and where: the sheet region `region` (mm) at `px_per_mm`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub region: BBox,
    pub px_per_mm: f64,
    /// Fill colour before drawing; `None` leaves it transparent.
    pub background: Option<Color>,
}

impl Target {
    /// Pixel size of the target (at least 1 x 1).
    pub fn size_px(&self) -> (u32, u32) {
        let w = (self.region.width() * self.px_per_mm).ceil().max(1.0);
        let h = (self.region.height() * self.px_per_mm).ceil().max(1.0);
        (
            w.min(f64::from(u32::MAX)) as u32,
            h.min(f64::from(u32::MAX)) as u32,
        )
    }

    fn device(&self) -> Affine {
        Affine::translate(-self.region.min.x, -self.region.min.y)
            .then(&Affine::scale(self.px_per_mm, self.px_per_mm))
    }
}

fn tf(m: &Affine) -> Transform {
    Transform::from_row(
        m.a as f32, m.b as f32, m.c as f32, m.d as f32, m.e as f32, m.f as f32,
    )
}

fn tpath(p: &Path) -> Option<tiny_skia::Path> {
    let mut b = PathBuilder::new();
    for c in &p.cmds {
        match *c {
            Cmd::Move(a) => b.move_to(a.x as f32, a.y as f32),
            Cmd::Line(a) => b.line_to(a.x as f32, a.y as f32),
            Cmd::Cubic(c1, c2, e) => b.cubic_to(
                c1.x as f32,
                c1.y as f32,
                c2.x as f32,
                c2.y as f32,
                e.x as f32,
                e.y as f32,
            ),
            Cmd::Close => b.close(),
        }
    }
    b.finish()
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.r, c.g, c.b, c.a);
    p.anti_alias = true;
    p
}

struct Glyphs<'f> {
    fonts: &'f Fonts,
    cache: HashMap<(bool, u16), Option<tiny_skia::Path>>,
}

struct Outline(PathBuilder);

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

impl Glyphs<'_> {
    fn path(&mut self, bold: bool, g: u16) -> Option<&tiny_skia::Path> {
        let fonts = self.fonts;
        self.cache
            .entry((bold, g))
            .or_insert_with(|| {
                let mut o = Outline(PathBuilder::new());
                if fonts.face(bold).outline(g, &mut o) {
                    o.0.finish()
                } else {
                    None
                }
            })
            .as_ref()
    }
}

struct Ctx<'f> {
    pix: Pixmap,
    glyphs: Glyphs<'f>,
}

fn draw_path(ctx: &mut Ctx<'_>, p: &PathItem, m: &Affine, mask: Option<&Mask>) {
    let Some(path) = tpath(&p.path) else { return };
    let t = tf(m);
    if let Some(f) = p.fill {
        let rule = if f.even_odd {
            FillRule::EvenOdd
        } else {
            FillRule::Winding
        };
        ctx.pix.fill_path(&path, &paint(f.color), rule, t, mask);
    }
    if let Some(st) = &p.stroke {
        // Strokes thinner than one device pixel are drawn one pixel wide
        // (as PDF viewers do); otherwise anti-aliasing fades them out at
        // low zoom.
        let scale = m.mean_scale();
        let min_w = if scale > 0.0 {
            MIN_DEVICE_STROKE_PX / scale
        } else {
            0.0
        };
        let stroke = TStroke {
            width: st.width.max(MIN_STROKE).max(min_w) as f32,
            miter_limit: 10.0,
            line_cap: match st.cap {
                LineCap::Butt => TCap::Butt,
                LineCap::Round => TCap::Round,
                LineCap::Square => TCap::Square,
            },
            line_join: match st.join {
                LineJoin::Miter => TJoin::Miter,
                LineJoin::Round => TJoin::Round,
                LineJoin::Bevel => TJoin::Bevel,
            },
            dash: st
                .dash
                .as_ref()
                .and_then(|d| StrokeDash::new(d.iter().map(|x| *x as f32).collect(), 0.0)),
        };
        ctx.pix
            .stroke_path(&path, &paint(st.color), &stroke, t, mask);
    }
}

fn draw_text(ctx: &mut Ctx<'_>, t: &TextRun, m: &Affine, mask: Option<&Mask>) {
    if t.invisible || t.text.is_empty() {
        return;
    }
    let fonts = ctx.glyphs.fonts;
    let placed = font::place(t, fonts);
    let face = fonts.face(t.bold);
    let k = placed.em / face.units_per_em();
    let base = placed.transform.then(m);
    let mut x = placed.origin.x;
    let pnt = paint(t.color);
    for (g, adv, _) in face.glyphs(&t.text) {
        let glyph_m = Affine::scale(k, -k)
            .then(&Affine::translate(x, placed.origin.y))
            .then(&base);
        if let Some(path) = ctx.glyphs.path(t.bold, g).cloned() {
            ctx.pix
                .fill_path(&path, &pnt, FillRule::Winding, tf(&glyph_m), mask);
        }
        x += adv * placed.em;
    }
}

fn clip_mask(ctx: &Ctx<'_>, clip: &BBox, m: &Affine, parent: Option<&Mask>) -> Option<Mask> {
    let rect = tpath(&Path::rect(clip))?;
    let mut mask = match parent {
        Some(p) => p.clone(),
        None => {
            let mut k = Mask::new(ctx.pix.width(), ctx.pix.height())?;
            k.fill_path(&rect, FillRule::Winding, true, tf(m));
            return Some(k);
        }
    };
    mask.intersect_path(&rect, FillRule::Winding, true, tf(m));
    Some(mask)
}

fn draw_image(ctx: &mut Ctx<'_>, i: &crate::dl::ImageItem, m: &Affine, mask: Option<&Mask>) {
    let Ok(img) = Pixmap::decode_png(&i.png) else {
        return;
    };
    let (w, h) = (i.rect.width(), i.rect.height());
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    // Image pixels to the rectangle, then to the device.
    let to_rect = Affine::new(
        w / f64::from(img.width()),
        0.0,
        0.0,
        h / f64::from(img.height()),
        i.rect.min.x,
        i.rect.min.y,
    )
    .then(m);
    let pp = tiny_skia::PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear,
        ..Default::default()
    };
    ctx.pix
        .draw_pixmap(0, 0, img.as_ref(), &pp, tf(&to_rect), mask);
}

fn draw_items(ctx: &mut Ctx<'_>, items: &[Item], m: &Affine, mask: Option<&Mask>) {
    for it in items {
        match it {
            Item::Path(p) => draw_path(ctx, p, m, mask),
            Item::Text(t) => draw_text(ctx, t, m, mask),
            Item::Image(i) => draw_image(ctx, i, m, mask),
            Item::Group(g) => {
                let own = g.clip.as_ref().and_then(|c| clip_mask(ctx, c, m, mask));
                let inner = g.transform.then(m);
                let mk = if g.clip.is_some() { own.as_ref() } else { mask };
                // A clip that could not be built (degenerate) hides nothing
                // rather than everything.
                draw_items(ctx, &g.items, &inner, mk.or(mask));
            }
        }
    }
}

/// Renders `items` (sheet mm) into a new pixmap covering `target.region`.
pub fn render(items: &[Item], target: &Target, fonts: &Fonts) -> Result<Pixmap, RenderError> {
    if !(target.px_per_mm.is_finite() && target.px_per_mm > 0.0) {
        return Err(RenderError::Raster(format!(
            "invalid resolution {} px/mm",
            target.px_per_mm
        )));
    }
    let (w, h) = target.size_px();
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(RenderError::Raster(format!(
            "{w} x {h} px is larger than the limit of {MAX_PIXELS} pixels"
        )));
    }
    let mut pix = Pixmap::new(w, h)
        .ok_or_else(|| RenderError::Raster(format!("cannot allocate {w} x {h} px")))?;
    if let Some(c) = target.background {
        pix.fill(tiny_skia::Color::from_rgba8(c.r, c.g, c.b, 255));
    }
    let mut ctx = Ctx {
        pix,
        glyphs: Glyphs {
            fonts,
            cache: HashMap::new(),
        },
    };
    draw_items(&mut ctx, items, &target.device(), None);
    Ok(ctx.pix)
}

/// A whole page on white.
pub fn page_pixmap(page: &PageList, px_per_mm: f64, fonts: &Fonts) -> Result<Pixmap, RenderError> {
    render(
        &page.items,
        &Target {
            region: BBox {
                min: Point::ORIGIN,
                max: Point::new(page.width, page.height),
            },
            px_per_mm,
            background: Some(Color::WHITE),
        },
        fonts,
    )
}

/// A whole page as PNG bytes.
pub fn page_png(page: &PageList, px_per_mm: f64, fonts: &Fonts) -> Result<Vec<u8>, RenderError> {
    page_pixmap(page, px_per_mm, fonts)?
        .encode_png()
        .map_err(|e| RenderError::Raster(e.to_string()))
}

/// PNG bytes from premultiplied RGBA (as returned by [`render`]).
pub fn encode_png_rgba(rgba: Vec<u8>, width: u32, height: u32) -> Result<Vec<u8>, RenderError> {
    let size = tiny_skia::IntSize::from_wh(width, height)
        .ok_or_else(|| RenderError::Raster(format!("invalid size {width} x {height}")))?;
    let p = Pixmap::from_vec(rgba, size)
        .ok_or_else(|| RenderError::Raster("pixel buffer does not match the size".into()))?;
    p.encode_png()
        .map_err(|e| RenderError::Raster(e.to_string()))
}

/// Premultiplied RGBA bytes of a pixmap (what egui's
/// `ColorImage::from_rgba_premultiplied` takes).
pub fn rgba_premultiplied(p: &Pixmap) -> &[u8] {
    p.data()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dl::{Fill, Group, HAlign, Stroke, VAlign};

    fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let c = p.pixel(x, y).unwrap();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    fn target(w: f64, h: f64, ppm: f64) -> Target {
        Target {
            region: BBox {
                min: Point::ORIGIN,
                max: Point::new(w, h),
            },
            px_per_mm: ppm,
            background: Some(Color::WHITE),
        }
    }

    #[test]
    fn fill_rules_clip_and_stroke() {
        let f = Fonts::load().unwrap();
        // Two nested squares in one path: even-odd leaves a hole.
        let mut p = Path::rect(&BBox {
            min: Point::new(1.0, 1.0),
            max: Point::new(9.0, 9.0),
        });
        p.cmds.extend(
            Path::rect(&BBox {
                min: Point::new(3.0, 3.0),
                max: Point::new(7.0, 7.0),
            })
            .cmds,
        );
        let item = |eo| {
            Item::Path(PathItem {
                path: p.clone(),
                stroke: None,
                fill: Some(Fill {
                    color: Color::BLACK,
                    even_odd: eo,
                }),
            })
        };
        let nz = render(&[item(false)], &target(10.0, 10.0, 10.0), &f).unwrap();
        let eo = render(&[item(true)], &target(10.0, 10.0, 10.0), &f).unwrap();
        assert_eq!(px(&nz, 50, 50), [0, 0, 0, 255]);
        assert_eq!(px(&eo, 50, 50), [255, 255, 255, 255]);
        assert_eq!(px(&eo, 20, 20), [0, 0, 0, 255]);
        // Clip to the left half.
        let g = Item::Group(Group {
            transform: Affine::IDENTITY,
            clip: Some(BBox {
                min: Point::ORIGIN,
                max: Point::new(5.0, 10.0),
            }),
            items: vec![item(false)],
        });
        let c = render(&[g], &target(10.0, 10.0, 10.0), &f).unwrap();
        assert_eq!(px(&c, 30, 50), [0, 0, 0, 255]);
        assert_eq!(px(&c, 70, 50), [255, 255, 255, 255]);
        // Dashed stroke: some pixels on the line are white.
        let mut st = Stroke::solid(0.5, Color::BLACK);
        st.dash = Some(vec![1.0, 1.0]);
        st.cap = LineCap::Butt;
        let l = Item::stroked(Path::line(Point::new(0.0, 5.0), Point::new(10.0, 5.0)), st);
        let d = render(&[l], &target(10.0, 10.0, 10.0), &f).unwrap();
        assert_eq!(px(&d, 5, 50), [0, 0, 0, 255]);
        assert_eq!(px(&d, 15, 50), [255, 255, 255, 255]);
    }

    #[test]
    fn text_is_drawn_and_region_offsets() {
        let f = Fonts::load().unwrap();
        let t = Item::Text(TextRun {
            text: "HHHH".into(),
            at: Point::new(12.0, 15.0),
            size: 5.0,
            h_align: HAlign::Left,
            v_align: VAlign::Baseline,
            rotation: 0.0,
            bold: true,
            italic: false,
            color: Color::BLACK,
            invisible: false,
        });
        let tg = Target {
            region: BBox {
                min: Point::new(10.0, 8.0),
                max: Point::new(40.0, 18.0),
            },
            px_per_mm: 10.0,
            background: None,
        };
        let p = render(std::slice::from_ref(&t), &tg, &f).unwrap();
        assert_eq!(tg.size_px(), (300, 100));
        let inked = p.pixels().iter().filter(|c| c.alpha() > 128).count();
        assert!(inked > 500, "{inked}");
        // Nothing above the cap height (y < 15 - 5 = 10 mm, i.e. row < 20).
        let above = (0..300).any(|x| (0..15).any(|y| px(&p, x, y)[3] > 0));
        assert!(!above);
        let mut inv = t.clone();
        if let Item::Text(r) = &mut inv {
            r.invisible = true;
        }
        let p = render(&[inv], &tg, &f).unwrap();
        assert!(p.pixels().iter().all(|c| c.alpha() == 0));
    }

    #[test]
    fn rejects_bad_targets() {
        let f = Fonts::load().unwrap();
        assert!(render(&[], &target(10.0, 10.0, 0.0), &f).is_err());
        assert!(render(&[], &target(1e6, 1e6, 100.0), &f).is_err());
    }
}
