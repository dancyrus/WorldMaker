//! Plate-generator competition (WO-0003 Fix 2; design of record:
//! docs/plan/feel-pass-design/d2-fix2-design.md §§1–2 as amended by F3, F13).
//!
//! During the competition this module is `pub` so the integration tests can
//! reach every generator; after judging (commit M3) the losers, the trait and
//! `all_generators` are deleted, the module is demoted to private, and only
//! the winner remains behind `generate_plates` + `PlateGenParams`.
//!
//! Determinism contract for every generator: bit-exact from
//! `(master_seed, grid level)`; randomness ONLY from
//! `sub_rng(master_seed, STAGE_ID, "plate-seeds")` (draw counts may differ
//! per candidate — nothing else reads this stream); all math via
//! `worldmaker_core::dmath` + integer ops, no std trig; serial priority
//! queues keyed (cost, cell id, owner) over integer costs; rayon only for
//! per-element writes (no float reductions).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rand::RngCore;
use rayon::prelude::*;

use worldmaker_core::dmath::{
    add3, arc_len3, dot3, normalize3, random_tangent, scale3, uniform_range,
};
use worldmaker_core::hash::splitmix64;
use worldmaker_core::rng::sub_rng;
use worldmaker_core::Grid;

use super::{TectonicsParams, STAGE_ID};
use crate::noise_stage::fbm;

/// The only parameters a plate generator may see. Built from a clamped
/// `TectonicsParams` by setup.rs; deliberately EXCLUDES craton_overlay,
/// hotspot_overlay, land_fraction and span — plate layout must be
/// overlay-independent (tectonics_tests.rs pins keyframe-0 plate_id identical
/// with and without craton paint). The firewall is structural: the overlays
/// are never passed in, so no candidate can read them.
pub struct PlateGenParams {
    /// Clamped 8..=24 by `TectonicsParams::clamped` when built through the
    /// stage; direct callers of `SimState::setup` bypass that clamp (F3), so
    /// every `generate` impl re-asserts the band with a `debug_assert`.
    pub plate_count: u32,
}

impl From<&TectonicsParams> for PlateGenParams {
    fn from(p: &TectonicsParams) -> Self {
        PlateGenParams {
            plate_count: p.plate_count,
        }
    }
}

pub trait PlateGenerator {
    /// "incumbent" | "growth" | "warped" | "hybrid".
    fn name(&self) -> &'static str;

    /// Returns plate_id per cell. Contract (gate-tested, debug_asserted):
    /// ids are contiguous `0..params.plate_count`, every id non-empty
    /// (dense: PlateState slots are indexed by id, keyframes store u16, and
    /// step.rs asserts alive plates ≤ 32 — 24 max here); deterministic to
    /// the bit; randomness only from the "plate-seeds" sub-stream; never
    /// reads any overlay (structurally impossible — see `PlateGenParams`).
    fn generate(&self, master_seed: u64, grid: &Grid, params: &PlateGenParams) -> Vec<u32>;
}

/// All four competitors in the fixed judging order.
pub fn all_generators() -> Vec<Box<dyn PlateGenerator>> {
    vec![
        Box::new(Incumbent),
        Box::new(MultiSeedGrowth),
        Box::new(WarpedVoronoi),
        Box::new(HybridGrowthWarp),
    ]
}

// ---------------------------------------------------------------------------
// Shared helpers (d2 §2.1)
// ---------------------------------------------------------------------------

