//! Layer baking: turn one keyframe into per-cell RGBA8 colors on the CPU.
//!
//! Palettes are testable Rust rather than shader code. The crust-age map is
//! viridis and the thickness map batlow — both perceptually uniform
//! sequential maps, anchor tables taken from verified control points (max
//! ΔE00 ≤ 2 under piecewise-linear sRGB interpolation); no rainbow anywhere.
//! Elevation keeps Phase 0's hypsometric ramp. Plate colors are a 24-entry
//! categorical palette with boundary cells overridden by boundary type.

use rayon::prelude::*;

use worldmaker_sim::tectonics::{Keyframe, F_BND_CONVERGENT, F_BND_DIVERGENT, F_BND_TRANSFORM};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Elevation,
    Plates,
    CrustAge,
    Thickness,
}

impl Layer {
    pub const ALL: [Layer; 4] = [
        Layer::Elevation,
        Layer::Plates,
        Layer::CrustAge,
        Layer::Thickness,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Layer::Elevation => "Elevation",
            Layer::Plates => "Plates",
            Layer::CrustAge => "Crust age",
            Layer::Thickness => "Thickness",
        }
    }
}

#[inline]
fn pack(r: f32, g: f32, b: f32) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    q(r) | (q(g) << 8) | (q(b) << 16) | 0xff00_0000
}

#[inline]
fn pack3(rgb: [f32; 3]) -> u32 {
    pack(rgb[0], rgb[1], rgb[2])
}

#[inline]
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Piecewise-linear interpolation over (position, rgb) anchors, positions
/// ascending in 0..=1.
fn ramp(anchors: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut prev = anchors[0];
    for &a in anchors.iter().skip(1) {
        if t <= a.0 {
            let span = (a.0 - prev.0).max(1e-6);
            return lerp3(prev.1, a.1, (t - prev.0) / span);
        }
        prev = a;
    }
    anchors[anchors.len() - 1].1
}

const fn rgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

/// viridis, 7 verified anchors (max ΔE00 1.5).
const VIRIDIS: [(f32, [f32; 3]); 7] = [
    (0.0000, rgb(68, 1, 84)),
    (0.1725, rgb(68, 59, 132)),
    (0.4157, rgb(40, 124, 142)),
    (0.5647, rgb(31, 160, 136)),
    (0.7059, rgb(70, 192, 111)),
    (0.8784, rgb(173, 220, 48)),
    (1.0000, rgb(253, 231, 37)),
];

/// batlow, 7 verified anchors (max ΔE00 1.8).
const BATLOW: [(f32, [f32; 3]); 7] = [
    (0.0000, rgb(1, 25, 89)),
    (0.1137, rgb(16, 64, 96)),
    (0.2745, rgb(40, 100, 95)),
    (0.5569, rgb(157, 137, 43)),
    (0.6627, rgb(209, 147, 66)),
    (0.7608, rgb(244, 159, 114)),
    (1.0000, rgb(250, 204, 250)),
];

/// 24 categorical plate colors, chosen for pairwise distinctness at full
/// saturation range (Kelly/Glasbey style, dark and light alternating).
const PLATE_COLORS: [[f32; 3]; 24] = [
    rgb(230, 25, 75),
    rgb(60, 180, 75),
    rgb(255, 225, 25),
    rgb(0, 130, 200),
    rgb(245, 130, 48),
    rgb(145, 30, 180),
    rgb(70, 240, 240),
    rgb(240, 50, 230),
    rgb(210, 245, 60),
    rgb(250, 190, 212),
    rgb(0, 128, 128),
    rgb(220, 190, 255),
    rgb(170, 110, 40),
    rgb(255, 250, 200),
    rgb(128, 0, 0),
    rgb(170, 255, 195),
    rgb(128, 128, 0),
    rgb(255, 215, 180),
    rgb(0, 0, 128),
    rgb(128, 128, 128),
    rgb(255, 255, 255),
    rgb(0, 220, 130),
    rgb(100, 155, 255),
    rgb(60, 60, 60),
];

