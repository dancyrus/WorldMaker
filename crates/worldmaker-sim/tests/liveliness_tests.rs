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
                "plate {:2}: overlap {:.3} ({:5} cells) speed {:.2} deg/My",
                p.id, ov, start, p.speed_deg_my
            );
        }
    }
}
