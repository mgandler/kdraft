//! The bundled font (IBM Plex Sans, OFL; see `assets/fonts/NOTICE`) and
//! text measurement. Both back ends position glyphs with these advances, so
//! layout and output agree.

use crate::error::RenderError;

pub const REGULAR: &[u8] = include_bytes!("../../../assets/fonts/IBMPlexSans-Regular.ttf");
pub const BOLD: &[u8] = include_bytes!("../../../assets/fonts/IBMPlexSans-Bold.ttf");
pub const FAMILY: &str = "IBM Plex Sans";

/// Slant applied for italic text (synthetic oblique), as tan(angle).
pub const ITALIC_SLANT: f64 = 0.2;

pub struct Face {
    face: ttf_parser::Face<'static>,
    upem: f64,
    cap: f64,
}

impl Face {
    fn load(data: &'static [u8]) -> Result<Face, RenderError> {
        let face =
            ttf_parser::Face::parse(data, 0).map_err(|e| RenderError::Font(e.to_string()))?;
        let upem = f64::from(face.units_per_em());
        let cap = face
            .capital_height()
            .map(f64::from)
            .filter(|c| *c > 0.0)
            .unwrap_or(0.7 * upem);
        Ok(Face { face, upem, cap })
    }

    /// Glyph id and advance (in em) per character, with byte ranges.
    pub fn glyphs(&self, text: &str) -> Vec<(u16, f64, std::ops::Range<usize>)> {
        let notdef = ttf_parser::GlyphId(0);
        text.char_indices()
            .map(|(i, c)| {
                let g = self.face.glyph_index(c).unwrap_or(notdef);
                let adv = self.face.glyph_hor_advance(g).map_or(0.0, f64::from) / self.upem;
                (g.0, adv, i..i + c.len_utf8())
            })
            .collect()
    }

    /// Font units per em.
    pub fn units_per_em(&self) -> f64 {
        self.upem
    }

    /// Glyph outline in font units (Y up); false if the glyph has none.
    pub fn outline(&self, glyph: u16, b: &mut dyn ttf_parser::OutlineBuilder) -> bool {
        self.face
            .outline_glyph(ttf_parser::GlyphId(glyph), b)
            .is_some()
    }

    /// Em size giving a cap height of `cap_mm`.
    pub fn em_for_cap(&self, cap_mm: f64) -> f64 {
        cap_mm * self.upem / self.cap
    }

    /// Width of `text` at cap height `cap_mm`, mm.
    pub fn width(&self, text: &str, cap_mm: f64) -> f64 {
        let em = self.em_for_cap(cap_mm);
        self.glyphs(text).iter().map(|g| g.1).sum::<f64>() * em
    }
}

pub struct Fonts {
    pub regular: Face,
    pub bold: Face,
}

impl Fonts {
    pub fn load() -> Result<Fonts, RenderError> {
        Ok(Fonts {
            regular: Face::load(REGULAR)?,
            bold: Face::load(BOLD)?,
        })
    }

    pub fn face(&self, bold: bool) -> &Face {
        if bold {
            &self.bold
        } else {
            &self.regular
        }
    }

    pub fn width(&self, text: &str, cap_mm: f64, bold: bool) -> f64 {
        self.face(bold).width(text, cap_mm)
    }
}

/// Where and how a text run is drawn: glyphs start at `origin` (on the
/// baseline) in a local frame mapped to the page by `transform`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub transform: kdraft_board::geom::Affine,
    pub origin: kdraft_board::geom::Point,
    /// Font size (em), mm.
    pub em: f64,
    pub width: f64,
}

pub fn place(t: &crate::dl::TextRun, fonts: &Fonts) -> Placed {
    use crate::dl::{HAlign, VAlign};
    use kdraft_board::geom::{Affine, Point};
    let face = fonts.face(t.bold);
    let width = face.width(&t.text, t.size);
    let dx = match t.h_align {
        HAlign::Left => 0.0,
        HAlign::Center => -width / 2.0,
        HAlign::Right => -width,
    };
    let dy = match t.v_align {
        VAlign::Top => t.size,
        VAlign::Center => t.size / 2.0,
        VAlign::Baseline => 0.0,
    };
    let mut m = Affine::IDENTITY;
    if t.italic {
        // Lean right: points above the baseline (negative y) move to +x.
        // Skew about the baseline so it stays in place.
        m = Affine::translate(0.0, -dy)
            .then(&Affine::new(1.0, 0.0, -ITALIC_SLANT, 1.0, 0.0, 0.0))
            .then(&Affine::translate(0.0, dy));
    }
    m = m
        .then(&Affine::rotate_deg(-t.rotation))
        .then(&Affine::translate(t.at.x, t.at.y));
    Placed {
        transform: m,
        origin: Point::new(dx, dy),
        em: face.em_for_cap(t.size),
        width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_proportionally() {
        let f = Fonts::load().unwrap();
        let w1 = f.width("ABC", 1.0, false);
        let w2 = f.width("ABC", 2.0, false);
        assert!(w1 > 0.5 && (w2 - 2.0 * w1).abs() < 1e-9, "{w1} {w2}");
        assert!(f.width("iii", 1.0, false) < f.width("WWW", 1.0, false));
        assert!(f.width("W", 1.0, true) >= f.width("W", 1.0, false));
        // Cap height ratio is plausible for a sans serif.
        let em = f.regular.em_for_cap(1.0);
        assert!(em > 1.2 && em < 1.6, "{em}");
        assert_eq!(f.width("", 1.0, false), 0.0);
    }

    #[test]
    fn bundled_fonts_have_drawing_symbols() {
        let f = Fonts::load().unwrap();
        let text = "µ°±ØøÜüÄäÖößé×÷≤≥≈∅Ω€²³½";
        for bold in [false, true] {
            let face = f.face(bold);
            let missing: Vec<char> = text
                .chars()
                .filter(|c| {
                    face.glyphs(&c.to_string())
                        .first()
                        .is_none_or(|(g, _, _)| *g == 0)
                })
                .collect();
            // IBM Plex Sans has no U+2205 EMPTY SET (use Ø for diameters).
            assert_eq!(missing, vec!['∅'], "bold={bold}");
        }
    }
}
