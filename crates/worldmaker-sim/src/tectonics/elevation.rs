//! Elevation derived from crust state (never integrated), plus the
//! hypsometric sea-level solve.
//!
//! Continents ride Airy isostasy; ocean floor follows the age-cooling curve;
//! trench/arc/hotspot flags apply their reliefs; the Phase 0 value-noise fBm
//! survives as a ±300 m detail texture so coastlines aren't blobby. The
//! solved sea-level offset is subtracted so 0 = sea level in every keyframe,
//! and recorded so the acceptance harness can recover physical depths.

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

/// Derive elevation from crust state, then solve the sea-level offset so the
/// ocean fraction matches the parameter (bisection over integer cell counts,
/// 40 fixed iterations) and store elevation relative to that level.
pub(super) fn derive_and_solve(sim: &mut SimState, params: &TectonicsParams) {
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

    // Sea-level solve: find s with count(elev < s) = ocean fraction. Integer
    // counts only, fixed iteration count — deterministic. Solved ONCE, at
    // the t = 0 anchor; afterwards the datum stays fixed so sea level drifts
    // naturally with the hypsometry — plenty of young, shallow ocean floor
    // raises it and floods low continents, an aging seafloor drains them
    // (decision log, Dan). Anchoring at t = 0 also keeps resume-from-keyframe
    // bit-exact: the offset rides along in every keyframe.
    if sim.t_my == 0.0 {
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
    }
    let s = sim.sea_offset_m;
    sim.elev.par_iter_mut().for_each(|e| *e -= s);
}
