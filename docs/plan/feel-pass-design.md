# Feel-pass design (WO-0003)

Pinned spec from the order, plus code-map findings (Stage U) and design
decisions (Stage D). Implementers code against THIS document.
Checklist lives in docs/work-orders/WO-0003-feel-pass.md.

## Pinned contracts (from the order — not negotiable)

### Fix 1 — interaction contract (Track A)

Standing contract for every brush in every phase; goes in decision-log.md.

- Pending edits are an **ordered list of strokes**. Stroke = { tool, payload };
  payload is `CratonPaint { cells, +1/−1 }` or `HotspotAdd { unit vector }` or
  `HotspotRemove { unit vector }`. Craton payloads are cell ids (per grid
  level); hotspot payloads are positions — matching how TectonicsParams stores
  each overlay today.
- **No stroke may trigger simulation**; the guard is structural: the
  stroke-handling path has no route to `Pipeline::run`, enforced by a test.
- Badge counts strokes. Ctrl+Z pops the newest pending stroke. Discard clears
  the list. Regenerate folds strokes into TectonicsParams
  `craton_overlay`/`hotspot_overlay`, clears the list, runs history off-thread
  with existing progress + cancel.
- Preset switch **discards pending craton strokes** (cell ids are per-level),
  **keeps pending hotspot strokes** (positions survive). Seed change keeps
  everything pending.
- Pending strokes render as **tint + outline overlay** with current world
  behind, through the frozen interface (below).
- Stroke type is serde Serialize/Deserialize, defined in **worldmaker-io**
  beside the save stubs (stubs stay stubs).
- Pure view controls stay live, never edits: sea level, layers, projections,
  timeline scrubbing, Detail slider.

### Frozen overlay interface (A↔C contract)

Track C publishes the exact signature here during Stage D (rebake() ending in
a call like `pending_edits::apply_overlay(...)`). A codes against it; neither
track changes it until A's rebase.

**STATUS: TO BE FROZEN IN STAGE D.**

### Fix 2 — plate generator (Track B)

- Metrics on the t=0 plate map: **plate area CV**; **boundary sinuosity** =
  per segment, length of the boundary-cell-center polyline ÷ great-circle
  distance between its two triple junctions, averaged weighted by that
  great-circle length (center-polyline, not cell-edge).
- Measure incumbent on 5 fixed seeds, commit numbers FIRST. Final gates must
  strictly exclude incumbent scores; provisional gates **CV ≥ 0.5, sinuosity
  ≥ 1.15**, adjusted only with logged reasoning. Metrics →
  docs/results/tectonics-feelpass-{machine}.json (phase-1 files untouched).
  Fast setup-only gate test (L7 seed 42 + two L6 seeds) in worldmaker-sim.
- Three candidates behind a common trait, judged on the same 5 seeds:
  1. **Multi-seed weighted growth** — heavy-tailed area targets (largest plate
     15–25% of sphere, smallest 1–3%), several seeds per big plate,
     priority-queue flood fill, per-plate growth costs steering areas.
  2. **Warped-distance Voronoi** — uniform seeds, distance fields warped by
     2–3 octaves of low-frequency noise, + boundary-annealing pass.
  3. **Hybrid.**
  Dev-only `#[ignore]` test renders candidate plate maps to equirect plate-id
  PNGs (image as dev-dependency). Judge panel scores metrics + PNGs; log
  scores + decision; delete losers.
- Determinism: all sim math via `worldmaker_core::dmath` (no std f32 trig; add
  arc-length helpers to dmath or use chord-based forms); id-ordered priority
  queues with deterministic tie-breaks; metric aggregation serial +
  id-ordered; randomness from the stage sub-stream only.
- Phase 1 acceptance re-passes end to end (age-depth, hypsometry, arcs, 2 Gy
  stability: plate count 6–24, land fraction ±5%) → new results file.
- B owns the L9 keyframe-cadence decision and app harness.rs (new metrics +
  optional L8/L9 rows). A and C must not touch harness.rs.
- Goldens GOLDEN_TECTONIC_ELEVATION / GOLDEN_TECTONIC_PLATES regenerate
  **exactly once**, final commit on B's branch, with decision-log entry.
  No other branch touches golden constants. After C merges and after A merges,
  re-run sim tests to prove hashes unmoved.

