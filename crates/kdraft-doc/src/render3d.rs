//! Options of 3D views (`kind render3d`), drawn from `kicad-cli pcb
//! render`: `(render3d (side top) (rotate -45 0 45) (perspective yes)
//! (zoom 1) (quality basic) (background white) (dpi 300))` in the view;
//! everything optional.

use kdraft_sexpr::Value;

use crate::model::View;

pub const RENDER3D: &str = "render3d";

/// Default resolution, dots per inch of the view's size on paper.
pub const DEFAULT_DPI: u32 = 300;
/// Largest picture side, pixels.
pub const MAX_PX: u32 = 4096;

pub const SIDES: [&str; 6] = ["top", "bottom", "front", "back", "left", "right"];

#[derive(Debug, Clone, PartialEq)]
pub struct Render3d {
    pub side: String,
    /// Degrees around X, Y, Z (`None`: the side's own view).
    pub rotate: Option<[f64; 3]>,
    pub perspective: bool,
    pub zoom: u32,
    /// `basic` or `high`.
    pub quality: String,
    /// `transparent` or `white`.
    pub background: String,
    pub dpi: u32,
    /// kicad-cli appearance preset (`--preset`); `None`: its default.
    pub appearance: Option<String>,
    /// `--use-board-stackup-colors`.
    pub stackup_colors: bool,
    /// `--floor` (floor, shadows, post-processing).
    pub floor: bool,
    pub extra: Vec<Value>,
}

impl Default for Render3d {
    fn default() -> Self {
        Render3d {
            side: "top".into(),
            rotate: None,
            perspective: false,
            zoom: 1,
            quality: "basic".into(),
            background: "transparent".into(),
            dpi: DEFAULT_DPI,
            appearance: None,
            stackup_colors: false,
            floor: false,
            extra: Vec::new(),
        }
    }
}

/// The isometric preset's rotation (KiCad's documented example).
pub const ISOMETRIC: [f64; 3] = [-45.0, 0.0, 45.0];

impl Render3d {
    pub fn from_value(v: &Value) -> Render3d {
        let mut r = Render3d::default();
        for c in v.args() {
            let a0 = c.args().first().and_then(Value::atom_text);
            let ok = match (c.head(), a0) {
                (Some("side"), Some(s)) if SIDES.contains(&s) => {
                    r.side = s.into();
                    true
                }
                (Some("rotate"), _) => match (c.f64_at(1), c.f64_at(2), c.f64_at(3)) {
                    (Ok(x), Ok(y), Ok(z)) if x.is_finite() && y.is_finite() && z.is_finite() => {
                        r.rotate = Some([x, y, z]);
                        true
                    }
                    _ => false,
                },
                (Some("perspective"), Some(s)) => {
                    r.perspective = s == "yes";
                    true
                }
                (Some("zoom"), Some(s)) => match s.parse::<u32>() {
                    Ok(z) if z >= 1 => {
                        r.zoom = z;
                        true
                    }
                    _ => false,
                },
                (Some("quality"), Some(s)) if s == "basic" || s == "high" => {
                    r.quality = s.into();
                    true
                }
                (Some("background"), Some(s)) if s == "transparent" || s == "white" => {
                    r.background = s.into();
                    true
                }
                (Some("appearance"), Some(s)) if !s.is_empty() => {
                    r.appearance = Some(s.to_string());
                    true
                }
                (Some("stackup_colors"), Some(s)) => {
                    r.stackup_colors = s == "yes";
                    true
                }
                (Some("floor"), Some(s)) => {
                    r.floor = s == "yes";
                    true
                }
                (Some("dpi"), Some(s)) => match s.parse::<u32>() {
                    Ok(d) if (10..=1200).contains(&d) => {
                        r.dpi = d;
                        true
                    }
                    _ => false,
                },
                _ => false,
            };
            if !ok {
                r.extra.push(c.clone());
            }
        }
        r
    }

    /// Canonical node; only values that differ from the defaults.
    pub fn to_value(&self) -> Option<Value> {
        let d = Render3d::default();
        let num = |v: f64| Value::num(v).unwrap_or_else(|_| Value::int(0));
        let mut o = Vec::new();
        if self.side != d.side {
            o.push(Value::node("side", [Value::sym(&self.side)]));
        }
        if let Some([x, y, z]) = self.rotate {
            o.push(Value::node("rotate", [num(x), num(y), num(z)]));
        }
        if self.perspective {
            o.push(Value::node("perspective", [Value::sym("yes")]));
        }
        if self.zoom != d.zoom {
            o.push(Value::node("zoom", [Value::int(i64::from(self.zoom))]));
        }
        if self.quality != d.quality {
            o.push(Value::node("quality", [Value::sym(&self.quality)]));
        }
        if self.background != d.background {
            o.push(Value::node("background", [Value::sym(&self.background)]));
        }
        if self.dpi != d.dpi {
            o.push(Value::node("dpi", [Value::int(i64::from(self.dpi))]));
        }
        if let Some(a) = &self.appearance {
            o.push(Value::node("appearance", [Value::str(a)]));
        }
        if self.stackup_colors {
            o.push(Value::node("stackup_colors", [Value::sym("yes")]));
        }
        if self.floor {
            o.push(Value::node("floor", [Value::sym("yes")]));
        }
        o.extend(self.extra.iter().cloned());
        (!o.is_empty()).then(|| Value::node(RENDER3D, o))
    }

    /// Whether the picture can be to scale: an orthographic top or bottom
    /// view without rotation or floor.
    pub fn to_scale(&self) -> bool {
        (self.side == "top" || self.side == "bottom")
            && self.rotate.is_none()
            && !self.perspective
            && !self.floor
    }

    /// Picture size for a view `w` x `h` mm on paper: `dpi`, the longest
    /// side capped at [`MAX_PX`].
    pub fn pixels(&self, w: f64, h: f64) -> (u32, u32) {
        let px = |mm: f64| mm / 25.4 * f64::from(self.dpi);
        let (pw, ph) = (px(w.max(1.0)), px(h.max(1.0)));
        let k = (f64::from(MAX_PX) / pw.max(ph)).min(1.0);
        (
            ((pw * k).round() as u32).max(16),
            ((ph * k).round() as u32).max(16),
        )
    }
}

impl View {
    pub fn render3d(&self) -> Render3d {
        self.extra
            .iter()
            .find(|v| v.is(RENDER3D))
            .map(Render3d::from_value)
            .unwrap_or_default()
    }

    pub fn set_render3d(&mut self, r: &Render3d) {
        let at = self.extra.iter().position(|v| v.is(RENDER3D));
        self.extra.retain(|v| !v.is(RENDER3D));
        if let Some(v) = r.to_value() {
            let i = at.unwrap_or(self.extra.len()).min(self.extra.len());
            self.extra.insert(i, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_pixels() {
        let r = Render3d {
            side: "bottom".into(),
            rotate: Some(ISOMETRIC),
            perspective: true,
            zoom: 2,
            background: "white".into(),
            dpi: 150,
            appearance: Some("follow_pcb_editor".into()),
            stackup_colors: true,
            floor: true,
            ..Default::default()
        };
        let v = r.to_value().unwrap();
        assert_eq!(Render3d::from_value(&v), r);
        assert!(Render3d::default().to_value().is_none());
        let (w, h) = Render3d::default().pixels(254.0, 127.0);
        // 10 x 5 inch at 300 dpi = 3000 x 1500.
        assert_eq!((w, h), (3000, 1500));
        let (w, _) = Render3d::default().pixels(1000.0, 100.0);
        assert_eq!(w, MAX_PX);
    }
}
