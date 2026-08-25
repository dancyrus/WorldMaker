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

Full report: `feel-pass-design/a-app-input.md` (line refs vs app.rs @ 9d5d272).

**Types.** `WorldApp` (app.rs:100–157); `Tool { None, CratonPaint, CratonErase,
Hotspot }` (:55); `Preset { Draft6, Standard7, High8 }` (:31, default Standard7
at :220); `SimJob { progress: Arc<Progress>, rx: mpsc::Receiver<anyhow::Result<
(WorldState, f64)>>, started }` (:93). Edit staging: `craton_paint:
BTreeMap<u32, i8>` (:118), `hotspot_overlay: Option<Vec<[f32;3]>>` (:119).
Stroke-boundary state is ONE bool `craton_stroke_dirty` shared by both canvases;
`Response::drag_started()` exists but is unused.

**Input path.** `globe_canvas` (:561) / `flat_canvas` (:618) → cursor→cell via
`Grid::nearest_cell(world, pick_hint)` (:595 / :650) → `canvas_common(&response,
hit, canvas)` (:528) → `apply_tool(cell, clicked) -> bool` (:475). Craton arms:
`cells_within_radius` BFS over `neighbors_of` gated by `dot >= cos(radius_km/
6371)` (:453–472) → `craton_paint.insert(c, ±1)` — the eraser inserts −1 (force
ocean), never removes +1. Hotspot arm (click-only): base set =
`hotspot_overlay.clone().or(history.hotspots)`, bails if neither exists; remove
= FIRST spot within 300 km in Vec order (:509–512 — not nearest), else push
`grid.positions[cell]`; then `hotspot_overlay = Some(spots)` (absolute
replacement) and `start_job()` directly (:520).

**Four stroke→sim routes to sever (Fix 1):**
1. craton stroke end — canvas_common :557 (`drag_stopped()||clicked()` + dirty flag);
2. hotspot add/remove click — apply_tool :520;
3. "Clear craton paint" button — side_panel :823–827;
4. "Reset to generated" (hotspots) — side_panel :832–835.
Non-stroke sim routes stay and become the fold points: Generate/Enter
(:688–691), "Generate history" (:782–787), preset switch → `rebuild_grid`
(:730 → :306), `WorldApp::new` (:258).

**Jobs.** `start_job` (:312–342): cancels any live job, **drops `history` +
`world_state` immediately** (keyframe memory budget), spawns `std::thread` →
`Pipeline::run`, result via mpsc; `poll_job` (:345–385) installs history and
sets `needs_bake`. `current_params()` (:262–272) folds `craton_paint`
(BTreeMap ⇒ already id-sorted) + `hotspot_overlay` into `TectonicsParams`
(mod.rs:73 — `Clone, Debug` only, **no serde**; every field feeds params_hash).
Guards are per-widget (`add_enabled(job.is_none())`), never structural.

**rebake** (:397–440; sole call site end of `ui`, :1215–1217): clears
needs_bake, **early-returns if `history` is None** (so mid-run pending-stroke
display is impossible today), builds `BakeOverlay` from the ACTIVE TOOL
(craton map / hotspot marker cells / NONE) → `layers::bake(layer, kf,
sea_level_m, &overlay)` (layers.rs:184) → bump `field_gen` → publish fresh
`Arc<WorldBundle>`. Overlay today is tint-only opaque color replacement
(layers.rs:159–163); no outline mechanism exists.

**Preset/seed.** `rebuild_grid` (:275–307) clears `craton_paint` (:298), keeps
`hotspot_overlay`, then regenerates unconditionally. Seed Generate keeps both.
Matches the pinned discard/keep rule — but Fix 1's pending-stroke discard/keep
must run BEFORE that rebuild.

**Undo.** None exists; the only key handling is Enter on the seed field (:687).
`craton_paint` cannot reproduce stroke boundaries (inserts absorbed) — the
pending-stroke list must record per-stroke cell payloads itself. "Stroke" =
frames between drag start and `drag_stopped()`/`clicked()`. macOS: implement
Ctrl+Z as `KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)` +
`consume_shortcut` (COMMAND = Cmd on mac), gated on
`!ctx.wants_keyboard_input()` so the seed TextEdit's own undo wins.

