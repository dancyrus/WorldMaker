# Fix 2 design — plate-generator competition (Track B)

Stage-D design for WO-0003 Fix 2. Implementers (Track B) code against THIS
document plus the pinned contracts in `../feel-pass-design.md`. Line refs are
against main @ 9d5d272. No code is changed by this document.

Scope recap (pinned): replace the t=0 plate map (setup.rs:46–89) via a
competition of 3 candidates + incumbent behind one trait; metrics = plate area
CV + boundary sinuosity, incumbent measured and committed FIRST on 5 fixed
seeds; fast setup-only CI gate test; dev-only PNG panel; L9 cadence decision;
harness.rs additions; goldens regenerated exactly once, final commit on B's
branch. Everything metric-feeding uses `worldmaker_core::dmath` only.

---

## 1. The common generator trait

**File:** `crates/worldmaker-sim/src/tectonics/plate_gen.rs` (new module,
declared in tectonics/mod.rs). During the competition the module is `pub`
(integration tests in `crates/worldmaker-sim/tests/` must reach it); after
judging it is demoted to a private `mod plate_gen` (see §6 phase P4).

```rust
/// The only parameters a plate generator may see. Built from a clamped
/// TectonicsParams by setup.rs; deliberately EXCLUDES craton_overlay,
/// hotspot_overlay, land_fraction and span — plate layout must be
/// overlay-independent (tectonics_tests.rs:371–396 pins keyframe-0 plate_id
/// identical with and without craton paint). The firewall is structural:
/// the overlays are never passed in, so no candidate can read them.
pub struct PlateGenParams {
    /// Already clamped 8..=24 by TectonicsParams::clamped.
    pub plate_count: u32,
}

pub trait PlateGenerator {
    fn name(&self) -> &'static str; // "incumbent" | "growth" | "warped" | "hybrid"

    /// Returns plate_id per cell. Contract (asserted by the gate test and
    /// debug_asserted in every impl):
    ///   - ids are contiguous 0..params.plate_count, every id non-empty
    ///     (dense: PlateState slots are indexed by id, keyframe stores u16,
    ///      step.rs:428 asserts alive plates <= 32 — 24 max here);
    ///   - deterministic to the bit from (master_seed, grid level);
    ///   - randomness ONLY from sub_rng(master_seed, STAGE_ID, "plate-seeds")
    ///     (the stream the incumbent already owns; draw counts may differ
    ///     per candidate — nothing else reads this stream);
    ///   - all math via worldmaker_core::dmath + integer ops; no std trig;
    ///   - never reads any overlay (structurally impossible, see above).
    fn generate(&self, master_seed: u64, grid: &Grid, params: &PlateGenParams) -> Vec<u32>;
}
```

**Wiring in setup.rs.** Sections 1–2 (lines 46–89) are replaced by:
`s.plate_id = plate_gen::generate(master_seed, grid, &PlateGenParams::from(params));`
Section 3 (plate motions, per-pid `"plate-init-{pid}"` streams) stays in
setup.rs and pushes exactly `plate_count` PlateStates as today — those streams
are geometry-independent, so poles/speeds are unchanged for a given
plate_count. Everything downstream (cratons, ocean ramp, hotspots,
detail-noise seed) is untouched code; only the "cratons" stream's *draw
alignment* shifts with the new geometry (setup.rs:161–185 gates two draws on
`target > 0`), which is world-data change covered by the one-time golden
regen — not a code change.

**Where the four generators live during the competition:** all in
plate_gen.rs — `Incumbent` (sections 1–2 moved verbatim: same single
`next_u64()` draw, same par_iter closeness update, same strict-`>`/lowest-index
tie rules, so the refactor is bit-identical and the goldens stay green — that
green run is the proof the refactor is exact), plus `MultiSeedGrowth`,
`WarpedVoronoi`, `HybridGrowthWarp`, plus
`pub fn all_generators() -> Vec<Box<dyn PlateGenerator>>` in the fixed order
[incumbent, growth, warped, hybrid]. Shared helpers (§2.1) live in the same
file. After judging: losers + trait + all_generators deleted; the file keeps
`pub(super) fn generate_plates(master_seed, &Grid, &PlateGenParams) -> Vec<u32>`
(the winner) and `PlateGenParams`.

---

## 2. Candidate algorithms

### 2.1 Shared helpers (used by ≥2 candidates)

**(H1) Heavy-tailed area-target ladder** — `draw_area_targets(rng, p_count, n)
-> Vec<u32>` (cell-count targets, indexed by plate id; plate 0 largest by
construction). Exact procedure, all draws in this order from the "plate-seeds"
stream:

1. `f_big  = uniform_range(rng, 0.15, 0.25)` — largest plate's sphere fraction.
2. `f_small = uniform_range(rng, 0.015, 0.03)` — smallest plate's fraction
   (upper half of the pinned 1–3% band; §11 risk R1 explains why the low edge
   is held in reserve, not used by default).
3. Ladder ratio ρ solving `ρ^(p_count−1) = f_small / f_big` by **fixed 32-
   iteration bisection** on ρ ∈ (0,1), with the power computed by repeated
   multiplication (≤ 23 multiplies) — same libm-free bisection pattern as the
   sea-level solve (elevation.rs:81–95). Deterministic, no `powf`.
4. Base ladder `g[i] = f_big·ρ^i` for i = 0..p_count (repeated multiplication).
5. Jitter middles only: for i in 1..p_count−1:
   `g[i] *= uniform_range(rng, 0.85, 1.15)`, then clamp
   `g[i] = g[i].clamp(1.1*f_small, 0.9*f_big)` — the extremes keep their
   bands by construction (largest stays in 15–25%, smallest in 1.5–3%).
6. `target[i] = round(g[i] * n as f32) as u32`, min 1.

Targets deliberately do NOT need to sum to n: the fills below are exhaustive
(every cell gets assigned); targets only steer costs. That removes any need
for iterative renormalization.

**(H2) Farthest-point seed placement** — the incumbent's exact loop
(setup.rs:46–73) factored as `farthest_point_seeds(rng, grid, k) -> Vec<u32>`:
first seed `(rng.next_u64() % n) as u32`, then argmin of the closeness array,
ties to the lower cell id, closeness updated by per-element `par_iter_mut`
max (deterministic — no reduction).

**(H3) Integer edge costs** — for candidate (a)/(c):
`base_edge_cost: Vec<u32>` aligned with `grid.neighbors` (one entry per
directed CSR edge): `b_e = max(1, round(1024 * arc_len3(pos[u], pos[v]) /
spacing_rad))` where `spacing_rad = sqrt(4π/n)` (the same formula SimState
uses for cell_spacing_km, radians instead of km; the constant 4π/n is plain
arithmetic, no trig). `arc_len3` is the new dmath helper (§3.3). Computed once
per generate() call, per-element parallel-safe, values ~1024 ± a few %.

### 2.2 Candidate (a) — `MultiSeedGrowth` (multi-seed weighted growth)

RNG: one stream, `sub_rng(seed, STAGE_ID, "plate-seeds")`, draws in the exact
order given.

1. **Targets:** `target = draw_area_targets(rng, p, n)` (H1). Plate 0 is the
   giant, plate p−1 the runt.
2. **Per-plate cost multipliers** (integer): with `f[i] = target[i] as f32 / n`
   and `f_ref = f[0]`: `m[i] = max(64, round(256.0 * (f_ref / f[i]).sqrt()))`
   — competitive Dijkstra fronts meet where `m_i·d_i = m_j·d_j`, so radii
   scale ~√f and areas ~f. `sqrt` is IEEE-exact, allowed.
3. **Seeds per plate:** `k[i] = clamp(round(target[i] as f32 / (0.06 * n as
   f32)), 1, 4)` — one sub-seed per 6% of sphere, so the 15–25% giant gets
   3–4 sub-seeds (non-circular macro shape), mid plates 1–2, runts 1.
