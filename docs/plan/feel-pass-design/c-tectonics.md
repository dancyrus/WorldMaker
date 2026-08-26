# Stage U reader (c) — tectonics code map

Territory: `crates/worldmaker-sim/src/tectonics/` (setup.rs, mod.rs, step.rs,
keyframe.rs, elevation.rs) plus worldmaker-core `dmath`/`grid` as they serve it.
All line numbers against main @ 9d5d272. No code was changed.

## 1. Plate generation today (setup.rs) and the downstream boundary

### Entry point and call graph

- `pub(super) fn setup(master_seed: u64, grid: &Arc<Grid>, params: &TectonicsParams) -> SimState`
  — setup.rs:42. Sole caller: `SimState::setup` (step.rs:260–262), whose sole
  caller is `run_history` (mod.rs:193, the `resume: None` arm). `TectonicsStage::run`
  (mod.rs:159) calls `run_history` and publishes the last keyframe via
  `Keyframe::write_fields`.

### Farthest-point seeding (setup.rs:46–73)

- RNG: `sub_rng(master_seed, STAGE_ID, "plate-seeds")` with
  `STAGE_ID = "phase1-tectonics"` (mod.rs:29). Exactly **one** u64 is drawn:
  `seeds.push((rng.next_u64() % n as u64) as u32)` for the first seed (line 50).
  All remaining `p_count - 1` seeds are argmin of a `closeness` array
  (max cosine to any seed so far), ties to the **lower cell id** (serial scan,
  lines 61–73). `closeness` update is `par_iter_mut` per element (per-element
  max, no reduction) — deterministic.
- `p_count = params.plate_count`, clamped **8–24** by `TectonicsParams::clamped`
  (mod.rs:107–114); default 12 (mod.rs:96). Note: the run-time band enforced by
  the sim is `PLATE_FLOOR = 6`, `PLATE_CEIL = 24` (step.rs:102–103) — the
  acceptance band "6–24" is about alive plates over the run, not the setup count.

### Great-circle Voronoi (setup.rs:75–89)

- Per-cell argmax of `dot3(cell_pos, seed_pos)` over the seed list; strict `>`
  means ties go to the **lowest seed index**. `par_iter_mut` per cell —
  deterministic. Writes `s.plate_id: Vec<u32>` and nothing else.

### Everything downstream of `plate_id` / `seeds` (the "nothing downstream changes" boundary)

The `seeds` vector is **local to setup** — it is never stored. The only
artifact of sections 1–2 is `s.plate_id`. Consumers, in order:

Inside setup.rs itself:
- Boundary-depth BFS for craton nuclei (112–134): multi-source BFS from cells
  with a foreign neighbor; serial `VecDeque`, id-ordered seeding.
- `plate_cells` counts (136–139).
- Craton budget: `total_cont = (land_fraction × CONT_AREA_FACTOR=1.35).min(0.85) × n`
  (141–143); per-plate weights from the `"cratons"` sub-stream (144–159):
  per pid **one** `uniform_f32` (oceanic test, `OCEAN_PLATE_CHANCE = 0.2`) and,
  only if not oceanic, **one** `uniform_range(0.5, 1.5)`. Then per plate with
  `weights > 0` **and** `target > 0`: two more draws (`peak`, `age`, lines
  184–185). ⚠ `target` depends on `plate_cells` — so **draw alignment in the
  "cratons" stream is entangled with plate geometry**. A new generator shifts
  all subsequent craton draws. That is world-data change (covered by the
  one-time golden regen), not a code change, but it means "downstream code
  unchanged" ≠ "downstream data unchanged".
- Craton growth BFS constrained to the plate (186–208), thickness taper
  (209–219): `crust_type=1`, `thickness = 35 + (peak−35)·taper`, `crust_age`,
  `orogeny_age = age` (primordial).
- Ocean-age ramp (221–230): `"ocean-age-ramp"` stream, 1 random unit vector;
  independent of plate geometry.
- Craton paint overlay applied **after** everything (232–250): `+1` →
  continent 40 km / 2000 My; `−1` → ocean 7 km with ramp age. Independent of
  plate geometry (test `craton_overlay_changes_world_deterministically`,
  tectonics_tests.rs:371–396, asserts `keyframes[0].plate_id` is **identical**
  with and without paint — the new generator must likewise never read
  `craton_overlay`).
- Hotspots (252–268): `"hotspots"` stream unless `hotspot_overlay` is Some;
  rejection sampling with min separation cos 15°. Independent of plate geometry.
