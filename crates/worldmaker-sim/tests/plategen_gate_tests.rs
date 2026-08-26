//! WO-0003 Fix 2: fast setup-only CI gate on the t=0 plate map (d2-fix2-
//! design §4; ARMED at commit M3, when the winning generator — the retuned
//! hybrid growth+warp — was wired into setup.rs). Configs:
//! `TectonicsParams::default()` (in-band, matches the goldens and the
//! harness); `SimState::setup` only — no pipeline run, no elevation derive;
//! the tests read plate geometry only.
//!
//! Gate triple (pinned in Stage D, before any measurement): L7 seed 42
//! (golden/harness seed), L6 seed 7 and L6 seed cyrus — both members of the
//! 5-seed competition set, so they cannot be cherry-picked later.

use std::collections::VecDeque;
use std::sync::Arc;

use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{metrics, SimState, TectonicsParams};

/// FINAL gate values (judge record §3, ruled by the panel and re-confirmed
/// unchanged for the retuned hybrid by the re-judging addendum §A4; the
/// canonical constants live in `metrics` so the harness echoes the same
/// values). Derivation rule: `max(provisional, incumbent_best + margin)` on
/// the gate triple, and at most `winner_worst − margin`; margins 0.05 (CV)
/// / 0.02 (sinuosity).
///
/// Measured on the gate triple, in order L7 seed42 / L6 seed7 / L6 seedcyrus
/// (committed record: docs/results/plategen-feelpass-Daniels-MacBook-Air.json
/// and the M1 incumbent rows of tectonics-feelpass-Daniels-MacBook-Air.json):
///
///   incumbent:      area_cv 0.1361 / 0.0789 / 0.0878
///                   sinuosity 1.1465 / 1.1232 / 1.1132
///   hybrid (wired): area_cv 0.8280 / 0.8213 / 0.9152
///                   sinuosity 1.2348 / 1.2831 / 1.2927
///
/// So the incumbent fails BOTH gates on every triple config independently
/// (CV best 0.1361 < 0.50; sinuosity best 1.1465 < 1.18 — and its best
/// anywhere across all 10 (seed, level) pairs, 1.1545, still falls short by
/// ≥ 0.0255), while the wired hybrid clears with margins ≥ 0.3213 (CV) and
/// ≥ 0.0548 (sinuosity) at its triple worst. The CV gate is the formula
/// value; the sinuosity gate is raised above the formula floor 1.1665 with
/// logged reasoning: values near the floor are barely distinguishable from
/// the ~1.10–1.15 hex-grid zigzag baseline of a straight boundary. Metrics
/// are bit-deterministic, so the margins protect future retuning, not
/// machines. Full derivation: judge record §3 + addendum §A4.
const GATE_CV: f64 = metrics::GATE_CV;
const GATE_SINUOSITY: f64 = metrics::GATE_SINUOSITY;

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

/// Judge record §4 condition 1 + addendum §A7, promoted into the gate suite:
/// every plate is exactly one connected component over CSR neighbors (the
/// generator's post-pass guarantees it; this asserts the guarantee), and no
/// plate is an enclave — every plate borders at least two distinct plates
/// (the criterion that sank the growth candidate). Serial BFS in ascending
/// cell id; fully deterministic.
fn assert_connected_no_enclaves(
    grid: &Grid,
    plate_id: &[u32],
    p_count: u32,
    level: u32,
    seed: u64,
) {
    let n = plate_id.len();
    let p = p_count as usize;
    // Component count per plate.
    let mut comp_count = vec![0u32; p];
    let mut seen = vec![false; n];
    let mut q: VecDeque<u32> = VecDeque::new();
    for c0 in 0..n {
        if seen[c0] {
            continue;
        }
        let pid = plate_id[c0];
        comp_count[pid as usize] += 1;
        seen[c0] = true;
        q.push_back(c0 as u32);
        while let Some(c) = q.pop_front() {
            for &nb in grid.neighbors_of(c) {
                let nu = nb as usize;
                if !seen[nu] && plate_id[nu] == pid {
                    seen[nu] = true;
                    q.push_back(nb);
                }
            }
        }
    }
    for (pid, &k) in comp_count.iter().enumerate() {
        assert_eq!(
            k, 1,
            "plate {pid} is {k} connected components at L{level} seed {seed:#x}"
        );
    }
    // Distinct-neighbor count per plate (enclave = exactly one neighbor).
    let mut adj = vec![false; p * p];
    for c in 0..n {
        let a = plate_id[c] as usize;
        for &nb in grid.neighbors_of(c as u32) {
            let b = plate_id[nb as usize] as usize;
            if a != b {
                adj[a * p + b] = true;
            }
        }
    }
    for pid in 0..p {
        let deg = (0..p).filter(|&qid| adj[pid * p + qid]).count();
        assert!(
            deg >= 2,
            "plate {pid} is an enclave ({deg} neighbor plate(s)) at L{level} seed {seed:#x}"
        );
    }
}

/// The metric gates — armed since commit M3 (the wired hybrid clears them;
/// the deleted incumbent measured far below both, which was the point of
/// Fix 2). Also asserts connectivity + no-enclave on every triple config.
#[test]
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
        assert_connected_no_enclaves(&grid, &s.plate_id, params.plate_count, level, seed);
    }
}
