//! Map projections, implemented in core as pure testable math.
//!
//! Both projections map (lat, lon) in radians to normalized map coordinates
//! (x, y) in [-1, 1] × [-1, 1], y up. Canvases scale x by `aspect()` to get
//! the correct shape. Inversion returns `None` for points outside the map
//! outline (e.g. the corners of a Robinson frame).

use std::f32::consts::{FRAC_PI_2, PI};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Projection {
    Equirectangular,
    Robinson,
}

impl Projection {
    pub const ALL: [Projection; 2] = [Projection::Equirectangular, Projection::Robinson];

    pub fn name(self) -> &'static str {
        match self {
            Projection::Equirectangular => "Equirectangular",
            Projection::Robinson => "Robinson",
        }
    }

    /// Width / height of the full map outline.
    pub fn aspect(self) -> f32 {
        match self {
            Projection::Equirectangular => 2.0,
            // (2 · 0.8487 · π) / (2 · 1.3523)
            Projection::Robinson => 0.8487 * PI / 1.3523,
        }
    }

    /// Project (lat, lon) in radians to normalized map coordinates.
    pub fn project(self, lat: f32, lon: f32) -> (f32, f32) {
        match self {
            Projection::Equirectangular => (lon / PI, lat / FRAC_PI_2),
            Projection::Robinson => {
                let (px, py) = robinson_interp(lat.abs());
                (px * lon / PI, py * lat.signum())
            }
        }
    }

    /// Invert normalized map coordinates back to (lat, lon) in radians.
    /// Returns `None` outside the map outline.
    pub fn invert(self, x: f32, y: f32) -> Option<(f32, f32)> {
        match self {
            Projection::Equirectangular => {
                if !(-1.0..=1.0).contains(&x) || !(-1.0..=1.0).contains(&y) {
                    return None;
                }
                Some((y * FRAC_PI_2, x * PI))
            }
            Projection::Robinson => {
                if !(-1.0..=1.0).contains(&y) {
                    return None;
                }
                let lat_abs = robinson_lat_from_y(y.abs());
                let (px, _) = robinson_interp(lat_abs);
                let lon = x * PI / px;
                // The forward X-scale is re-derived from the inverted latitude,
                // which can differ from the original by a float ulp — points
                // exactly on the antimeridian would otherwise be rejected.
                // Accept a hair beyond ±π and clamp.
                if lon.abs() > PI * 1.0001 {
                    return None;
                }
                Some((lat_abs * y.signum(), lon.clamp(-PI, PI)))
            }
        }
    }
}

/// Robinson's tabulated coefficients every 5° of latitude:
/// X = parallel length scale (1 at the equator), Y = distance of the parallel
/// from the equator (1 at the poles). Standard table (Robinson 1974; the
/// 0.8487 / 1.3523 scale factors are folded into `aspect()` and normalization).
const ROBINSON_TABLE: [(f32, f32); 19] = [
    (1.0000, 0.0000), // 0°
    (0.9986, 0.0620), // 5°
    (0.9954, 0.1240), // 10°
    (0.9900, 0.1860), // 15°
    (0.9822, 0.2480), // 20°
    (0.9730, 0.3100), // 25°
    (0.9600, 0.3720), // 30°
    (0.9427, 0.4340), // 35°
    (0.9216, 0.4958), // 40°
    (0.8962, 0.5571), // 45°
    (0.8679, 0.6176), // 50°
    (0.8350, 0.6769), // 55°
    (0.7986, 0.7346), // 60°
    (0.7597, 0.7903), // 65°
    (0.7186, 0.8435), // 70°
    (0.6732, 0.8936), // 75°
    (0.6213, 0.9394), // 80°
    (0.5722, 0.9761), // 85°
    (0.5322, 1.0000), // 90°
];

const STEP_RAD: f32 = 5.0 * PI / 180.0;

/// Linear interpolation of (X, Y) at |lat| (radians, >= 0).
fn robinson_interp(lat_abs: f32) -> (f32, f32) {
    let t = (lat_abs / STEP_RAD).clamp(0.0, 18.0);
    let i = (t as usize).min(17);
    let f = t - i as f32;
    let (x0, y0) = ROBINSON_TABLE[i];
    let (x1, y1) = ROBINSON_TABLE[i + 1];
    (x0 + (x1 - x0) * f, y0 + (y1 - y0) * f)
}

/// Inverse of the Y column: |y| in [0, 1] → |lat| in radians. The table's Y is
/// strictly increasing, and we use the same linear interpolation as the
/// forward mapping, so project→invert round-trips exactly (to f32 precision).
fn robinson_lat_from_y(y_abs: f32) -> f32 {
    let y = y_abs.clamp(0.0, 1.0);
    // 18 segments: linear search is branch-predictable and trivially fast.
    let mut i = 0usize;
    while i < 17 && ROBINSON_TABLE[i + 1].1 < y {
        i += 1;
    }
    let (_, y0) = ROBINSON_TABLE[i];
    let (_, y1) = ROBINSON_TABLE[i + 1];
    let f = if y1 > y0 { (y - y0) / (y1 - y0) } else { 0.0 };
    (i as f32 + f) * STEP_RAD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equator_and_poles_land_where_expected() {
        for p in Projection::ALL {
            let (x, y) = p.project(0.0, 0.0);
            assert!(x.abs() < 1e-6 && y.abs() < 1e-6);
            let (_, yn) = p.project(FRAC_PI_2, 0.0);
            assert!((yn - 1.0).abs() < 1e-6, "{:?} north pole y = {yn}", p);
            let (_, ys) = p.project(-FRAC_PI_2, 0.0);
            assert!((ys + 1.0).abs() < 1e-6, "{:?} south pole y = {ys}", p);
        }
    }

    #[test]
    fn invert_rejects_outside_points() {
        assert!(Projection::Equirectangular.invert(1.2, 0.0).is_none());
        // Robinson frame corner: beyond the pole-line length at high latitude.
        assert!(Projection::Robinson.invert(0.99, 0.99).is_none());
        assert!(Projection::Robinson.invert(0.0, 1.2).is_none());
    }

    #[test]
    fn dense_latlon_roundtrip() {
        for p in Projection::ALL {
            for ilat in -89..=89 {
                for ilon in (-179..=179).step_by(7) {
                    let lat = ilat as f32 * PI / 180.0;
                    let lon = ilon as f32 * PI / 180.0;
                    let (x, y) = p.project(lat, lon);
                    let (lat2, lon2) = p.invert(x, y).unwrap_or_else(|| {
                        panic!("{:?} invert failed at lat {ilat} lon {ilon}", p)
                    });
                    assert!(
                        (lat - lat2).abs() < 1e-4 && (lon - lon2).abs() < 1e-4,
                        "{:?} roundtrip drift at lat {ilat} lon {ilon}: {lat2} {lon2}",
                        p
                    );
                }
            }
        }
    }
}