**Frictions for Stage D** (report §10): hotspot strokes are deltas but the
param is an absolute set, and history is dropped at job start so a fold base
can be absent; the remove-matching rule must be pinned deterministically
(incl. removes hitting pending adds); pending overlays must render tool- and
history-independently, so frozen `apply_overlay` must take the pending set
explicitly; the serde Stroke type goes in worldmaker-io (app already depends
on it — no new dependency edges).

### (b) render.rs / shaders.wgsl / layers.rs

Full report: `feel-pass-design/b-rendering.md` (line refs @ 9d5d272).

**Coloring today — one CPU function:** `layers::bake(layer: Layer, kf:
&Keyframe, sea_level_m: f32, overlay: &BakeOverlay) -> Vec<u32>` (layers.rs:184)
— packed RGBA8 LE per cell (`pack` :40), rayon order-preserving map (:208–248),
overlay applied afterwards as serial per-cell overwrites (:251–267). Result =
`WorldBundle.colors`, uploaded as a read-only storage buffer in
`SceneResources::sync_world` (render.rs:399–416).

**Globe.** Vertices = cell centers 1:1 with cell ids (`Grid.positions`), index
buffer = icosphere faces (`Grid.triangles`), one indexed TriangleList draw.
Vertex layout: single Float32x3 position, stride 12 (render.rs:248–252).
`vs_globe` (shaders.wgsl:29–37): `color = unpack4x8unorm(cell_colors[
vertex_index]).rgb` as a plain varying — the rasterizer Gouraud-interpolates
COLORS. Each triangle spans 3 different cells ⇒ crisp per-cell plates need
per-fragment nearest-of-3-corners resolution (barycentrics + per-corner cell
data); `@interpolate(flat)` paints whole triangles wrong. Plan the varying
layout for this from the start — it doubles as the debug cell-boundary view.

**Flat.** Fullscreen triangle (`vs_flat`, no vertex buffer); the projection
inverse runs IN the shader: `map_invert(proj, mx, my)` (shaders.wgsl:79–109),
`proj = misc.x` float switch (0 = equirect, 1 = Robinson; Robinson 5°-table
duplicated in WGSL `ROB` :65–73, must match core proj.rs:80). Fragment lat/lon
→ equirect texel → `textureLoad` of the R32Uint **4096×2048** cell-id texture
(render.rs:19–20; built once per grid level by `rasterize_cell_ids` :43–61) →
`flat_colors[cell]`. That raster (~0.088°/texel) caps flat resolution at high
zoom (clamp 0.5–80) regardless of grid level — the Stage-D flat mechanism must
fix or bypass it (blend-of-nearest needs >1 id per texel; rasterizing the
projected mesh bypasses the raster but needs antimeridian/seam splitting).
Eckert IV = a third projection locus: core forward+inverse AND a WGSL Newton
inverse (`misc.x = 2.0`), matched per-pixel with identical iteration cap +
tolerance, like the equirect strict-gate comment (shaders.wgsl:84–86).

**Sea level** is applied in exactly one place: the `e <= 0` branch of
`hypsometric` (layers.rs:140–156), slider subtracted CPU-side (:214). The GPU
never sees elevation today; a slider drag costs a full rebake (app.rs:746).
Post-Fix-3 it becomes a uniform write and stays a pure view control (Fix 1).

**wgpu specifics.** Globe binding 1 (cell colors) is **VERTEX-only visibility**
(render.rs:183) — every per-fragment plan needs FRAGMENT-visible value buffers
on BOTH pipelines. `GlobeUniforms{rot, params}` / `FlatUniforms{center_px,
half_px, misc, tex}` (render.rs:125–139) have spare lanes (params.zw; misc.y/w,
tex.zw) but not enough for detail + sea level + seed — extend both structs.
**u64 master seed goes in as two u32 lanes, never through f32.** Bind groups
already rebuild on every field_gen bump (:418–454), so new bindings are cheap
plumbing. No depth buffer; BlendState::REPLACE; globe back-face cull only.
Color space: bake writes sRGB-ish bytes straight through — keep per-fragment
math in the same space or colors shift.