/// Boundary-line styling on the plates layer, by classified type.
const BOUNDARY_RIDGE: [f32; 3] = rgb(235, 60, 40); // divergent
const BOUNDARY_TRENCH: [f32; 3] = rgb(15, 20, 60); // convergent
const BOUNDARY_TRANSFORM: [f32; 3] = rgb(255, 210, 60);

/// Continents on the crust-age layer (the map reads as ocean age).
const AGE_CONTINENT: [f32; 3] = rgb(72, 70, 72);
/// Age range that spans the viridis ramp: 0 (bright, at the ridge) to this.
const AGE_MAX_MY: f32 = 150.0;

/// Phase 0's hypsometric palette, ported from the old WGSL.
fn hypsometric(elev_rel_m: f32) -> [f32; 3] {
    let e = elev_rel_m;
    if e <= 0.0 {
        let t = (-e / 6000.0).clamp(0.0, 1.0);
        let shallow = [0.16, 0.40, 0.55];
        let deep = [0.03, 0.10, 0.23];
        return lerp3(shallow, deep, t.sqrt());
    }
    let t = (e / 5500.0).clamp(0.0, 1.0);
    if t < 0.35 {
        lerp3([0.23, 0.44, 0.22], [0.55, 0.50, 0.28], t / 0.35)
    } else if t < 0.7 {
        lerp3([0.55, 0.50, 0.28], [0.58, 0.55, 0.52], (t - 0.35) / 0.35)
    } else {
        lerp3([0.58, 0.55, 0.52], [0.95, 0.95, 0.97], (t - 0.7) / 0.3)
    }
}

/// Craton-brush preview tints (blended over the base layer color).
const PAINT_CONTINENT: [f32; 3] = rgb(255, 140, 40);
const PAINT_OCEAN: [f32; 3] = rgb(40, 120, 255);
/// Hotspot markers when the hotspot tool is active: magenta — measured
/// distinct (ΔE00 > 20) from every colormap in use.
const HOTSPOT_MARK: [f32; 3] = rgb(255, 0, 255);

/// Extra decoration painted over the base layer.
pub struct BakeOverlay<'a> {
    /// Craton paint: sorted (cell, +1/−1) pairs, shown while the brush is
    /// active.
    pub craton: Option<&'a [(u32, i8)]>,
    /// Hotspot marker cells (center + ring), shown while the tool is active.
    pub hotspot_cells: Option<&'a [u32]>,
}

impl BakeOverlay<'_> {
    pub const NONE: BakeOverlay<'static> = BakeOverlay {
        craton: None,
        hotspot_cells: None,
    };
}

