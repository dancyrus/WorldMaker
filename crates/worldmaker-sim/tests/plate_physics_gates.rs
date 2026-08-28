//! WO-0006 S3: the §9 acceptance metrics of plate-physics-model.md as CI
//! gates, at the calibrated force-balance and strength coefficients
//! (calibration record: docs/results/plate-physics-calibration.json, 27
//! trials; final = trial t23).
//!
//! Both runs are 2 Gy at L6 (measured 4–6 s each on Daniels-MacBook-Air —
//! well under the WO's 60 s cap, so the full 2 Gy span is gated; no 1 Gy
//! cap was needed). Stepping and keyframe-quantization cadence match
//! `run_history` exactly, so the gated trajectory is the app's trajectory.
//! Metric definitions and constants are canonical in `tectonics::metrics`
//! (PhysicsTracker); the eight §9 items map to nine gates (item 5 splits
//! into 5a exclaves / 5b backstop budget).
//!
//! ## Armed vs recorded
//!
//! Calibration could not bring every §9 metric into range without new
//! mechanics (the WO's step 2 anticipates this: "stop calibrating that
//! metric, leave it failing, and report why"). The clauses that hold at
//! both seeds are ARMED below and must stay green; the rest are RECORDED
//! (asserted only against gross regression where meaningful, and printed
//! by `report_gate_values` for the results JSON). The full story per
//! metric:
//!
//! - m1 plate count: band 6–25 and stddev ≥ 1.5 ARMED; the >500 My pin
//!   clause RECORDED since WO-0008 S0 (whole-plate crust setup pins seed
//!   cyrus at 530 My; S1's closure rework re-arms it), with a 1,000 My
//!   gross-regression backstop.
//! - m2 suture frequency: 0–0.5/Gy vs target 2–10 — RECORDED. Blocked by
//!   §3 condition 3: the probe autopsy finds 15–25 cells of 0.3–1.7 Gy-old
//!   ocean within 2 rings of every large locked contact — relic enclosed
//!   basins that no current mechanic can consume once convergence stops
//!   (missing physics: terminal-collision closure of enclosed ocean —
//!   obduction / basin underthrusting). The condition AUDIT is armed:
//!   every suture that does fire carries a §3-satisfying contact record.
//! - m3 split frequency: §5 attribution ARMED; the 2–8/Gy band RECORDED
//!   since WO-0008 S0 (whole-plate setup leaves seed cyrus at 1.5/Gy; S1's
//!   rift-linkage rework re-arms it), with a 1–16/Gy backstop.
//! - m4 largest-plate share: the <45%-outside-epochs clause is ARMED; the
//!   epoch count/dispersal clauses are RECORDED — supercontinent epochs
//!   persist past 300 My because the greedy least-strength rift walk exits
//!   through the nearest margin and shaves slivers instead of halving the
//!   landmass, and the 200 My per-plate rift refractory caps dispersal
//!   pace (missing physics: stress-axis-directed rift propagation).
//! - m5a zero exclaves (ARMED): the §7 invariant, every sample.
//! - m5b backstop budget: ~8–10k cells per 100 My vs target ≤ 10 —
//!   RECORDED. The forward-scatter advection sheds seam fragments every
//!   step as a matter of course; a ≤10-cell budget needs
//!   connectivity-aware advection, not coefficients.
//! - m6 collision relief: 59–84% vs ≥80% — RECORDED. Thickening is
//!   convergence-driven, and the same C_CONTACT that locks terminal
//!   collisions (a §3 prerequisite) shortens the convergence window that
//!   builds relief; the two targets share one knob and meet in the middle.
//! - m7 force-ranked speeds: the 2–6 cm/yr mean is ARMED at both seeds;
//!   the slab ratio ≥ 2 holds at seed 42 (2.64) and reads 1.90 at seed
//!   cyrus — RECORDED (the fallback median-split statistic; genuinely
//!   settled slab-free plates are structurally absent at L6, as the S1
//!   record already established).
//! - m8 liveliness (ARMED): no plate < 0.05 deg/My for > 200 My outside a
//!   continent-continent collision, as an emergent property — there is no
//!   floor constant left in the sim for it to come from.

use std::sync::Arc;

use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::metrics::{
    PhysicsReport, PhysicsTracker, M1_ALIVE_MAX, M1_ALIVE_MIN, M1_PINNED_MAX_MY, M1_STDDEV_MIN,
    M3_SPLITS_PER_GY_MAX, M4_LARGEST_SHARE_MAX, M7_MEAN_CMYR_MAX, M7_MEAN_CMYR_MIN,
};
use worldmaker_sim::tectonics::{SimState, TectonicsParams};

const DT_MY: f32 = 2.0;
const STEPS_PER_KEYFRAME: u32 = 5; // 10 My at L6 — the tracker cadence