**Boundaries.** Per-cell flag checks only (F_BND_* bits, priority convergent >
divergent > transform, layers.rs:216–222). No polyline geometry exists anywhere
— Chaikin smoothing means building boundary chains from scratch.

**rebake seam.** Steps today (app.rs:397–440): base bake → inline BakeOverlay
build from tool state → publish bundle. Natural frozen-interface shape: bake
base colors (post-Fix-3: values), then `pending_edits::apply_overlay(...)`,
then publish. **BIGGEST RISK: do not freeze the A↔C interface over `Vec<u32>`
colors** — Fix 3 removes CPU-baked colors from WorldBundle while A codes
against the interface. Freeze it over an artifact that survives C's rewrite:
a per-cell overlay buffer (tint + outline/flag bits) or stroke geometry that
C's shaders composite. Note "tint + outline" is not expressible in the current
mechanism at all (opaque replacement only).

**Palettes.** layers.rs anchors are ΔE-verified and tested (:271–314) — bake
palettes into a small GPU LUT from that Rust (source of truth stays testable;
the same LUT bound to both pipelines satisfies "shared verbatim") rather than
hand-porting ramp math to WGSL.

**Scripts.** Perf mode measures only the current preset (default Standard7 —
contract wants default High8 + new Ultra9; touches `Preset::level/label`, the
combo :722–733, `fps_grid_level` :1136), 40 warmup + 240 frames per view, vsync
off. The L7/L8/L9+detail matrix needs a preset stage loop — each preset switch
= grid rebuild + full sim run; budget L9 wall time. CLI is a hand-rolled
`parse_args` (main.rs:29) — `--seed/--preset/--detail` slot in trivially.

### (c) tectonics: setup.rs seeding + elevation.rs fBm

Full report: `feel-pass-design/c-tectonics.md` (line refs @ 9d5d272).

**Replaceable unit = setup.rs:46–89** (farthest-point seeding + great-circle
Voronoi) inside `pub(super) fn setup(master_seed, &Arc<Grid>, &TectonicsParams)
-> SimState` (setup.rs:42; sole caller `SimState::setup`, step.rs:260). The
local `seeds` vec is never stored; the ONLY artifact is `SimState.plate_id:
Vec<u32>`. A generator writing contiguous ids `0..p_count` (every plate
non-empty) and pushing `p_count` PlateStates needs zero downstream code
changes. Keyframe plate_id is u16; PlateState ids are dense indices into an
append-only vec — no sparse/large ids.

**RNG streams** (all `sub_rng(seed, "phase1-tectonics", purpose)`):
"plate-seeds" (one u64), "plate-init-{pid}" (per-pid, geometry-independent),
"cratons", "ocean-age-ramp", "hotspots", "detail-noise". WARNING: the
"cratons" stream's draw alignment is entangled with plate geometry (per-plate
`target > 0` gates 2 draws, setup.rs:161–185) — a new plate map shifts craton
draws, so continents change too, not just plate outlines. Expected under the
one-time golden regen; judge-panel PNGs should render plate ids.

**Hard constraints:**
- `assert!(nd <= 32)` step.rs:428 — alive plates must stay ≤ 32 (u32 candidate
  bitmask); multi-seed growth must collapse helper seeds to final plate ids
  before returning.
- Setup `plate_count` clamp 8..=24 (`TectonicsParams::clamped`, mod.rs:107);
  run-time band `PLATE_FLOOR/CEIL` = 6/24 (step.rs:102) — "6–24" is about
  alive plates over the run; do not "fix" the clamp.
- tectonics_tests.rs:390–396 pins keyframe-0 plate_id IDENTICAL with and
  without craton overlay — the new generator must never read overlays.
- Determinism idioms in place: Voronoi strict `>` ⇒ ties to lowest seed index;
  farthest-point argmin ties to lower cell id; PQ keys must be (cost, cell id)
  with a total order (integer/quantized cost safest).