/// Bake one keyframe into per-cell RGBA8 colors. `sea_level_m` is the UI
/// slider: an offset around the keyframe's solved sea level (elevations are
/// stored relative to that solution, so 0 means "as solved").
pub fn bake(layer: Layer, kf: &Keyframe, sea_level_m: f32, overlay: &BakeOverlay) -> Vec<u32> {
    let n = kf.elev_m.len();

    // Plate ids grow monotonically across breakups, so raw id % 24 lets two
    // alive plates share a color. Rank the ids actually present in this
    // keyframe instead: at most 24 plates are alive, so ranks never collide.
    let plate_rank: Vec<u16> = if layer == Layer::Plates {
        let mut ids: Vec<u16> = kf
            .plates
            .iter()
            .filter(|p| p.alive)
            .map(|p| p.id as u16)
            .collect();
        ids.sort_unstable();
        let mut rank = vec![0u16; ids.last().map(|&m| m as usize + 1).unwrap_or(1)];
        for (r, &id) in ids.iter().enumerate() {
            rank[id as usize] = r as u16;
        }
        rank
    } else {
        Vec::new()
    };

    let mut out: Vec<u32> = Vec::with_capacity(n);
    (0..n)
        .into_par_iter()
        .map(|c| {
            let flags = kf.flags[c] as u32;
            let continent = flags & (1 << 15) != 0;
            match layer {
                Layer::Elevation => pack3(hypsometric(kf.elev_m[c] as f32 - sea_level_m)),
                Layer::Plates => {
                    if flags & F_BND_CONVERGENT != 0 {
                        pack3(BOUNDARY_TRENCH)
                    } else if flags & F_BND_DIVERGENT != 0 {
                        pack3(BOUNDARY_RIDGE)
                    } else if flags & F_BND_TRANSFORM != 0 {
                        pack3(BOUNDARY_TRANSFORM)
                    } else {
                        let rank = plate_rank
                            .get(kf.plate_id[c] as usize)
                            .copied()
                            .unwrap_or(kf.plate_id[c]);
                        let base = PLATE_COLORS[rank as usize % PLATE_COLORS.len()];
                        // Darken oceanic parts so continents read through.
                        let k = if continent { 1.0 } else { 0.55 };
                        pack(base[0] * k, base[1] * k, base[2] * k)
                    }
                }
                Layer::CrustAge => {
                    if continent {
                        pack3(AGE_CONTINENT)
                    } else {
                        // Young (ridge) bright, old abyss dark.
                        let t = 1.0 - (kf.crust_age_my[c] as f32 / AGE_MAX_MY).clamp(0.0, 1.0);
                        pack3(ramp(&VIRIDIS, t))
                    }
                }
                Layer::Thickness => {
                    let t = ((kf.thickness_ckm[c] as f32 * 0.01 - 5.0) / 65.0).clamp(0.0, 1.0);
                    pack3(ramp(&BATLOW, t))
                }
            }
        })
        .collect_into_vec(&mut out);

    // Overlays are sparse; serial writes after the parallel bake.
    if let Some(paint) = overlay.craton {
        for &(cell, v) in paint {
            let c = cell as usize;
            if c < n {
                let tint = if v > 0 { PAINT_CONTINENT } else { PAINT_OCEAN };
                out[c] = pack3(tint);
            }
        }
    }
    if let Some(cells) = overlay.hotspot_cells {
        for &cell in cells {
            let c = cell as usize;
            if c < n {
                out[c] = pack3(HOTSPOT_MARK);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_interpolate_and_clamp() {
        let lo = ramp(&VIRIDIS, -1.0);
        let hi = ramp(&VIRIDIS, 2.0);
        assert_eq!(lo, VIRIDIS[0].1);
        assert_eq!(hi, VIRIDIS[6].1);
        let mid = ramp(&VIRIDIS, 0.4157);
        assert!((mid[0] - 40.0 / 255.0).abs() < 1e-4);
        let b = ramp(&BATLOW, 0.5569);
        assert!((b[1] - 137.0 / 255.0).abs() < 1e-4);
    }

    #[test]
    fn pack_is_rgba8_little_endian() {
        assert_eq!(pack(1.0, 0.0, 0.0), 0xff00_00ff);
        assert_eq!(pack(0.0, 1.0, 0.0), 0xff00_ff00);
        assert_eq!(pack(0.0, 0.0, 1.0), 0xffff_0000);
    }

    #[test]
    fn hypsometric_has_a_coastline_break() {
        let below = hypsometric(-1.0);
        let above = hypsometric(1.0);
        // Ocean is blue-ish, land green-ish: the coastline must be visible.
        assert!(below[2] > below[1], "ocean should be blue");
        assert!(above[1] > above[2], "lowland should be green");
    }

    #[test]
    fn plate_colors_are_pairwise_distinct() {
        for (i, a) in PLATE_COLORS.iter().enumerate() {
            for (j, b) in PLATE_COLORS.iter().enumerate().skip(i + 1) {
                let d2 = (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
                assert!(
                    d2 > 0.015,
                    "plate colors {i} and {j} too similar (d2 = {d2})"
                );
            }
        }
    }
}
