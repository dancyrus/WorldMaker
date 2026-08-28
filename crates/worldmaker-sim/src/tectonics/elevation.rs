//! Elevation derived from crust state (never integrated), plus the
//! sea-level solve.
//!
//! Continents ride Airy isostasy; ocean floor follows the age-cooling curve;
//! trench/arc/hotspot flags apply their reliefs; the Phase 0 value-noise fBm
//! survives as a ±300 m detail texture so coastlines aren't blobby. The
//! solved sea-level offset is subtracted so 0 = sea level in every keyframe,
//! and recorded so the acceptance harness can recover physical depths.
//!
//! Sea level (WO-0009): the world owns a fixed water mass, set once at t = 0
//! from the land-fraction parameter. Every later keyframe solves the level
//! at which that mass exactly fills the basins — a young, shallow seafloor
//! (big ridge volume) pushes the ocean up onto the continents; an old, deep
//! one drains them (Hays & Pitman 1973). Mass is conserved; volume is
//! derived via seawater density.

use rayon::prelude::*;

use crate::noise_stage::fbm;

use super::step::SimState;
use super::{TectonicsParams, F_ARC, F_TRENCH};

// ----- constants (spec-fixed unless marked; decision log for the rest) -----
/// Continental elevation: 150 m per km of thickness above 35 km (spec).
const CONT_M_PER_KM: f32 = 150.0;
const CONT_REF_KM: f32 = 35.0;
/// Ocean depth −(2600 + 365·√age), flattening at −5,600 m (spec).
const OCEAN_RIDGE_M: f32 = 2600.0;
const OCEAN_SQRT_M: f32 = 365.0;
const OCEAN_FLOOR_M: f32 = -5600.0;
/// Trench cells blend 75% toward −8,500 m (decision log).
const TRENCH_TARGET_M: f32 = -8500.0;
const TRENCH_BLEND: f32 = 0.75;
/// Arc relief bonus (decision log): lifts island arcs near sea level.
const ARC_RELIEF_M: f32 = 2000.0;
/// Hotspot shield relief per km of buildup (decision log).
const BUILDUP_OCEAN_M_PER_KM: f32 = 1000.0;
const BUILDUP_CONT_M_PER_KM: f32 = 400.0;
/// Detail-noise amplitude (decision log: low, coastline texture only).
const DETAIL_AMP_M: f32 = 300.0;
const DETAIL_OCTAVES: u32 = 6;

/// Seawater density, kg/m³ (WO-0009): the mass ↔ flooded-volume conversion.
pub(super) const RHO_SEAWATER: f64 = 1027.0;

/// Uniform cell area in m²: sphere area over cell count — the same
/// uniform-cell assumption the fraction solve and `cell_spacing_km` use.
pub(super) fn cell_area_m2(cell_count: usize) -> f64 {
    let r_m = super::step::R_EARTH_KM as f64 * 1000.0;
    4.0 * std::f64::consts::PI * r_m * r_m / cell_count as f64
}

/// Water mass (kg) flooding raw elevations at sea-level offset `s_m`:
/// Σ max(0, s − elev(c)) · cell_area · ρ. f64 accumulation in cell-id
/// order — a serial loop by determinism rule (rayon float reductions are
/// order-nondeterministic).
pub(super) fn flooded_mass_kg(elev: &[f32], s_m: f64, cell_area_m2: f64) -> f64 {
    let mut depth_sum = 0.0f64;
    for &e in elev {
        let d = s_m - e as f64;
        if d > 0.0 {
            depth_sum += d;
        }
    }
    depth_sum * cell_area_m2 * RHO_SEAWATER
}

/// Derive elevation from crust state, then solve sea level and store
/// elevation relative to it (0 = sea level downstream).
///
/// At t = 0 the offset comes from the land-fraction parameter (bisection
/// over integer cell counts, 40 fixed iterations — unchanged from Phase 1)
/// and the flooded mass at that level is banked as the world's water
/// inventory. Every later keyframe bisects the offset (40 fixed f64
/// iterations) so the flooded mass equals that inventory minus whatever is
/// locked in ice (WO-0009).
pub(super) fn derive_and_solve(sim: &mut SimState, params: &TectonicsParams) {
    derive(sim);
    solve(sim, params);
}