**fBm detail term (world data — untouched).** elevation.rs:69 inside
`derive_and_solve` (runs at EVERY keyframe): `elev += 300.0 * fbm(pos,
noise_seed, 6)` (DETAIL_AMP_M/DETAIL_OCTAVES, elevation.rs:34–35); seed from
the "detail-noise" stream (setup.rs:272, re-derived identically in
`SimState::from_keyframe`, step.rs:310 — resume-safe). `fbm` is `pub(crate)`
in noise_stage.rs — the renderer CANNOT link it (good for the render-only
guard); C implements its own noise. It feeds the t=0-only sea-level bisection
(elevation.rs:81–95) and both goldens.

**Keyframes/cadence.** Exactly 16 B/cell (eight u16-wide arrays; crust_type =
flags bit 15). `keyframe_interval_my` (mod.rs:63): 10 My for L≤7, 20 My for
L≥8 — the branch is `>= 8`, so an L9-only cadence needs a NEW arm; L6/L7 must
not move (goldens), L8 stays 20 My. L9 @ 20 My × 2 Gy ≈ 4.2 GB; ~100 My
spacing ≈ 0.88 GB. Span clamp 200–2000 My. `TectonicsHistory` stores its own
`keyframe_interval_my`, so a per-level cadence is safe for the era picker.

**dmath gap — Fix 2's one genuinely new piece of math.** dmath has NO inverse
trig and no arc-length helper; `det_sin_cos` is valid only |x| ≤ 0.75 rad.
Polyline steps (adjacent cell centers, ≤ ~0.03 rad at L6) are chord-safe; the
triple-junction ENDPOINT distance (up to π) is not. Stage D must pick and log
one of: a tested deterministic arc-length helper in dmath; the metric defined
consistently in chord space; or midpoint-normalize subdivision (`normalize3` +
fixed depth). proj.rs / grid lat-lon are display-path (std trig fine) — Eckert
IV needs no dmath work; only committed plate metrics fall under the rule.

**Boundary/junction structure: none exists.** Only per-cell F_BND_* display
bits (keyframe flags bits 5–7), recomputed each step. Sinuosity builds its own
structure from `plate_id` + CSR: boundary edge = unordered pair (a<b) with
differing plate_id, enumerated in id order; triple junction = grid triangle
(`grid.triangles`) with 3 distinct corner plate ids; walk chains with CCW
neighbor order (rings canonically start at lowest neighbor id); all serial.
`SimState::setup` is pub with pub `plate_id` — the fast t=0 gate test needs no
sim steps (elevation is underived at that point: read plate geometry only).

**Acceptance/practicals.** Phase-1 harness (`run_tectonics_harness`, app
harness.rs, SEED=42): age-depth (adds sea_offset back), hypsometry (2-means +
Ashman D>2), arc-trench ≤400 km same-plate, 2 Gy L6 stability, 1 Gy L7 ≤60 s,
2 Gy L7 ≤1 GB → NEW file tectonics-feelpass-{machine}.json; phase-1 files
untouched. worldmaker-sim has no dev-dependencies today — add `image` as a
dev-dep for the PNG panel test. Expectation check: `maybe_breakup` splits
plates along random great circles (step.rs:1051, 1099–1117), so late-era
boundaries re-grow straight regardless of the t=0 generator — gates are
t=0-only by design.

### (d) goldens, determinism tests, harness.rs, results schema

Full report: `feel-pass-design/d-determinism.md` (read @ 9d5d272 + docs commit
8381450).

**Goldens** — all in crates/worldmaker-sim/tests/determinism_tests.rs; FNV-1a-64
over LE bit patterns (`hash_f32_slice`/`hash_u32_slice`, core hash.rs:34–56):
- `GOLDEN_HASH_L6_SEED42` = 0xa86a_7471_79a3_5a46 (:29, noise stage — must not move)
- `GOLDEN_TECTONIC_ELEVATION_L6_SEED42` = 0xf751_0e72_14ed_5b62 (:59)
- `GOLDEN_TECTONIC_PLATES_L6_SEED42` = 0x70df_6db8_ec5f_653d (:60)
Tectonic config: L6, seed 42, `TectonicsParams::default()` (500 My), FINAL
keyframe fields. The crust-type hash (0xd5c797a8cc26afb5) is results-only, not
a golden. Regen procedure (manual, used twice on 2026-08-19): run the harness —
its `determinism_*_hash_l6_500my_seed42` metrics use the EXACT golden config —
paste the value, update the constant's `/// History:` doc-comment, add a
decision-log row, commit constant + results JSON together, once, on B's branch.
The new feelpass JSON hashes must equal the new constants (free cross-check).

