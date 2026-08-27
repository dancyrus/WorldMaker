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
/// (500 My). Produced by the acceptance harness (config identical to this
/// test); every CI platform must reproduce them bit-for-bit — the sim path
/// uses only IEEE-exact operations (see worldmaker_core::dmath), and this
/// test is what enforces it.
///
/// History: elevation regenerated 2026-08-19 when sea level switched from a
/// per-keyframe re-solve to a fixed t=0 datum (decision log, Dan; the plate
/// golden was untouched — proof the change was display-datum only). BOTH
/// regenerated 2026-08-26 for WO-0003 Fix 2 — the one sanctioned M3 move:
/// the t=0 plate generator was replaced by the retuned hybrid growth+warp
/// (judge panel 3–0, re-judging panel 3–0; decision log), and the craton
/// stream's draw alignment is geometry-entangled with the plate map, so
/// continents and elevation move with it — a whole-world regeneration, not
/// a red flag. Old M3 values 0x7b43_ec03_a6ef_ca2a / 0x1690_72d7_7080_3f71.
/// BOTH regenerated again 2026-08-27 for WO-0003 Fix 4 — the second
/// sanctioned move (decision log): the plate-motion force model changed
/// (jammed speed floor 0.05 instead of 0, suture threshold 1.2 with timer
/// hysteresis, floor gridlock breaker, SPEED_MAX 2.0), which alters every
/// step of every run — a whole-world change by design. Values from the
/// Fix 4 harness run on Daniels-MacBook-Air
/// (docs/results/tectonics-fix4-Daniels-MacBook-Air.json, whose determinism
/// hashes equal these constants by construction).
const GOLDEN_TECTONIC_ELEVATION_L6_SEED42: u64 = 0x857b_8233_0e24_2c03;
const GOLDEN_TECTONIC_PLATES_L6_SEED42: u64 = 0xa8c4_9d9b_f779_59e8;
/// Pinned at WO-0003 close (S4 audit): the harness had recorded this hash in
/// every results JSON but no test asserted it, leaving crust type free to
/// drift cross-platform unnoticed. Value from the same Fix 4 harness run as
/// the two constants above.
const GOLDEN_TECTONIC_CRUST_TYPE_L6_SEED42: u64 = 0xf771_678a_67d5_19a4;

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
    let crust_hash = hash_u32_slice(world.fields.get_u32(tectonics::CRUST_TYPE).unwrap());
    assert_eq!(
        crust_hash, GOLDEN_TECTONIC_CRUST_TYPE_L6_SEED42,
        "tectonic crust type drifted from the committed golden"
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
