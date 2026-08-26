//! WO-0003 Fix 2: fast setup-only CI gate on the t=0 plate map (d2-fix2-
//! design §4). Configs: `TectonicsParams::default()` (in-band, matches the
//! goldens and the harness); `SimState::setup` only — no pipeline run, no
//! elevation derive; the test reads plate geometry only.
//!
//! Gate triple (pinned in Stage D, before any measurement): L7 seed 42
//! (golden/harness seed), L6 seed 7 and L6 seed cyrus — both members of the
//! 5-seed competition set, so they cannot be cherry-picked later.
//!
//! During the competition phase (commit M2) the metric-threshold test is
//! `#[ignore]`: the default generator is still the incumbent, which the
//! gates exist to exclude. At commit M3, when the winner is wired into
//! setup.rs, the ignore attribute comes off and the constants are finalized
//! per the §6 P3 rule. The structural + determinism assertions hold for any
//! generator and run in CI already.

use std::sync::Arc;

use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{metrics, SimState, TectonicsParams};

/// Provisional gates (pinned). Final values at M3:
/// `max(provisional, incumbent_best_on_gate_triple + margin)` and
/// `≤ winner_worst_on_gate_triple − margin`, margins 0.05 (CV) / 0.02
/// (sinuosity).
///
/// Incumbent measured on the gate triple (commit M1,
/// docs/results/tectonics-feelpass-Daniels-MacBook-Air.json):
///   L7 seed42:    area_cv 0.1361, sinuosity 1.1465
///   L6 seed7:     area_cv 0.0789, sinuosity 1.1232
///   L6 seedcyrus: area_cv 0.0878, sinuosity 1.1132
/// so the incumbent fails the CV gate on all three configs (best 0.1361 vs
/// 0.5) and the sinuosity gate on all three (best 1.1465 vs 1.15) — the
/// "strictly excludes incumbent" requirement holds for the provisional
/// values. Note R2: the sinuosity margin over the hex-zigzag floor is thin;
/// the P3 rule will raise the final gate above incumbent_best + 0.02.
const GATE_CV: f64 = 0.5;
const GATE_SINUOSITY: f64 = 1.15;

/// (level, master_seed) gate triple. 0xc4be0bf8f497a575 =
/// seed_from_text("cyrus"), the app-default seed of the committed BEFORE
/// screenshots — the exact world whose look raised Fix 2.
const GATE_CONFIGS: [(u32, u64); 3] = [(7, 42), (6, 7), (6, 0xc4be_0bf8_f497_a575)];

/// Structural contract + determinism smoke, valid for ANY generator: dense
/// non-empty ids, 12 alive plates (⇒ ≤ 32 trivially), and a second setup
/// with the same inputs is bit-identical — catches stray nondeterminism
/// immediately rather than at the goldens.
#[test]
fn plate_setup_is_structurally_sound_and_deterministic() {
    let params = TectonicsParams::default();
    for (level, seed) in GATE_CONFIGS {
        let grid = Arc::new(Grid::build(level));
        let s = SimState::setup(seed, &grid, &params);
        let n = grid.cell_count() as usize;
        assert_eq!(s.plate_id.len(), n);
        let p = params.plate_count;
        let mut counts = vec![0u64; p as usize];
        for &pid in &s.plate_id {
            assert!(
                pid < p,
                "plate id {pid} out of range at L{level} seed {seed:#x}"
            );
            counts[pid as usize] += 1;
        }
        assert!(
            counts.iter().all(|&c| c > 0),
            "empty plate at L{level} seed {seed:#x}"
        );
        assert_eq!(s.plates.len(), p as usize);
        assert!(s.plates.iter().all(|pl| pl.alive));

        // Determinism smoke: bit-identical re-run.
        let s2 = SimState::setup(seed, &grid, &params);
        assert_eq!(
            s.plate_id, s2.plate_id,
            "setup is not bit-deterministic at L{level} seed {seed:#x}"
        );
    }
}

/// The metric gates. `#[ignore]` until commit M3 wires the winner — the
/// incumbent (today's default) measures far below both gates, which is the
/// point of Fix 2.
#[test]
#[ignore = "armed at commit M3 when the winning generator is wired; the incumbent default fails by design"]
fn plate_setup_clears_feel_gates() {
    let params = TectonicsParams::default();
    for (level, seed) in GATE_CONFIGS {
        let grid = Arc::new(Grid::build(level));
        let s = SimState::setup(seed, &grid, &params);
        let cv = metrics::plate_area_cv(&s.plate_id, params.plate_count);
        assert!(
            cv >= GATE_CV,
            "plate area CV {cv:.4} below gate {GATE_CV} at L{level} seed {seed:#x}"
        );
        let rep = metrics::boundary_sinuosity(&grid, &s.plate_id);
        assert!(
            rep.weighted_mean >= GATE_SINUOSITY,
            "boundary sinuosity {:.4} below gate {GATE_SINUOSITY} at L{level} seed {seed:#x}",
            rep.weighted_mean
        );
    }
}
