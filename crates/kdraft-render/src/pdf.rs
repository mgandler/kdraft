//! PDF back end with krilla: one page per document page, vector paths,
//! real (searchable) text in the embedded bundled font, one bookmark per
//! page. No creation date is written; krilla derives the file ID from the
//! content, so equal input gives equal bytes.

use kdraft_board::geom::Affine;
use krilla::color::rgb;
use krilla::destination::XyzDestination;
use krilla::geom::{PathBuilder, Point as KPoint, Size, Transform};
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::outline::{Outline, OutlineNode};
use krilla::page::PageSettings;
use krilla::paint::{
    Fill as KFill, FillRule, LineCap as KCap, LineJoin as KJoin, Stroke as KStroke, StrokeDash,
};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::Document;

use crate::dl::{Cmd, Color, DocList, Item, LineCap, LineJoin, Path, PathItem, TextRun};
use crate::error::RenderError;
use crate::font::{self, Fonts};

pub const PT_PER_MM: f64 = 72.0 / 25.4;

fn kcolor(c: Color) -> rgb::Color {
    rgb::Color::new(c.r, c.g, c.b)
}

fn nf(v: f32) -> NormalizedF32 {
    NormalizedF32::new(v.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE)
}

fn ktransform(m: &Affine) -> Transform {
    Transform::from_row(
        m.a as f32, m.b as f32, m.c as f32, m.d as f32, m.e as f32, m.f as f32,
    )
}

fn kpath(p: &Path) -> Option<krilla::geom::Path> {
    let mut b = PathBuilder::new();
    let mut any = false;
    for c in &p.cmds {
        match *c {
            Cmd::Move(a) => b.move_to(a.x as f32, a.y as f32),
            Cmd::Line(a) => {
                b.line_to(a.x as f32, a.y as f32);
                any = true;
            }
            Cmd::Cubic(c1, c2, e) => {
                b.cubic_to(
                    c1.x as f32,
                    c1.y as f32,
                    c2.x as f32,
                    c2.y as f32,
                    e.x as f32,
                    e.y as f32,
                );
                any = true;
            }
            Cmd::Close => b.close(),
        }
    }
    if !any {
        return None;
    }
    b.finish()
}

struct Ctx {
    regular: Font,
    bold: Font,
}

fn draw_path(s: &mut Surface<'_>, p: &PathItem) {
    let Some(path) = kpath(&p.path) else { return };
    s.set_fill(p.fill.map(|f| KFill {
        paint: kcolor(f.color).into(),
        opacity: nf(f.color.alpha()),
        rule: if f.even_odd {
            FillRule::EvenOdd
        } else {
            FillRule::NonZero
        },
    }));
    s.set_stroke(p.stroke.as_ref().map(|st| KStroke {
        paint: kcolor(st.color).into(),
        width: st.width.max(0.01) as f32,
        miter_limit: 10.0,
        line_cap: match st.cap {
            LineCap::Butt => KCap::Butt,
            LineCap::Round => KCap::Round,
            LineCap::Square => KCap::Square,
        },
        line_join: match st.join {
            LineJoin::Miter => KJoin::Miter,
            LineJoin::Round => KJoin::Round,
            LineJoin::Bevel => KJoin::Bevel,
        },
        opacity: nf(st.color.alpha()),
        dash: st.dash.as_ref().map(|d| StrokeDash {
            array: d.iter().map(|x| *x as f32).collect(),
            offset: 0.0,
        }),
    }));
    if p.fill.is_none() && p.stroke.is_none() {
        return;
    }
    s.draw_path(&path);
}

