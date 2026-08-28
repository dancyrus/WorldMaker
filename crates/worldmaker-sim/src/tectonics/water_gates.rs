//! WO-0009 water-inventory CI gates.
//!
//! (a) Conservation: over a full 2 Gy L6 run, every keyframe's solved sea
//! level floods exactly the banked t = 0 water mass — "exactly" meaning to
//! the storage precision of the offset: the solve bisects in f64 (terminal
//! bracket ~2e-8 m) and stores the offset as f32, so the gate allows 1 mm
//! of global sea level (~5e14 kg), orders of magnitude above solver error
//! and orders of magnitude below any real leak or drift.
//!
//! (b) Highstand sign: with the same water inventory, a uniformly YOUNG
//! ocean floor (shallow, big ridge volume) must solve a HIGHER sea level
//! than a uniformly old one (Hays & Pitman 1973 ridge-volume effect, sign
//! only).

use std::sync::Arc;

use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;

use super::step::SimState;
use super::{elevation, run_history, TectonicsParams};
use crate::pipeline::{StageContext, WorldState};

/// One millimetre of global sea level, in kg — the conservation bound.
fn tol_kg(cell_count: usize) -> f64 {
    elevation::RHO_SEAWATER * elevation::cell_area_m2(cell_count) * cell_count as f64 * 1e-3
}

fn assert_conserved(seed: u64, label: &str) {
    let grid = Arc::new(Grid::build(6));
    let world = WorldState::new(grid.clone());
    let params = TectonicsParams {
        span_my: 2000.0,
        ..TectonicsParams::default()
    };
    let hist = run_history(&StageContext::new(seed), &world, &params, None).unwrap();
    let n = grid.cell_count() as usize;
    let a_cell = elevation::cell_area_m2(n);
    let tol = tol_kg(n);
    let m0 = hist.keyframes[0].water_mass_kg;
    assert!(m0 > 1.0e20, "{label}: implausible inventory {m0} kg");
    for kf in &hist.keyframes {
        assert_eq!(
            kf.water_mass_kg, m0,
            "{label}: inventory changed at t = {} My",
            kf.t_my
        );
        // Re-derive the raw elevations this keyframe was solved on (the
        // crust fields round-trip the keyframe quantization bit-exactly)
        // and integrate the flooded mass at its stored offset — the same
        // f64 cell-id-order integral the solve used.
        let mut s = SimState::from_keyframe(&grid, seed, &hist.hotspots, kf);
        elevation::derive(&mut s);
        let mass = elevation::flooded_mass_kg(&s.elev, kf.sea_offset_m as f64, a_cell);
        assert!(
            (mass - m0).abs() <= tol,
            "{label}: t = {} My floods {mass} kg vs inventory {m0} kg \
             (off by {} kg, tolerance {tol} kg = 1 mm of sea level)",
            kf.t_my,
            mass - m0,
        );
    }
}

#[test]
fn water_mass_conserved_2gy_l6_seed42() {
    assert_conserved(42, "seed 42");
}

#[test]
fn water_mass_conserved_2gy_l6_seed_cyrus() {
    assert_conserved(seed_from_text("cyrus"), "seed cyrus");
}

/// Build the probe world: a polar-cap continent (~30% of cells, 38 km
/// crust) over an ocean whose crust is uniformly `ocean_age_my` old, with
/// the raw elevations derived. Everything is deterministic and identical
/// between the two probes except the ocean age.
fn probe_world(grid: &Arc<Grid>, ocean_age_my: f32) -> SimState {
    let mut sim = SimState::new_empty(grid);
    sim.noise_seed = 0x77a7_e12d;
    for c in 0..grid.cell_count() as usize {
        if grid.positions[c][2] > 0.4 {
            sim.crust_type[c] = 1;
            sim.thickness[c] = 38.0;
        } else {
            sim.crust_type[c] = 0;
            sim.crust_age[c] = ocean_age_my;
        }
    }
    elevation::derive(&mut sim);
    sim
}

#[test]
fn young_ocean_solves_a_higher_sea_level_than_old() {
    let grid = Arc::new(Grid::build(5));
    let params = TectonicsParams::default();

    // The shared inventory: whatever mass sits below the 0 m datum of the
    // OLD world — both probes then solve for the level that holds it.
    let a_cell = elevation::cell_area_m2(grid.cell_count() as usize);
    let old_raw = probe_world(&grid, 150.0);
    let water = elevation::flooded_mass_kg(&old_raw.elev, 0.0, a_cell);
    assert!(water > 0.0, "probe world has no ocean volume below 0 m");

    let solve_at = |ocean_age_my: f32| -> f32 {
        let mut sim = probe_world(&grid, ocean_age_my);
        sim.t_my = 10.0; // any t > 0: the mass-inventory solve path
        sim.water_mass_kg = water;
        elevation::solve(&mut sim, &params);
        sim.sea_offset_m
    };
    let s_young = solve_at(5.0);
    let s_old = solve_at(150.0);
    assert!(
        s_young > s_old,
        "young ocean solved {s_young} m, old ocean {s_old} m — the \
         ridge-volume highstand sign is wrong"
    );
}