### Fix 3 — rendering + resolution (Track C)

- **Sim fBm (±300 m in elevation.rs) is world data, part of goldens, not
  touched.** New effect = **render detail**: additional, sub-cell-scale,
  renderer-only.
- Globe: upload per-cell **scalar values** (no neighbor averaging), rasterizer
  interpolates the value, palette + sea-level threshold + render detail
  applied **per fragment**. Reverses the logged "palettes baked on CPU"
  decision — re-log. Plates layer becomes **crisp per-cell** on the globe.
- Flat: mechanism chosen in Stage D (extend cell-id lookup to blend nearest
  cell centers, or rasterize projected triangle mesh into flat view); same
  per-fragment palette/threshold/detail. Only noise + palette functions are
  shared verbatim between canvases.
- Render detail: deterministic 3D noise on the sphere seeded from master
  seed; amplitude conditioned on local slope + land/ocean; added to
  interpolated elevation before tint/threshold (fractal coastlines). Default
  tuned by sweep: octave/amplitude matrix on 2 seeds, screenshot panel picks
  default; Detail slider off→full; params logged.
- Plate boundaries: smoothed polylines (Chaikin or similar), same three
  boundary-type colors, both canvases; old one-cell bands only under debug
  toggle. Keep a debug toggle showing true cell boundaries.
- **Eckert IV** in worldmaker-core projections, forward + inverse (Snyder,
  unit sphere): solve θ + sinθ·cosθ + 2·sinθ = (2+π/2)·sinφ by Newton from
  θ₀ = φ/2, fixed iteration cap + tolerance (deterministic);
  x = (2/√(π(4+π)))·λ·(1+cosθ); y = 2·√(π/(4+π))·sinθ. Inverse: θ from y in
  closed form, φ from the same relation, λ from x. Round-trip tests; graticule
  + smoothed rendering work; same ground position resolves to same cell as
  other projections; brushes + cursor readout work.
- Render-only guard, testable as written: app-crate test builds the world
  through the same path Regenerate uses, Detail 0 vs max ⇒ identical
  params_hash + identical committed field hashes; plus a check worldmaker-sim
  exposes no render-detail parameter.
- Resolution: default generation preset → **high L8 (~28 km)**; Draft L6
  stays; **Ultra L9 enabled** with cadence decision implemented by B
  (L9 20 My × 2 Gy ≈ 4.2 GB vs 1 GB budget; options: ~100 My spacing
  (~0.9 GB @ 2 Gy), cap span, or recorded-not-budgeted). Update
  keyframe_interval_my doc comment + CLAUDE.md key facts.
- C owns main.rs + script surface: `--seed`, `--preset`, `--detail` flags for
  screenshot/perf scripts; perf script records fps at L7/L8/L9 with smooth
  shading + render detail on → docs/results/perf-feelpass-{machine}.json.
  B's harness rows cover 1 Gy wall time + measured keyframe bytes at L8/L9.

### Track file partition (function-level)

- **A owns:** new pending-edits module; in app.rs the canvas_common/apply_tool
  input path, side-panel paint section, job triggering; pending-set type in
  worldmaker-io. A does NOT edit rebake, WorldBundle, render.rs, shaders.wgsl,
  layers.rs, main.rs, harness.rs.
- **B owns:** worldmaker-sim/src/tectonics/ (setup.rs, metrics + tests, CI
  gate test, keyframe cadence in mod.rs, elevation.rs only via logged decision
  before golden regen), plate-panel dev test, app harness.rs. No UI code.
- **C owns:** render.rs, shaders.wgsl, layers.rs, main.rs + script flags,
  core projection module; in app.rs: WorldBundle plumbing (new, rebuild_grid,
  rebake), top-bar view controls, projection dropdown, Detail slider, preset
  list, debug toggle.
- Merge order **B → C → A**, each rebasing on the last.

## Stage U — code map (filled by readers)

### (a) app.rs input handling + UI state
*pending*

### (b) render.rs / shaders.wgsl / layers.rs
*pending*

### (c) tectonics: setup.rs seeding + elevation.rs fBm
*pending*

### (d) goldens, determinism tests, harness.rs, results schema
*pending*

## Stage D — decisions

*pending*
