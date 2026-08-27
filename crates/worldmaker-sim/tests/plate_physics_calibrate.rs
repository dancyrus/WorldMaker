//! WO-0006 S3 calibration harness: runs the tectonic sim at seeds `cyrus`
//! and 42 (L6, 2 Gy, defaults) with the §9 metrics tracker and prints one
//! machine-readable JSON line per seed (prefix `CALIB_JSON `). The
//! calibration driver sweeps the force-balance and strength coefficients in
//! `step.rs` between runs and records every trial in
//! `docs/results/plate-physics-calibration.json`.
//!
//! Stepping and keyframe-quantization cadence match `run_history` exactly
//! (the elevation derive is skipped — nothing in the step dynamics reads
//! it), so the trajectory is bit-identical to what the app produces.
//!
//! Run with:
//!   cargo test -p worldmaker-sim --release plate_physics_calibrate -- --ignored --nocapture

use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::metrics::PhysicsTracker;
use worldmaker_sim::tectonics::{RiftDriverKind, SimState, TectonicEvent, TectonicsParams};

const DT_MY: f32 = 2.0;
const STEPS_PER_KEYFRAME: u32 = 5; // 10 My at L6 — the tracker cadence too

fn run_seed(label: &str, seed: u64, grid: &Arc<Grid>, span_my: f32) -> serde_json::Value {
    let params = TectonicsParams {
        span_my,
        ..TectonicsParams::default()
    };
    let total_steps = (params.span_my / DT_MY) as u32;
    let t0 = Instant::now();
    let mut sim = SimState::setup(seed, grid, &params);
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
    let rep = tracker.finish(&sim);
    let wall_s = t0.elapsed().as_secs_f64();

    // Suture-blocker autopsy: the largest continent-continent contact at
    // run end — how big, how fast, and what ocean sits within 2 rings.
    let autopsy = suture_autopsy(&sim);
    let cont_share: Vec<serde_json::Value> = rep
        .cont_share_series
        .iter()
        .step_by(10)
        .map(|&(t, s)| json!([t, (s * 1000.0).round() / 1000.0]))
        .collect();
    let mut split_by = [0u32; 3];
    let mut start_by = [0u32; 3];
    for e in &sim.events {
        let slot = |d: &RiftDriverKind| match d {
            RiftDriverKind::Plume => 0,
            RiftDriverKind::BackArc => 1,
            RiftDriverKind::OpposingSlabs => 2,
        };
        match e {
            TectonicEvent::Split { driver, .. } => split_by[slot(driver)] += 1,
            TectonicEvent::RiftStart { driver, .. } => start_by[slot(driver)] += 1,
            _ => {}
        }
    }
    let gates: serde_json::Map<String, serde_json::Value> = rep
        .gates()
        .iter()
        .map(|(name, ok, detail)| ((*name).to_owned(), json!({ "pass": ok, "detail": detail })))
        .collect();
    json!({
        "seed_text": label,
        "seed_u64": seed,
        "span_my": rep.span_my,
        "wall_s": wall_s,
        "pass_all": rep.pass(),
        "gates": gates,
        "values": {
            "alive_min": rep.alive_min,
            "alive_max": rep.alive_max,
            "alive_stddev": rep.alive_stddev,
            "alive_pinned_max_my": rep.alive_pinned_max_my,
            "sutures_per_gy": rep.sutures_per_gy,
            "suture_bad_condition_count": rep.suture_bad_condition_count,
            "splits_per_gy": rep.splits_per_gy,
            "splits_unattributed": rep.splits_unattributed,
            "max_share_outside_epochs": rep.max_share_outside_epochs,
            "supercontinent_epochs": rep.supercontinent_epochs,
            "longest_epoch_my": rep.longest_epoch_my,
            "open_epoch_my": rep.open_epoch_my,
            "exclave_samples": rep.exclave_samples,
            "backstop_max_per_100my": rep.backstop_max_per_100my,
            "relief_episodes": rep.relief_episodes,
            "relief_reached": rep.relief_reached,
            "mean_speed_cmyr": rep.mean_speed_cmyr,
            "slab_attached_mean_cmyr": rep.slab_attached_mean_cmyr,
            "slab_free_mean_cmyr": rep.slab_free_mean_cmyr,
            "slab_free_plate_samples": rep.slab_free_plate_samples,
            "slab_free_cont_mean_cmyr": rep.slab_free_cont_mean_cmyr,
            "slab_free_cont_plate_samples": rep.slab_free_cont_plate_samples,
            "slow_violation_count": rep.slow_violations.len(),
        },
        "diagnostics": {
            "splits_by_driver_pbo": split_by,
            "rift_starts_by_driver_pbo": start_by,
            "suture_fail_extent": sim.suture_fail_extent,
            "suture_fail_lock": sim.suture_fail_lock,
            "suture_fail_ocean": sim.suture_fail_ocean,
            "pair_timer_final": sim.collisions.len(),
            "suture_count": sim.suture_count,
            "split_count": sim.breakup_count,
            "rift_start_count": sim.rift_start_count,
            "rift_failed_count": sim.rift_failed_count,
            "microplate_count": sim.microplate_count,
            "connectivity_reassigned": sim.connectivity_reassigned,
            "final_alive": sim.alive_plates(),
            "largest_pair_autopsy": autopsy,
            "cont_share_every_100my": cont_share,
        },
    })
}