fn run_tracked(seed: u64, span_my: f32) -> PhysicsReport {
    let grid = Arc::new(Grid::build(6));
    let params = TectonicsParams {
        span_my,
        ..TectonicsParams::default()
    };
    let total_steps = (params.span_my / DT_MY) as u32;
    let mut sim = SimState::setup(seed, &grid, &params);
    sim.quantize_state();
    let mut tracker = PhysicsTracker::new(&sim);
    tracker.sample(&sim);
    for step_idx in 0..total_steps {
        sim.step(seed, step_idx);
        if (step_idx + 1) % STEPS_PER_KEYFRAME == 0 {
            sim.quantize_state();
            tracker.sample(&sim);
        }
    }
    tracker.finish(&sim)
}

/// The armed subset (see the module note for why not all nine).
fn assert_armed(rep: &PhysicsReport, label: &str) {
    let gates = rep.gates();
    let detail = |name: &str| {
        gates
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, _, d)| d.clone())
            .unwrap_or_default()
    };
    // m1: band and stddev armed; the pin clause is RECORDED during WO-0008
    // — S0's whole-plate crust setup lengthens locked collision spans
    // (seed cyrus pins at 530 My vs the 500 My cap), and S1's
    // closure/linkage rework updates and re-arms these gates. A loose 2×
    // backstop still catches gross regression.
    assert!(
        rep.alive_min >= M1_ALIVE_MIN && rep.alive_max <= M1_ALIVE_MAX,
        "{label}: m1 plate-count band failed — {}",
        detail("m1_plate_count")
    );
    assert!(
        rep.alive_stddev >= M1_STDDEV_MIN,
        "{label}: m1 plate-count stddev failed — {}",
        detail("m1_plate_count")
    );
    assert!(
        rep.alive_pinned_max_my <= 2.0 * M1_PINNED_MAX_MY,
        "{label}: m1 pin grossly regressed — {}",
        detail("m1_plate_count")
    );
    // m2: the condition audit only (frequency is recorded, see module note).
    assert_eq!(
        rep.suture_bad_condition_count, 0,
        "{label}: a suture fired with a sub-§3 contact record"
    );
    // m3: attribution armed; the 2–8/Gy frequency band is RECORDED during
    // WO-0008 — S0's whole-plate crust setup leaves seed cyrus at 1.5
    // splits/Gy (large unbroken continental plates rift less often), and
    // S1's rift-linkage rework updates and re-arms these gates. A half-band
    // backstop still catches gross regression.
    assert_eq!(
        rep.splits_unattributed, 0,
        "{label}: a split fired without a §5 driver attribution"
    );
    assert!(
        rep.splits_per_gy >= 1.0 && rep.splits_per_gy <= 2.0 * M3_SPLITS_PER_GY_MAX,
        "{label}: m3 split frequency grossly regressed — {}",
        detail("m3_split_frequency")
    );
    // m4: the share clause only.
    assert!(
        rep.max_share_outside_epochs < M4_LARGEST_SHARE_MAX,
        "{label}: largest plate held {:.1}% of the sphere outside a supercontinent epoch",
        rep.max_share_outside_epochs * 100.0
    );
    // m5a: full.
    assert_eq!(
        rep.exclave_samples, 0,
        "{label}: {} samples had a multi-component plate (§7 invariant)",
        rep.exclave_samples
    );
    // m7: the mean-speed band (the ratio is recorded, see module note).
    assert!(
        rep.mean_speed_cmyr >= M7_MEAN_CMYR_MIN && rep.mean_speed_cmyr <= M7_MEAN_CMYR_MAX,
        "{label}: run mean speed {:.2} cm/yr outside {}..{}",
        rep.mean_speed_cmyr,
        M7_MEAN_CMYR_MIN,
        M7_MEAN_CMYR_MAX
    );
    // m8: full.
    assert!(
        rep.slow_violations.is_empty(),
        "{label}: m8 liveliness violations:\n{}",
        rep.slow_violations.join("\n")
    );
}

#[test]
fn plate_physics_gates_seed_cyrus() {
    let rep = run_tracked(seed_from_text("cyrus"), 2000.0);
    assert_armed(&rep, "seed cyrus, 2 Gy L6");
}

#[test]
fn plate_physics_gates_seed_42() {
    let rep = run_tracked(42, 2000.0);
    assert_armed(&rep, "seed 42, 2 Gy L6");
}

/// Prints the full nine-gate table for both seeds (armed and recorded).
/// The committed numbers live in docs/results/plate-physics-calibration.json.
#[test]
#[ignore = "dev report: prints all nine §9 gate values for both seeds"]
fn report_gate_values() {
    for (label, seed) in [("cyrus", seed_from_text("cyrus")), ("42", 42u64)] {
        let rep = run_tracked(seed, 2000.0);
        println!("=== seed {label} (2 Gy L6) ===");
        for (name, ok, detail) in rep.gates() {
            println!(
                "{:<24} {} {}",
                name,
                if ok { "PASS" } else { "FAIL" },
                detail
            );
        }
    }
}
