//! 3D view pictures: trimming `kicad-cli pcb render` output to its
//! content, calibrating the scale of orthographic top and bottom views,
//! and placing the picture on the sheet.
//!
//! kicad-cli leaves a transparent margin around the board at zoom 1. The
//! content box is the box of pixels with alpha above a threshold. For an
//! orthographic top or bottom view the camera looks at the centre of the
//! board (UNVERIFIED beyond the renders seen), so the board outline box is
//! a rectangle centred on the picture with the board's aspect ratio; its
//! size follows from the content box: on each side the content reaches at
//! least as far as the outline (parts may overhang, the board cannot be
//! smaller than its outline), so the outline half-size is the smallest
//! ratio of content half-extent to board half-size over the four sides.
//! This is exact as soon as one side has no overhanging part. No margin
//! factor of kicad-cli is assumed.

use std::sync::Arc;

use kdraft_board::geom::{BBox, Point};

/// A 3D view's picture or its progress.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Being rendered: the placeholder text.
    Pending(String),
    Ready(Arc<Picture>),
    Failed(String),
}

/// Pictures by key.
pub type Renders = std::collections::HashMap<String, State>;

/// The sheet rectangle a 3D view occupies: the placed picture when there
/// is one, else the view frame (placeholder).
pub fn sheet_rect(state: Option<&State>, frame: &BBox) -> BBox {
    match state {
        Some(State::Ready(p)) => place(p, frame),
        _ => *frame,
    }
}

/// Pixels with alpha at or below this count as background.
pub const ALPHA_THRESHOLD: u8 = 8;
/// Padding around the trimmed content on paper, mm.
pub const PADDING_MM: f64 = 1.0;

/// A trimmed 3D picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// The cropped PNG (content plus padding).
    pub png: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// The board outline box in the cropped picture's pixels (top and
    /// bottom orthographic views): the view's scale applies to it.
    pub outline_px: Option<[f64; 4]>,
    /// Pixels of the content per pixel of the picture's longest side
    /// before cropping (how much of the render the board filled).
    pub fill: f64,
    /// How the picture was made, for the inspector.
    pub note: String,
}

