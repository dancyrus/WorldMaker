//! Placeholder elevation: seeded fractal value noise on the sphere.
//!
//! This stands in for Phase 1's tectonics so the walking skeleton has terrain
//! to show. It is still held to the full determinism contract, and it
//! deliberately avoids transcendental functions (only +, −, ×, /, floor and
//! polynomial smoothing touch the data), so the elevation field hashes
//! bit-identically across platforms — that hash is a committed regression test.

use rand::RngCore;
use rayon::prelude::*;

use crate::pipeline::{Stage, StageContext, WorldState};
use worldmaker_core::hash::splitmix64;
use worldmaker_core::rng::sub_rng;

/// Name of the field this stage writes.
pub const ELEVATION_FIELD: &str = "elevation_m";

pub struct NoiseElevationStage {
    /// Octave count for the fBm sum.
    pub octaves: u32,
}

impl Default for NoiseElevationStage {
    fn default() -> Self {
        NoiseElevationStage { octaves: 6 }
    }
}

impl Stage for NoiseElevationStage {
    fn id(&self) -> &'static str {
        "phase0-noise-elevation"
    }

    fn params_hash(&self) -> u64 {
        splitmix64(self.octaves as u64)
    }

    fn run(&self, ctx: &StageContext, world: &mut WorldState) -> anyhow::Result<()> {
        // Base seed for the noise lattice, drawn through the one sanctioned
        // RNG path: a PCG sub-stream keyed by (master seed, stage id, purpose).
        // The lattice hash then consumes this u64 directly (value noise needs
        // random access by coordinate, not a sequential stream).
        let seed = sub_rng(ctx.master_seed, self.id(), "lattice-noise").next_u64();
        let grid = world.grid.clone();
        let octaves = self.octaves;
        let out = world.fields.get_or_insert_mut(ELEVATION_FIELD);
        out.par_iter_mut().enumerate().for_each(|(i, e)| {
            let p = grid.positions[i];
            let n = fbm(p, seed, octaves);
            // Shape into elevation meters: mostly within ±3000 m, tails to
            // roughly ±8000 m; the -800 m offset gives an Earth-ish ~35% land
            // fraction at sea level 0.
            *e = 8000.0 * n - 800.0;
        });
        Ok(())
    }
}

/// Fractal Brownian motion: `octaves` layers of value noise, lacunarity 2,
/// gain 0.5. Output roughly in [-1, 1]. Also used by the tectonics stage as
/// its low-amplitude elevation detail texture.
pub(crate) fn fbm(p: [f32; 3], seed: u64, octaves: u32) -> f32 {
    let mut sum = 0.0f32;
    let mut amp = 0.5f32;
    let mut freq = 1.6f32;
    for oct in 0..octaves {
        sum += amp * value_noise(p, freq, splitmix64(seed ^ oct as u64));
        amp *= 0.5;
        freq *= 2.0;
    }
    sum * 1.9
}

/// Trilinearly interpolated lattice value noise at frequency `freq`.
/// Deterministic: the lattice values come from SplitMix64 on the integer cell
/// coordinates, and only arithmetic and `floor` touch the result.
fn value_noise(p: [f32; 3], freq: f32, seed: u64) -> f32 {
    let x = p[0] * freq + 64.0; // offset keeps coordinates positive-ish; floor handles the rest
    let y = p[1] * freq + 64.0;
    let z = p[2] * freq + 64.0;
    let xi = x.floor();
    let yi = y.floor();
    let zi = z.floor();
    let fx = smooth(x - xi);
    let fy = smooth(y - yi);
    let fz = smooth(z - zi);
    let (xi, yi, zi) = (xi as i64, yi as i64, zi as i64);

    let mut corners = [0.0f32; 8];
    let mut k = 0;
    for dz in 0..2i64 {
        for dy in 0..2i64 {
            for dx in 0..2i64 {
                corners[k] = lattice(xi + dx, yi + dy, zi + dz, seed);
                k += 1;
            }
        }
    }
    let c00 = lerp(corners[0], corners[1], fx);
    let c10 = lerp(corners[2], corners[3], fx);
    let c01 = lerp(corners[4], corners[5], fx);
    let c11 = lerp(corners[6], corners[7], fx);
    let c0 = lerp(c00, c10, fy);
    let c1 = lerp(c01, c11, fy);
    lerp(c0, c1, fz)
}

/// Pseudo-random lattice value in [-1, 1] for integer coordinates.
#[inline]
fn lattice(x: i64, y: i64, z: i64, seed: u64) -> f32 {
    let h = splitmix64(
        seed ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
            ^ (z as u64).wrapping_mul(0x1656_67b1_9e37_79f9),
    );
    // Top 24 bits → [-1, 1). Plenty of resolution for terrain noise.
    ((h >> 40) as f32) / 8_388_608.0 - 1.0
}

#[inline]
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