/// (H2) Farthest-point seed placement — the incumbent's exact loop factored
/// out: first seed one `next_u64()` draw, the rest argmin of a closeness
/// array (max dot to any seed so far), ties to the lower cell id; closeness
/// updated by per-element `par_iter_mut` max (no reduction).
fn farthest_point_seeds(rng: &mut impl RngCore, grid: &Grid, k: usize) -> Vec<u32> {
    let n = grid.cell_count() as usize;
    let mut seeds: Vec<u32> = Vec::with_capacity(k);
    seeds.push((rng.next_u64() % n as u64) as u32);
    // closeness[c] = max dot to any seed so far (higher = closer).
    let mut closeness = vec![-2.0f32; n];
    let update = |closeness: &mut [f32], seed_cell: u32| {
        let sp = grid.positions[seed_cell as usize];
        closeness
            .par_iter_mut()
            .enumerate()
            .for_each(|(c, cl)| *cl = cl.max(dot3(grid.positions[c], sp)));
    };
    update(&mut closeness, seeds[0]);
    for _ in 1..k {
        // Farthest cell = strictly minimal closeness, ties to the lower id.
        let mut best = 0usize;
        let mut best_cl = f32::MAX;
        for (c, &cl) in closeness.iter().enumerate() {
            if cl < best_cl {
                best_cl = cl;
                best = c;
            }
        }
        seeds.push(best as u32);
        update(&mut closeness, best as u32);
    }
    seeds
}

