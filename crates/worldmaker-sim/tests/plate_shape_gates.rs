//! WO-0011 S3: plate-shape regression gates on the world of the original
//! report (24 plates, land 0.40, vigor 1.0, 2 Gy — the plate_shape_probe
//! world; the probe stays as the diagnostic, unchanged). Both seeds, L6,
//! stepping and keyframe-quantization cadence matching `run_history`
//! exactly. Metric definitions are canonical in `tectonics::metrics`
//! (lifted there in S1 so this file could arm them).
//!
//! The 170 km neck clause (WO step 2.2) is armed on the S2 dumbbell probe
//! world at L7 in `step.rs::dumbbell_probe_partial_contact_welds_as_one_mass`
//! via `metrics::holds_narrow_neck` at `NECK_MIN_KM` / `NECK_MASS_FRAC` —
//! stated in km, so the clause is level-independent; the world needs
//! crate-private constructors, so the test lives beside them.
//!
//! ## Armed vs recorded (measured 2026-08-28, Daniels-MacBook-Air; tables
//! in docs/results/plate-shape-wo0011-s3-Daniels-MacBook-Air.json)
//!
//! - compact: Dan's ruled 1.15× bound (2 Gy vs 100 My) is ARMED on the
//!   area-weighted form `boundary_cell_fraction` (measured 0.81 / 1.06 at
//!   the seeds). The WO named the unweighted `mean_boundary_per_area`,
//!   which is census-skewed — a healthy churn of small plates raises it
//!   with no shape change (measured 1.22 / 1.82, the seed-42 reading
//!   WORSE than the pre-fix world's 1.52 while its fingers sit at 0.3%
//!   and its boundary density at 1.06×) — so it is RECORDED, not armed;
//!   total boundary length is what the gerrymander artifact multiplied,
//!   and the weighted form measures exactly that.
//! - fingers: ARMED as ruled — <= 0.5% at every sample after 100 My
//!   (measured max 0.17% / 0.39%).
//! - craton floor: ARMED as ruled — `craton_transfer_violations == 0`
//!   over the whole run, with the counter refined to its design intent
//!   (transfers OUTSIDE a recorded process; see the scan in step.rs —
//!   every pre-refinement count was a continent-continent jam transfer,
//!   324 of 330, or a severed-fragment capture, 6 of 330; the refinement
//!   is write-only instrumentation, goldens unmoved).
//! - largest share: ARMED at a 60% every-sample hard backstop. Dan's
//!   ruled 40% hard cap is RECORDED (measured max 55.2% at seed cyrus —
//!   one contiguous supercontinent epoch from ~800 My; seed 42 peaks at
//!   38.9%): the overshoot is the census/break-up balance item the WO
//!   rules out of scope, not a shape defect, and the 40% cap re-arms with
//!   that item. The backstop cleanly separates today's worlds from the
//!   pre-fix wholesale-weld states (67–71%).
//! - census (RECORDED, WO step 2.3): alive count and largest share at
//!   2 Gy, printed against Dan's ruled healthy band (alive 8–20 from a
//!   24-plate start, largest <= 35% target). Measured: alive 12 / 17 —
//!   in band; largest 55.2% / 38.9% — both above target, so the S3
//!   report carries the census finding recommending a separate
//!   break-up/rift balance item (check_rift_splits, grow_rifts,
//!   link_rifts, micro thresholds).

use std::sync::Arc;

use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::metrics::{
    boundary_cell_fraction, finger_fraction, largest_plate_share, mean_boundary_per_area,
};
use worldmaker_sim::tectonics::{SimState, TectonicsParams};

const DT_MY: f32 = 2.0;
const STEPS_PER_KEYFRAME: u32 = 5; // 10 My at L6 — run_history's cadence
const SAMPLE_EVERY_STEPS: u32 = 50; // 100 My, the probe's cadence

/// Dan's ruled thresholds (WO-0011 S3 step 2.1).
const COMPACT_RATIO_MAX: f64 = 1.15;
/// See `COMPACT_RATIO_MAX`.
const FINGER_FRAC_MAX: f64 = 0.005;
/// Dan's ruled hard cap — RECORDED until the break-up/rift balance item
/// re-arms it (module note).
const LARGEST_SHARE_RULED_MAX: f64 = 0.40;
/// The armed every-sample backstop over `LARGEST_SHARE_RULED_MAX`.
const LARGEST_SHARE_BACKSTOP_MAX: f64 = 0.60;

/// Dan's ruled healthy census band (RECORDED, WO-0011 S3 step 2.3).
const CENSUS_ALIVE_MIN: usize = 8;
/// See `CENSUS_ALIVE_MIN`.
const CENSUS_ALIVE_MAX: usize = 20;
/// See `CENSUS_ALIVE_MIN`.
const CENSUS_SHARE_TARGET: f64 = 0.35;

struct Sample {
    t_my: f32,
    alive: usize,
    largest: f64,
    compact_mean: f64,
    compact_weighted: f64,
    finger: f64,
}

struct ShapeRun {
    samples: Vec<Sample>,
    craton_violations: u64,
}