4. **Primary seeds:** `farthest_point_seeds(rng, grid, p)` (H2); primary of
   plate i = i-th seed (plate 0's primary is the random first draw).
5. **Helper seeds:** for plate i in 0..p (fixed order), for j in 1..k[i]:
   `dir = random_tangent(rng, pos[primary_i])`;
   `r = uniform_range(rng, 0.30, 0.70) * 2.0 * f[i].sqrt()` (2√f = the
   Euclidean chord radius of a cap of fraction f — no trig needed);
   `cell = grid.nearest_cell(normalize3(add3(pos[primary_i], scale3(dir, r))),
   Some(primary_cell_i))`. If that cell is already any seed's cell, the helper
   is skipped (deterministic; keeps "every plate non-empty" trivially true:
   distinct primaries each claim their own cell at cost 0).
   NOTE: the RNG draws for a helper happen unconditionally before the skip
   check, so the draw sequence never depends on grid-level collision luck.
6. **Priority-queue flood fill** (multi-source Dijkstra, serial):
   `BinaryHeap<Reverse<(u64, u32, u32)>>` keyed **(total_cost, cell id, owner
   plate id)** — cost first, cell id second per the pinned rule, owner as the
   final component so the key is a total order over pushes. Integer costs make
   ties exact and platform-free. Seed pushes at cost 0 in placement order.
   Pop loop: first pop of an unassigned cell assigns `plate_id[cell] = owner`
   and increments `count[owner]`; later pops of the cell are skipped. For each
   CSR neighbor v (ring order) push
   `(cost + step_cost(edge, owner), v, owner)` if v unassigned, where
   `step_cost = max(1, (b_e as u64 * m_eff as u64 + 128) >> 8)` and
   **per-plate growth-cost steering** is: `m_eff = m[i]` while
   `count[i] < target[i]`, else `m[i] * 4` (soft cap: an over-target plate
   keeps growing only where nobody else competes — the sphere is always fully
   covered because the fill is exhaustive). All heap ops serial; counts are
   updated at pop time, so `m_eff` is a deterministic function of the serial
   pop sequence.
7. **Helper-seed collapse:** by construction — sub-seeds carry their plate id
   as owner from the first push; there are never provisional per-seed labels,
   so there is no remap step to get wrong, and ids are dense `0..p` on
   return. (This is the design's resolution of the ≤32/dense-id risk: the
   generator emits at most 24 ids, period.)
8. `debug_assert`: every count > 0; every id < p.

Complexity: O(E log E) with E ≈ 6n pushes — ~0.3 s at L7, ~5 s at L9 (setup
runs once per generation; acceptable).

### 2.3 Candidate (b) — `WarpedVoronoi` (warped-distance Voronoi + annealing)

RNG: same single "plate-seeds" stream, draws in this order: (1) three warp
seeds `w1, w2, w3 = rng.next_u64()` ×3; (2) anneal seed
`wa = rng.next_u64()`; (3) targets via H1 (for the area bias); (4) seed
placement via H2.

1. **Uniform seeds:** `farthest_point_seeds(rng, grid, p)` — "uniform" =
   uniformly spread positions, exactly the incumbent's placement.
2. **Warp noise:** reuse `noise_stage::fbm` — it is `pub(crate)` in
   worldmaker-sim (noise_stage.rs:63), so tectonics code CAN link it while the
   renderer cannot (the render-only guard is unaffected). It is libm-free
   value-noise fBm (splitmix64 lattice, base freq 1.6, lacunarity 2, gain
   0.5). Call it with `octaves = 3` — base freq 1.6 on the unit sphere gives
   ~0.6 rad features, continent-scale, exactly the "2–3 octave low-frequency"
   the contract pins. **No new noise is invented; no std trig anywhere.**
   Warp field: `W(p) = [fbm(p, w1, 3), fbm(p, w2, 3), fbm(p, w3, 3)]`,
   warped sample point `q(c) = normalize3(add3(pos[c], scale3(W(pos[c]),
   WARP_AMP)))` with `WARP_AMP = 0.18` (chord units; typical |W| ≈ 0.9 →
   ~9–10° of displacement — tuned by the judge panel, logged if changed).
3. **Assignment with area bias:** per cell (par_iter_mut, per-element, same
   pattern as setup.rs:77): `pid = argmax_k (dot3(q(c), seed_pos[k]) +
   bias[k])`, strict `>` so ties go to the lowest seed index. `bias[k] =
   BIAS * ((f[k] / f_mean).sqrt() - 1.0)` with `BIAS = 0.15`, `f[k]` from the
   H1 ladder, `f_mean = 1/p` — an additive cosine-space bias grows/shrinks
   caps toward the heavy-tailed targets, otherwise farthest-point Voronoi
   areas are near-equal and the CV gate is unreachable. (This is a designed
   extension of the pinned candidate wording — "uniform seeds" is read as
   uniform placement — logged here so the judge scores it as-is.)
4. **Non-empty repair (deterministic serial):** count ids; for k in 0..p, if
   `count[k] == 0`, set `plate_id[seed_cell[k]] = k` (and fix both counts).
   Warping can in principle steal a seed's own cell; this restores the
   invariant with a one-cell plate (rare; visible in PNGs if it ever fires).
5. **Boundary-annealing pass — exact definition.** 3 fixed Gauss–Seidel
   sweeps, serial, cells in ascending id order, updates visible within the
   sweep (fixed order ⇒ deterministic). Noise-temperature schedule
   `LAMBDA = [1.5, 0.75, 0.0]` (sweep t uses λ = LAMBDA[t]):
   - Only cells with ≥1 foreign CSR neighbor are considered.
   - Candidate set = {current plate} ∪ {plate ids of the CSR neighbors},
     iterated in ascending plate id.
   - Score(c, k) = (number of CSR neighbors of c with plate k) as f32
     + λ · η(c, k), where η(c, k) =
     `(splitmix64(wa ^ ((c as u64) << 32) ^ k as u64) >> 40) as f32 /
     16_777_216.0` ∈ [0,1) — pure hash noise, no stream state, so sweep
     order can't desynchronize draws.
   - Flip to argmax score, ties to the LOWEST plate id; a flip is refused if
     it would empty a plate (`count[current] == 1`) or if c is a seed cell
     (seeds stay pinned — non-emptiness is preserved by construction).
   - Effect: sweep 1 (λ=1.5) roughens — noise outvotes 1-neighbor deficits,
     boundary cells defect and re-defect into fjords; sweep 2 (λ=0.75) only
     breaks near-ties; sweep 3 (λ=0) is a strict-majority cleanup that
     removes 1-cell specks so the map isn't salted with orphans.

### 2.4 Candidate (c) — `HybridGrowthWarp`

Precisely: **candidate (a)'s machinery with (b)'s noise injected into the
growth costs, then (b)'s annealing pass on the result.**

1. Draws from "plate-seeds", in order: hybrid noise seed
   `wn = rng.next_u64()`; anneal seed `wa = rng.next_u64()`; then (a)'s
   full sequence (targets H1, primaries H2, helpers).
2. Fill exactly as §2.2 steps 1–8 with one change: the per-edge cost gains a
   destination-cell terrain factor,
   `step_cost = max(1, (b_e * m_eff * noise_f[v]) >> 16)` with per-cell
   `noise_f[v] = clamp(round(256.0 * (1.0 + 0.6 * fbm(pos[v], wn, 3))), 64,
   512) as u64` precomputed once (per-element parallel). Growth fronts advance
   unevenly through the noise field, so the meeting lines are irregular at
   the fBm scale — sinuosity from the same mechanism as (b), area control
   from (a).
3. (b)'s annealing pass (§2.3 step 5) verbatim, seeded by `wa`, on the filled
   map — adds cell-scale roughness on top of the front-scale wiggle.

---

## 3. Metrics implementation

**File:** `crates/worldmaker-sim/src/tectonics/metrics.rs`, `pub mod metrics`
re-exported from tectonics/mod.rs — it must be public forever: the CI gate
test (worldmaker-sim/tests/) and harness.rs (worldmaker-app) both call it.
Everything here is **serial and id-ordered**; f64 accumulation in fixed order
(IEEE-exact per op ⇒ bit-stable); geometry via dmath only.

```rust
pub fn plate_area_cv(plate_id: &[u32], plate_count: u32) -> f64;

pub struct SinuosityReport {
    pub weighted_mean: f64,      // the gated number
    pub open_segment_count: u32,
    pub loop_count: u32,         // includes zero-endpoint lassos
    pub junction_count: u32,
    pub total_polyline_rad: f64, // numerator
    pub total_baseline_rad: f64, // denominator
}
pub fn boundary_sinuosity(grid: &Grid, plate_id: &[u32]) -> SinuosityReport;
```

### 3.1 Plate area CV

Cell counts (pinned; pentagon area deficit is noise at this precision):
serial loop over `plate_id` in id order into `counts[plate_count]`;
`mean = n / p`; `cv = sqrt(Σ(count_i − mean)² / p) / mean` — the Σ runs in
plate-id order, f64. Asserts every id < plate_count and every count > 0.

### 3.2 Boundary sinuosity

Definitions (implements the pinned metric exactly):

- **Interface edge** = unordered adjacent cell pair (a,b), a<b,
  `plate_id[a] != plate_id[b]`. Enumerated by iterating cells in id order and
  CSR neighbors in ring order, keeping pairs with b>a — into a **sorted Vec**
  (already sorted by construction) with a parallel `visited: Vec<bool>`;
  edge→index by binary search. No HashMap anywhere.
- **Goldberg vertex** shared by two consecutive interface edges = a grid
  triangle = a mutually-adjacent triple {a,b,x}. From CSR + CCW rings
  (canonically rotated to start at the lowest neighbor id, grid.rs:334–343):
  the two triangles flanking edge (a,b) are `{a, b, ring_a[i−1]}` and
  `{a, b, ring_a[i+1]}` (cyclic), where `ring_a = neighbors_of(a)` (a = the
  lower cell id) and `ring_a[i] = b`. This is the "CCW ring walk" — no
  auxiliary triangle map is needed for walking.
- **Triple junction** = a grid triangle whose 3 corner cells carry 3 distinct
  plate ids. Enumerated canonically by iterating `grid.triangles` in index
  order. Junction point = `normalize3` of the sum of the three corner cell
  positions **added in ascending cell-id order** (fixed order ⇒ the same
  physical junction always yields the bit-identical point).
- At a 2-plate triangle {P,P,Q} exactly 2 of the 3 edges are interface edges
  ⇒ curve continuation is **unique** (no tie-break needed mid-walk). At a
  junction all 3 edges are interface edges of 3 different pairs ⇒ curves
  terminate there. So the interface of each plate pair is a disjoint union of
  junction-terminated **open segments** and junction-free **closed loops**.

**Traversal (all serial):**

1. For each triangle in `grid.triangles` index order: if it is a junction,
   take its 3 interface edges in ascending (a,b) order; for each unvisited
   one, walk away from this triangle: at edge (a,b) with previous third-cell
   x_prev, the next third cell x_next is the other flanking common neighbor
   (from a's CCW ring as above). If `plate_id[x_next]` is a third plate ⇒
   terminate at junction {a,b,x_next}. Else the next edge is (x_next, b) if
   x_next is on a's plate, or (a, x_next) if on b's. Mark every edge visited.
2. After all junction-seeded walks: scan the interface-edge Vec in order; any
   unvisited edge starts a **loop**; the initial walk direction is toward the
   flanking common neighbor with the lower cell id (deterministic); walk
   until the start edge recurs.

**Polyline and lengths.** For each segment/loop, the plate pair (P,Q), P<Q,
is constant along it. The polyline is the **lower-plate-side cell-center
chain**: per interface edge take the side cell whose plate is P; deduplicate
consecutive repeats (consecutive distinct side cells are always grid-adjacent
— they share a flanking triangle — so steps are single-cell hops,
≤ ~0.03 rad at L6). For open segments, prepend/append the two junction
points. All step lengths and the endpoint distance use `arc_len3` (§3.3) —
**one implementation for every distance in the metric**, so numerator,
denominator, incumbent measurement and gate all move together by
construction.

**Aggregation (the pinned weighted average, plus the loop rule):** per open
segment, sinuosity = polyline length ÷ great-circle distance between its two
junction points, weight = that great-circle distance. Because
weight × sinuosity = polyline length, the weighted mean needs no per-segment
division:

```
weighted_mean = (Σ_open len + Σ_loop len) / (Σ_open gc(J1,J2) + Σ_loop π·diam)
```

— numerically stable even when two junctions are near-coincident (a tiny
denominator contributes its tiny weight, never a blow-up).

**Closed loops (zero junctions) and lassos (both endpoints the same junction
triangle, gc = 0):** contribution defined deterministically as
len_loop / (π · diam), weighted by π · diam, where `diam` is the two-sweep
pseudo-diameter of the loop's polyline vertices: v0 = lowest cell id in the
loop; v1 = farthest vertex from v0 (arc_len3, ties to lower cell id);
v2 = farthest from v1 (same tie rule); diam = arc(v1, v2). O(k), fully
deterministic, and equals ~1 for a clean circular enclave (perimeter of a
spherical cap ≈ π × its diameter for small caps). Loops are counted in
`loop_count` either way.

Sums accumulate in f64, in traversal order, which is itself canonical
(triangle index order, then edge-list order). Report rounds nothing; the
harness rounds for JSON.

### 3.3 The dmath gap — RESOLVED: new tested arc-length helper

**Decision: option (i) — add `pub fn arc_len3(a: [f32; 3], b: [f32; 3]) ->
f32` to worldmaker-core dmath — implemented by midpoint-normalize subdivision
(option (iii)'s algorithm) rather than a polynomial/Newton `asin`/`acos`.**

Algorithm (full range [0, π], only +, −, ×, / and sqrt, fixed order, fixed
iteration count — the exact op set dmath already licenses):

```rust
pub fn arc_len3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let s = add3(a, b);
    if dot3(s, s) < 1e-12 { return PI_F32; } // antipodal guard, documented
    let mut m = b;
    for _ in 0..4 {               // fixed depth: angle/16, always <= ~0.196 rad
        m = normalize3(add3(a, m));
    }
    let d = sub3(m, a);
    let c = dot3(d, d).sqrt();    // residual chord
    // 2*asin(c/2) as a fixed-order odd series in c (exact to ~3e-9 abs
    // at c = pi/16 — below f32 rounding after the *16):
    16.0 * (c * (1.0 + c * c * (1.0 / 24.0 + c * c * (3.0 / 640.0))))
}
```

Justification vs the alternatives:
- A polynomial/Newton `acos`/`asin` is deterministic too, but is
  ill-conditioned near ±1 and needs range-splitting + approximation-theory
  arguments; the bisection form needs none — its error analysis is one line
  (residual ≤ π/16 ⇒ next series term ≤ 3e-9 abs), and every op is already
  on dmath's approved list.
- An all-chord metric (option ii) silently distorts long baselines (chord(π)
  = 2 vs arc π) and would make sinuosity depend on segment scale;
- The antipodal guard returns π when |a+b|² < 1e-12 (true angle there is
  within 1e-4 rad of π — unreachable in practice for junction pairs of a
  ≥8-plate map, but the function must be total and deterministic).

Tests (dmath tests module): against `f64 acos(dot)` at angles
{0, 1e-4, 0.01, 0.3, 0.75, π/2, 2.5, 3.0, 3.1, π−1e-3} and 200 random pairs
from a fixed `sub_rng(9, "dmath-test", "arclen")` stream; tolerance 1e-5
relative + 1e-6 absolute; plus exact symmetry `arc_len3(a,b) ==
arc_len3(b,a)` (the algorithm is asymmetric in form — bisecting toward `a` —
so the test pins that the fixed roles still commute in value within f32; if
it does not hold bit-exactly, the implementation canonicalizes by swapping so
that the smaller-id... — NO: canonicalize on VALUE, not ids: order (a,b) by
lexicographic comparison of their f32 bit patterns before computing, making
symmetry exact by construction. Implement the canonicalization; test asserts
bit-equality both ways.)

**Gate numbers depend on this choice** — therefore the incumbent measurement
(§6, commit M1), all candidate scores, the gate constants and the harness
rows all call the same `metrics.rs`, which calls the same `arc_len3`. There
is no second implementation anywhere.

Decision-log rows to add (drafts): see §9 step 6.

---

## 4. The fast CI gate test

**File:** `crates/worldmaker-sim/tests/plategen_gate_tests.rs` (new; runs in
normal CI — NOT `#[ignore]`).

**Configs:** `TectonicsParams::default()` (in-band, matches goldens/harness);
setup-only via the public `SimState::setup(seed, &grid, &params)`
(step.rs:260) + pub `plate_id` — no pipeline run, no elevation derive; the
test reads plate geometry only.

| grid | seed | why |
|---|---|---|
| L7 | 42 | pinned by the order; the golden/harness seed |
| L6 | 7 | competition-set member (§6); a second independent stream at the golden level, small documented constant (already used in rng tests) |
| L6 | 0xc4be0bf8f497a575 | competition-set member; `seed_from_text("cyrus")` — the app-default seed of the committed BEFORE screenshots, so the gate pins the exact world whose look raised Fix 2 |

Both L6 seeds are members of the 5-seed competition set, chosen NOW (Stage D,
before any measurement exists) so they cannot be cherry-picked later, and so
the incumbent + winner numbers for the gate triple are already committed by
the competition protocol before the gate constants are set.

**Assertions per config:**
1. Structural: `plate_id.len() == n`; every id `< 12`; every plate non-empty;
   `plates.len() == 12` and all alive (⇒ ≤ 32 trivially).
2. `plate_area_cv(...) >= GATE_CV`.
3. `boundary_sinuosity(...).weighted_mean >= GATE_SINUOSITY`.
4. Determinism smoke: a second `SimState::setup` with the same inputs yields
   a bit-identical `plate_id` (cheap at these levels, catches any stray
   nondeterminism in the winner immediately rather than at the goldens).

`GATE_CV` / `GATE_SINUOSITY` are consts with a doc comment listing, verbatim,
the incumbent's measured values on the gate triple (from commit M1's JSON)
and the winner's — the "strictly excludes incumbent" proof is readable at the
constant. Provisional values 0.5 / 1.15; final values set at M3 by the rule
in §6 step P3.

**Expected runtime:** grid builds ~28 + 11 + 11 ms (Air-measured), three
setups (winner's Dijkstra ≈ 0.2–0.4 s at L7, less at L6, ×2 for the
determinism re-run) + metrics (ms) ⇒ **≈ 1–2 s release**, comfortably inside
the 3.2 s current whole-suite budget's ballpark; CI's test job runs
`--release`.

---

## 5. The dev-only `#[ignore]` PNG panel test

**File:** `crates/worldmaker-sim/tests/plate_panel.rs`. Precedent for
`#[ignore]`: `generates_l9_without_panic`. Requires `image` added under
`[dev-dependencies]` of worldmaker-sim (approved workspace dep; note: CI
clippy `--all-targets` will compile it — accepted cost, flagged in the code
map).

Two ignored tests during the competition:

**`render_plate_maps`** — for each generator in `all_generators()` × each of
the 5 seeds (§6) at **L7**: run `generate(...)` (candidates) or
`SimState::setup` (sanity that wiring matches), rasterize an equirect
plate-id map, write PNG.
- Raster: 1024×512 RGB. Per row: lat from the row center; per pixel: lon,
  unit vector via std `sin/cos` — allowed: this is a display path, nothing
  here feeds a hash or a committed metric — then
  `grid.nearest_cell(p, hint)` with the previous pixel's cell as hint
  (the exact pattern of `rasterize_cell_ids`, render.rs:43–61). Row-parallel
  rayon is fine (rows are independent).
- Color assignment: a fixed 24-entry RGB table copied from layers.rs
  `PLATE_COLORS` values (duplicated into the test file — the sim crate
  cannot link the app binary), indexed by `plate_id % 24`; cells with ≥1
  foreign CSR neighbor are darkened ×0.45 so boundary sinuosity is visible
  at a glance.
- File naming: `{dir}/plates-{generator}-L7-seed{label}.png` where
  {generator} ∈ incumbent|growth|warped|hybrid and {label} is the decimal
  seed, except `cyrus` for 0xc4be0bf8f497a575 → e.g.
  `plates-hybrid-L7-seedcyrus.png` (20 files).
- Output dir: env `WM_PLATE_PANEL_DIR`, default `target/plate-panel/`
  (created; never committed wholesale). B commits a curated judge set —
  one PNG per generator at seed cyrus plus the seed-42 set — under
  `docs/media/feel-pass/plate-panel/` alongside the logged decision.

**`score_generators`** — computes CV + sinuosity for all 4 generators × 5
seeds × {L6, L7} and writes them via `worldmaker_io::ResultsFile` to
`docs/results/plategen-feelpass-Daniels-MacBook-Air.json` (schema-conformant:
topic `plategen`, phase `feelpass`; keys like
`{generator}_area_cv_l7_seed42`, `{generator}_sinuosity_l6_seedcyrus`).
Rule-3 compliance: the judge's numbers are committed JSON, not chat. Requires
`worldmaker-io` as a **dev-dependency** of worldmaker-sim (internal crate, no
cycle: io does not depend on sim; not an external-dep approval matter).

After judging (phase P4): `score_generators` is deleted with the losers (its
output file remains as the permanent record); `render_plate_maps` is kept,
reduced to rendering `SimState::setup` output for the 5 seeds — a standing
dev tool for future plate work.

---

## 6. Competition seeds and the measure/commit-first protocol

**The 5 fixed seeds** (chosen now, in Stage D, before any measurement):

| seed | label | why |
|---|---|---|
| 42 | seed42 | pinned by the order; golden + harness seed |
| 0xc4be0bf8f497a575 | seedcyrus | app default (`seed_from_text("cyrus")`); the committed BEFORE screenshots' world — the map the feel complaint is about |
| 7 | seed7 | small documented constant, distinct stream family (used in core rng tests) |
| 1002 | seed1002 | arbitrary committed constant |
| 271828 | seed271828 | arbitrary committed constant (e); documents that seeds are picked for provenance, not results |

All 5 measured at **both L6 and L7** (10 setup runs per generator; seconds of
wall time). Judging weighs the L7 numbers (the default preset the feel
complaint was raised on) with L6 as a cross-level stability check.

**Protocol — ordered commits, every commit green, all on B's branch:**

- **P0 (commit M1) — metrics + incumbent baseline, commit numbers FIRST.**
  Adds `dmath::arc_len3` (+tests), `tectonics::metrics` (+unit tests on
  hand-built plate maps: e.g. an L4 two-half sphere split — CV 0 (equal
  halves), one loop... actually two junction-free loops? a bisected sphere
  has ONE closed-loop boundary and zero junctions — pins the loop rule; and a
  three-plate wedge map pinning junction count = 2 and sinuosity ≈ 1 within
  the hex-zigzag allowance), and the harness plategen rows (§8). **No
  generator change.** Run the harness →
  `docs/results/tectonics-feelpass-Daniels-MacBook-Air.json` now contains the
  incumbent's CV/sinuosity on all 10 (seed, level) pairs plus the unchanged
  phase-1 metrics and determinism hashes (which must equal the CURRENT golden
  constants — free proof nothing moved). Commit code + JSON together.
- **P1 (commit M2) — refactor + candidates.** setup.rs sections 1–2 move
  verbatim into `plate_gen::Incumbent`; goldens must stay green (proof of
  bit-exactness). Trait + 3 candidates + panel tests land. Default behavior
  still incumbent.
- **P1.5 (commit M2.5) — cadence + harness XL rows** (§7, §8): golden-safe
  (L9-only arm; L8 untouched), committed separately so the world-changing
  commit stays minimal.
- **P2 — judging (no commit until decided).** Run `render_plate_maps` +
  `score_generators`. Judge panel = subagents per repo rule 4, scoring the
  committed metrics JSON + the PNGs (criteria: gates clearable with margin on
  all 10 measurements; Earth-likeness of the panel — one giant plate, varied
  sizes, wiggly-but-not-noisy boundaries, no confetti). Scores + decision →
  decision-log; curated PNGs + `plategen-feelpass-*.json` committed with the
  decision (commit M2.6).
- **P3 — set final gates.** Rule: `GATE = max(provisional, incumbent_best_on
  _gate_triple + margin)` and `≤ winner_worst_on_gate_triple − margin`, with
  margin 0.05 (CV) / 0.02 (sinuosity); metrics are exactly reproducible, so
  margins buy future-tuning headroom, not noise tolerance. If the two bounds
  cross, adjust with logged reasoning (the pinned contract allows exactly
  this). Expected incumbent ballpark (prediction, NOT data: near-equal
  Voronoi areas ⇒ CV ~0.2–0.35; hex zigzag on near-great-circle boundaries ⇒
  sinuosity ~1.03–1.10) — M1's JSON is the arbiter.
- **P4 (commit M3, THE final code commit) — winner wired + goldens, one
  commit:** setup.rs calls the winner; losers/trait/`score_generators`
  deleted; module demoted to private; gate test added with final constants;
  goldens regenerated per §9 (same commit — this is what keeps every branch
  commit green while honoring "regenerate exactly once, final commit");
  harness re-run → feelpass JSON now carries the winner's plategen rows +
  new determinism hashes equal to the new constants; decision-log rows;
  CLAUDE.md cadence line if not already in M2.5. PR → CI green → merge
  (B merges first per the pinned order).

The incumbent's numbers remain committed twice over: in M1's version of
`tectonics-feelpass-*.json` (git history) and permanently in
`plategen-feelpass-*.json` — "final gates strictly exclude incumbent scores"
stays auditable after the incumbent code is deleted.

---

## 7. L9 keyframe-cadence decision (decided now; B implements)

**Numbers.** Keyframes are exactly 16 B/cell (keyframe.rs:67–84). L9 =
2,621,442 cells ⇒ 41.9 MB/keyframe. Span clamp 200–2000 My (mod.rs:111).
At the current `>= 8` branch (20 My): 2 Gy ⇒ 101 keyframes ≈ **4.24 GB** vs
the 1 GB budget (and vs 16 GB total on the current primary dev machine). At
100 My: 2 Gy ⇒ 21 keyframes ≈ **0.88 GB**; 500 My default ⇒ 6 keyframes ≈
0.25 GB. At 40 My: 2 Gy ≈ 2.1 GB — still over. Capping span per-level would
push preset knowledge into params/UI (C's surface) and contradict the 200–
2000 clamp; keep-20-My-recorded-not-budgeted quadruples L8's already-tolerated
excursion on a machine with 16 GB.

**DECISION: L9 keyframe interval = 100 My.** New arm in
`keyframe_interval_my` (mod.rs:63): `level >= 9 → 100.0`, `level == 8 →
20.0`, else `10.0`. L6/L7 (goldens, phase-1 results) and L8 (pinned) do not
move; the L5 sanity pin (21 keyframes @ 200 My) is untouched. DT_MY stays
2.0 ⇒ total step count and wall time are unchanged (fewer elevation solves,
marginally faster). `TectonicsHistory` stores its own `keyframe_interval_my`
(keyframe.rs:237–259), so the era picker and `nearest_index` are correct per
run with no app change. Worst-case future resume replay (Phase 2 plate drag)
is 50 steps ≈ ~6 s at L9 — acceptable for an Ultra preset. Scrub granularity
at Ultra is 100 My — 21 stops over a full 2 Gy history; Ultra is a
final-render preset, not the exploration preset (L7/L8 keep 10/20 My).

**Doc comment** (replaces mod.rs:59–62):

```rust
/// Keyframe cadence (My): 10 My per the spec at L6/L7 — the levels the WO's
/// 1 GB / 2 Gy budget is defined for (527 MB measured) — 20 My at L8
/// (~1.06 GB at 2 Gy; recorded, not budgeted), and 100 My at L9: at
/// 16 B/cell a 2.62 M-cell keyframe is ~42 MB, so 20 My would cost 4.2 GB
/// over 2 Gy; 100 My keeps a maximum-span Ultra history at ~0.88 GB, the
/// same ballpark as L8. Histories carry their own interval, so mixed
/// cadences never confuse the era picker. Decision log 2026-08 (WO-0003).
```

**CLAUDE.md** (Key technical facts, one added bullet — B edits this line
only):

```
- Keyframes: 16 B/cell; cadence 10 My (L6/L7), 20 My (L8), 100 My (L9); a
  history stores its own interval. The 1 GB / 2 Gy budget is defined at L7.
```

**Decision-log row** (draft): "L9 keyframe cadence = 100 My (new
keyframe_interval_my arm; L6/L7 goldens and L8 untouched): 42 MB/keyframe
makes 20 My cost 4.2 GB over 2 Gy vs the 1 GB budget; 100 My keeps max-span
Ultra at ~0.88 GB, the same ballpark as L8's 1.06 GB. Histories carry their
own interval. | default"

---

## 8. harness.rs additions (B-owned; A and C must not touch)

`SEED = 42` (harness.rs:19) stays. `stability()` keeps its **hardcoded 0.29
target** (harness.rs:311) — it is intentionally NOT `params.land_fraction`
(the code-map trap); no new code paths parameterize it, and the new rows
never call `stability()`.

**New plategen section** (runs before the existing 500 My L7 block; setup-only,
adds ~2–4 s): for each of the 5 seeds × {L6, L7}: `Grid::build`,
`SimState::setup(seed, &grid, &TectonicsParams::default())`,
`metrics::plate_area_cv` + `metrics::boundary_sinuosity`. Keys:

```
plategen_area_cv_l{6|7}_seed{42|cyrus|7|1002|271828}      (f64, 4 dp)
plategen_sinuosity_l{6|7}_seed{...}                        (f64, 4 dp)
plategen_open_segments_l{6|7}_seed{...}   plategen_loops_l{6|7}_seed{...}
plategen_seed_cyrus: "0xc4be0bf8f497a575"                  (echo)
plategen_gate_cv / plategen_gate_sinuosity                 (echo constants)
plategen_gates_pass          (bool, evaluated on the gate triple only:
                              L7 seed42 + L6 seed7 + L6 seedcyrus)
```

`all_acceptance_pass &= plategen_gates_pass` — but ONLY from commit M3
onward; in M1 the section records incumbent numbers with the pass key
omitted (no gate constants exist yet), so M1's committed JSON keeps
`all_acceptance_pass` meaningful.

**Optional L8/L9 rows** (pinned: "1 Gy wall time, measured keyframe bytes"),
behind env `WM_HARNESS_XL=1` (precedent: `debug_keyframe_stats` env gating)
so the default harness stays fast:

```
run_1gy_l8_s, keyframe_bytes_1gy_l8        (1 Gy @ L8, 20 My → 51 kf ≈ 0.53 GB)
run_1gy_l9_s, keyframe_bytes_1gy_l9        (1 Gy @ L9, 100 My → 11 kf ≈ 0.46 GB)
keyframe_interval_my_l9: 100.0             (echo of §7)
```

Records, not gates (Air interim; CLAUDE.md machine note): no `all_pass`
contribution. Estimated Air wall: L8 1 Gy ≈ 15 s, L9 1 Gy ≈ 55–70 s (L7 1 Gy
= 3.44 s × ~16 cells ratio), both within a manual run's patience; memory
peaks < 1 GB history + ~0.3 GB SimState — safe on 16 GB.

The perf-side fps rows at L8/L9 belong to C's perf script
(`perf-feelpass-*`), not this harness — partition per the pinned track split.

---

## 9. Golden regeneration protocol — exactly once, final commit (M3)

Per the code map's recipe (d-determinism.md §1), executed once, inside commit
M3 on B's branch:

1. Working tree state: winner wired, losers deleted, gate test in, cadence +
   harness rows already committed (M2.5). The golden tests now fail in the
   working tree — expected and momentary; they are green again within the
   same commit.
2. Run `cargo run --release -p worldmaker-app -- --tectonics-results
   docs/results/tectonics-feelpass-Daniels-MacBook-Air.json`. Its
   determinism section's config (L6, 500 My, seed 42, defaults) is IDENTICAL
   to the golden config, so `determinism_elevation_hash_l6_500my_seed42` and
   `determinism_plate_hash_l6_500my_seed42` ARE the new goldens.
3. Paste both values into determinism_tests.rs:
   `GOLDEN_TECTONIC_ELEVATION_L6_SEED42` and
   `GOLDEN_TECTONIC_PLATES_L6_SEED42` **move together** — the plate map
   changed, and the "cratons" stream's draw alignment is geometry-entangled
   (setup.rs:161–185), so continents and elevation change too. This is the
   expected whole-world change, not a red flag. The crust-type hash changes
   in the results JSON only (not a golden). `GOLDEN_HASH_L6_SEED42` (phase-0
   noise) and `determinism-phase0-*.json` MUST NOT move — verify.
4. Update both constants' `/// History:` doc-comments: "regenerated
   2026-XX-XX for WO-0003 Fix 2: t=0 plate generator replaced
   ({winner}); craton-stream draw alignment is geometry-entangled, so this
   is a whole-world regeneration (decision log)."
5. `cargo test --workspace --release` — everything green, including the L5
   sanity pins, resume-bit-exact, overlay-independence and the new gate
   test. If the L5 sanity test fails (land anchor, plate band, feature
   counts), the WINNER gets tuned and steps 2–5 repeat BEFORE any commit —
   the tests are never edited to fit ("never 'fix' the test").
6. Decision-log rows (drafts):
   - "dmath gains arc_len3: full-range great-circle length by 4 fixed
     midpoint-normalize bisections + fixed-order odd series on the residual
     chord; only IEEE-exact ops; canonicalized argument order; tested vs f64
     acos. Sole distance used by the committed plate metrics. | default"
   - "Fix 2: t=0 plate generator replaced by {winner} (competition of
     incumbent + 3 candidates on 5 fixed seeds × L6/L7; scores in
     docs/results/plategen-feelpass-Daniels-MacBook-Air.json; judge decision
     summarized here). Metrics: plate-area CV (cell counts) and boundary
     sinuosity (junction-to-junction center-polylines; loop rule
     len/(π·two-sweep-diameter)). Final gates CV ≥ {X}, sinuosity ≥ {Y} —
     strictly above incumbent {measured values}. Both tectonic goldens
     regenerated once (craton-stream alignment is geometry-entangled ⇒
     whole-world change); phase-0 golden untouched. | default"
7. Commit M3 = setup.rs swap + deletions + gate test + both constants + the
   regenerated `tectonics-feelpass-Daniels-MacBook-Air.json` + decision-log
   (+ CLAUDE.md line if pending). Cross-check inside the commit: the JSON's
   determinism hex strings equal the new constants.
8. After C's merge and after A's merge: re-run the sim test suite and record
   in the WO checklist that the hashes did not move (pinned obligation; B
   notes the expectation in the PR description so whoever lands later runs
   it).

No other branch or commit touches golden constants; A and C cannot move them
without failing CI (their territories never touch the sim path).

---

## 10. Phase-1 acceptance constraints — candidate compliance checklist

Constraints every candidate honors BY DESIGN (verified again at P2/P4):

1. **`assert!(nd <= 32)`** (step.rs:428): generators emit exactly
   `plate_count ≤ 24` dense ids; sub-seeds are labeled with final plate ids
   from the first push (no helper-id namespace exists to leak). Breakup's
   ceiling (PLATE_CEIL = 24) is untouched.
2. **Setup clamp 8..=24 stays** (mod.rs:108): not touched; the 6 in "6–24"
   is the runtime alive band (PLATE_FLOOR, step.rs:102), enforced by the sim,
   gated by harness `stability()` — also untouched.
3. **Overlay independence** (tectonics_tests.rs:371–396): structural —
   `PlateGenParams` contains only `plate_count`; overlays are unreachable
   from any generator. The existing test remains the enforcement.
4. **L5 sanity pins** (tectonics_tests.rs:73–141): 21 keyframes @ 200 My
   (cadence <8 arm untouched), `hotspots.len() == 6` ("hotspots" stream
   untouched — candidates draw only from "plate-seeds"), anchor land
   ±0.005 (the t=0 sea-level bisection solves land fraction on elevation,
   independent of plate geometry), final land ±0.05, alive plates 6..=24,
   ridge/trench/ocean-age cell-count floors (generic tectonic behavior, not
   layout-specific). Verified at P2 for the front-runner and at P4 for the
   winner.
5. **Bit-exact rerun + resume** (tectonics_tests.rs): generators are
   deterministic pure functions of (master_seed, grid); resume reads
   keyframes, never re-runs setup — unaffected.
6. **Breakup makes gates t=0-only** (step.rs:1051, 1099–1117 splits along
   random great circles): accepted and by design — all Fix 2 metrics and
   gates are computed on `SimState::setup` output (t=0) only. Expectation
   note for the AFTER screenshots: at the default 500 My, ~4 of the
   baseline's 17-per-2-Gy breakups have occurred, so most visible boundaries
   are still generator-shaped; late-era maps will always re-grow straighter
   boundaries and that is out of Fix 2's scope.
7. **Alive-band floor**: plates die at zero cells with no floor
   (step.rs:673–674) — see risk R1.
8. **Keyframe u16 plate ids / dense PlateState indexing**: ids 0..p_count
   contiguous — safe.
9. **Harness 0.29 trap**: untouched (§8).
10. **No new external deps**: `image` (approved) as sim dev-dep;
    `worldmaker-io` as sim dev-dep is an internal workspace edge.

---

## 11. Residual risks

- **R1 — heavy-tailed runts vs the alive-plate floor.** Death-by-consumption
  has no PLATE_FLOOR guard (step.rs:673–674 flips `alive = false` at zero
  cells; only suturing respects the floor), and the phase-1 baseline already
  touches min alive = 6 over 2 Gy. A 1.5–3% plate is more consumable than an
  incumbent equal-share plate. Mitigations: f_small is drawn from the upper
  half of the pinned band (§2.1); the L6 2 Gy stability run is executed for
  the front-runner at P2, before the winner is declared, not discovered at
  P4; the tuning lever (raise f_small toward 3%, or lower the giant's band
  edge) stays inside the pinned bands. If the band itself proves
  incompatible with stability, that is a logged gate/band adjustment per the
  pinned "adjusted only with logged reasoning".
- **R2 — sinuosity floor from hex zigzag.** Even razor-straight boundaries
  measure > 1.0 on a cell-center polyline (lattice zigzag, up to ~1.15 in
  the worst orientation, less after averaging). If the incumbent measures
  near 1.10, the 1.15 provisional gate discriminates weakly; the P3 rule
  (incumbent + 0.02 margin, below winner − 0.02) may need a raised gate —
  logged, and the CV gate carries the discrimination burden regardless.
- **R3 — annealing sweep cost.** Sweeps are serial over all n cells × 3
  iterations; at L9 that is ~7.9 M serial score evaluations per generation
  (~1–2 s) on top of the fill. Acceptable (setup runs once per generation);
  if profiling disagrees, restrict sweeps to the boundary cell set +
  1-ring, which is a pure optimization with identical output ONLY if the
  visit order over that subset stays ascending-id — implementers must keep
  the order rule, not just the subset.
- **R4 — warped-Voronoi CV reachability.** Even with the additive bias,
  candidate (b) may undershoot CV ≥ 0.5 (bias tuning is indirect). That is a
  legitimate competition outcome (it can lose on CV while demonstrating the
  warp mechanism the hybrid inherits), not a design failure.
- **R5 — arc_len3 symmetry.** Resolved by canonicalizing argument order on
  f32 bit patterns (§3.3); the test pins it. Without canonicalization the
  bisection is order-sensitive in the last ulp.
- **R6 — competition wall time.** score_generators = 4 generators × 10
  setups; growth/hybrid ≈ 0.3–0.5 s each at L7 ⇒ well under a minute total.
  Panel PNGs similar. No CI impact (`#[ignore]`; clippy compiles them once).

---

## Commit plan (summary)

| commit | contents | goldens |
|---|---|---|
| M1 | arc_len3 + metrics + tests; harness plategen rows; feelpass JSON with INCUMBENT numbers | green (proof: JSON hashes == current constants) |
| M2 | plate_gen module: trait + incumbent (verbatim) + 3 candidates + panel tests; `image`/`worldmaker-io` dev-deps | green (proof of bit-exact refactor) |
| M2.5 | L9 cadence arm + doc comment; harness XL rows; CLAUDE.md line | green (L9-only) |
| M2.6 | judge record: plategen-feelpass JSON + curated PNGs + decision-log scores | green |
| M3 | winner wired; losers/trait/score test deleted; CI gate test (final constants); BOTH goldens regenerated; feelpass JSON re-run (winner); decision-log | regenerated exactly once, here |

---

## Adversarial review (fix2)

Adversarial pass against real sources @ main 9d5d272 (setup.rs, step.rs,
mod.rs, elevation.rs, noise_stage.rs, dmath.rs, grid.rs, hash.rs, results.rs,
harness.rs, determinism_tests.rs, tectonics_tests.rs, both Cargo.tomls).
Every code claim below was read, not trusted. Findings first, then the
verification ledger. Design elements above stand as written EXCEPT where an
amendment below overrides them; implementers follow the amendments.

### Findings

**F1 — major — §3.2 interface-edge Vec is NOT "already sorted by
construction". AMENDED.** CSR rings are CCW-angle-sorted and only *rotated*
to start at the lowest neighbor id (grid.rs:297–343); ids after the first are
in CCW order, not ascending. Iterating cells in id order and keeping pairs
b>a therefore yields a Vec sorted by `a` but unsorted in `b` within each
cell's run — binary search over it misses edges, corrupting `visited`
marking and every count downstream, and the committed M1 incumbent numbers
(which set the gates) would be produced by the broken traversal. **Fix
(binding):** after enumeration, explicitly sort the edge Vec by `(a, b)`
(deterministic integer sort, e.g. `sort_unstable`), and normalize every pair
produced by the walk — `(x_next, b)` / `(a, x_next)` — to `(min, max)` before
the binary-search lookup. The M1 hand-built-map unit tests must include a map
where some cell's ring order puts a larger id before a smaller one (any L4
map does) so a regression here cannot pass.