**Test inventory** (green on this Air, 2026-08-25): 52 pass + 2 `#[ignore]`
(L9 grid build; `debug_keyframe_stats`), 3.2 s warm. worldmaker-app is a
binary-only crate — new app tests must be `#[cfg(test)]` modules in src/
(layers.rs/render.rs precedent). Eckert IV tests join core
tests/mapping_tests.rs.

**CLI/scripts.** `parse_args` (main.rs:29–47) knows exactly 4 flags
(`--screenshots/--perf-out/--determinism-out/--tectonics-results`); unknown
args are WARN-AND-IGNORE — an old binary silently swallows
`--seed/--preset/--detail`; scripts must verify flag support after C lands.
Screenshot script: 6 fixed stages (globe/flat/split/plates/mountains/
timeline.png), each captured at frame 30; defaults seed "cyrus"
(0xc4be0bf8f497a575), Standard L7, 500 My, equirect, graticule on. **The
BEFORE set is already committed** (8381450, docs/media/feel-pass/before/) —
the AFTER set must reuse the same seed/preset/eras. Perf: 40 warmup + 240
sampled frames per view, AutoNoVsync; grid builds L6–L9 timed in `main()`
pre-window; `fps_grid_level` = current preset (today always 7). Tectonics
harness hardcodes SEED=42; the stability land target is a hardcoded 0.29
(harness.rs:311, NOT `params.land_fraction` — a trap if parameterized).

**Results schema.** `ResultsFile{machine, date, app_version, metrics}` via
worldmaker_io (results.rs:11–42); filename `<topic>-<phase>-<machine>.json`,
`machine` must match filename; `machine_name()` → Daniels-MacBook-Air. New
files: `tectonics-feelpass-*` and `perf-feelpass-*` (phase token "feelpass").
Fix 2's incumbent metrics must land in the tectonics-feelpass file BEFORE the
replacement generator does. Phase-1/phase-0 files stay untouched.

**CI.** fmt/clippy/test on ubuntu gate merges; macos-14 build informational.
`clippy --all-targets` compiles `#[ignore]` tests — the panel test costs CI
compile time; worldmaker-sim needs `image` as a dev-dependency; worldmaker-io
already has serde for the Stroke type.

**Detail-guard plumbing (Track C).** `Stage::params_hash` is pub;
`TectonicsParams` has exactly 7 fields, none render-related (guard clause
trivially true today). WARNING: "same path Regenerate uses" =
`WorldApp::start_job`/`current_params` (app.rs:262–342), unreachable headlessly
(WorldApp needs an eframe/wgpu CreationContext) — C must factor the
world-building recipe into a free function shared by start_job and the test,
or the guard is by-convention only. Do not confuse `SimState.noise_seed` (sim
fBm, golden-feeding) with C's renderer detail seed. Fix 1's structural guard:
design the pending-edits module so it simply cannot name `Pipeline`/`start_job`
(Rust has no call-graph reflection).

**Fix 2 test constraints.** L5 sanity test pins: exactly 21 keyframes at
200 My, `hotspots.len() == 6`, alive plates 6..=24, anchor land ±0.005, final
±0.05 — the new generator must pass at L5 too. Bit-exact rerun and
resume-from-keyframe tests must survive. Setup-only gate: `SimState::setup`
pub + pub `plate_id`, no pipeline run needed.

**Phase-1 baselines** (hashes, wall times, fps, keyframe bytes) are recorded
in the full report §8 for later comparison.

## Stage D — decisions

*pending*