/// (H1) Heavy-tailed area-target ladder (cell counts, indexed by plate id;
/// plate 0 largest by construction). Draw order is part of the contract.
fn draw_area_targets(rng: &mut impl RngCore, p_count: usize, n: usize) -> Vec<u32> {
    let f_big = uniform_range(rng, 0.15, 0.25); // largest plate's fraction
    let f_small = uniform_range(rng, 0.015, 0.03); // smallest plate's fraction
    let t = f_small / f_big;
    // Ladder ratio rho solving rho^(p-1) = t by fixed 32-iteration bisection
    // on (0, 1); powers by repeated multiplication (≤ 23 multiplies) — the
    // same libm-free bisection pattern as the sea-level solve. No powf.
    let powi = |x: f32, e: usize| -> f32 {
        let mut r = 1.0f32;
        for _ in 0..e {
            r *= x;
        }
        r
    };
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for _ in 0..32 {
        let mid = 0.5 * (lo + hi);
        if powi(mid, p_count - 1) > t {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let rho = 0.5 * (lo + hi);
    let mut g = vec![0.0f32; p_count];
    g[0] = f_big;
    for i in 1..p_count {
        g[i] = g[i - 1] * rho;
    }
    // Jitter middles only; the extremes keep their bands by construction.
    // (F9: away from p = 12 the target sum drifts off 1.0 — expected, the
    // fills below are exhaustive and targets only steer costs.)
    for gi in g.iter_mut().take(p_count - 1).skip(1) {
        *gi *= uniform_range(rng, 0.85, 1.15);
        *gi = gi.clamp(1.1 * f_small, 0.9 * f_big);
    }
    g.iter()
        .map(|&gi| ((gi * n as f32).round() as u32).max(1))
        .collect()
}

/// (H3) Integer edge costs, one entry per directed CSR edge, aligned with
/// `grid.neighbors`: 1024 × arc length in units of the mean cell spacing.
fn base_edge_costs(grid: &Grid) -> Vec<u32> {
    let n = grid.cell_count() as usize;
    // Same formula SimState uses for cell_spacing_km, in radians.
    let spacing_rad = (4.0 * std::f64::consts::PI / n as f64).sqrt() as f32;
    let mut out = vec![0u32; grid.neighbors.len()];
    for u in 0..n {
        let start = grid.neighbor_offsets[u] as usize;
        let pu = grid.positions[u];
        for (ei, &v) in grid.neighbors_of(u as u32).iter().enumerate() {
            let arc = arc_len3(pu, grid.positions[v as usize]);
            out[start + ei] = ((1024.0 * arc / spacing_rad).round() as u32).max(1);
        }
    }
    out
}

/// Debug-only contract check: dense ids `0..p_count`, every plate non-empty.
fn debug_assert_dense(plate_id: &[u32], p_count: u32) {
    if cfg!(debug_assertions) {
        let mut counts = vec![0u64; p_count as usize];
        for &p in plate_id {
            assert!(p < p_count, "plate id {p} out of range 0..{p_count}");
            counts[p as usize] += 1;
        }
        for (i, &c) in counts.iter().enumerate() {
            assert!(c > 0, "plate {i} is empty");
        }
    }
}

// ---------------------------------------------------------------------------
// Incumbent (setup.rs sections 1–2, moved verbatim — the goldens staying
// green at commit M2 is the proof this refactor is bit-exact)
// ---------------------------------------------------------------------------

pub struct Incumbent;

impl PlateGenerator for Incumbent {
    fn name(&self) -> &'static str {
        "incumbent"
    }

    fn generate(&self, master_seed: u64, grid: &Grid, params: &PlateGenParams) -> Vec<u32> {
        debug_assert!((8..=24).contains(&params.plate_count));
        let n = grid.cell_count() as usize;
        let p_count = params.plate_count as usize;

        // --- 1. plate seed cells by farthest-point sampling ---
        let mut rng = sub_rng(master_seed, STAGE_ID, "plate-seeds");
        let seeds = farthest_point_seeds(&mut rng, grid, p_count);

        // --- 2. ownership by great-circle Voronoi ---
        let seed_pos: Vec<[f32; 3]> = seeds.iter().map(|&c| grid.positions[c as usize]).collect();
        let mut plate_id = vec![0u32; n];
        plate_id.par_iter_mut().enumerate().for_each(|(c, pid)| {
            let x = grid.positions[c];
            let mut best = 0u32;
            let mut best_d = -2.0f32;
            for (k, sp) in seed_pos.iter().enumerate() {
                let d = dot3(x, *sp);
                if d > best_d {
                    best_d = d;
                    best = k as u32;
                }
            }
            *pid = best;
        });
        debug_assert_dense(&plate_id, params.plate_count);
        plate_id
    }
}

// ---------------------------------------------------------------------------
// Candidate (a) — MultiSeedGrowth (d2 §2.2)
// ---------------------------------------------------------------------------

pub struct MultiSeedGrowth;

/// The growth machinery shared by candidates (a) and (c): H1 targets, H2
/// primaries, helper seeds, then the serial multi-source Dijkstra fill.
/// Returns (plate_id, primary seed cells). `noise_f` is `None` for (a);
/// `Some` switches the step cost to (c)'s 3-factor form (all u64 — F13).
fn growth_fill(
    rng: &mut impl RngCore,
    grid: &Grid,
    p_count: usize,
    noise_f: Option<&[u64]>,
) -> (Vec<u32>, Vec<u32>) {
    let n = grid.cell_count() as usize;

    // 1. Targets (H1): plate 0 the giant, plate p-1 the runt.
    let target = draw_area_targets(rng, p_count, n);
    let f: Vec<f32> = target.iter().map(|&t| t as f32 / n as f32).collect();

    // 2. Per-plate cost multipliers (integer): fronts meet where
    //    m_i·d_i = m_j·d_j, so radii scale ~sqrt(f) and areas ~f.
    let f_ref = f[0];
    let m: Vec<u64> = f
        .iter()
        .map(|&fi| (((256.0 * (f_ref / fi).sqrt()).round()) as u64).max(64))
        .collect();

    // 3. Seeds per plate: one sub-seed per 6% of sphere, clamped 1..=4.
    let k: Vec<u32> = target
        .iter()
        .map(|&t| ((t as f32 / (0.06 * n as f32)).round() as u32).clamp(1, 4))
        .collect();

    // 4. Primary seeds (H2); primary of plate i = i-th seed.
    let primaries = farthest_point_seeds(rng, grid, p_count);

    // 5. Helper seeds. Draws happen unconditionally BEFORE the skip check so
    //    the draw sequence never depends on grid-level collision luck.
    let mut is_seed = vec![false; n];
    for &c in &primaries {
        is_seed[c as usize] = true;
    }
    let mut seed_list: Vec<(u32, u32)> = primaries
        .iter()
        .enumerate()
        .map(|(i, &c)| (c, i as u32))
        .collect();
    for i in 0..p_count {
        let pc = primaries[i];
        let ppos = grid.positions[pc as usize];
        for _j in 1..k[i] {
            let dir = random_tangent(rng, ppos);
            // 2·sqrt(f) = the Euclidean chord radius of a cap of fraction f.
            let r = uniform_range(rng, 0.30, 0.70) * 2.0 * f[i].sqrt();
            let cell = grid.nearest_cell(normalize3(add3(ppos, scale3(dir, r))), Some(pc));
            if is_seed[cell as usize] {
                continue; // deterministic skip; plate stays non-empty via its primary
            }
            is_seed[cell as usize] = true;
            seed_list.push((cell, i as u32));
        }
    }

    // 6. Serial multi-source Dijkstra flood fill. Heap key (cost, cell id,
    //    owner) — cost first, cell id second per the pinned rule, owner as
    //    the final component so the key is a total order over pushes.
    //    Sub-seeds carry their final plate id as owner from the first push
    //    (§2.2 step 7: no helper-id namespace exists to collapse).
    let b_e = base_edge_costs(grid);
    let mut plate_id = vec![u32::MAX; n];
    let mut count = vec![0u32; p_count];
    let mut heap: BinaryHeap<Reverse<(u64, u32, u32)>> = BinaryHeap::new();
    for &(cell, owner) in &seed_list {
        heap.push(Reverse((0, cell, owner)));
    }
    while let Some(Reverse((cost, cell, owner))) = heap.pop() {
        let cu = cell as usize;
        if plate_id[cu] != u32::MAX {
            continue;
        }
        plate_id[cu] = owner;
        let o = owner as usize;
        count[o] += 1;
        // Soft cap: an over-target plate keeps growing only where nobody
        // else competes (the fill is exhaustive). Counts update at pop time
        // under a serial heap, so m_eff is deterministic.
        let m_eff = if count[o] < target[o] { m[o] } else { m[o] * 4 };
        let start = grid.neighbor_offsets[cu] as usize;
        for (ei, &v) in grid.neighbors_of(cell).iter().enumerate() {
            if plate_id[v as usize] != u32::MAX {
                continue;
            }
            let b = b_e[start + ei] as u64;
            let step = match noise_f {
                None => ((b * m_eff + 128) >> 8).max(1),
                // (c): 3-factor step cost, every factor already u64 (F13).
                Some(nf) => ((b * m_eff * nf[v as usize]) >> 16).max(1),
            };
            heap.push(Reverse((cost + step, v, owner)));
        }
    }

    (plate_id, primaries)
}

impl PlateGenerator for MultiSeedGrowth {
    fn name(&self) -> &'static str {
        "growth"
    }

    fn generate(&self, master_seed: u64, grid: &Grid, params: &PlateGenParams) -> Vec<u32> {
        debug_assert!((8..=24).contains(&params.plate_count));
        let mut rng = sub_rng(master_seed, STAGE_ID, "plate-seeds");
        let (plate_id, _primaries) = growth_fill(&mut rng, grid, params.plate_count as usize, None);
        debug_assert_dense(&plate_id, params.plate_count);
        plate_id
    }
}

