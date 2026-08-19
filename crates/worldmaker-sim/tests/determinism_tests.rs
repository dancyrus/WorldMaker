//! Determinism: a fixed seed must reproduce fixed hashes of the generated
//! fields — the golden values below are committed and must never drift, on
//! any platform. If one changes, a stage changed behavior; that needs a
//! decision-log entry and a new golden, not a shrug.

use std::sync::Arc;

use worldmaker_core::hash::{hash_f32_slice, hash_u32_slice};
use worldmaker_core::Grid;
use worldmaker_sim::noise_stage::ELEVATION_FIELD;
use worldmaker_sim::tectonics::{self, TectonicsParams, TectonicsStage};
use worldmaker_sim::{NoiseElevationStage, Pipeline, StageContext, WorldState};

fn elevation_hash(level: u32, seed: u64) -> u64 {
    let grid = Arc::new(Grid::build(level));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(NoiseElevationStage::default()));
    pipe.run(&StageContext::new(seed), &mut world).unwrap();
    hash_f32_slice(world.fields.get(ELEVATION_FIELD).unwrap())
}

/// Produced once on Dan's PC (2026-08-19) and committed; CI on Linux must
/// reproduce it bit-for-bit, which is what makes this a cross-platform
/// determinism proof.
///
/// History: regenerated 2026-08-19 when the noise stage switched its seed
/// derivation to the sanctioned sub_rng path (decision log).
const GOLDEN_HASH_L6_SEED42: u64 = 0xa86a_7471_79a3_5a46;

#[test]
fn same_seed_same_hash_twice() {
    assert_eq!(elevation_hash(5, 42), elevation_hash(5, 42));
}

#[test]
fn different_seed_different_hash() {
    assert_ne!(elevation_hash(5, 42), elevation_hash(5, 43));
}

#[test]
fn fixed_seed_reproduces_committed_hash() {
    let h = elevation_hash(6, 42);
    assert_eq!(
        h, GOLDEN_HASH_L6_SEED42,
        "elevation field hash drifted from the committed golden value"
    );
}

/// Phase 1 goldens: the tectonic world at L6, seed 42, default parameters
/// (500 My). Produced by the acceptance harness on Dan's PC (2026-08-19,
/// docs/results/tectonics-phase1-DESKTOP-VKD81C6.json); Linux CI must
/// reproduce them bit-for-bit — the sim path uses only IEEE-exact operations
/// (see worldmaker_core::dmath), and this test is what enforces it.
///
/// History: elevation regenerated 2026-08-19 when sea level switched from a
/// per-keyframe re-solve to a fixed t=0 datum (decision log, Dan). The plate
/// golden was untouched — proof the change was display-datum only.
const GOLDEN_TECTONIC_ELEVATION_L6_SEED42: u64 = 0xf751_0e72_14ed_5b62;
const GOLDEN_TECTONIC_PLATES_L6_SEED42: u64 = 0x70df_6db8_ec5f_653d;

#[test]
fn tectonics_reproduces_committed_goldens() {
    let grid = Arc::new(Grid::build(6));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(TectonicsStage::new(TectonicsParams::default())));
    pipe.run(&StageContext::new(42), &mut world).unwrap();
    let elev_hash = hash_f32_slice(world.fields.get(tectonics::ELEVATION_M).unwrap());
    let plate_hash = hash_u32_slice(world.fields.get_u32(tectonics::PLATE_ID).unwrap());
    assert_eq!(
        elev_hash, GOLDEN_TECTONIC_ELEVATION_L6_SEED42,
        "tectonic elevation drifted from the committed golden"
    );
    assert_eq!(
        plate_hash, GOLDEN_TECTONIC_PLATES_L6_SEED42,
        "tectonic plate ids drifted from the committed golden"
    );
}

#[test]
fn elevation_field_has_no_nans_and_sane_range() {
    let grid = Arc::new(Grid::build(5));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(NoiseElevationStage::default()));
    pipe.run(&StageContext::new(7), &mut world).unwrap();
    let elev = world.fields.get(ELEVATION_FIELD).unwrap();
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for &e in elev {
        assert!(e.is_finite());
        lo = lo.min(e);
        hi = hi.max(e);
    }
    assert!(
        lo < -500.0 && hi > 500.0,
        "elevation range implausible: [{lo}, {hi}]"
    );
    assert!(
        lo > -20_000.0 && hi < 20_000.0,
        "elevation range implausible: [{lo}, {hi}]"
    );
}
