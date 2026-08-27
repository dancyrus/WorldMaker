//! Layer baking: turn one keyframe into per-cell shading VALUES, plus the
//! palette LUT the shaders read (WO-0003 Fix 3, d3a §2).
//!
//! Palettes remain testable Rust: the anchor tables below are the single
//! source of truth, and `bake_palette_lut` evaluates them into a 256×8 RGBA8
//! texture that both canvases sample per fragment (two `textureLoad`s + a
//! manual mix — interpolation stays in sRGB-encoded space, exactly the space
//! the ΔE00 verification was done in). The crust-age map is viridis and the
//! thickness map batlow — both perceptually uniform sequential maps, anchor
//! tables taken from verified control points (max ΔE00 ≤ 2 under
//! piecewise-linear sRGB interpolation); no rainbow anywhere. Elevation keeps
//! Phase 0's hypsometric ramp. Plate colors are a 24-entry categorical
//! palette; boundary types color the debug band overlay and the (leg 3)
//! smoothed polylines.

use rayon::prelude::*;

use worldmaker_sim::tectonics::{
    Keyframe, F_BND_CONVERGENT, F_BND_DIVERGENT, F_BND_TRANSFORM, SLAB_DETACH_MY, SLAB_NONE,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Elevation,
    Plates,
    CrustAge,
    Thickness,
    PlateVelocity,
    VelocityField,
    /// WO-0006 S3: the slab ledger made visible — the Plates layer dimmed
    /// to 40% with each cell above a subducted slab drawn in the slab
    /// plate's color, fading as the slab ages toward detachment.
    Overlay,
}

