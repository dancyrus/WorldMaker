//! WO-0005 dev probe (extended in WO-0006 S1 with the connectivity counter
//! and per-plate attached slab area): per-100-My plate-physics statistics
//! for the model audit. Runs the tectonic sim at seed "cyrus" and seed 42
//! (L6, 2 Gy, default params) and writes
//! docs/results/plate-physics-probe-s1-<seed>.json.
//!
//! The probe drives `SimState::step` directly with the same stepping and
//! keyframe-quantization cadence as `run_history` (the quantize round-trip
//! is replicated below from the public arrays; the elevation derive is
//! skipped because nothing in the step dynamics reads `elev` or
//! `sea_offset_m`), so the trajectory is bit-identical to what the app
//! produces. Between steps it records the plate-level state the audit
//! needs: alive counts, suture/breakup events with the condition that
//! fired, per-plate connected components (exclaves), and speeds.
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
use worldmaker_sim::tectonics::{SimState, TectonicsParams};

const DT_MY: f32 = 2.0;
const STEPS_PER_KEYFRAME: u32 = 5; // 10 My at L6
const SAMPLE_EVERY_STEPS: u32 = 50; // 100 My
const SUTURE_AFTER_MY: f32 = 30.0;
const SLOW_SPEED_DEG_MY: f32 = 0.05;
const PLATE_FLOOR: usize = 6;
const BREAKUP_AREA_FRACTION: f32 = 1.0 / 3.0;