// ---------------------------------------------------------------------------
// Candidate (b) — WarpedVoronoi (d2 §2.3)
// ---------------------------------------------------------------------------

pub struct WarpedVoronoi;

/// Warp displacement in chord units (typical |W| ≈ 0.9 ⇒ ~9–10° of
/// displacement). Judge-tunable; log any change.
const WARP_AMP: f32 = 0.18;
/// Additive cosine-space area bias toward the heavy-tailed targets.
const BIAS: f32 = 0.15;
/// Noise-temperature schedule of the 3 Gauss–Seidel annealing sweeps.
const LAMBDA: [f32; 3] = [1.5, 0.75, 0.0];

/// The boundary-annealing pass (d2 §2.3 step 5), shared verbatim with the
/// hybrid: 3 fixed serial Gauss–Seidel sweeps in ascending cell id, updates
/// visible within the sweep; score = same-plate neighbor count + λ·hash
/// noise; flips refused if they would empty a plate or move a pinned cell.
fn anneal(grid: &Grid, plate_id: &mut [u32], count: &mut [u32], pinned: &[bool], wa: u64) {
    let n = plate_id.len();
    for &lambda in &LAMBDA {
        for c in 0..n {
            if pinned[c] {
                continue; // seeds stay pinned — non-emptiness by construction
            }
            let cur = plate_id[c] as usize;
            let ring = grid.neighbors_of(c as u32);
            // Candidate set = {current} ∪ neighbor plate ids, ascending.
            let mut cands = [0u32; 7];
            let mut nc = 0usize;
            cands[nc] = cur as u32;
            nc += 1;
            for &nb in ring {
                let p = plate_id[nb as usize];
                if !cands[..nc].contains(&p) {
                    cands[nc] = p;
                    nc += 1;
                }
            }
            if nc == 1 {
                continue; // only boundary cells are considered
            }
            cands[..nc].sort_unstable();
            let mut best_k = cands[0];
            let mut best_score = f32::MIN;
            for &k in &cands[..nc] {
                let mut same = 0u32;
                for &nb in ring {
                    if plate_id[nb as usize] == k {
                        same += 1;
                    }
                }
                // Pure hash noise — no stream state, so sweep order cannot
                // desynchronize draws.
                let eta =
                    ((splitmix64(wa ^ ((c as u64) << 32) ^ k as u64) >> 40) as f32) / 16_777_216.0;
                let score = same as f32 + lambda * eta;
                // Strict > with ascending candidates ⇒ ties to the LOWEST id.
                if score > best_score {
                    best_score = score;
                    best_k = k;
                }
            }
            if best_k as usize != cur {
                if count[cur] == 1 {
                    continue; // refuse a flip that would empty a plate
                }
                plate_id[c] = best_k;
                count[cur] -= 1;
                count[best_k as usize] += 1;
            }
        }
    }
}