- Detail-noise seed (271–272): `"detail-noise"` stream, one u64. Independent.
- Plate motions (91–110): per-plate streams `"plate-init-{pid}"` — depend only
  on pid and vigor, **not** on geometry. Unchanged for the same `plate_count`.

Outside setup.rs:
- step.rs consumes `plate_id` pervasively: `advect` (414–680, incl. the
  **`assert!(nd <= 32)`** candidate-bitmask cap at 428 — alive plates must
  stay ≤ 32), `classify_boundaries` (684–769), `accumulate_boundary_stats`
  (773–797), `apply_arcs` (804–851), sutures (915–987), `maybe_breakup`
  (991–1118), `apply_hotspots`, `encode_keyframe`.
- elevation.rs does **not** read `plate_id`.
- keyframe.rs stores `plate_id` as `Vec<u16>` (keyframe.rs:73; cast at 145) —
  fine, ids only grow via breakup and stay small.
- App: `layers.rs` (~line 190–230) ranks alive plate ids into a palette and
  draws one-cell boundary bands from `F_BND_*` bits; `app.rs:855` cursor
  readout prints `kf.plate_id[c]`; `harness.rs:178` (arc metric same-plate
  test), 393–394 (plate hash).
- Tests: `GOLDEN_TECTONIC_PLATES_L6_SEED42 = 0x70df_6db8_ec5f_653d` and
  `GOLDEN_TECTONIC_ELEVATION_L6_SEED42 = 0xf751_0e72_14ed_5b62`
  (determinism_tests.rs:59–60) — regenerate **exactly once** on B's branch.

**Bottom line:** the replaceable unit is setup.rs sections 1–2 (lines 46–89).
Everything after consumes only `plate_id` (+ `plate_cells` derived from it)
through stable interfaces, plus RNG streams that are either per-pid or
geometry-independent — except the "cratons" stream alignment noted above.
A new generator that writes the same `plate_id: Vec<u32>` (values `0..p_count`,
every plate non-empty) and pushes the same `p_count` `PlateState`s needs **no
other code change** anywhere downstream.

## 2. The ±300 m fBm detail term (stays untouched)

- Applied at elevation.rs:69, inside `derive_and_solve` (runs at **every
  keyframe**, mod.rs:210/224): `elev += DETAIL_AMP_M * fbm(grid.positions[c], noise_seed, DETAIL_OCTAVES)`
  with `DETAIL_AMP_M = 300.0`, `DETAIL_OCTAVES = 6` (elevation.rs:34–35).
- Seed: `SimState::noise_seed`, derived `sub_rng(master_seed, STAGE_ID, "detail-noise").next_u64()`
  at setup.rs:272 and re-derived identically in `SimState::from_keyframe`
  (step.rs:310) — resume-safe.
- `fbm` lives in noise_stage.rs:63–73 (`pub(crate)`): value-noise fBm, base
  freq 1.6, lacunarity 2, gain 0.5, ×1.9 normalization; lattice via
  `splitmix64`; libm-free. **It is crate-private** — Track C's render detail
  cannot (and must not) link to it; the renderer implements its own noise.
- It feeds the sea-level solve (elevation.rs:81–95: t=0-only bisection, 40
  iterations, integer counts) and both goldens. Any change here moves
  GOLDEN_TECTONIC_ELEVATION.

## 3. Keyframes: cadence, layout, span

- `DT_MY = 2.0` (mod.rs:57). `keyframe_interval_my(grid_level)` (mod.rs:63–69):
  **10 My for level ≤ 7, 20 My for level ≥ 8** — one `if`, doc comment cites the
  L7 budget (527 MB measured @ 2 Gy) and L8 ≈ 1.06 GB "recorded, not budgeted".
  The L9 cadence decision lands here; note the branch is `>= 8`, so an L9-only
  change needs a new arm. L6/L7 cadence must not move (goldens are L6, phase-1
  results are L6/L7).
- `run_history` (mod.rs:185–247): `steps_per_keyframe = kf_my / DT_MY`;
  `keyframe_count = (span_my / kf_my).round().max(1)`; span is clamped
  **200–2000 My** (default 500). At every keyframe: `quantize_state()` →
  `derive_and_solve` → `encode_keyframe()` — the quantize-before-snapshot
  round-trip is what makes resume bit-exact (step.rs:269–279 must mirror
  keyframe.rs `enc_u16`/`enc_i16` exactly).