**F2 — minor — §3.3 arc_len3 error bound misstated. AMENDED.** The next
series term of `2·asin(c/2)` is `(30/43008)·c⁷` ≈ 7.7e-9 at c = 2·sin(π/32)
≈ 0.196, i.e. ≈ **1.2e-7 absolute after the ×16**, not "~3e-9". The
conclusion survives — 1.2e-7 ≈ 0.5 ulp of π in f32 and far inside the 1e-5
rel / 1e-6 abs test tolerance — but the doc comment must carry the correct
bound. Also verified: the antipodal guard `|a+b|² < 1e-12` triggers only for
θ within ~1e-6 rad of π (tighter than the "1e-4" stated — conservative,
fine); all named dmath helpers exist, including `random_tangent`
(dmath.rs:155), so §2.2 step 5 needs no new dmath sampler.

**F3 — minor — §1 PlateGenParams "already clamped" is by-convention only.
AMENDED.** Nothing in setup.rs clamps: the clamp lives solely in
`TectonicsStage::new` (mod.rs:107–114), and the gate test/harness call
`SimState::setup` directly (step.rs:260–262), bypassing it. They pass
`TectonicsParams::default()` (plate_count 12 — in band), but the doc-comment
invariant is not structural. **Fix:** `PlateGenParams::from` keeps the value
as-is, and `generate()` implementations open with
`debug_assert!((8..=24).contains(&params.plate_count))` so any future
direct caller with an out-of-band count fails loudly instead of tripping
step.rs:428 mid-run.