impl PlateGenerator for WarpedVoronoi {
    fn name(&self) -> &'static str {
        "warped"
    }

    fn generate(&self, master_seed: u64, grid: &Grid, params: &PlateGenParams) -> Vec<u32> {
        debug_assert!((8..=24).contains(&params.plate_count));
        let n = grid.cell_count() as usize;
        let p_count = params.plate_count as usize;
        let mut rng = sub_rng(master_seed, STAGE_ID, "plate-seeds");

        // Draw order: 3 warp seeds, anneal seed, targets (H1), seeds (H2).
        let w1 = rng.next_u64();
        let w2 = rng.next_u64();
        let w3 = rng.next_u64();
        let wa = rng.next_u64();
        let target = draw_area_targets(&mut rng, p_count, n);
        let seeds = farthest_point_seeds(&mut rng, grid, p_count);

        let seed_pos: Vec<[f32; 3]> = seeds.iter().map(|&c| grid.positions[c as usize]).collect();
        let f_mean = 1.0 / p_count as f32;
        let bias: Vec<f32> = target
            .iter()
            .map(|&t| {
                let fk = t as f32 / n as f32;
                BIAS * ((fk / f_mean).sqrt() - 1.0)
            })
            .collect();

        // Assignment with warped sample points and area bias (per-element
        // parallel, strict > ⇒ ties to the lowest seed index).
        let mut plate_id = vec![0u32; n];
        plate_id.par_iter_mut().enumerate().for_each(|(c, pid)| {
            let pos = grid.positions[c];
            let w = [fbm(pos, w1, 3), fbm(pos, w2, 3), fbm(pos, w3, 3)];
            let q = normalize3(add3(pos, scale3(w, WARP_AMP)));
            let mut best = 0u32;
            let mut best_d = -2.0f32;
            for (k, sp) in seed_pos.iter().enumerate() {
                let d = dot3(q, *sp) + bias[k];
                if d > best_d {
                    best_d = d;
                    best = k as u32;
                }
            }
            *pid = best;
        });

        // Non-empty repair (deterministic serial): warping can in principle
        // steal a seed's own cell; restore a one-cell plate.
        let mut count = vec![0u32; p_count];
        for &p in &plate_id {
            count[p as usize] += 1;
        }
        for k in 0..p_count {
            if count[k] == 0 {
                let sc = seeds[k] as usize;
                let old = plate_id[sc] as usize;
                plate_id[sc] = k as u32;
                count[old] -= 1;
                count[k] += 1;
            }
        }

        // Boundary annealing; H2 seed cells stay pinned.
        let mut pinned = vec![false; n];
        for &c in &seeds {
            pinned[c as usize] = true;
        }
        anneal(grid, &mut plate_id, &mut count, &pinned, wa);

        debug_assert_dense(&plate_id, params.plate_count);
        plate_id
    }
}