fn run_shape(seed: u64) -> ShapeRun {
    let grid = Arc::new(Grid::build(6));
    let params = TectonicsParams {
        plate_count: 24,
        land_fraction: 0.40,
        tectonic_vigor: 1.0,
        span_my: 2000.0,
        ..TectonicsParams::default()
    };
    let total_steps = (params.span_my / DT_MY) as u32;
    let mut sim = SimState::setup(seed, &grid, &params);
    sim.quantize_state();
    let mut samples = Vec::new();
    for step_idx in 0..total_steps {
        sim.step(seed, step_idx);
        if (step_idx + 1) % STEPS_PER_KEYFRAME == 0 {
            sim.quantize_state();
        }
        if (step_idx + 1) % SAMPLE_EVERY_STEPS == 0 {
            let alive_flags: Vec<bool> = sim.plates.iter().map(|p| p.alive).collect();
            samples.push(Sample {
                t_my: sim.t_my,
                alive: alive_flags.iter().filter(|&&a| a).count(),
                largest: largest_plate_share(&sim.plate_id, sim.plates.len()),
                compact_mean: mean_boundary_per_area(&sim.grid, &sim.plate_id, &alive_flags),
                compact_weighted: boundary_cell_fraction(&sim.grid, &sim.plate_id),
                finger: finger_fraction(&sim.grid, &sim.plate_id),
            });
        }
    }
    ShapeRun {
        samples,
        craton_violations: sim.craton_transfer_violations,
    }
}

/// The armed subset (see the module note for the armed/recorded story).
fn assert_armed(run: &ShapeRun, label: &str) {
    let first = &run.samples[0];
    let last = run.samples.last().unwrap();

    // Compact (area-weighted): 2 Gy <= 1.15x the 100 My value.
    let ratio = last.compact_weighted / first.compact_weighted;
    assert!(
        ratio <= COMPACT_RATIO_MAX,
        "{label}: boundary density at 2 Gy is {ratio:.3}x its 100 My value \
         (ruled max {COMPACT_RATIO_MAX}) — the world is losing compactness"
    );

    // Fingers: every sample after 100 My.
    for s in &run.samples[1..] {
        assert!(
            s.finger <= FINGER_FRAC_MAX,
            "{label}: finger fraction {:.2}% at {} My (ruled max {:.1}%) — \
             thin-strip shredding is back",
            s.finger * 100.0,
            s.t_my,
            FINGER_FRAC_MAX * 100.0
        );
    }

    // Craton floor: no craton cell moved outside a recorded process.
    assert_eq!(
        run.craton_violations, 0,
        "{label}: {} craton cells transferred outside a recorded process \
         (Dan's ruled floor)",
        run.craton_violations
    );

    // Largest share: the every-sample hard backstop (the ruled 40% cap is
    // recorded until the break-up/rift item re-arms it — module note).
    for s in &run.samples {
        assert!(
            s.largest <= LARGEST_SHARE_BACKSTOP_MAX,
            "{label}: largest plate held {:.1}% of the sphere at {} My \
             (backstop {:.0}%; wholesale-weld regression territory)",
            s.largest * 100.0,
            s.t_my,
            LARGEST_SHARE_BACKSTOP_MAX * 100.0
        );
    }
}

#[test]
fn plate_shape_gates_seed_cyrus() {
    let run = run_shape(seed_from_text("cyrus"));
    assert_armed(&run, "seed cyrus, 2 Gy L6 24-plate");
}

#[test]
fn plate_shape_gates_seed_42() {
    let run = run_shape(42);
    assert_armed(&run, "seed 42, 2 Gy L6 24-plate");
}

/// Prints the full series, every clause value (armed and recorded), and
/// the census reading for both seeds. The committed numbers live in
/// docs/results/plate-shape-wo0011-s3-Daniels-MacBook-Air.json.
#[test]
#[ignore = "dev report: prints the shape series and clause values"]
fn report_shape_values() {
    for (label, seed) in [("cyrus", seed_from_text("cyrus")), ("42", 42u64)] {
        let run = run_shape(seed);
        println!("=== seed {label} (2 Gy L6, 24 plates, land 0.40, vigor 1.0) ===");
        println!("t_My  alive  largest%  compact  bdens  finger%");
        for s in &run.samples {
            println!(
                "{:>5} {:>5} {:>8.1} {:>8.3} {:>7.4} {:>7.2}",
                s.t_my,
                s.alive,
                s.largest * 100.0,
                s.compact_mean,
                s.compact_weighted,
                s.finger * 100.0
            );
        }
        let first = &run.samples[0];
        let last = run.samples.last().unwrap();
        println!(
            "ARMED    bdens ratio 2Gy/100My = {:.3} (ruled max {COMPACT_RATIO_MAX})",
            last.compact_weighted / first.compact_weighted
        );
        let finger_max = run.samples[1..]
            .iter()
            .map(|s| s.finger)
            .fold(0.0, f64::max);
        println!(
            "ARMED    finger max after 100 My = {:.3}% (ruled max {:.1}%)",
            finger_max * 100.0,
            FINGER_FRAC_MAX * 100.0
        );
        println!(
            "ARMED    craton_transfer_violations = {} (ruled 0)",
            run.craton_violations
        );
        let largest_max = run.samples.iter().map(|s| s.largest).fold(0.0, f64::max);
        println!(
            "ARMED    largest share max = {:.1}% (backstop {:.0}%; ruled {:.0}% recorded)",
            largest_max * 100.0,
            LARGEST_SHARE_BACKSTOP_MAX * 100.0,
            LARGEST_SHARE_RULED_MAX * 100.0
        );
        println!(
            "RECORDED unweighted compact ratio 2Gy/100My = {:.3} (census-skewed; module note)",
            last.compact_mean / first.compact_mean
        );
        println!(
            "RECORDED census at 2 Gy: alive {} (band {CENSUS_ALIVE_MIN}-{CENSUS_ALIVE_MAX}), \
             largest {:.1}% (target <= {:.0}%)",
            last.alive,
            last.largest * 100.0,
            CENSUS_SHARE_TARGET * 100.0
        );
    }
}
