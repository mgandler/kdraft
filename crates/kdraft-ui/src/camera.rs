//! Canvas camera: sheet millimetres to screen pixels (relative to the
//! canvas origin), zoom at the cursor, pan, fit; zoom buckets for texture
//! resolution.

use kdraft_board::geom::{BBox, Point};

/// Zoom limits, screen pixels per sheet mm.
pub const MIN_ZOOM: f64 = 0.1;
pub const MAX_ZOOM: f64 = 400.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Screen pixels (points) per sheet mm.
    pub zoom: f64,
    /// Screen position of the sheet origin.
    pub pan: Point,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            zoom: 2.0,
            pan: Point::new(20.0, 20.0),
        }
    }
}

impl Camera {
    pub fn to_screen(&self, p: Point) -> Point {
        Point::new(p.x * self.zoom + self.pan.x, p.y * self.zoom + self.pan.y)
    }

    pub fn to_sheet(&self, s: Point) -> Point {
        Point::new(
            (s.x - self.pan.x) / self.zoom,
            (s.y - self.pan.y) / self.zoom,
        )
    }

    pub fn box_to_screen(&self, b: &BBox) -> BBox {
        BBox {
            min: self.to_screen(b.min),
            max: self.to_screen(b.max),
        }
    }

    /// Multiplies the zoom by `factor`, keeping the sheet point under
    /// `cursor` (screen) in place. The zoom is clamped.
    pub fn zoom_at(&mut self, cursor: Point, factor: f64) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let anchor = self.to_sheet(cursor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = Point::new(
            cursor.x - anchor.x * self.zoom,
            cursor.y - anchor.y * self.zoom,
        );
    }

    pub fn pan_by(&mut self, dx: f64, dy: f64) {
        self.pan = Point::new(self.pan.x + dx, self.pan.y + dy);
    }

    /// Camera showing `rect` (sheet mm) centred in a `w` x `h` viewport with
    /// `margin` pixels around it.
    pub fn fit(rect: &BBox, w: f64, h: f64, margin: f64) -> Camera {
        let aw = (w - 2.0 * margin).max(1.0);
        let ah = (h - 2.0 * margin).max(1.0);
        let rw = rect.width().max(1e-3);
        let rh = rect.height().max(1e-3);
        let zoom = (aw / rw).min(ah / rh).clamp(MIN_ZOOM, MAX_ZOOM);
        let c = rect.center();
        Camera {
            zoom,
            pan: Point::new(w / 2.0 - c.x * zoom, h / 2.0 - c.y * zoom),
        }
    }
}

/// Zoom bucket: half-octave steps, so textures are re-rendered only when
/// the zoom changed by more than about 41 %.
pub fn zoom_bucket(zoom: f64) -> i32 {
    (zoom.max(1e-6).log2() * 2.0).round() as i32
}

/// The zoom a bucket stands for.
pub fn bucket_zoom(bucket: i32) -> f64 {
    2f64.powf(f64::from(bucket) / 2.0)
}

/// Largest texture side the canvas requests, pixels.
pub const MAX_TEXTURE_SIDE: f64 = 4096.0;

/// Raster resolution (pixels per mm) for a region at a zoom bucket and a
/// display scale (physical pixels per point), limited so neither side
/// exceeds [`MAX_TEXTURE_SIDE`].
pub fn texture_ppm(region: &BBox, bucket: i32, pixels_per_point: f64) -> f64 {
    texture_ppm_max(region, bucket, pixels_per_point, MAX_TEXTURE_SIDE)
}

/// [`texture_ppm`] with the largest side the GPU accepts (`max_side`,
/// capped at [`MAX_TEXTURE_SIDE`]); the image stays below it even after
/// rounding up.
pub fn texture_ppm_max(region: &BBox, bucket: i32, pixels_per_point: f64, max_side: f64) -> f64 {
    let want = bucket_zoom(bucket) * pixels_per_point.max(0.5);
    let side = region.width().max(region.height()).max(1e-3);
    let limit = (max_side.min(MAX_TEXTURE_SIDE) - 2.0).max(64.0);
    want.min(limit / side).max(0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_zoom_at_cursor() {
        let mut c = Camera {
            zoom: 3.0,
            pan: Point::new(10.0, -5.0),
        };
        let p = Point::new(12.5, 40.0);
        assert!(c.to_sheet(c.to_screen(p)).approx_eq(p, 1e-12));
        let cursor = Point::new(300.0, 200.0);
        let under = c.to_sheet(cursor);
        c.zoom_at(cursor, 1.7);
        assert!((c.zoom - 5.1).abs() < 1e-12);
        assert!(c.to_sheet(cursor).approx_eq(under, 1e-9));
        c.zoom_at(cursor, 1e9);
        assert_eq!(c.zoom, MAX_ZOOM);
        assert!(c.to_sheet(cursor).approx_eq(under, 1e-6));
        c.zoom_at(cursor, f64::NAN);
        assert_eq!(c.zoom, MAX_ZOOM);
    }

    #[test]
    fn fit_centres_and_contains() {
        let r = BBox {
            min: Point::new(0.0, 0.0),
            max: Point::new(420.0, 297.0),
        };
        let c = Camera::fit(&r, 1000.0, 600.0, 20.0);
        let s = c.box_to_screen(&r);
        assert!(s.min.x >= 19.99 && s.max.x <= 980.01 && s.min.y >= 19.99 && s.max.y <= 580.01);
        assert!((s.center().x - 500.0).abs() < 1e-9 && (s.center().y - 300.0).abs() < 1e-9);
        // Height-limited here: touches the top and bottom margins.
        assert!((s.min.y - 20.0).abs() < 1e-9);
    }

    #[test]
    fn buckets_and_texture_resolution() {
        for z in [0.3, 1.0, 2.0, 3.7, 50.0] {
            let b = zoom_bucket(z);
            let back = bucket_zoom(b);
            assert!(back / z < 1.2 && z / back < 1.2, "{z} {back}");
        }
        assert_eq!(zoom_bucket(2.0), 2);
        assert_eq!(zoom_bucket(2.1), 2);
        let r = BBox {
            min: Point::ORIGIN,
            max: Point::new(400.0, 100.0),
        };
        assert!((texture_ppm(&r, zoom_bucket(4.0), 1.0) - 4.0).abs() < 1e-9);
        // Capped by the texture size (less a margin for rounding up):
        // 4094 / 400; with a GPU limit of 2048: 2046 / 400.
        assert!((texture_ppm(&r, zoom_bucket(100.0), 2.0) - 10.235).abs() < 1e-9);
        assert!((texture_ppm_max(&r, zoom_bucket(100.0), 2.0, 2048.0) - 5.115).abs() < 1e-9);
    }
}
