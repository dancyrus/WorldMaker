//! WO-0003 Fix 4: plate-liveliness gates.
//!
//! Measured on v0.2.0 (Dan's report, reproduced by the ignored probe below):
//! plates froze permanently — ownership overlap 1.000 over 1.6 Gy and final
//! speeds of exactly 0.00 deg/My. These tests are the gate class Phase 1
//! missed: no alive plate may keep near-identical cell ownership across a
//! 300 My window (outside a young continent-continent collision), and no
//! alive plate may hold speed < 0.05 deg/My for more than 200 My.

use std::sync::Arc;

use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{self, Keyframe, TectonicsHistory, TectonicsParams};
use worldmaker_sim::{StageContext, WorldState};

fn run_history(level: u32, seed: u64, params: TectonicsParams) -> TectonicsHistory {
    let grid = Arc::new(Grid::build(level));
    let world = WorldState::new(grid);
    tectonics::run_history(&StageContext::new(seed), &world, &params, None).unwrap()
}

/// Cells owned by `plate` in `a` that are still owned by it in `b`, divided
/// by the count in `a` (0 cells in `a` → overlap 0).
fn ownership_overlap(a: &Keyframe, b: &Keyframe, plate: u16) -> (f64, u32) {
    let (mut start, mut kept) = (0u32, 0u32);
    for c in 0..a.plate_id.len() {
        if a.plate_id[c] == plate {
            start += 1;
            if b.plate_id[c] == plate {
                kept += 1;
            }
        }
    }
    let overlap = if start == 0 {
        0.0
    } else {
        kept as f64 / start as f64
    };
    (overlap, start)
}

const OVERLAP_WINDOW_MY: f32 = 300.0;
const OVERLAP_MAX: f64 = 0.985;
const MIN_PLATE_CELLS: u32 = 50;
const SPEED_FLOOR_GATE: f32 = 0.05;
const SPEED_FLOOR_MAX_MY: f32 = 200.0;
const SUTURE_AFTER_MY: f32 = 30.0;

/// Gate 7.1: alive plates of >= MIN_PLATE_CELLS cells must not keep
/// ownership overlap >= OVERLAP_MAX across any 300 My window — unless the
/// plate sits in a continent-continent collision younger than
/// SUTURE_AFTER_MY at the window's end (a fresh jam is allowed to hold
/// still until the suture rule resolves it). Collision age is measured as
/// the consecutive keyframe run over which the plate has had a pair timer.
fn overlap_violations(hist: &TectonicsHistory) -> Vec<String> {
    let kf_my = hist.keyframe_interval_my;
    let per_window = (OVERLAP_WINDOW_MY / kf_my) as usize;
    // Age (My) of each plate's oldest continuous continent-continent
    // contact, per keyframe.
    let np = hist
        .keyframes
        .iter()
        .map(|kf| kf.plates.len())
        .max()
        .unwrap();
    let mut contact_age = vec![vec![0.0f32; np]; hist.keyframes.len()];
    for (k, kf) in hist.keyframes.iter().enumerate() {
        for t in &kf.collisions {
            for pid in [t.a as usize, t.b as usize] {
                let prev = if k == 0 {
                    0.0
                } else {
                    contact_age[k - 1][pid]
                };
                contact_age[k][pid] = contact_age[k][pid].max(prev + kf_my);
            }
        }
    }
    let mut out = Vec::new();
    for k in 0..hist.keyframes.len().saturating_sub(per_window) {
        let a = &hist.keyframes[k];
        let b = &hist.keyframes[k + per_window];
        for p in &b.plates {
            if !p.alive || !a.plates.iter().any(|q| q.id == p.id && q.alive) {
                continue;
            }
            let (ov, start) = ownership_overlap(a, b, p.id as u16);
            if start < MIN_PLATE_CELLS || ov < OVERLAP_MAX {
                continue;
            }
            let age = contact_age[k + per_window][p.id as usize];
            if age > 0.0 && age <= SUTURE_AFTER_MY {
                continue; // fresh collision: the suture rule owns this
            }
            out.push(format!(
                "plate {} overlap {:.3} over {}..{} My ({} cells, speed {:.2}, coll {}, contact age {} My)",
                p.id, ov, a.t_my, b.t_my, start, p.speed_deg_my, p.colliding_cells, age
            ));
        }
    }
    out
}

/// Gate 7.2: no alive plate holds speed < SPEED_FLOOR_GATE for more than
/// SPEED_FLOOR_MAX_MY contiguous.
fn speed_violations(hist: &TectonicsHistory) -> Vec<String> {
    let kf_my = hist.keyframe_interval_my;
    let np = hist
        .keyframes
        .iter()
        .map(|kf| kf.plates.len())
        .max()
        .unwrap();
    let mut slow_my = vec![0.0f32; np];
    let mut out = Vec::new();
    let mut flagged = vec![false; np];
    for kf in &hist.keyframes {
        for p in &kf.plates {
            let pid = p.id as usize;
            if !p.alive {
                slow_my[pid] = 0.0;
                continue;
            }
            if p.speed_deg_my < SPEED_FLOOR_GATE {
                slow_my[pid] += kf_my;
                if slow_my[pid] > SPEED_FLOOR_MAX_MY && !flagged[pid] {
                    flagged[pid] = true;
                    out.push(format!(
                        "plate {} below {} deg/My for {} My ending {} My (speed {:.3})",
                        pid, SPEED_FLOOR_GATE, slow_my[pid], kf.t_my, p.speed_deg_my
                    ));
                }
            } else {
                slow_my[pid] = 0.0;
            }
        }
    }
    out
}

/// Step-1 probe: per-plate ownership overlap and speed across 200 My
/// windows at defaults, 2 Gy, L6, seed 42. Run with
/// `cargo test -p worldmaker-sim --release probe_freeze -- --ignored --nocapture`.
#[test]
#[ignore = "dev probe: prints per-plate overlap/speed tables"]
fn probe_freeze_report() {
    let params = TectonicsParams {
        span_my: 2000.0,
        ..TectonicsParams::default()
    };
    let hist = run_history(6, 42, params);
    let per_window = (200.0 / hist.keyframe_interval_my) as usize;
    println!("keyframes: {}", hist.keyframes.len());
    for w in 0..(hist.keyframes.len() - 1) / per_window {
        let a = &hist.keyframes[w * per_window];
        let b = &hist.keyframes[(w + 1) * per_window];
        println!("--- window {} .. {} My ---", a.t_my, b.t_my);
        for p in &b.plates {
            if !p.alive {
                continue;
            }
            let (ov, start) = ownership_overlap(a, b, p.id as u16);
            println!(
                "plate {:2}: overlap {:.3} ({:5} cells) speed {:.2} deg/My coll={}",
                p.id, ov, start, p.speed_deg_my, p.colliding_cells
            );
        }
        let timers: Vec<String> = b
            .collisions
            .iter()
            .map(|t| format!("({},{})={:.0}", t.a, t.b, t.slow_collision_my))
            .collect();
        println!("pair timers: {}", timers.join(" "));
    }
    println!("=== gate 7.1 (overlap) violations ===");
    for v in overlap_violations(&hist) {
        println!("{v}");
    }
    println!("=== gate 7.2 (speed) violations ===");
    for v in speed_violations(&hist) {
        println!("{v}");
    }
}