// ---------------------------------------------------------------------------
// Candidate (c) — HybridGrowthWarp (d2 §2.4)
// ---------------------------------------------------------------------------

pub struct HybridGrowthWarp;

/// Post-pass (judge record §4 condition 1): every plate must be exactly one
/// connected component over CSR neighbors — the anneal can pinch a thin lobe
/// off its plate (observed: L6 seed 7, plate 8 in 3 components). Fix rule,
/// verbatim from the record: deterministically reassign each minority
/// component to the neighboring plate sharing the longest boundary
/// (interface-edge cell-center arcs via `arc_len3`; ties → lowest plate id).
///
/// Determinism: components are labelled by a serial ascending-cell-id scan
/// (component ids ascend with their lowest cell id); the majority component
/// of a plate is the largest, ties to the lowest component id; minority
/// components are processed in component-id order with boundary sums
/// accumulated in f64 over cells in ascending cell id and CSR ring order.
/// Reassigning a blob can in principle re-split its target plate, so the
/// pass iterates to a fixed point (a no-op pass on an already-clean map).
fn reassign_minority_components(grid: &Grid, plate_id: &mut [u32], p_count: usize) {
    let n = plate_id.len();
    for _pass in 0..16 {
        // Label connected components (serial BFS, ascending seed cell id).
        let mut comp = vec![u32::MAX; n];
        let mut comp_plate: Vec<u32> = Vec::new();
        let mut comp_cells: Vec<Vec<u32>> = Vec::new();
        for c0 in 0..n {
            if comp[c0] != u32::MAX {
                continue;
            }
            let pid = plate_id[c0];
            let cid = comp_plate.len() as u32;
            comp_plate.push(pid);
            comp[c0] = cid;
            let mut cells: Vec<u32> = vec![c0 as u32];
            let mut q: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
            q.push_back(c0 as u32);
            while let Some(c) = q.pop_front() {
                for &nb in grid.neighbors_of(c) {
                    let nu = nb as usize;
                    if comp[nu] == u32::MAX && plate_id[nu] == pid {
                        comp[nu] = cid;
                        cells.push(nb);
                        q.push_back(nb);
                    }
                }
            }
            comp_cells.push(cells);
        }
        // Majority component per plate: most cells, ties to the lowest
        // component id (strict > over ascending component ids).
        let mut major = vec![u32::MAX; p_count];
        let mut major_size = vec![0usize; p_count];
        for (cid, cells) in comp_cells.iter().enumerate() {
            let p = comp_plate[cid] as usize;
            if cells.len() > major_size[p] {
                major_size[p] = cells.len();
                major[p] = cid as u32;
            }
        }
        // Reassign minority components in component-id order. Later blobs see
        // earlier reassignments (fixed order ⇒ deterministic).
        let mut changed = false;
        for (cid, cells) in comp_cells.iter().enumerate() {
            let p = comp_plate[cid] as usize;
            if major[p] == cid as u32 {
                continue;
            }
            changed = true;
            let mut sorted = cells.clone();
            sorted.sort_unstable();
            let mut blen = vec![0.0f64; p_count];
            for &c in &sorted {
                let pc = grid.positions[c as usize];
                for &nb in grid.neighbors_of(c) {
                    let q = plate_id[nb as usize] as usize;
                    if q != p {
                        blen[q] += arc_len3(pc, grid.positions[nb as usize]) as f64;
                    }
                }
            }
            let mut best = usize::MAX;
            let mut best_len = 0.0f64;
            for (q, &l) in blen.iter().enumerate() {
                if l > best_len {
                    best_len = l;
                    best = q;
                }
            }
            assert!(
                best != usize::MAX,
                "minority component has no foreign neighbor"
            );
            for &c in &sorted {
                plate_id[c as usize] = best as u32;
            }
        }
        if !changed {
            return;
        }
    }
    panic!("plate component reassignment did not converge in 16 passes");
}