**F4 — minor — boundary_sinuosity is undefined on a boundary-free map.
AMENDED.** A single-plate (or all-one-id hand-built) map has zero interface
edges ⇒ `weighted_mean = 0/0 = NaN`. Unreachable through generators (dense
non-empty ids, p ≥ 8) but `metrics` is pub forever and unit tests feed it
hand-built maps. **Fix:** `assert!(total_baseline_rad > 0.0, "sinuosity
undefined: no interface edges")` — a loud panic, never NaN into JSON.

**F5 — minor — §5 test paths resolve against the crate dir, not the
workspace root. AMENDED.** Integration tests run with cwd =
`crates/worldmaker-sim`, so the literal `docs/results/...` and
`target/plate-panel/` would land inside the sim crate. **Fix:** both
`#[ignore]` tests resolve output paths as
`concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/results/...")` (and
`.../target/plate-panel/`); `WM_PLATE_PANEL_DIR` when set is used verbatim.

**F6 — minor — §3.2 loop closure under-specified. AMENDED.** For closed
loops/lassos the polyline must include the closing hop: dedup consecutive
side cells *across the wrap* (last vs first vertex) and add the closing
`arc_len3(last, first)` to the length. Without this a loop's length omits
one step and small enclaves (the ~6-edge case §3.2's own example uses) lose
~15% of their perimeter.

**F7 — note — cancel latency during setup (state the design forgot).
VERIFIED with note.** `run_history` checks cancellation only inside the step
loop (mod.rs:214–221); `SimState::setup` is uncancellable. The winner raises
worst-case setup from near-instant to ~5–7 s at L9 (Dijkstra + anneal), so a
Cancel pressed during an Ultra regenerate stalls that long. No contract is
violated (Fix 1's guard is about strokes never *starting* runs); accepted
for WO-0003, recorded here so Track A does not chase it as a bug.

**F8 — note — P2 stability pre-check requires uncommitted wiring. AMENDED
(procedure).** At P2 the default path still runs the incumbent (winner wires
in M3 only), so the front-runner's 2 Gy L6 stability run needs a local,
temporary setup.rs edit. Binding rule: that edit is never committed — P2
stability evidence goes into the decision-log entry (numbers + config), and
the committed proof remains M3's full harness run. This keeps "every branch
commit green" and "goldens move exactly once" literal.

**F9 — note — H1 target sums drift off 1.0 away from p = 12. VERIFIED with
note.** At p = 24 the ladder targets sum to ~1.7·n (soft caps rarely
engage); at p = 8, ~0.66·n (everything over-target late in the fill). Both
remain deterministic and heavy-tailed — steering just weakens toward the
band edges. Competition, gates, and goldens all run at the default 12, where
the sum ≈ 0.95·n; recorded so nobody "fixes" the ladder mid-implementation.

**F10 — note — panel PNG colors will not match the app. VERIFIED with
note.** layers.rs indexes PLATE_COLORS by size *rank* (layers.rs:227), the
panel test by `plate_id % 24`. Dev-only; judges compare shapes, not hues.

### Verification ledger

- **§1 trait + wiring — VERIFIED.** setup.rs:46–89 is exactly the seeding +
  Voronoi unit; "plate-seeds" is read nowhere else (checked all of
  setup.rs); the incumbent's only draw is one `next_u64` (:50); closeness
  update is per-element `par_iter_mut` max (:53–58, no reduction); Voronoi
  is strict `>` ties-to-lowest (:81–86). Verbatim move ⇒ bit-identical;
  goldens-green-at-M2 is a valid proof. Section 3's `plate-init-{pid}`
  streams (:92–110) are geometry-independent as claimed. Craton-stream
  draw entanglement confirmed at :147–151 (`uniform_range` gated on the
  oceanic draw) and :161–185 (2 draws gated on `target > 0`).
- **Overlay firewall — VERIFIED.** tectonics_tests.rs:371–394 pins
  keyframe-0 plate_id equality with/without paint; PlateGenParams carrying
  only plate_count makes the firewall structural.
- **§2.3 fbm reuse — VERIFIED.** `pub(crate) fn fbm` at noise_stage.rs:63;
  base freq 1.6, lacunarity 2, gain 0.5, output ≈ ±1.66 (×1.9 at :72 —
  slightly hotter than §2.3's "±1" phrasing; WARP_AMP 0.18 stays a
  judge-tunable). `splitmix64` is pub in worldmaker_core::hash — the anneal
  hash needs no new export. No std trig anywhere in the candidates.
- **§3.1/3.2 metric algebra — VERIFIED.** Σlen/Σbaseline ≡ the pinned
  gc-weighted mean of per-segment ratios (weight·ratio = len, exactly).
  Flanking-triangle identity (ring neighbors of b at i±1) holds for the
  link of a vertex in a triangulation, pentagons included; junction-triangle
  enumeration via `grid.triangles` (pub, grid.rs:38) is canonical;
  2-plate-triangle continuation is unique, so walks need no mid-walk
  tie-break. Two-sweep diameter ties pinned to lower cell id — total order.
- **§3.3 — VERIFIED as amended (F2).** Bisection halves the angle exactly
  4× (θ/16 ≤ π/16); chord/series algebra checked term-by-term;
  bit-pattern canonicalization makes symmetry exact by construction.
- **§4 gate test — VERIFIED.** `SimState::setup` pub (step.rs:260),
  `plate_id` pub (:175); default plate_count = 12 (mod.rs:95) so the "12
  ids, 12 alive plates" assertions are correct; seed_from_text("cyrus")
  recomputed = 0xc4be0bf8f497a575 (FNV-1a-64, hash.rs:72).
- **§6/§9 protocol — VERIFIED.** Harness determinism keys and config match
  the goldens exactly: `run(6, 500.0)` = SEED 42 + default params
  (harness.rs:19, :389–403), same L6/500 My/final-fields config as
  determinism_tests.rs:59–76 (values 0xf751…5b62 / 0x70df…653d confirmed);
  §9 step 2's "the harness rows ARE the new goldens" holds. Phase-0 golden
  (:29) is noise-stage-only — untouched by any Fix 2 change. M1/M2/M2.5
  are golden-safe; only M3 moves them.
- **§7 cadence — VERIFIED.** Branch is `>= 8` (mod.rs:63–69) ⇒ the new ≥9
  arm leaves L8 at 20 and L5–L7 at 10; L5 pin (21 kf @ 200 My,
  hotspots == 6, anchor ±0.005, final ±0.05, alive 6..=24) confirmed at
  tectonics_tests.rs:73–141; history stores its own interval (mod.rs:234).
  Byte math re-done: 2,621,442 cells × 16 B = 41.9 MB; 21 kf = 0.88 GB. ✓
- **§8 harness — VERIFIED.** SEED = 42 (:19) and the hardcoded 0.29
  (:311) untouched; no key collisions with existing metrics; ResultsFile
  metrics is `serde_json::Value` (results.rs:11–42) so the string/bool echo
  keys fit; worldmaker-io depends only on core ⇒ sim dev-dep is acyclic;
  `image` is on the approved list (rule 6).
- **§10/§11 — VERIFIED.** nd ≤ 32 assert (step.rs:428); PLATE_FLOOR/CEIL
  6/24 (:102–103); death-at-zero-cells with no floor (:673–678) — R1's
  premise is accurate; suture floor at :959; breakup ceiling at :991–993.
  Dijkstra key (cost, cell, owner) is a strict total order satisfying the
  pinned (cost, cell-id) rule; counts update at pop time under a serial
  heap ⇒ m_eff deterministic. 2√f chord-radius identity is exact
  (chord = 2·sin(θ/2), sin²(θ/2) = f). Anneal is serial ascending-id
  Gauss–Seidel with stateless hash noise ⇒ order-safe. L9 transient memory
  (heap ≲ 0.4 GB + cost arrays ≲ 0.1 GB) fits the Air alongside the
  0.88 GB history.

**Verdict:** no BLOCKER stands after amendment. F1 must be folded into the
M1 implementation (it changes metrics.rs code and its unit tests, before the
incumbent numbers are committed); F2–F6, F8 are one-line-to-one-function
amendments; F7, F9, F10 are recorded expectations. The commit plan, gate
protocol, cadence decision, and golden-regeneration procedure survive
adversarial reading intact.

### Second adversarial pass (independent re-verification + new findings)

Independent pass against the same sources @ 9d5d272, re-reading every cited
line rather than trusting the first pass. Confirmed against code: F1's
premise (grid.rs:297–343 — rings are CCW-angle-sorted, only *rotated* to the
lowest id; the explicit `(a,b)` sort + walk-pair normalization is mandatory),
F2's corrected bound (series term 15c⁷/21504; ×16 ≈ 1.2e-7), F3 (the clamp
lives only in `TectonicsStage::new`, mod.rs:107–114; `SimState::setup`
bypasses it), F4, F5 (CARGO_MANIFEST_DIR = crate dir for integration tests),
F6, F7 (mod.rs:214–221), F9 (ladder sums re-derived: ≈1.9n at p=24, 0.97n at
p=12, 0.67n at p=8 — same conclusion), F10 (layers.rs:227 ranks by size).
Also verified fresh: CI's test job runs `cargo test --workspace --release
--locked` (ci.yml — §4's 1–2 s runtime claim is valid *because* of this);
`SimState.plates` is pub (step.rs:187), so gate assertion 1 compiles;
`grid.triangles` pub (grid.rs:38); seed_from_text("cyrus") recomputed =
0xc4be0bf8f497a575; keyframe = eight u16-wide arrays = 16 B/cell; sim crate
has no dev-deps today; worldmaker-io deps = core+serde only (acyclic);
elevation.rs:81–95 is the 40-iteration bisection H1 step 3 mimics; §3.2's
"consecutive distinct side cells are grid-adjacent" holds (they share the
flanking triangle); (a)'s cost-0 seed pops guarantee non-emptiness with no
repair step. New findings:

**F11 — major — §3.3 arc_len3 TEST SPEC fails on its own test points.
AMENDED.** Simulated the exact f32 algorithm (300 random orientations per
angle) against f64 acos: near-antipodal cancellation in `add3(a, b)` gives a
direction error ~6e-8/(π−θ) in the first midpoint, doubling into the result.
Measured: max abs error 3.4e-6 at θ=3.1 (passes), **1.3e-4 at θ=π−1e-3 —
fails the specified `1e-5 rel + 1e-6 abs` tolerance on ~50% of
orientations**; the small-angle error floor (~1.3e-6 abs, chord rounding
×16) also grazes the 1e-6 abs term (1/300 failures at θ=0.01). The
ALGORITHM is fine for the metric (worst error ~1e-3 rad only for baselines
within 1e-3 of π — unreachable-in-practice junction pairs; sinuosity gates
sit at 1e-2 granularity); the TEST as written would be red on day one. Fix
(binding): tolerance = `1e-5·θ + 5e-6` for test angles ≤ 3.0; for the
near-antipodal rows use documented absolute tolerances (2e-5 at 3.1, 5e-4 at
π−1e-3); doc-comment arc_len3 with the error envelope "~1e-6 abs typical,
degrading as ~1.2e-7/(π−θ) near antipodal". The bit-exact symmetry test
(canonicalized argument order) is unaffected.

**F12 — minor — §5 score_generators hardcodes the machine into the
filename. AMENDED.** `ResultsFile::new` stamps `machine: machine_name()` at
runtime (results.rs:26), and the schema requires machine == filename
(docs/results/README.md). A hardcoded `plategen-feelpass-Daniels-MacBook-
Air.json` run on any other machine commits a self-inconsistent file. Fix:
build the filename as `format!("plategen-feelpass-{}.json",
worldmaker_io::results::machine_name())` — same pattern the app uses for the
other results files.

**F13 — minor — hybrid anneal's pinned-seed set is ambiguous; cost product
types unstated. AMENDED.** (b)'s anneal refuses flips on "seed cells" = its
H2 seeds; (c) reuses the pass "verbatim" but has primaries AND helper seeds
— unpinned, two implementations could differ deterministically-but-
differently. Fix (binding): in the hybrid, the pinned set = the p PRIMARY
seed cells only; helpers may flip (non-emptiness is already protected by the
`count[current] == 1` refusal). Same clause pins (c)'s step-cost arithmetic:
`b_e·m_eff·noise_f` peaks ≈ 2.3e9 — fits u64 only by the casts written in
(a)'s formula; all three factors are cast to u64 before multiplying, as in
§2.2 (an i32/u32 build would overflow silently in release). Also: the M1
hand-built-map unit tests must assert CV on cell counts the test itself
constructed (an L4 hemisphere split by `z > 0` is NOT exactly equal-halves;
assert the constructed counts, or CV < epsilon — never == 0).

**Second-pass verdict:** F11 is major but test-spec-only — it must be folded
into M1 alongside F1 (both change metrics/dmath tests before the incumbent
numbers are committed); F12/F13 are one-line amendments. No BLOCKER. With
F1–F6, F8, F11–F13 applied, the design is fit for implementation: pinned
contracts hold (metric definition, gate protocol, single golden regen,
overlay firewall, dmath-only sim math, ≤32/dense-id, harness ownership,
L6/L7/L8 cadence untouched), and both passes agree the commit plan keeps
every branch commit green.