impl Layer {
    pub const ALL: [Layer; 7] = [
        Layer::Elevation,
        Layer::Plates,
        Layer::CrustAge,
        Layer::Thickness,
        Layer::PlateVelocity,
        Layer::VelocityField,
        Layer::Overlay,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Layer::Elevation => "Elevation",
            Layer::Plates => "Plates",
            Layer::CrustAge => "Crust age",
            Layer::Thickness => "Thickness",
            Layer::PlateVelocity => "Plate velocity",
            Layer::VelocityField => "Velocity field",
            Layer::Overlay => "Overlay",
        }
    }
    /// The two velocity layers draw the Plates layer underneath their
    /// arrows (WO-0004): they bake and shade exactly as Plates.
    pub fn shades_as_plates(self) -> bool {
        matches!(
            self,
            Layer::Plates | Layer::PlateVelocity | Layer::VelocityField
        )
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

/// 48 categorical plate colors, chosen for pairwise distinctness at full
/// saturation range (Kelly/Glasbey style, dark and light alternating).
/// Extended 24 → 48 in WO-0006 S2: with no plate ceiling and §6
/// microplates, the census can exceed the old palette.
const PLATE_COLORS: [[f32; 3]; 48] = [
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
    rgb(120, 60, 255),
    rgb(0, 90, 60),
    rgb(255, 90, 130),
    rgb(150, 200, 20),
    rgb(70, 25, 130),
    rgb(255, 170, 0),
    rgb(40, 90, 140),
    rgb(200, 0, 150),
    rgb(120, 220, 180),
    rgb(90, 40, 20),
    rgb(250, 230, 130),
    rgb(30, 30, 200),
    rgb(180, 180, 180),
    rgb(110, 140, 80),
    rgb(255, 120, 255),
    rgb(0, 60, 20),
    rgb(140, 90, 200),
    rgb(230, 80, 20),
    rgb(20, 160, 90),
    rgb(200, 200, 90),
    rgb(90, 200, 250),
    rgb(160, 20, 60),
    rgb(240, 170, 140),
    rgb(60, 110, 20),
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

/// Craton-brush preview tints (composited over the base layer color by the
/// overlay pass, LUT row 5).
const PAINT_CONTINENT: [f32; 3] = rgb(255, 140, 40);
const PAINT_OCEAN: [f32; 3] = rgb(40, 120, 255);
/// Hotspot markers: magenta — measured distinct (ΔE00 > 20) from every
/// colormap in use.
const HOTSPOT_MARK: [f32; 3] = rgb(255, 0, 255);
/// Background outside the flat map's projection outline (LUT row 5 code 7).
const OUTSIDE_MAP: [f32; 3] = [0.10, 0.11, 0.13];
/// Velocity arrows on the two velocity layers (WO-0004): white, drawn
/// through the boundary-ribbon path as btype `boundaries::BTYPE_ARROW`.
const ARROW_WHITE: [f32; 3] = rgb(255, 255, 255);

/// Continent flag in the category word (bit 16, frozen in d3a §2.2).
pub const CAT_CONTINENT: u32 = 1 << 16;

/// Bake one keyframe into per-cell shading records for the active layer
/// (d3a §2.2). Per cell: `x` = f32 scalar bits, `y` = category word.
///
/// - `x`: Elevation/Plates = raw keyframe-relative meters (no neighbor
///   averaging — pinned); CrustAge = ramp coordinate t (continents store 0,
///   never read — masked interpolation); Thickness = ramp coordinate t.
/// - `y`: bits 0..=7 plate color index (id % 48, WO-0006 S2), bits 8..=9
///   boundary code (0 none, 1 trench/convergent, 2 ridge/divergent,
///   3 transform, priority as the old bake), bit 16 continent flag, rest
///   zero.
///
/// No sea level (GPU uniform), no overlay (separate buffer), no colors
/// (GPU LUT).
pub fn bake_values(layer: Layer, kf: &Keyframe) -> Vec<[u32; 2]> {
    let n = kf.elev_m.len();

    let mut out: Vec<[u32; 2]> = Vec::with_capacity(n);
    (0..n)
        .into_par_iter()
        .map(|c| {
            let flags = kf.flags[c] as u32;
            let continent = flags & (1 << 15) != 0;
            let scalar = match layer {
                Layer::Elevation | Layer::Plates | Layer::PlateVelocity | Layer::VelocityField => {
                    kf.elev_m[c] as f32
                }
                Layer::CrustAge => {
                    if continent {
                        0.0
                    } else {
                        // Young (ridge) bright, old abyss dark.
                        1.0 - (kf.crust_age_my[c] as f32 / AGE_MAX_MY).clamp(0.0, 1.0)
                    }
                }
                Layer::Thickness => {
                    ((kf.thickness_ckm[c] as f32 * 0.01 - 5.0) / 65.0).clamp(0.0, 1.0)
                }
                Layer::Overlay => {
                    // Slab fade: 1.0 the step it went under, 0.0 at the
                    // detachment age — detached slabs fade out into the
                    // dimmed plates base.
                    if kf.slab_plate[c] == SLAB_NONE {
                        0.0
                    } else {
                        (1.0 - (kf.t_my - kf.slab_since_my[c] as f32) / SLAB_DETACH_MY)
                            .clamp(0.0, 1.0)
                    }
                }
            };
            let mut cat = 0u32;
            if layer.shades_as_plates() || layer == Layer::Overlay {
                // Plate ids are stable across a plate's whole life, so its
                // color never changes as the census around it does; id % 48
                // (WO-0006 S2) accepts occasional collisions in exchange.
                cat |= (kf.plate_id[c] as u32 % PLATE_COLORS.len() as u32) & 0xFF;
            }
            if layer == Layer::Overlay && kf.slab_plate[c] != SLAB_NONE {
                // Bits 20..=27: the slab plate's color index (same stable
                // id % 48 rule), read by the shader's Overlay branch.
                cat |= ((kf.slab_plate[c] as u32 % PLATE_COLORS.len() as u32) & 0xFF) << 20;
            }
            let bnd: u32 = if flags & F_BND_CONVERGENT != 0 {
                1
            } else if flags & F_BND_DIVERGENT != 0 {
                2
            } else if flags & F_BND_TRANSFORM != 0 {
                3
            } else {
                0
            };
            cat |= bnd << 8;
            if continent {
                cat |= CAT_CONTINENT;
            }
            [scalar.to_bits(), cat]
        })
        .collect_into_vec(&mut out);
    out
}

// ----- palette LUT -----

/// Palette LUT dimensions: 256 texels × 8 rows, RGBA8 (Rgba8Unorm, NOT srgb —
/// bytes pass through untransformed, as the CPU bake's bytes did).
pub const LUT_W: u32 = 256;
pub const LUT_ROWS: u32 = 8;

/// Bake the palette LUT from the Rust ramps (single source of truth; tested
/// texel-for-texel below). Row map (shader must match):
///   0  hypsometric ocean, sqrt-warped (graft 5): texel u = ramp at depth
///      (u/255)²·6000 — i.e. linear in u = sqrt(−e/6000), so the shader's
///      two-texel mix reproduces the CPU ramp exactly
///   1  hypsometric land: texel i = hypsometric((i/255)·5500)
///   2  viridis (age t)     3  batlow (thickness t)
///   4  48 plate colors in texels 0..48; oceanic-darkened ×0.55 in 64..112
///   5  fixed colors: 0 trench, 1 ridge, 2 transform, 3 age-continent,
///      4 paint-continent, 5 paint-ocean, 6 hotspot, 7 outside-map
///      background, 8 velocity-arrow white
///   6–7 reserved (zero)
pub fn bake_palette_lut() -> Vec<u8> {
    let w = LUT_W as usize;
    let mut out = vec![0u8; w * LUT_ROWS as usize * 4];
    let mut set = |row: usize, i: usize, c: [f32; 3]| {
        let o = (row * w + i) * 4;
        out[o..o + 4].copy_from_slice(&pack3(c).to_le_bytes());
    };
    for i in 0..w {
        let t = i as f32 / 255.0;
        set(0, i, hypsometric(-(t * t) * 6000.0));
        set(1, i, hypsometric(t * 5500.0));
        set(2, i, ramp(&VIRIDIS, t));
        set(3, i, ramp(&BATLOW, t));
    }
    for (r, c) in PLATE_COLORS.iter().enumerate() {
        set(4, r, *c);
        set(4, 64 + r, [c[0] * 0.55, c[1] * 0.55, c[2] * 0.55]);
    }
    for (i, c) in [
        BOUNDARY_TRENCH,
        BOUNDARY_RIDGE,
        BOUNDARY_TRANSFORM,
        AGE_CONTINENT,
        PAINT_CONTINENT,
        PAINT_OCEAN,
        HOTSPOT_MARK,
        OUTSIDE_MAP,
        ARROW_WHITE,
    ]
    .iter()
    .enumerate()
    {
        set(5, i, *c);
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

    /// The GPU LUT must equal the Rust ramps texel-for-texel (d3a §2.3): the
    /// anchors stay the single, ΔE-verified source of truth.
    #[test]
    fn lut_rows_match_rust_ramps() {
        let lut = bake_palette_lut();
        assert_eq!(lut.len(), (LUT_W * LUT_ROWS * 4) as usize);
        let texel = |row: usize, i: usize| -> u32 {
            let o = (row * LUT_W as usize + i) * 4;
            u32::from_le_bytes([lut[o], lut[o + 1], lut[o + 2], lut[o + 3]])
        };
        for i in 0..256 {
            let t = i as f32 / 255.0;
            assert_eq!(texel(0, i), pack3(hypsometric(-(t * t) * 6000.0)));
            assert_eq!(texel(1, i), pack3(hypsometric(t * 5500.0)));
            assert_eq!(texel(2, i), pack3(ramp(&VIRIDIS, t)));
            assert_eq!(texel(3, i), pack3(ramp(&BATLOW, t)));
        }
        for (r, c) in PLATE_COLORS.iter().enumerate() {
            assert_eq!(texel(4, r), pack3(*c));
            assert_eq!(
                texel(4, 64 + r),
                pack3([c[0] * 0.55, c[1] * 0.55, c[2] * 0.55])
            );
        }
        let fixed = [
            BOUNDARY_TRENCH,
            BOUNDARY_RIDGE,
            BOUNDARY_TRANSFORM,
            AGE_CONTINENT,
            PAINT_CONTINENT,
            PAINT_OCEAN,
            HOTSPOT_MARK,
            OUTSIDE_MAP,
            ARROW_WHITE,
        ];
        for (i, c) in fixed.iter().enumerate() {
            assert_eq!(texel(5, i), pack3(*c));
        }
        // Reserved texels stay zero.
        for i in 48..64 {
            assert_eq!(texel(4, i), 0);
        }
        for i in 112..256 {
            assert_eq!(texel(4, i), 0);
        }
        for i in 9..256 {
            assert_eq!(texel(5, i), 0);
        }
        for row in 6..8 {
            for i in 0..256 {
                assert_eq!(texel(row, i), 0);
            }
        }
    }

    /// WO-0004 step 9: `Layer::ALL` must list every enum variant exactly
    /// once. The exhaustive match (no wildcard) fails to compile when a
    /// variant is added, forcing this test — and ALL — to be updated.
    #[test]
    fn layer_all_lists_every_variant_once() {
        const VARIANT_COUNT: usize = 7;
        assert_eq!(Layer::ALL.len(), VARIANT_COUNT);
        for l in Layer::ALL {
            match l {
                Layer::Elevation
                | Layer::Plates
                | Layer::CrustAge
                | Layer::Thickness
                | Layer::PlateVelocity
                | Layer::VelocityField
                | Layer::Overlay => {}
            }
        }
        for (i, a) in Layer::ALL.iter().enumerate() {
            for b in Layer::ALL.iter().skip(i + 1) {
                assert!(a != b, "Layer::ALL contains a duplicate");
            }
        }
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
