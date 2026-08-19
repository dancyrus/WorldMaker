//! Determinism: a fixed seed must reproduce a fixed hash of the elevation
//! field — the golden value below is committed and must never drift, on any
//! platform. If it changes, the noise stage changed behavior; that needs a
//! decision-log entry and a new golden, not a shrug.

use std::sync::Arc;

use worldmaker_core::hash::hash_f32_slice;
use worldmaker_core::Grid;
use worldmaker_sim::noise_stage::ELEVATION_FIELD;
use worldmaker_sim::{NoiseElevationStage, Pipeline, StageContext, WorldState};

fn elevation_hash(level: u32, seed: u64) -> u64 {
    let grid = Arc::new(Grid::build(level));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(NoiseElevationStage::default()));
    pipe.run(&StageContext { master_seed: seed }, &mut world).unwrap();
    hash_f32_slice(world.fields.get(ELEVATION_FIELD).unwrap())
}

/// Produced once on Dan's PC (2026-08-19) and committed; CI on Linux must
/// reproduce it bit-for-bit, which is what makes this a cross-platform
/// determinism proof.
const GOLDEN_HASH_L6_SEED42: u64 = 0x7533_a0e9_ea9a_55b4;

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

#[test]
fn elevation_field_has_no_nans_and_sane_range() {
    let grid = Arc::new(Grid::build(5));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(NoiseElevationStage::default()));
    pipe.run(&StageContext { master_seed: 7 }, &mut world).unwrap();
    let elev = world.fields.get(ELEVATION_FIELD).unwrap();
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for &e in elev {
        assert!(e.is_finite());
        lo = lo.min(e);
        hi = hi.max(e);
    }
    assert!(lo < -500.0 && hi > 500.0, "elevation range implausible: [{lo}, {hi}]");
    assert!(lo > -20_000.0 && hi < 20_000.0, "elevation range implausible: [{lo}, {hi}]");
}
