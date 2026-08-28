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
/// step of every run — a whole-world change by design. Old Fix 4 values
/// 0x857b_8233_0e24_2c03 / 0xa8c4_9d9b_f779_59e8 / crust
/// 0xf771_678a_67d5_19a4.
/// ALL THREE regenerated 2026-08-28 for WO-0006 — the third sanctioned
/// golden move (decision log, "third sanctioned golden move, WO-0006"):
/// S1–S3 replaced the plate-motion model wholesale (force balance + slab
/// ledger, strength-field suture/rifting, S3 calibration of every §1/§4
/// coefficient), which alters every step of every run from t = 0 — a
/// whole-world change by design. Values from print_tectonic_goldens on
/// Daniels-MacBook-Air at the S3 calibrated constants; the phase-0 noise
/// golden above verified UNMOVED in the same suite run.
/// ALL THREE regenerated 2026-08-28 for WO-0008 — the fourth sanctioned
/// golden move (decision log, "fourth sanctioned golden move, WO-0008"):
/// S0 replaced the t = 0 crust layout (whole-plate continents + craton
/// cores), S1 rebuilt closure/linkage/seam/balance (relic-basin closure,
/// the three-part seam rule, insulation venting, fossil capture, axis
/// rifts), and S2 rebuilt orogeny and arcs (underthrust-funded
/// distributed zones, spreading, foreland loading, discrete arc sites,
/// crust-volume ledger) — a whole-world change by design at every stage.
/// Old WO-0006 values 0x2916_52b9_61e3_f74c / 0xa154_6f8c_7944_6284 /
/// crust 0x08f5_c8f9_67db_bc04. Values from print_tectonic_goldens on
/// Daniels-MacBook-Air; the phase-0 noise golden above verified UNMOVED
/// in the same suite run.
/// ELEVATION ONLY regenerated 2026-08-28 for WO-0009 — the fifth
/// sanctioned golden move (decision log, "fifth sanctioned golden move,
/// WO-0009"): sea level switched from the fixed t = 0 datum to a
/// conserved water mass re-solved per keyframe, which shifts every
/// elevation's datum after t = 0 but touches no dynamics — the plate and
/// crust goldens verified UNMOVED in the same run, exactly the
/// display-datum-only signature. Old elevation value
/// 0xabb5_3577_2c81_9c99. Value from print_tectonic_goldens on
/// Daniels-MacBook-Air; the phase-0 noise golden above again UNMOVED.
/// ALL THREE regenerated 2026-08-28 for WO-0011 S1+S2 — the SIXTH
/// sanctioned golden move (announced in WO-0011-S1, decision log,
/// "sixth sanctioned golden move, WO-0011 S1+S2"), moved ONCE for both
/// sessions together: S1's boundary regularization reverts advection
/// re-sampling flips through strong lithosphere every step, and S2
/// replaced the wholesale suture relabel with the front-limited weld
/// (progressive contact-proportional merge, motion-slaved losers) —
/// both alter plate-id trajectories from the first steps by design, a
/// whole-world change. Old WO-0009-era values 0xaa15_8d4f_1563_9cd8 /
/// 0x7680_b5da_d03d_486a / crust 0xebe0_d5a5_a29e_2745. Values from
/// print_tectonic_goldens on Daniels-MacBook-Air; the phase-0 noise
/// golden above verified UNMOVED in the same suite run.
const GOLDEN_TECTONIC_ELEVATION_L6_SEED42: u64 = 0xee3c_1341_0e60_7b05;
const GOLDEN_TECTONIC_PLATES_L6_SEED42: u64 = 0xaa22_f87a_dacd_7acd;
/// Pinned at WO-0003 close (S4 audit): the harness had recorded this hash in
/// every results JSON but no test asserted it, leaving crust type free to
/// drift cross-platform unnoticed. Regenerated with the pair above.
const GOLDEN_TECTONIC_CRUST_TYPE_L6_SEED42: u64 = 0x1a9b_7e90_1e25_4785;
/// WO-0009 S2: the per-cell GLiM lithology tracer, NEW with this session.
/// The seventh sanctioned golden move was budgeted for it but turned out a
/// no-op: lithology is a passive tracer (no RNG draws, no dynamics reads),
/// so all three goldens above verified UNMOVED when it landed — this
/// golden was added, none moved (decision log, WO-0009 S2).
const GOLDEN_TECTONIC_LITHOLOGY_L6_SEED42: u64 = 0x0580_68b5_b4f4_1de9;

/// Golden-regeneration aid: prints the three tectonic hashes for the
/// current build. Used exactly once per sanctioned golden move.
#[test]
#[ignore = "dev aid: prints the tectonic golden hashes"]
fn print_tectonic_goldens() {
    let grid = Arc::new(Grid::build(6));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(TectonicsStage::new(TectonicsParams::default())));
    pipe.run(&StageContext::new(42), &mut world).unwrap();
    let elev_hash = hash_f32_slice(world.fields.get(tectonics::ELEVATION_M).unwrap());
    let plate_hash = hash_u32_slice(world.fields.get_u32(tectonics::PLATE_ID).unwrap());
    let crust_hash = hash_u32_slice(world.fields.get_u32(tectonics::CRUST_TYPE).unwrap());
    let lith_hash = hash_u32_slice(world.fields.get_u32(tectonics::LITHOLOGY).unwrap());
    println!("GOLDEN_TECTONIC_ELEVATION_L6_SEED42 = {elev_hash:#018x}");
    println!("GOLDEN_TECTONIC_PLATES_L6_SEED42 = {plate_hash:#018x}");
    println!("GOLDEN_TECTONIC_CRUST_TYPE_L6_SEED42 = {crust_hash:#018x}");
    println!("GOLDEN_TECTONIC_LITHOLOGY_L6_SEED42 = {lith_hash:#018x}");
}

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
    let lith_hash = hash_u32_slice(world.fields.get_u32(tectonics::LITHOLOGY).unwrap());
    assert_eq!(
        lith_hash, GOLDEN_TECTONIC_LITHOLOGY_L6_SEED42,
        "tectonic lithology drifted from the committed golden"
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