- Keyframe memory (keyframe.rs:67–84): **exactly 16 B/cell** — eight u16-wide
  per-cell arrays: `elev_m: Vec<i16>`, `plate_id: Vec<u16>`,
  `crust_age_my: Vec<u16>`, `thickness_ckm: Vec<u16>` (km×100),
  `orogeny_age_my: Vec<u16>`, `rift_age_my: Vec<u16>`, `buildup_ckm: Vec<u16>`,
  `flags: Vec<u16>` (feature bits 0..=7; crust_type at bit 15,
  `KF_CONTINENT_BIT`). Plus `plates: Vec<PlateState>` (pole, speed, base speed,
  suture time, 3×3 pending_rot, pending_deg, 3 boundary-stat u32s — keyframe.rs:24–46)
  and `collisions: Vec<PairTimer>`. `approx_bytes` = n×16 + plate/pair sizes.
- L9 arithmetic check: 2,621,442 cells × 16 B ≈ 42 MB/keyframe. 2 Gy at 20 My
  = 101 keyframes ≈ 4.2 GB (matches the pinned contract's number); ~100 My
  spacing → 21 keyframes ≈ 0.88 GB.
- `TectonicsHistory` (keyframe.rs:237–259) stores `keyframe_interval_my`;
  `nearest_index` and the app's era picker divide by it — a per-level cadence
  is fine because it is captured per run.

## 4. Stage trait / Pipeline / params_hash

- `Stage` trait: `id()`, `params_hash() -> u64`, `run(&self, ctx, world)`
  (pipeline.rs:103–113). `Pipeline::run` (165–197) chains a key:
  FNV-1a over (upstream_key, stage id bytes, master_seed, grid.level,
  params_hash); a stage re-runs iff key changed, upstream ran, or the
  `WorldState` instance differs (world `id`, pipeline.rs:174). Failed/cancelled
  runs clear `last_key` first (188–190), so cancel never poisons the cache.
- `TectonicsParams` (mod.rs:74–90): `plate_count`, `land_fraction`,
  `tectonic_vigor`, `span_my`, `hotspot_count`,
  `craton_overlay: Vec<(u32, i8)>` (sorted by cell id; +1 continent / −1 ocean),
  `hotspot_overlay: Option<Vec<[f32; 3]>>`. `params_hash` (mod.rs:136–157)
  hashes all five scalars LE, every overlay pair, and `[1u8]` + vectors when
  `hotspot_overlay` is Some. So folding strokes into params (Fix 1's
  Regenerate) changes the hash → full stage re-run — as intended.
- In practice the app never exploits the cache: `start_job` (app.rs:311–341)
  builds a **fresh Pipeline + WorldState per job** on a worker thread with
  `Progress` (fraction + cancel, polled every step at mod.rs:214–220). Fix 1's
  "existing progress + cancel" is exactly this `SimJob` machinery.
- ⚠ Fix 1's structural guard: today stroke handling **does** route to
  simulation — e.g. `craton_stroke_dirty` re-runs on stroke end and the
  "Clear craton paint" button calls `start_job()` (app.rs:823–827). Track A
  removes those routes; Track B/C should not touch them.
- Track C's guard "worldmaker-sim exposes no render-detail parameter" is
  trivially true today (grep: no such field anywhere in worldmaker-sim).

## 5. dmath surface (worldmaker-core/src/dmath.rs)

Present: `det_sin_cos(x)` (Taylor, **valid only |x| ≤ 0.75 rad**, debug_assert,
dmath.rs:21–28); vec3 ops `dot3/cross3/normalize3/scale3/add3/sub3`;
`rotation3` (Rodrigues via det_sin_cos), `mat3_mul`, `mat3_mul3`,
`mat3_transpose`; RNG-derived `uniform_f32`, `uniform_range`, `gaussian_f32`
(Irwin–Hall), `random_unit_vec`, `random_tangent`. Bare `f32::sqrt` is used
throughout (IEEE-exact, allowed).

**Absent: any inverse trig or arc length.** No `asin`, `acos`, `atan2`, no
great-circle-distance helper. Consequences for Fix 2:

- Polyline step lengths (adjacent cell centers) are tiny angles (≤ ~0.03 rad
  at L6) — chord ≈ arc to ~1e-4 relative; a chord-based polyline length is
  safe and libm-free.
- The **endpoint** great-circle distance between two triple junctions can be
  any angle up to π — far outside `det_sin_cos` range, and chord/arc diverge
  badly there (chord π ↦ 2 vs arc π). Either add a deterministic arc-length
  helper to dmath (e.g. polynomial/Newton `asin` on `chord/2` with fixed
  iteration count — must be documented and tested), or define the metric
  consistently in chord space **on both numerator and denominator with a
  correction**, or subdivide the endpoint arc via midpoint-normalize recursion
  (only `normalize3` + fixed depth — fully deterministic, converges fast).
  The pinned contract explicitly allows "chord-based forms"; the design must
  pick one and log it.
- Eckert IV (Track C) lives in `proj.rs`, which is **display-path**: it and
  `grid.rs` lat/lon already use std `sin/cos/atan2/asin` freely (grid.rs:11–13
  documents that nothing hash-feeding may depend on them). Newton with a fixed
  iteration cap there is for reproducible round-trip tests, not for goldens —
  no dmath work needed for Fix 3. Only the **plate metrics** (committed JSON +
  CI gate) fall under the dmath-only rule.

## 6. Grid API relevant to flood fill / metrics

- `Grid` (grid.rs:24–39): `level`, `positions: Vec<[f32;3]>` (unit, f64-built),
  `lat`/`lon` (display only), CSR `neighbor_offsets` (len n+1) + `neighbors`,
  `triangles: Vec<[u32;3]>` (icosphere render mesh = dual-vertex list).
- `neighbors_of(cell) -> &[u32]` (89–93): CCW viewed from outside, ring rotated
  to start at the **lowest neighbor id** (canonical, 334–343).
- `cell_count_for_level(level) = 10·4^level + 2` (19–21): L6 = 40,962,
  L7 = 163,842, L8 = 655,362, L9 = 2,621,442. `Grid::build` asserts level ≤ 9.
- Pentagons: always the 12 base vertices, **ids 0..12**, degree 5;
  `pentagon_count()` (96–100). Flood fill needs no special-casing — CSR degree
  handles it — but per-cell area is not uniform (pentagons ~5/6 of a hex);
  current plate-area metrics everywhere count cells, which is fine at the CV
  precision required.
- `nearest_cell(target, hint)` (109–149): greedy walk, strict-improvement +
  lower-id tie-break (cannot cycle), O(1) with hint. The one true position→cell
  map; the sim uses it in `advect` and `apply_hotspots`.
- Deterministic-PQ note for candidate 1: order keys must be (cost, cell id)
  with a total order; f32 costs are fine if NaN-free, but an integer or
  quantized cost is safer. Existing BFS patterns to copy: setup.rs:112–134
  and 186–208, step.rs:804–851, harness.rs:169–202.
- For the t=0 gate test, `SimState::setup` is **public** (step.rs:260) and
  `SimState.plate_id` is a pub field — the fast setup-only test can build a
  grid, call setup, and compute metrics without running a single step.
- Triple junctions have a natural grid primitive: an icosphere triangle
  (`grid.triangles`) whose three corner cells carry three distinct plate ids
  is exactly a Goldberg vertex where three plates meet.

## 7. Phase-1 acceptance: where and how

- Harness: `run_tectonics_harness(out)` in worldmaker-app/src/harness.rs
  (320–424), reached by `--tectonics-results <file>`; writes machine-labelled
  JSON via `worldmaker_io::ResultsFile` (schema in docs/results/README.md).
  Fixed `SEED: u64 = 42` (harness.rs:19). Existing committed files:
  `docs/results/tectonics-phase1-{DESKTOP-VKD81C6,Daniels-MacBook-Air}.json` —
  Fix 2 writes a **new** `tectonics-feelpass-{machine}.json`, phase-1 files
  untouched.
- Metrics, all on keyframe data (i16/u16 decoded):
  - `age_depth` (44–92): mean ocean depth per 10 My bin (0–80), excluding
    continent/trench/arc/hotspot/buildup>0.1 km cells, vs 2600 + 365·√age,
    gate ≤ 10% max bin error; **adds `sea_offset_m` back** for physical depth.
  - `hypsometry` (97–147): 2-means (fixed init −4000/400, 40 iterations) +
    Ashman's D; gate D > 2, ocean mode < −2500, land mode |·| < 1500.
  - `arc_trench` (153–216): ≥95% of arc cells have a **same-plate** trench
    within 400 km (per-cell BFS).
  - `stability` (236–318), on 2 Gy L6: plates 6–24 across **all** keyframes,
    anchor (t=0) land fraction within ±5% relative of 0.29, continental-crust
    inventory drift ≤ 5%; sea-offset drift recorded as data.
  - Determinism double-run at L6 500 My; hashes recorded.
  - Runtime/memory gates: 1 Gy L7 ≤ 60 s; 2 Gy L7 keyframes ≤ 1 GB.
- Sim-side tests: `crates/worldmaker-sim/tests/tectonics_tests.rs` (sanity at
  L5 incl. land-fraction ±0.005 at anchor and ±0.05 at end, resume bit-exact,
  cancel mid-run, overlay determinism + plate-layout independence) and
  `determinism_tests.rs` (the two tectonic goldens at L6 seed 42, default
  params = 500 My). B's gate test joins these.
- ⚠ worldmaker-sim/Cargo.toml has **no dev-dependencies** today (deps:
  core, rayon, anyhow, log, rand) — the plate-map PNG dev test needs `image`
  added as a dev-dependency (already an approved workspace dep).

## 8. Boundary / triple-junction representation today

**None exists.** Available primitives only:

- Per-cell `plate_id`.
- Per-cell display bits `F_BND_DIVERGENT/CONVERGENT/TRANSFORM` (mod.rs:51–53),
  recomputed each step in `classify_boundaries` (step.rs:684–769) from relative
  Euler velocities across each foreign edge (threshold `CLASSIFY_CMYR = 0.4`);
  fresh each step because `advect` rewrites `features` per cell before the
  classify OR-in. Present in keyframe `flags` bits 5–7, including keyframe 0
  (setup → `init_stats` → `classify_boundaries`, step.rs:332–343).
- `ClassOut.boundary: bool` per cell — transient scratch, not persisted.
- decision-log 2026-08-19: boundary "lines" are one-cell bands; "true polylines
  deferred until a phase needs them". This is that phase (Track C draws them;
  Track B's sinuosity needs segments between triple junctions).

The sinuosity implementation must therefore build its own structure from
`plate_id` + CSR, e.g.: boundary edge = unordered cell pair (a,b), a<b, with
`plate_id[a] != plate_id[b]`, enumerated in id order; junction = grid triangle
with 3 distinct plate ids (or a boundary cell touching ≥ 3 plates, if
cell-center polylines are walked cell-to-cell); segments walked with CCW
neighbor order for deterministic traversal. All serial, id-ordered.

## Contradiction / complication flags (summary)

1. **`assert!(nd <= 32)`** (step.rs:428): alive plates must never exceed 32.
   Multi-seed growth must collapse helper seeds to final plate ids before
   returning; `plate_count` clamp 8–24 keeps setup safe, breakup is capped at 24.
2. **"cratons" RNG alignment is geometry-entangled** (setup.rs:161–185): the
   new plate map shifts craton draws → whole-world change. Expected, but it
   means the incumbent-vs-candidate comparison changes continents too, not
   just plate outlines; judge-panel PNGs should render **plate ids**, as ordered.
3. **dmath has no inverse trig / arc length** — the sinuosity endpoint
   distance is the one place Fix 2 genuinely needs new deterministic math
   (§5). Polyline steps can stay chord-based.
4. **Plate layout must ignore `craton_overlay`** — enforced by an existing
   test (tectonics_tests.rs:390–396); the new generator must not read overlays.
5. **Goldens**: any change in setup.rs sections 1–2, elevation.rs, or L6/L7
   cadence moves GOLDEN_TECTONIC_* (determinism_tests.rs:59–60). The L9-only
   cadence option is golden-safe; the `keyframe_interval_my` branch is
   currently `>= 8`, so L9 needs a new arm, leaving L8 at 20 My.
6. **Breakup makes straight boundaries**: `maybe_breakup` splits along a random
   great circle (step.rs:1051, 1099–1117), so late-era maps re-grow low-sinuosity
   boundaries regardless of the t=0 generator. The pinned gates are t=0-only
   (correctly), but expectations for the final-era look should account for this;
   the boundary-annealing idea (candidate 2) does not run during the sim.
7. `TectonicsParams.plate_count` doc comment says "8–24" (mod.rs:75) while WO
   text says plate count 6–24 — the 6 refers to the run-time floor
   (`PLATE_FLOOR = 6`). No action needed, just don't "fix" the clamp.
8. Keyframe `plate_id` is u16; `PlateState` ids are u32 indices into an
   append-only `plates` vec (dead plates keep slots). A generator emitting
   sparse/large ids would break the dense-index assumption — keep ids
   `0..p_count`, contiguous, all non-empty.