fn draw_text(s: &mut Surface<'_>, t: &TextRun, fonts: &Fonts, ctx: &Ctx) {
    if t.text.is_empty() {
        return;
    }
    let p = font::place(t, fonts);
    let glyphs: Vec<KrillaGlyph> = fonts
        .face(t.bold)
        .glyphs(&t.text)
        .into_iter()
        .map(|(g, adv, range)| {
            KrillaGlyph::new(
                GlyphId::new(u32::from(g)),
                adv as f32,
                0.0,
                0.0,
                0.0,
                range,
                None,
            )
        })
        .collect();
    s.push_transform(&ktransform(&p.transform));
    s.set_stroke(None);
    s.set_fill(Some(KFill {
        paint: kcolor(t.color).into(),
        opacity: if t.invisible {
            NormalizedF32::ZERO
        } else {
            nf(t.color.alpha())
        },
        rule: FillRule::NonZero,
    }));
    let font = if t.bold {
        ctx.bold.clone()
    } else {
        ctx.regular.clone()
    };
    s.draw_glyphs(
        KPoint::from_xy(p.origin.x as f32, p.origin.y as f32),
        &glyphs,
        font,
        &t.text,
        p.em as f32,
        false,
    );
    s.pop();
}

fn draw_image(s: &mut Surface<'_>, i: &crate::dl::ImageItem) {
    let data: krilla::Data = i.png.as_ref().clone().into();
    let Ok(img) = krilla::image::Image::from_png(data, true) else {
        return;
    };
    let (w, h) = (i.rect.width() as f32, i.rect.height() as f32);
    let Some(size) = krilla::geom::Size::from_wh(w, h) else {
        return;
    };
    s.push_transform(&Transform::from_translate(
        i.rect.min.x as f32,
        i.rect.min.y as f32,
    ));
    s.draw_image(img, size);
    s.pop();
}

fn draw_items(s: &mut Surface<'_>, items: &[Item], fonts: &Fonts, ctx: &Ctx) {
    for it in items {
        match it {
            Item::Path(p) => draw_path(s, p),
            Item::Text(t) => draw_text(s, t, fonts, ctx),
            Item::Image(i) => draw_image(s, i),
            Item::Group(g) => {
                let mut pops = 0;
                if let Some(c) = g.clip {
                    if let Some(cp) = kpath(&Path::rect(&c)) {
                        s.push_clip_path(&cp, &FillRule::NonZero);
                        pops += 1;
                    }
                }
                if g.transform != Affine::IDENTITY {
                    s.push_transform(&ktransform(&g.transform));
                    pops += 1;
                }
                draw_items(s, &g.items, fonts, ctx);
                for _ in 0..pops {
                    s.pop();
                }
            }
        }
    }
}

/// Writes the document as PDF bytes.
pub fn write_pdf(doc: &DocList, fonts: &Fonts) -> Result<Vec<u8>, RenderError> {
    let regular = Font::new(font::REGULAR.into(), 0)
        .ok_or_else(|| RenderError::Font("krilla cannot read the regular font".into()))?;
    let bold = Font::new(font::BOLD.into(), 0)
        .ok_or_else(|| RenderError::Font("krilla cannot read the bold font".into()))?;
    let ctx = Ctx { regular, bold };
    let mut document = Document::new();
    let mut outline = Outline::new();
    for (i, page) in doc.pages.iter().enumerate() {
        let size = Size::from_wh(
            (page.width * PT_PER_MM) as f32,
            (page.height * PT_PER_MM) as f32,
        )
        .ok_or_else(|| {
            RenderError::Pdf(format!(
                "invalid page size {} x {}",
                page.width, page.height
            ))
        })?;
        let mut p = document.start_page_with(PageSettings::new(size));
        let mut s = p.surface();
        s.push_transform(&Transform::from_scale(PT_PER_MM as f32, PT_PER_MM as f32));
        draw_items(&mut s, &page.items, fonts, &ctx);
        s.pop();
        s.finish();
        p.finish();
        outline.push_child(OutlineNode::new(
            page.title.clone(),
            XyzDestination::new(i, KPoint::from_xy(0.0, 0.0)),
        ));
    }
    document.set_outline(outline);
    document.set_metadata(
        Metadata::new()
            .title(doc.title.clone())
            .creator(format!("kdraft {}", env!("CARGO_PKG_VERSION"))),
    );
    document
        .finish()
        .map_err(|e| RenderError::Pdf(e.to_string()))
}
