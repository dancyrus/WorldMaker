//! WO-0005 dev probe, extended in WO-0006 (S1: connectivity counter and
//! per-plate attached slab area; S2: the tectonic event log — every suture
//! and split carries its §3 condition record or §5 driver). Runs the
//! tectonic sim at seed "cyrus" and seed 42 (L6, 2 Gy, default params) and
//! writes docs/results/plate-physics-probe-s2-<seed>.json.
//!
//! The probe drives `SimState::step` directly with the same stepping and
//! keyframe-quantization cadence as `run_history` (the quantize round-trip
//! is replicated below from the public arrays; the elevation derive is
//! skipped because nothing in the step dynamics reads `elev` or
//! `sea_offset_m`), so the trajectory is bit-identical to what the app
//! produces. Between steps it records the plate-level state the audit
//! needs: alive counts, per-plate connected components (exclaves), and
//! speeds; events come straight from `SimState::events`.
//!
//! Run with:
//!   cargo test -p worldmaker-sim --release plate_physics_probe -- --ignored --nocapture

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;
use worldmaker_io::results::{today_utc_iso, ResultsFile};
use worldmaker_sim::tectonics::{
    MicroplateOrigin, RiftDriverKind, SimState, TectonicEvent, TectonicsParams,
};

const DT_MY: f32 = 2.0;
const STEPS_PER_KEYFRAME: u32 = 5; // 10 My at L6
const SAMPLE_EVERY_STEPS: u32 = 50; // 100 My
const SLOW_SPEED_DEG_MY: f32 = 0.05;

/// Mirror of SimState::quantize_state (private): the keyframe u16
/// round-trip run_history applies every 10 My before continuing.
fn quantize(s: &mut SimState) {
    let q_u16 = |v: f32| -> f32 { (v.round().clamp(0.0, 65_535.0) as u16) as f32 };
    let q_suture = |v: f32| -> f32 {
        if v < 0.0 {
            v // NEVER_SUTURED sentinel passes through
        } else {
            (v.round().clamp(0.0, 65_534.0) as u16) as f32
        }
    };
    for i in 0..s.crust_age.len() {
        s.crust_age[i] = q_u16(s.crust_age[i]);
        s.thickness[i] = q_u16(s.thickness[i] * 100.0) * 0.01;
        s.orogeny_age[i] = q_u16(s.orogeny_age[i]);
        s.rift_age[i] = q_u16(s.rift_age[i]);
        s.buildup[i] = q_u16(s.buildup[i] * 100.0) * 0.01;
        s.slab_since_my[i] = q_u16(s.slab_since_my[i]);
        s.suture_at_my[i] = q_suture(s.suture_at_my[i]);
    }
    for v in s.hotspot_cont_my.iter_mut() {
        *v = q_u16(*v);
    }
}

fn driver_str(d: RiftDriverKind) -> &'static str {
    match d {
        RiftDriverKind::Plume => "plume",
        RiftDriverKind::BackArc => "back_arc",
        RiftDriverKind::OpposingSlabs => "opposing_slabs",
    }
}

fn event_json(e: &TectonicEvent) -> serde_json::Value {
    match e {
        TectonicEvent::Suture {
            a,
            b,
            t,
            contact_fraction,
        } => json!({
            "event": "suture", "t_my": t, "winner_plate": a, "loser_plate": b,
            "contact_fraction_of_smaller_perimeter": contact_fraction,
        }),
        TectonicEvent::RiftStart { plate, driver, t } => json!({
            "event": "rift_start", "t_my": t, "plate": plate,
            "driver": driver_str(*driver),
        }),
        TectonicEvent::RiftFailed { plate, t } => json!({
            "event": "rift_failed", "t_my": t, "plate": plate,
        }),
        TectonicEvent::Split {
            parent,
            child,
            driver,
            t,
        } => json!({
            "event": "split", "t_my": t, "parent_plate": parent,
            "child_plate": child, "driver": driver_str(*driver),
        }),
        TectonicEvent::Microplate { id, origin, t } => json!({
            "event": "microplate", "t_my": t, "plate": id,
            "origin": match origin {
                MicroplateOrigin::TrenchTrapped => "trench_trapped",
                MicroplateOrigin::BackArcBasin => "back_arc_basin",
                MicroplateOrigin::RidgeJump => "ridge_jump",
            },
        }),
    }
}