/// Fill `sim.elev` with the raw (pre-sea-level) elevation derived from
/// crust state. Pure function of the quantized crust fields + noise seed,
/// so a keyframe-restored state re-derives bit-identical elevations — the
/// water_gates conservation test depends on that.
pub(super) fn derive(sim: &mut SimState) {
    let grid = sim.grid.clone();
    let noise_seed = sim.noise_seed;
    {
        let ctype = &sim.crust_type;
        let thick = &sim.thickness;
        let age = &sim.crust_age;
        let build = &sim.buildup;
        let feats = &sim.features;
        sim.elev.par_iter_mut().enumerate().for_each(|(c, e)| {
            let mut elev = if ctype[c] == 1 {
                CONT_M_PER_KM * (thick[c] - CONT_REF_KM)
            } else {
                (-(OCEAN_RIDGE_M + OCEAN_SQRT_M * age[c].max(0.0).sqrt())).max(OCEAN_FLOOR_M)
            };
            if feats[c] & F_TRENCH != 0 {
                elev = TRENCH_BLEND * TRENCH_TARGET_M + (1.0 - TRENCH_BLEND) * elev;
            }
            if feats[c] & F_ARC != 0 {
                elev += ARC_RELIEF_M;
            }
            if build[c] > 0.0 {
                elev += build[c]
                    * if ctype[c] == 0 {
                        BUILDUP_OCEAN_M_PER_KM
                    } else {
                        BUILDUP_CONT_M_PER_KM
                    };
            }
            elev += DETAIL_AMP_M * fbm(grid.positions[c], noise_seed, DETAIL_OCTAVES);
            *e = elev;
        });
    }
}

/// Solve the sea-level offset for the current raw elevations (see
/// [`derive_and_solve`]) and shift them so 0 = sea level.
pub(super) fn solve(sim: &mut SimState, params: &TectonicsParams) {
    let a_cell = cell_area_m2(sim.elev.len());
    if sim.t_my == 0.0 {
        // t = 0 anchor: find s with count(elev < s) = ocean fraction —
        // integer counts, fixed iteration count, deterministic (the Phase 1
        // solve, run one final time; WO-0009). The flooded mass at that
        // level becomes the world's water inventory for the whole run; it
        // rides in every keyframe, so resume stays bit-exact.
        let n = sim.elev.len();
        let target_ocean = (((1.0 - params.land_fraction) as f64) * n as f64).round() as usize;
        let (mut lo, mut hi) = (-9000.0f32, 9000.0f32);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            let below = sim.elev.par_iter().filter(|&&e| e < mid).count();
            if below < target_ocean {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        sim.sea_offset_m = 0.5 * (lo + hi);
        sim.water_mass_kg = flooded_mass_kg(&sim.elev, sim.sea_offset_m as f64, a_cell);
    } else {
        // Later keyframes: bisect s so the flooded mass equals the
        // inventory (minus ice — 0 until Phase 3+). 40 fixed f64
        // iterations; the mass integral is serial in cell-id order, so the
        // solve is bit-reproducible everywhere. The offset lands within
        // ~2e-8 m of the exact level; storage as f32 dominates the residual
        // (≲ 1 mm of sea level — the water_gates tolerance).
        let target = sim.water_mass_kg - sim.mass_in_ice();
        let (mut lo, mut hi) = (-9000.0f64, 9000.0f64);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if flooded_mass_kg(&sim.elev, mid, a_cell) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        sim.sea_offset_m = (0.5 * (lo + hi)) as f32;
    }
    let s = sim.sea_offset_m;
    sim.elev.par_iter_mut().for_each(|e| *e -= s);
}