/// The largest cc contact pair at run end: contact cells, the smaller
/// plate's perimeter fraction, and the ocean within 2 rings (count + ages).
fn suture_autopsy(sim: &SimState) -> serde_json::Value {
    use std::collections::VecDeque;
    let n = sim.plate_id.len();
    let mut pairs: Vec<(u32, u32, Vec<u32>)> = Vec::new();
    for c in 0..n {
        if sim.crust_type[c] != 1 {
            continue;
        }
        let pc = sim.plate_id[c];
        for &nb in sim.grid.neighbors_of(c as u32) {
            let nbu = nb as usize;
            let pn = sim.plate_id[nbu];
            if pn == pc || sim.crust_type[nbu] != 1 {
                continue;
            }
            let (a, b) = (pc.min(pn), pc.max(pn));
            match pairs.iter_mut().find(|e| e.0 == a && e.1 == b) {
                Some(e) => {
                    if !e.2.contains(&(c as u32)) {
                        e.2.push(c as u32);
                    }
                }
                None => pairs.push((a, b, vec![c as u32])),
            }
        }
    }
    let Some((a, b, cells)) = pairs.into_iter().max_by_key(|e| e.2.len()) else {
        return json!(null);
    };
    let mut depth = vec![u16::MAX; n];
    let mut queue: VecDeque<u32> = VecDeque::new();
    for &c in &cells {
        depth[c as usize] = 0;
        queue.push_back(c);
    }
    let (mut ocean, mut young_ocean) = (0u32, 0u32);
    let mut age_sum = 0.0f64;
    while let Some(c) = queue.pop_front() {
        let dc = depth[c as usize];
        if dc >= 2 {
            continue;
        }
        for &nb in sim.grid.neighbors_of(c) {
            let nbu = nb as usize;
            let p = sim.plate_id[nbu];
            if depth[nbu] == u16::MAX && (p == a || p == b) {
                depth[nbu] = dc + 1;
                queue.push_back(nb);
                if sim.crust_type[nbu] == 0 {
                    ocean += 1;
                    age_sum += sim.crust_age[nbu] as f64;
                    if sim.crust_age[nbu] < 60.0 {
                        young_ocean += 1;
                    }
                }
            }
        }
    }
    json!({
        "pair": [a, b],
        "contact_cells": cells.len(),
        "smaller_perimeter": sim.plates[a as usize].boundary_cells
            .min(sim.plates[b as usize].boundary_cells),
        "ocean_within_2_rings": ocean,
        "young_ocean_lt60my": young_ocean,
        "mean_ocean_age_my": if ocean > 0 { age_sum / ocean as f64 } else { 0.0 },
    })
}

/// WO-0006 keyframe-budget re-measure (decision log, S2 note g): the
/// 22 B/cell keyframe at L7 × a 2 Gy history (201 keyframes at the 10 My
/// cadence). Prints the measured t=0 keyframe size and the 2 Gy projection.
#[test]
#[ignore = "dev measure: prints the L7 keyframe budget"]
fn keyframe_budget_l7() {
    let grid = Arc::new(Grid::build(7));
    let sim = SimState::setup(seed_from_text("cyrus"), &grid, &TectonicsParams::default());
    let kf = sim.encode_keyframe();
    let bytes = kf.approx_bytes();
    let total = bytes as f64 * 201.0;
    println!(
        "KEYFRAME_BUDGET_JSON {}",
        json!({
            "l7_keyframe_bytes": bytes,
            "l7_bytes_per_cell": bytes as f64 / grid.cell_count() as f64,
            "l7_2gy_201_keyframes_gb": total / 1e9,
        })
    );
}

/// WO-0006 S3 step 2. Ignored: calibration driver harness (~2×2 Gy L6).
#[test]
#[ignore = "calibration harness: prints CALIB_JSON lines for the S3 driver"]
fn plate_physics_calibrate() {
    let grid = Arc::new(Grid::build(6));
    let span_my: f32 = std::env::var("WM_CALIB_SPAN_MY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000.0);
    for (label, seed) in [("cyrus", seed_from_text("cyrus")), ("42", 42u64)] {
        let v = run_seed(label, seed, &grid, span_my);
        println!("CALIB_JSON {}", serde_json::to_string(&v).unwrap());
    }
}