/// Connected components of each alive plate's cell set (grid BFS, id
/// order). Returns, per plate id, the sorted-descending component sizes.
fn plate_components(grid: &Grid, plate_id: &[u32], plate_slots: usize) -> Vec<Vec<u32>> {
    let n = plate_id.len();
    let mut comps: Vec<Vec<u32>> = vec![Vec::new(); plate_slots];
    let mut visited = vec![false; n];
    let mut queue: VecDeque<u32> = VecDeque::new();
    for c0 in 0..n {
        if visited[c0] {
            continue;
        }
        let p = plate_id[c0];
        let mut size = 0u32;
        visited[c0] = true;
        queue.push_back(c0 as u32);
        while let Some(c) = queue.pop_front() {
            size += 1;
            for &nb in grid.neighbors_of(c) {
                let nbu = nb as usize;
                if !visited[nbu] && plate_id[nbu] == p {
                    visited[nbu] = true;
                    queue.push_back(nb);
                }
            }
        }
        comps[p as usize].push(size);
    }
    for c in comps.iter_mut() {
        c.sort_unstable_by(|a, b| b.cmp(a));
    }
    comps
}

fn run_probe(seed_label: &str, seed: u64, grid: &Arc<Grid>) -> serde_json::Value {
    let params = TectonicsParams {
        span_my: 2000.0,
        ..TectonicsParams::default()
    };
    let total_steps = (params.span_my / DT_MY) as u32;

    let mut sim = SimState::setup(seed, grid, &params);
    quantize(&mut sim); // t=0 keyframe round-trip, as in run_history

    let mut samples: Vec<serde_json::Value> = Vec::new();
    let mut prev_events = 0usize;
    let mut window_events = [0u64; 5]; // suture, rift_start, rift_failed, split, microplate

    let sample =
        |sim: &SimState, window_events: &[u64; 5], samples: &mut Vec<serde_json::Value>| {
            let comps = plate_components(grid, &sim.plate_id, sim.plates.len());
            let mut multi: Vec<serde_json::Value> = Vec::new();
            for (pid, sizes) in comps.iter().enumerate() {
                if sizes.len() > 1 {
                    multi.push(json!({ "plate_id": pid, "fragment_cells": sizes }));
                }
            }
            let alive: Vec<&_> = sim.plates.iter().filter(|p| p.alive).collect();
            let mean_speed = if alive.is_empty() {
                0.0
            } else {
                alive.iter().map(|p| p.speed_deg_my as f64).sum::<f64>() / alive.len() as f64
            };
            let slow = alive
                .iter()
                .filter(|p| p.speed_deg_my < SLOW_SPEED_DEG_MY)
                .count();
            // Per-plate attached slab area (the slab-pull driver).
            let attached_slab: Vec<serde_json::Value> = alive
                .iter()
                .map(|p| {
                    let area: u32 = p
                        .slab
                        .iter()
                        .filter(|s| s.attached)
                        .map(|s| s.area_cells)
                        .sum();
                    json!({
                        "plate_id": p.id,
                        "attached_slab_cells": area,
                        "speed_deg_my": p.speed_deg_my,
                    })
                })
                .collect();
            let mut cells = vec![0u32; sim.plates.len()];
            for &p in &sim.plate_id {
                cells[p as usize] += 1;
            }
            let largest_share =
                cells.iter().copied().max().unwrap_or(0) as f64 / sim.plate_id.len() as f64;
            samples.push(json!({
                "t_my": sim.t_my,
                "alive_plate_count": alive.len(),
                "suture_count_cum": sim.suture_count,
                "split_count_cum": sim.breakup_count,
                "rift_start_count_cum": sim.rift_start_count,
                "rift_failed_count_cum": sim.rift_failed_count,
                "microplate_count_cum": sim.microplate_count,
                "sutures_in_window_count": window_events[0],
                "rift_starts_in_window_count": window_events[1],
                "rift_failures_in_window_count": window_events[2],
                "splits_in_window_count": window_events[3],
                "microplates_in_window_count": window_events[4],
                "pair_timer_count": sim.collisions.len(),
                "active_rift_count": sim.rifts.len(),
                "multi_component_plate_count": multi.len(),
                "multi_component_plates": multi,
                "mean_plate_speed_deg_my": mean_speed,
                "largest_plate_cell_share": largest_share,
                "plates_below_0p05_deg_my_count": slow,
                "connectivity_reassigned_cum": sim.connectivity_reassigned,
                "attached_slab_per_plate": attached_slab,
            }));
        };

    sample(&sim, &window_events, &mut samples);

    for step_idx in 0..total_steps {
        sim.step(seed, step_idx);

        // Tally this step's events into the 100 My window counters.
        for e in &sim.events[prev_events..] {
            let slot = match e {
                TectonicEvent::Suture { .. } => 0,
                TectonicEvent::RiftStart { .. } => 1,
                TectonicEvent::RiftFailed { .. } => 2,
                TectonicEvent::Split { .. } => 3,
                TectonicEvent::Microplate { .. } => 4,
            };
            window_events[slot] += 1;
        }
        prev_events = sim.events.len();

        if (step_idx + 1) % STEPS_PER_KEYFRAME == 0 {
            quantize(&mut sim); // keyframe round-trip, as in run_history
        }
        if (step_idx + 1) % SAMPLE_EVERY_STEPS == 0 {
            sample(&sim, &window_events, &mut samples);
            window_events = [0; 5];
        }
    }

    // Headline aggregates over the run.
    let final_alive = sim.alive_plates();
    let max_multi = samples
        .iter()
        .map(|s| s["multi_component_plate_count"].as_u64().unwrap_or(0))
        .max()
        .unwrap_or(0);
    let samples_with_exclaves = samples
        .iter()
        .filter(|s| s["multi_component_plate_count"].as_u64().unwrap_or(0) > 0)
        .count();
    let min_alive = samples
        .iter()
        .map(|s| s["alive_plate_count"].as_u64().unwrap_or(0))
        .min()
        .unwrap_or(0);
    let max_alive = samples
        .iter()
        .map(|s| s["alive_plate_count"].as_u64().unwrap_or(0))
        .max()
        .unwrap_or(0);
    let per_gy = |count: u64| count as f64 / (params.span_my as f64 / 1000.0);
    // The event list is committed whole (a 2 Gy run stays in the hundreds);
    // cap defensively so a pathological run cannot bloat the results file.
    let events: Vec<serde_json::Value> = sim.events.iter().take(2000).map(event_json).collect();

    json!({
        "config": {
            "seed_text": seed_label,
            "seed_u64": seed,
            "grid_level": 6,
            "span_my": 2000.0,
            "plate_count_param": 12,
            "land_fraction": 0.29,
            "tectonic_vigor": 1.0,
            "dt_my": DT_MY,
            "sample_interval_my": SAMPLE_EVERY_STEPS as f32 * DT_MY,
        },
        "headline": {
            "final_alive_plate_count": final_alive,
            "min_sampled_alive_plate_count": min_alive,
            "max_sampled_alive_plate_count": max_alive,
            "suture_count_total": sim.suture_count,
            "split_count_total": sim.breakup_count,
            "rift_start_count_total": sim.rift_start_count,
            "rift_failed_count_total": sim.rift_failed_count,
            "microplate_count_total": sim.microplate_count,
            "sutures_per_gy": per_gy(sim.suture_count),
            "splits_per_gy": per_gy(sim.breakup_count),
            "rift_starts_per_gy": per_gy(sim.rift_start_count),
            "rift_failures_per_gy": per_gy(sim.rift_failed_count),
            "microplates_per_gy": per_gy(sim.microplate_count),
            "events_recorded": events.len(),
            "events_total": sim.events.len(),
            "max_multi_component_plates_in_sample": max_multi,
            "samples_with_exclaves_count": samples_with_exclaves,
            "connectivity_reassigned_total": sim.connectivity_reassigned,
            "sample_count": samples.len(),
        },
        "samples": samples,
        "events": events,
    })
}

/// WO-0006 S2 step 10. Ignored: dev probe, ~2×2 Gy L6 runs (minutes).
#[test]
#[ignore = "dev probe: writes docs/results/plate-physics-probe-s2-<seed>.json"]
fn plate_physics_probe() {
    let grid = Arc::new(Grid::build(6));
    let date = today_utc_iso();
    for (label, seed) in [("cyrus", seed_from_text("cyrus")), ("42", 42u64)] {
        eprintln!("probing seed {label} ({seed:#x}) ...");
        let metrics = run_probe(label, seed, &grid);
        eprintln!(
            "seed {label}: headline {}",
            serde_json::to_string_pretty(&metrics["headline"]).unwrap()
        );
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/results"))
            .join(format!("plate-physics-probe-s2-{label}.json"));
        ResultsFile::new(&date, metrics).write(&path).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
