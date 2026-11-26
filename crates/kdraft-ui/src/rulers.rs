//! Ruler and grid spacing for the canvas: tick steps in millimetres that
//! follow the zoom.

/// Steps a ruler can use, mm.
const STEPS: [f64; 13] = [
    0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0,
];

/// Major tick step (labelled) so that labels are at least `min_px`
/// apart at `zoom` pixels per mm, and the minor step (a fifth or a half
/// of it).
pub fn ticks(zoom: f64, min_px: f64) -> (f64, f64) {
    let major = STEPS
        .iter()
        .copied()
        .find(|s| s * zoom >= min_px)
        .unwrap_or(1000.0);
    let first = major.to_string().chars().next();
    let minor = if first == Some('2') {
        major / 2.0
    } else {
        major / 5.0
    };
    (major, minor)
}

/// Grid spacing to draw for a snap grid of `grid` mm: the grid itself, or
/// a multiple (2, 5, 10, ...) when its lines would be closer than
/// `min_px`. `None` for a non-positive grid.
pub fn grid_step(grid: f64, zoom: f64, min_px: f64) -> Option<f64> {
    if !(grid > 0.0 && grid.is_finite() && zoom > 0.0) {
        return None;
    }
    let mut g = grid;
    for k in [2.0, 2.5, 2.0].iter().cycle().take(40) {
        if g * zoom >= min_px {
            return Some(g);
        }
        g *= k;
    }
    None
}

/// Values of ticks of `step` inside `[lo, hi]`.
pub fn tick_values(lo: f64, hi: f64, step: f64) -> impl Iterator<Item = f64> {
    let n0 = (lo / step).ceil() as i64;
    let n1 = (hi / step).floor() as i64;
    let n1 = n1.min(n0 + 10_000);
    (n0..=n1).map(move |n| n as f64 * step)
}

/// Ruler label: no decimals for whole millimetres.
pub fn label(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_zoom() {
        assert_eq!(ticks(2.0, 50.0), (50.0, 10.0));
        assert_eq!(ticks(10.0, 50.0), (5.0, 1.0));
        assert_eq!(ticks(100.0, 50.0), (0.5, 0.1));
        assert_eq!(ticks(30.0, 50.0), (2.0, 1.0));
        assert_eq!(grid_step(0.5, 20.0, 6.0), Some(0.5));
        assert_eq!(grid_step(0.5, 2.0, 6.0), Some(5.0));
        assert_eq!(grid_step(0.0, 2.0, 6.0), None);
        let v: Vec<f64> = tick_values(-1.0, 21.0, 10.0).collect();
        assert_eq!(v, vec![0.0, 10.0, 20.0]);
        assert_eq!(label(10.0), "10");
        assert_eq!(label(0.5), "0.5");
    }
}
