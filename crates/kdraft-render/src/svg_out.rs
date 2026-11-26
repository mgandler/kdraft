//! SVG back end: one SVG per page, deterministic output.
//!
//! Text is written as `<text>` in the bundled font's family with glyph
//! positions computed from that font; viewers without the font substitute
//! another and widths can differ slightly.

use std::fmt::Write as _;

use kdraft_board::geom::Affine;

use crate::dl::{Cmd, Color, Item, LineCap, LineJoin, PageList, Path, PathItem, TextRun};
use crate::font::{self, Fonts};

/// Shortest decimal with at most 4 fraction digits.
pub fn num(v: f64) -> String {
    let v = if v.abs() < 5e-5 { 0.0 } else { v };
    let mut s = format!("{v:.4}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

fn opacity(out: &mut String, attr: &str, c: Color) {
    if !c.is_opaque() {
        let _ = write!(out, " {attr}=\"{}\"", num(f64::from(c.alpha())));
    }
}

/// Standard base64 with padding.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= c.len() {
                o.push(T[((n >> (18 - 6 * k)) & 63) as usize] as char);
            } else {
                o.push('=');
            }
        }
    }
    o
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

pub fn path_data(p: &Path) -> String {
    let mut d = String::new();
    for c in &p.cmds {
        match c {
            Cmd::Move(a) => {
                let _ = write!(d, "M{} {}", num(a.x), num(a.y));
            }
            Cmd::Line(a) => {
                let _ = write!(d, "L{} {}", num(a.x), num(a.y));
            }
            Cmd::Cubic(a, b, e) => {
                let _ = write!(
                    d,
                    "C{} {} {} {} {} {}",
                    num(a.x),
                    num(a.y),
                    num(b.x),
                    num(b.y),
                    num(e.x),
                    num(e.y)
                );
            }
            Cmd::Close => d.push('Z'),
        }
    }
    d
}

fn matrix(m: &Affine) -> String {
    format!(
        "matrix({} {} {} {} {} {})",
        num(m.a),
        num(m.b),
        num(m.c),
        num(m.d),
        num(m.e),
        num(m.f)
    )
}

struct Writer<'a> {
    out: String,
    clip_id: usize,
    fonts: &'a Fonts,
}

impl Writer<'_> {
    fn path(&mut self, p: &PathItem) {
        if p.path.is_empty() {
            return;
        }
        let _ = write!(self.out, "<path d=\"{}\"", path_data(&p.path));
        match &p.fill {
            Some(f) => {
                let _ = write!(self.out, " fill=\"{}\"", hex(f.color));
                opacity(&mut self.out, "fill-opacity", f.color);
                if f.even_odd {
                    self.out.push_str(" fill-rule=\"evenodd\"");
                }
            }
            None => self.out.push_str(" fill=\"none\""),
        }
        if let Some(s) = &p.stroke {
            let _ = write!(
                self.out,
                " stroke=\"{}\" stroke-width=\"{}\"",
                hex(s.color),
                num(s.width)
            );
            opacity(&mut self.out, "stroke-opacity", s.color);
            let cap = match s.cap {
                LineCap::Butt => None,
                LineCap::Round => Some("round"),
                LineCap::Square => Some("square"),
            };
            if let Some(c) = cap {
                let _ = write!(self.out, " stroke-linecap=\"{c}\"");
            }
            let join = match s.join {
                LineJoin::Miter => None,
                LineJoin::Round => Some("round"),
                LineJoin::Bevel => Some("bevel"),
            };
            if let Some(j) = join {
                let _ = write!(self.out, " stroke-linejoin=\"{j}\"");
            }
            if let Some(d) = &s.dash {
                let v: Vec<String> = d.iter().map(|x| num(*x)).collect();
                let _ = write!(self.out, " stroke-dasharray=\"{}\"", v.join(" "));
            }
        }
        self.out.push_str("/>\n");
    }

    fn text(&mut self, t: &TextRun) {
        if t.text.is_empty() {
            return;
        }
        let p = font::place(t, self.fonts);
        let _ = write!(
            self.out,
            "<text x=\"{}\" y=\"{}\" font-family=\"{}, sans-serif\" font-size=\"{}\"",
            num(p.origin.x),
            num(p.origin.y),
            font::FAMILY,
            num(p.em)
        );
        if t.bold {
            self.out.push_str(" font-weight=\"bold\"");
        }
        let _ = write!(self.out, " fill=\"{}\"", hex(t.color));
        if t.invisible {
            self.out.push_str(" fill-opacity=\"0\"");
        } else {
            opacity(&mut self.out, "fill-opacity", t.color);
        }
        let _ = writeln!(
            self.out,
            " xml:space=\"preserve\" transform=\"{}\">{}</text>",
            matrix(&p.transform),
            esc(&t.text)
        );
    }

    fn items(&mut self, items: &[Item]) {
        for it in items {
            match it {
                Item::Path(p) => self.path(p),
                Item::Text(t) => self.text(t),
                Item::Image(i) => {
                    let _ = writeln!(
                        self.out,
                        "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{}\"/>",
                        num(i.rect.min.x),
                        num(i.rect.min.y),
                        num(i.rect.width()),
                        num(i.rect.height()),
                        base64(&i.png)
                    );
                }
                Item::Group(g) => {
                    let mut closes = 0;
                    if let Some(c) = g.clip {
                        self.clip_id += 1;
                        let id = self.clip_id;
                        let _ = writeln!(
                            self.out,
                            "<clipPath id=\"clip{id}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>",
                            num(c.min.x),
                            num(c.min.y),
                            num(c.width()),
                            num(c.height())
                        );
                        let _ = writeln!(self.out, "<g clip-path=\"url(#clip{id})\">");
                        closes += 1;
                    }
                    if g.transform != Affine::IDENTITY {
                        let _ = writeln!(self.out, "<g transform=\"{}\">", matrix(&g.transform));
                        closes += 1;
                    }
                    self.items(&g.items);
                    for _ in 0..closes {
                        self.out.push_str("</g>\n");
                    }
                }
            }
        }
    }
}