/// Mirror of SimState::quantize_state (private): the keyframe u16
/// round-trip run_history applies every 10 My before continuing.
fn quantize(s: &mut SimState) {
    let q_u16 = |v: f32| -> f32 { (v.round().clamp(0.0, 65_535.0) as u16) as f32 };
    for i in 0..s.crust_age.len() {
        s.crust_age[i] = q_u16(s.crust_age[i]);
        s.thickness[i] = q_u16(s.thickness[i] * 100.0) * 0.01;
        s.orogeny_age[i] = q_u16(s.orogeny_age[i]);
        s.rift_age[i] = q_u16(s.rift_age[i]);
        s.buildup[i] = q_u16(s.buildup[i] * 100.0) * 0.01;
        s.slab_since_my[i] = q_u16(s.slab_since_my[i]);
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

fn counts_per_plate(plate_id: &[u32], crust_type: &[u32], slots: usize) -> (Vec<u32>, Vec<u32>) {
    let mut cells = vec![0u32; slots];
    let mut cont = vec![0u32; slots];
    for (c, &p) in plate_id.iter().enumerate() {
        cells[p as usize] += 1;
        if crust_type[c] == 1 {
            cont[p as usize] += 1;
        }
    }
    (cells, cont)
}

fn run_probe(seed_label: &str, seed: u64, grid: &Arc<Grid>) -> serde_json::Value {
    let params = TectonicsParams {
        span_my: 2000.0,
        ..TectonicsParams::default()
    };
    let total_steps = (params.span_my / DT_MY) as u32;
    let n = grid.cell_count() as usize;

    let mut sim = SimState::setup(seed, grid, &params);
    quantize(&mut sim); // t=0 keyframe round-trip, as in run_history

    let mut samples: Vec<serde_json::Value> = Vec::new();
    let mut suture_events: Vec<serde_json::Value> = Vec::new();
    let mut breakup_events: Vec<serde_json::Value> = Vec::new();
    let mut prev_sutures = 0u64;
    let mut prev_breakups = 0u64;
    // Per-trigger breakup tallies (all events, not just the recorded ones).
    let mut trig_area = 0u64;
    let mut trig_cont = 0u64;
    let mut trig_gridlock = 0u64;
    let mut trig_unattributed = 0u64;
    let mut window_sutures = 0u64;
    let mut window_breakups = 0u64;

    let sample = |sim: &SimState,
                  window_sutures: u64,
                  window_breakups: u64,
                  samples: &mut Vec<serde_json::Value>| {
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
        let slow_at_floor = alive
            .iter()
            .filter(|p| p.speed_deg_my <= SLOW_SPEED_DEG_MY + 1e-6)
            .count();
        // WO-0006 S1: per-plate attached slab area (the slab-pull driver).
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
        let (cells, _) = counts_per_plate(&sim.plate_id, &sim.crust_type, sim.plates.len());
        let largest_share =
            cells.iter().copied().max().unwrap_or(0) as f64 / sim.plate_id.len() as f64;
        samples.push(json!({
            "t_my": sim.t_my,
            "alive_plate_count": alive.len(),
            "suture_count_cum": sim.suture_count,
            "breakup_count_cum": sim.breakup_count,
            "sutures_in_window_count": window_sutures,
            "breakups_in_window_count": window_breakups,
            "pair_timer_count": sim.collisions.len(),
            "multi_component_plate_count": multi.len(),
            "multi_component_plates": multi,
            "mean_plate_speed_deg_my": mean_speed,
            "largest_plate_cell_share": largest_share,
            "plates_below_0p05_deg_my_count": slow,
            "plates_at_or_below_0p05_deg_my_count": slow_at_floor,
            "connectivity_reassigned_cum": sim.connectivity_reassigned,
            "attached_slab_per_plate": attached_slab,
        }));
    };

    sample(&sim, 0, 0, &mut samples);

    for step_idx in 0..total_steps {
        // Pre-step snapshot for event attribution.
        let pre_t = sim.t_my;
        let pre_alive: Vec<bool> = sim.plates.iter().map(|p| p.alive).collect();
        let pre_suture_my: Vec<f32> = sim.plates.iter().map(|p| p.youngest_suture_my).collect();
        let pre_plate_slots = sim.plates.len();
        let pre_alive_count = pre_alive.iter().filter(|&&a| a).count();
        let pre_timers: Vec<(u32, u32, f32)> = sim
            .collisions
            .iter()
            .map(|t| (t.a, t.b, t.slow_collision_my))
            .collect();
        let (pre_cells, pre_cont) =
            counts_per_plate(&sim.plate_id, &sim.crust_type, pre_plate_slots);

        sim.step(seed, step_idx);

        // Suture fired this step?
        if sim.suture_count > prev_sutures {
            // The step consumes the first matured pair in timer order.
            let fired = pre_timers
                .iter()
                .find(|&&(_, _, t)| t >= SUTURE_AFTER_MY)
                .copied();
            if let Some((a, b, timer)) = fired {
                let (winner, loser) = if pre_cells[a as usize] >= pre_cells[b as usize] {
                    (a, b)
                } else {
                    (b, a)
                };
                if suture_events.len() < 80 {
                    suture_events.push(json!({
                        "t_my": pre_t,
                        "winner_plate": winner,
                        "loser_plate": loser,
                        "pair_timer_my": timer,
                        "winner_cells": pre_cells[winner as usize],
                        "loser_cells": pre_cells[loser as usize],
                        "pre_alive_count": pre_alive_count,
                    }));
                }
            }
            window_sutures += sim.suture_count - prev_sutures;
            prev_sutures = sim.suture_count;
        }

        // Breakup fired this step?
        if sim.breakup_count > prev_breakups {
            let new_id = pre_plate_slots; // plates only grow by breakup
                                          // The split plate: its youngest_suture_my was reset to pre_t.
            let split = (0..pre_plate_slots).find(|&pid| {
                sim.plates[pid].alive
                    && sim.plates[pid].youngest_suture_my == pre_t
                    && pre_suture_my[pid] != pre_t
            });
            if let Some(pid) = split {
                let area_threshold = (n as f32 * BREAKUP_AREA_FRACTION) as u32;
                let cont_total: u32 = pre_cont.iter().sum();
                let cont_threshold = (cont_total as f32 * BREAKUP_AREA_FRACTION) as u32;
                let by_area = pre_cells[pid] > area_threshold;
                let by_cont_share = cont_total > n as u32 / 20 && pre_cont[pid] > cont_threshold;
                // The step runs suture timers BEFORE maybe_breakup, so a
                // timer can mature mid-step: gridlock is also attributed
                // when a pre-step timer sits one step (2 My) below the
                // threshold.
                let gridlock = pre_alive_count <= PLATE_FLOOR
                    && pre_timers
                        .iter()
                        .any(|&(_, _, t)| t >= SUTURE_AFTER_MY - DT_MY);
                let trigger = if by_area {
                    "area_over_third_of_sphere"
                } else if by_cont_share {
                    "continental_share_over_third"
                } else if gridlock {
                    "gridlock_breaker_at_plate_floor"
                } else {
                    "unattributed"
                };
                match trigger {
                    "area_over_third_of_sphere" => trig_area += 1,
                    "continental_share_over_third" => trig_cont += 1,
                    "gridlock_breaker_at_plate_floor" => trig_gridlock += 1,
                    _ => trig_unattributed += 1,
                }
                if breakup_events.len() < 80 {
                    breakup_events.push(json!({
                        "t_my": pre_t,
                        "split_plate": pid,
                        "new_plate": new_id,
                        "trigger": trigger,
                        "plate_cells": pre_cells[pid],
                        "area_threshold_cells": area_threshold,
                        "plate_cont_cells": pre_cont[pid],
                        "cont_threshold_cells": cont_threshold,
                        "suture_age_my": pre_t - pre_suture_my[pid],
                        "pre_alive_count": pre_alive_count,
                    }));
                }
            }
            window_breakups += sim.breakup_count - prev_breakups;
            prev_breakups = sim.breakup_count;
        }

        if (step_idx + 1) % STEPS_PER_KEYFRAME == 0 {
            quantize(&mut sim); // keyframe round-trip, as in run_history
        }
        if (step_idx + 1) % SAMPLE_EVERY_STEPS == 0 {
            sample(&sim, window_sutures, window_breakups, &mut samples);
            window_sutures = 0;
            window_breakups = 0;
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
            "suture_count_total": sim.suture_count,
            "breakup_count_total": sim.breakup_count,
            "breakups_by_area_count": trig_area,
            "breakups_by_cont_share_count": trig_cont,
            "breakups_by_gridlock_breaker_count": trig_gridlock,
            "breakups_unattributed_count": trig_unattributed,
            "max_multi_component_plates_in_sample": max_multi,
            "samples_with_exclaves_count": samples_with_exclaves,
            "connectivity_reassigned_total": sim.connectivity_reassigned,
            "sample_count": samples.len(),
        },
        "samples": samples,
        "suture_events": suture_events,
        "breakup_events": breakup_events,
    })
}

/// WO-0005 step 4. Ignored: dev probe, ~2×2 Gy L6 runs (minutes).
#[test]
#[ignore = "dev probe: writes docs/results/plate-physics-probe-s1-<seed>.json"]
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
            .join(format!("plate-physics-probe-s1-{label}.json"));
        ResultsFile::new(&date, metrics).write(&path).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
