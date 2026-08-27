//! WO-0003 Fix 4: plate-liveliness CI gates.
//!
//! Measured on v0.2.0 (Dan's report, reproduced by the ignored probe below):
//! plates froze permanently — ownership overlap up to 1.000 per 200 My
//! window from 400 My to the end of a 2 Gy run, with final speeds of
//! exactly 0.00 deg/My. These tests are the gate class Phase 1 missed: no
//! alive plate may keep near-identical cell ownership across a 300 My
//! window (outside a young continent-continent collision), and no alive
//! plate may hold speed < 0.05 deg/My for more than 200 My. Constants and
//! implementation are canonical in `tectonics::metrics`; the acceptance
//! harness evaluates the same functions into the results JSON.

use std::sync::Arc;

use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{self, metrics, TectonicsHistory, TectonicsParams};
use worldmaker_sim::{StageContext, WorldState};

fn run_history(level: u32, seed: u64, params: TectonicsParams) -> TectonicsHistory {
    let grid = Arc::new(Grid::build(level));
    let world = WorldState::new(grid);
    tectonics::run_history(&StageContext::new(seed), &world, &params, None).unwrap()
}

fn assert_lively(hist: &TectonicsHistory, label: &str) {
    let rep = metrics::liveliness(hist);
    assert!(
        rep.pass(),
        "{label}: liveliness gates failed\n7.1 overlap:\n{}\n7.2 speed:\n{}",
        rep.overlap_violations.join("\n"),
        rep.speed_violations.join("\n"),
    );
}

/// Gates 7.1 + 7.2 on the committed configuration: defaults, 2 Gy, L6,
/// seed 42 (the same run the harness stability gate measures).
#[test]
fn liveliness_gates_defaults_seed42() {
    let hist = run_history(
        6,
        42,
        TectonicsParams {
            span_my: 2000.0,
            ..TectonicsParams::default()
        },
    );
    assert_lively(&hist, "defaults 2 Gy L6 seed 42");
}

/// Gate 7.3: echo run at Dan's recorded settings — the configuration the
/// original freeze was observed at (seed box "dan", 8 plates, land 0.40,
/// vigor 1.73, 2 Gy). The seed goes through `seed_from_text`, the exact
/// app seed-box path, so this reproduces what Dan saw.
#[test]
fn liveliness_gates_dan_echo() {
    let hist = run_history(
        6,
        seed_from_text("dan"),
        TectonicsParams {
            plate_count: 8,
            land_fraction: 0.40,
            tectonic_vigor: 1.73,
            span_my: 2000.0,
            ..TectonicsParams::default()
        },
    );
    assert_lively(&hist, "dan echo 2 Gy L6");
}

/// Step-1 probe: per-plate ownership overlap and speed across 200 My
/// windows at defaults, 2 Gy, L6, seed 42. Run with
/// `cargo test -p worldmaker-sim --release probe_freeze -- --ignored --nocapture`.
#[test]
#[ignore = "dev probe: prints per-plate overlap/speed tables"]
fn probe_freeze_report() {
    // Env overrides for probing other configurations (WM_PROBE_DAN=1 probes
    // the gate-7.3 echo configuration).
    let dan = std::env::var("WM_PROBE_DAN")
        .map(|v| v == "1")
        .unwrap_or(false);
    let params = if dan {
        TectonicsParams {
            plate_count: 8,
            land_fraction: 0.40,
            tectonic_vigor: 1.73,
            span_my: 2000.0,
            ..TectonicsParams::default()
        }
    } else {
        TectonicsParams {
            span_my: 2000.0,
            ..TectonicsParams::default()
        }
    };
    let seed = if dan { seed_from_text("dan") } else { 42 };
    let hist = run_history(6, seed, params);
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
            let (ov, start) = metrics::ownership_overlap(a, b, p.id as u16);
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
    let rep = metrics::liveliness(&hist);
    for v in &rep.overlap_violations {
        println!("{v}");
    }
    println!("=== gate 7.2 (speed) violations ===");
    for v in &rep.speed_violations {
        println!("{v}");
    }
}