/// One page as an SVG document in millimetres.
pub fn page_svg(page: &PageList, fonts: &Fonts) -> String {
    let mut w = Writer {
        out: String::new(),
        clip_id: 0,
        fonts,
    };
    let _ = writeln!(
        w.out,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"0 0 {} {}\">",
        num(page.width),
        num(page.height),
        num(page.width),
        num(page.height)
    );
    let _ = writeln!(w.out, "<title>{}</title>", esc(&page.title));
    let _ = writeln!(
        w.out,
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#FFFFFF\"/>",
        num(page.width),
        num(page.height)
    );
    w.items(&page.items);
    w.out.push_str("</svg>\n");
    w.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dl::{Group, HAlign, Stroke, VAlign};
    use kdraft_board::geom::{BBox, Point};

    #[test]
    fn numbers_and_escaping() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(num(0.12345), "0.1235");
        assert_eq!(num(-2.5), "-2.5");
        assert_eq!(esc("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
    }

    #[test]
    fn page_output_parses_back_with_same_geometry() {
        let fonts = Fonts::load().unwrap();
        let page = PageList {
            width: 100.0,
            height: 50.0,
            title: "T & t".into(),
            items: vec![
                Item::Group(Group {
                    transform: Affine::translate(10.0, 0.0),
                    clip: Some(BBox {
                        min: Point::new(0.0, 0.0),
                        max: Point::new(50.0, 50.0),
                    }),
                    items: vec![Item::stroked(
                        Path::line(Point::new(0.0, 10.0), Point::new(20.0, 10.0)),
                        Stroke::solid(0.5, Color::BLACK),
                    )],
                }),
                Item::Text(TextRun {
                    text: "Hello <1>".into(),
                    at: Point::new(50.0, 25.0),
                    size: 2.5,
                    h_align: HAlign::Center,
                    v_align: VAlign::Center,
                    rotation: 90.0,
                    bold: true,
                    italic: true,
                    color: Color::BLACK,
                    invisible: false,
                }),
            ],
        };
        let s = page_svg(&page, &fonts);
        assert_eq!(s, page_svg(&page, &fonts));
        let doc = kdraft_plot::svg::parse(&s).unwrap();
        assert_eq!(doc.text_elements, 1);
        let b = doc.content_bbox().unwrap();
        assert!(b.min.approx_eq(Point::new(10.0, 10.0), 1e-9), "{b:?}");
        assert!(b.max.approx_eq(Point::new(30.0, 10.0), 1e-9), "{b:?}");
        assert!((doc.mm_per_user_unit() - 1.0).abs() < 1e-12);
        assert!(s.contains("Hello &lt;1&gt;"));
    }
}