/// Box `[x0, y0, x1, y1)` of pixels with alpha above the threshold in
/// premultiplied or straight RGBA data; `None` when empty.
pub fn content_box(rgba: &[u8], w: u32, h: u32) -> Option<[u32; 4]> {
    let (w, h) = (w as usize, h as usize);
    if rgba.len() < w * h * 4 {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        let row = &rgba[y * w * 4..(y + 1) * w * 4];
        let mut any = false;
        for x in 0..w {
            if row[x * 4 + 3] > ALPHA_THRESHOLD {
                any = true;
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
            }
        }
        if any {
            y0 = y0.min(y);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > x0 && y1 > y0).then_some([x0 as u32, y0 as u32, x1 as u32, y1 as u32])
}

/// Pixels per board mm of an orthographic top/bottom render whose content
/// box is `c` in a `w` x `h` picture of a board `bw` x `bh` mm (the
/// outline box centred on the picture).
pub fn calibrate(c: [u32; 4], w: u32, h: u32, bw: f64, bh: f64) -> Option<f64> {
    if bw <= 0.0 || bh <= 0.0 {
        return None;
    }
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let sides = [
        (cx - f64::from(c[0])) / (bw / 2.0),
        (f64::from(c[2]) - cx) / (bw / 2.0),
        (cy - f64::from(c[1])) / (bh / 2.0),
        (f64::from(c[3]) - cy) / (bh / 2.0),
    ];
    let s = sides.iter().copied().fold(f64::INFINITY, f64::min);
    (s.is_finite() && s > 0.0).then_some(s)
}

/// Content half-extent from the picture centre as a fraction of the half
/// picture size, the larger of both axes (1 = touches an edge).
pub fn centred_fill(c: [u32; 4], w: u32, h: u32) -> f64 {
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let fx = (cx - f64::from(c[0])).max(f64::from(c[2]) - cx) / cx.max(1.0);
    let fy = (cy - f64::from(c[1])).max(f64::from(c[3]) - cy) / cy.max(1.0);
    fx.max(fy)
}

/// Whether the content touches the picture's border (cut off).
pub fn touches_border(c: [u32; 4], w: u32, h: u32) -> bool {
    c[0] == 0 || c[1] == 0 || c[2] >= w || c[3] >= h
}

/// Decodes a PNG to straight RGBA.
pub fn decode(png: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let p = tiny_skia::Pixmap::decode_png(png).ok()?;
    let (w, h) = (p.width(), p.height());
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for px in p.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some((out, w, h))
}

/// Crops straight RGBA to `[x0, y0, x1, y1)` and encodes it as PNG.
pub fn crop_png(rgba: &[u8], w: u32, b: [u32; 4]) -> Option<Vec<u8>> {
    let (cw, ch) = (b[2].saturating_sub(b[0]), b[3].saturating_sub(b[1]));
    if cw == 0 || ch == 0 {
        return None;
    }
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for y in b[1]..b[3] {
        let s = ((y * w + b[0]) * 4) as usize;
        out.extend_from_slice(rgba.get(s..s + (cw * 4) as usize)?);
    }
    crate::raster::encode_png_rgba(out, cw, ch).ok()
}

/// Trims a render: content box plus `pad_px`, cropped and re-encoded. For
/// `board` (`Some((w, h))` mm, orthographic top/bottom) the outline box is
/// calibrated and given in the cropped pixels.
pub fn trim(png: &[u8], pad_px: f64, board: Option<(f64, f64)>) -> Option<Picture> {
    let (rgba, w, h) = decode(png)?;
    let c = content_box(&rgba, w, h)?;
    let p = pad_px.max(0.0).round() as u32;
    let b = [
        c[0].saturating_sub(p),
        c[1].saturating_sub(p),
        (c[2] + p).min(w),
        (c[3] + p).min(h),
    ];
    let out = crop_png(&rgba, w, b)?;
    let outline_px = board.and_then(|(bw, bh)| {
        let s = calibrate(c, w, h, bw, bh)?;
        let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
        Some([
            cx - s * bw / 2.0 - f64::from(b[0]),
            cy - s * bh / 2.0 - f64::from(b[1]),
            cx + s * bw / 2.0 - f64::from(b[0]),
            cy + s * bh / 2.0 - f64::from(b[1]),
        ])
    });
    let longest = f64::from(w.max(h));
    Some(Picture {
        png: Arc::new(out),
        width: b[2] - b[0],
        height: b[3] - b[1],
        outline_px,
        fill: f64::from((c[2] - c[0]).max(c[3] - c[1])) / longest,
        note: String::new(),
    })
}

/// Where a picture goes on the sheet. `frame` is the view's frame (the
/// board box at the view's scale, centred on the view); with an outline
/// box the picture is placed so that box lands exactly on the frame (the
/// scale is true); otherwise it is fitted to the frame's width, centred
/// (not to scale). Returns the picture's sheet rectangle.
pub fn place(pic: &Picture, frame: &BBox) -> BBox {
    let (pw, ph) = (f64::from(pic.width), f64::from(pic.height));
    match pic.outline_px {
        Some(o) if o[2] > o[0] && o[3] > o[1] => {
            let kx = frame.width() / (o[2] - o[0]);
            let ky = frame.height() / (o[3] - o[1]);
            let min = Point::new(frame.min.x - o[0] * kx, frame.min.y - o[1] * ky);
            BBox {
                min,
                max: Point::new(min.x + pw * kx, min.y + ph * ky),
            }
        }
        _ => {
            let c = frame.center();
            let w = frame.width();
            let h = w * ph / pw.max(1.0);
            BBox {
                min: Point::new(c.x - w / 2.0, c.y - h / 2.0),
                max: Point::new(c.x + w / 2.0, c.y + h / 2.0),
            }
        }
    }
}

/// A stand-in `kicad-cli pcb render` picture of a board for tests and
/// screenshots: at zoom 1 the board rectangle (the picture's aspect) fills
/// 55 % of each side, centred, with a part overhanging the right edge by
/// 6 % of the board width; `zoom` scales it about the centre.
pub fn fake_board_render(w: u32, h: u32, zoom: f64) -> Vec<u8> {
    let (fw, fh) = (f64::from(w), f64::from(h));
    let (hw, hh) = (0.55 * zoom * fw / 2.0, 0.55 * zoom * fh / 2.0);
    let (cx, cy) = (fw / 2.0, fh / 2.0);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let (dx, dy) = (px - cx, py - cy);
            let board = dx.abs() <= hw && dy.abs() <= hh;
            // Overhanging connector on the right, middle third.
            let conn = dx > hw * 0.7 && dx <= hw * 1.12 && dy.abs() <= hh / 3.0;
            let chip = dx.abs() <= hw * 0.3 && dy.abs() <= hh * 0.35;
            let px4: [u8; 4] = if conn {
                [60, 60, 66, 255]
            } else if chip {
                [28, 28, 32, 255]
            } else if board {
                let t = (dy / hh.max(1.0) + 1.0) / 2.0;
                [24, (100.0 + 30.0 * t) as u8, 52, 255]
            } else {
                [0, 0, 0, 0]
            };
            let i = ((y * w + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&px4);
        }
    }
    crate::raster::encode_png_rgba(rgba, w, h).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_calibrates_a_top_view() {
        // A 50 x 30 mm board rendered 500 x 300 px at zoom 1: the board is
        // 275 x 165 px, so 5.5 px/mm; a part overhangs on the right.
        let png = fake_board_render(500, 300, 1.0);
        let (rgba, w, h) = decode(&png).unwrap();
        let c = content_box(&rgba, w, h).unwrap();
        assert_eq!([c[0], c[1], c[3]], [112, 67, 233]);
        assert!(c[2] > 387, "overhang included: {c:?}");
        let s = calibrate(c, w, h, 50.0, 30.0).unwrap();
        assert!((s - 5.5).abs() < 0.05, "{s}");
        let pic = trim(&png, 4.0, Some((50.0, 30.0))).unwrap();
        assert_eq!(pic.width, c[2] - c[0] + 8);
        let o = pic.outline_px.unwrap();
        assert!(
            (o[0] - 4.0).abs() < 1.0 && (o[2] - o[0] - 275.0).abs() < 2.0,
            "{o:?}"
        );
        // On the sheet the outline box lands exactly on the frame.
        let frame = BBox {
            min: Point::new(100.0, 50.0),
            max: Point::new(150.0, 80.0),
        };
        let r = place(&pic, &frame);
        let k = r.width() / f64::from(pic.width);
        assert!((r.min.x + o[0] * k - 100.0).abs() < 1e-9);
        assert!((r.min.x + o[2] * k - 150.0).abs() < 1e-9);
        assert!((r.min.y + o[3] * (r.height() / f64::from(pic.height)) - 80.0).abs() < 1e-9);
        // Zoom 1.5: the same calibration in pixels per mm times 1.5.
        let png2 = fake_board_render(500, 300, 1.5);
        let (rgba2, w2, h2) = decode(&png2).unwrap();
        let c2 = content_box(&rgba2, w2, h2).unwrap();
        let s2 = calibrate(c2, w2, h2, 50.0, 30.0).unwrap();
        assert!((s2 / s - 1.5).abs() < 0.02, "{s} {s2}");
        assert!(!touches_border(c2, w2, h2));
        assert!(touches_border(
            content_box(
                &decode(&fake_board_render(500, 300, 2.0)).unwrap().0,
                500,
                300
            )
            .unwrap(),
            500,
            300
        ));
    }

    #[test]
    fn empty_picture_has_no_content() {
        let png = crate::raster::encode_png_rgba(vec![0; 16 * 16 * 4], 16, 16).unwrap();
        assert!(trim(&png, 2.0, None).is_none());
    }
}