impl PlateGenerator for HybridGrowthWarp {
    fn name(&self) -> &'static str {
        "hybrid"
    }

    fn generate(&self, master_seed: u64, grid: &Grid, params: &PlateGenParams) -> Vec<u32> {
        debug_assert!((8..=24).contains(&params.plate_count));
        let n = grid.cell_count() as usize;
        let p_count = params.plate_count as usize;
        let mut rng = sub_rng(master_seed, STAGE_ID, "plate-seeds");

        // Draw order: hybrid noise seed, anneal seed, then (a)'s sequence.
        let wn = rng.next_u64();
        let wa = rng.next_u64();

        // Per-cell terrain factor for the step cost (per-element parallel;
        // round then clamp, as pinned).
        let mut noise_f = vec![0u64; n];
        noise_f.par_iter_mut().enumerate().for_each(|(c, nf)| {
            let x = 256.0 * (1.0 + 0.6 * fbm(grid.positions[c], wn, 3));
            *nf = x.round().clamp(64.0, 512.0) as u64;
        });

        let (mut plate_id, primaries) = growth_fill(&mut rng, grid, p_count, Some(&noise_f));

        // (b)'s annealing pass verbatim on the filled map. Pinned set = the
        // p PRIMARY seed cells only (F13); helpers may flip — non-emptiness
        // is protected by the count[current] == 1 refusal.
        let mut count = vec![0u32; p_count];
        for &p in &plate_id {
            count[p as usize] += 1;
        }
        let mut pinned = vec![false; n];
        for &c in &primaries {
            pinned[c as usize] = true;
        }
        anneal(grid, &mut plate_id, &mut count, &pinned, wa);

        // Judge record §4 condition 1: exactly one component per plate.
        reassign_minority_components(grid, &mut plate_id, p_count);

        debug_assert_dense(&plate_id, params.plate_count);
        plate_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every competitor emits a dense, non-empty, bit-deterministic map.
    /// Runs at L5 in normal CI so the debug_assert paths in every generator
    /// are exercised on each test run, not only when the ignored panel tests
    /// are invoked.
    #[test]
    fn all_generators_emit_dense_deterministic_maps() {
        let grid = Grid::build(5);
        let params = PlateGenParams { plate_count: 12 };
        for g in all_generators() {
            let a = g.generate(42, &grid, &params);
            let b = g.generate(42, &grid, &params);
            assert_eq!(a, b, "{} is not bit-deterministic", g.name());
            assert_eq!(a.len(), grid.cell_count() as usize);
            let mut counts = vec![0u32; 12];
            for &p in &a {
                assert!(p < 12, "{}: plate id {p} out of range", g.name());
                counts[p as usize] += 1;
            }
            assert!(
                counts.iter().all(|&c| c > 0),
                "{}: emitted an empty plate: {counts:?}",
                g.name()
            );
        }
    }
}
