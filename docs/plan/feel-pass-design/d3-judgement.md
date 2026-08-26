# Stage D judgement — Fix 3 (Track C): d3a lookup-extension vs d3b projected-mesh

Judge for WO-0003 Stage D. Both designs read in full; conflicting claims
verified against main @ 9d5d272 (render.rs, shaders.wgsl, grid.rs, proj.rs,
app.rs, layers.rs, tectonics/mod.rs). Verdict binds Track C: the winner's
design plus the grafts below IS the Track C plan.

## Verification notes (what I checked in the real code)

- Both designs describe today's renderer accurately: VERTEX-only visibility on
  globe binding 1 (render.rs:183), no sampler object anywhere, fullscreen flat
  triangle + WGSL `map_invert` + 4096×2048 R32Uint raster (render.rs:19–20,
  43–61; shaders.wgsl:79–139), flat zoom clamp 0.5–80 (app.rs:631), CPU picking
  via `Projection::invert` + `nearest_cell` (app.rs:647–652), CSR neighbors
  CCW-ordered (grid.rs:34), hypsometric ocean ramp sqrt-warped (layers.rs:147),
  Robinson hair-tolerance 1.0001 (proj.rs:67), `TectonicsParams` all-pub
  7 fields (mod.rs:73–90).
- **Found, neither design caught it:** `Grid::nearest_cell` (grid.rs:131–148)
  moves on **exact dot ties toward the lower id** (`d > best_d || (d == best_d
  && nb < best)`). d3a §4.1's "first strictly-better in ring order… mirrors the
  CPU walk's tie behavior" is wrong as written — a strict-> walk stops on ties
  the CPU walk crosses. Fixable in two lines (requirement R1 below); with the
  fix the "GPU winner == nearest_cell by construction" claim becomes true.
- **Found:** d3a §4.1 step 6's raw solve `[P_c P_a P_b]·β = p` is
  ill-conditioned in f32 at L9 (triple products of near-parallel unit vectors:
  result ~5e-6 against ~1e-7 rounding per term → percent-level weight error).
  The differenced/tangent-frame form is well-conditioned (requirement R2).
- Both designs' shared globe scheme (non-indexed vertex pulling, one-hot
  barycentrics, flat `vec3<u32>` corner ids, w=1 ⇒ linear interpolation) is
  sound and identical in substance — the globe is not a differentiator.
- Both rest on the same geometric fact (well-centered icosphere triangulation
  ⇒ nearest-of-3-corners is the global nearest cell). d3a states the acute-fan
  justification explicitly; d3b invokes duality. Same assumption, shared risk,
  covered by d3a's §4.5 CPU property test either way.

## Scores (10 = ideal)

| Dimension | d3a lookup | d3b mesh | Notes |
|---|---|---|---|
| No-visible-facets, default + high zoom | **9** | 7.5 | Globe identical. Flat: d3a is analytically exact at any zoom (inverse projection per fragment, exact Voronoi, chord-plane barycentrics numerically shared with the globe). d3b is correct but interpolates in projected space (weights distorted vs the globe at high latitude), admits pole-cap distortion, and has sub-pixel-triangle shimmer at L9 default zoom (5.2M tris into ~1M px). |
| Implementation risk on M1/wgpu | **8** | 6 | d3a's risk concentrates in one fragment function with a CPU mirror property test (§4.5); no new geometry code; keeps the proven raster as a mere hint. d3b's seam/pole exception mesh, wrap frames, FlatGeom lifecycle, rect_px NDC plumbing and background pipeline are the classic crack/sliver/notch bug surface — unit-testable, but visual-geometric bugs are the slowest kind to burn down. |
| Perf, L8 default + L9 | **8** | 6 | Globe identical (shared Ultra9 VS-pull risk, 15.7M invocations). Flat: d3a's per-pixel walk cost is resolution-independent across levels (cap 4 × ≤6 neighbors, cache-coherent); d3b's flat scales with cell count — self-estimated 25–40 fps at L9 full map on the Air, and Split view doubles the pull count. Projection switch: d3a free, d3b tens-of-ms hitch. Grid rebuild: d3b wins (raster deleted) — not enough to flip the row. |
| Robustness across 3 projections incl. Eckert IV | **8.5** | 7 | d3a: no seam code exists at all (the sphere walk is seamless); Eckert IV = one closed-form WGSL inverse arm under the twice-proven strict-gate pattern. d3b kills WGSL inverses (genuinely nice) but the ROB table stays in WGSL for the outline either way, and projection robustness moves into the new seam/pole machinery. |
| Brush/cursor correctness | 9 | 9 | Both keep app.rs picking untouched and are exact by construction (d3a via the mirrored walk after R1; d3b via duality). Tie. |
| Complexity A absorbs via frozen interface | 7 | **9** | d3b's interface is better: `Stroke { tool, payload }` matches the pinned Fix 1 wording literally (d3a's collapsed enum drops `tool`); `generated_hotspots: Option<…>` models the history-dropped mid-run case explicitly instead of pushing marker resolution to the caller; alpha bits give A latitude. |
| Memory | 6.5 | **8.5** | d3a L9 worst ≈235 MB GPU + keeps the 33 MB CPU raster and its rebuild cost; d3b ≈150 MB and deletes `rasterize_cell_ids` entirely. Both fit both machines. |
| **Total** | **56** | 53 | |

## WINNER: d3a — lookup extension (hint raster + per-fragment exact Voronoi)

The three heaviest dimensions — facet-free correctness at every zoom,
implementation risk on the machine we actually develop on, and L9 flat
performance — all fall to d3a, and they fall for the same structural reason:
the fullscreen-inverse architecture has no geometry to get wrong and no cost
that scales with cell count. d3b's genuine wins (memory, interface shape,
several sharp implementation details) are exactly the kind that graft cleanly;
its losses (seam/pole geometry risk, L9 raster scaling, projected-space
interpolation) are structural to the mesh approach and cannot be grafted away.

## Grafts from d3b (mandatory — part of the Track C plan)

1. **Stroke type shape** (d3b §5.1 verbatim): `StrokeTool` + `StrokePayload` +
   `Stroke { tool, payload }` — the pinned Fix 1 contract says
   "Stroke = { tool, payload }"; follow it literally. Replaces d3a §7.1's
   collapsed enum.
2. **`OverlayInput` container with `generated_hotspots: Option<&[[f32; 3]]>`**
   (d3b §5.1), replacing d3a's `hotspot_markers` argument. The mid-run case
   (history dropped at job start) is modeled in the interface: A must render
   pending hotspot adds/removes even when the base set is `None`.
3. **Merged overlay word layout**: d3a's tint codes + force-outline bit, plus
   d3b's alpha byte (exact layout in the frozen interface below).
4. **Exhaustive-literal `TectonicsParams` guard** (d3b §9.2): construct the
   params with an exhaustive 7-field struct literal (no `..Default`) so any
   added field breaks the build. Verified: all fields are pub, so this test
   lives in C's app-side `worldgen.rs` `#[cfg(test)]` — no sim-crate edit, no
   partition violation. It replaces d3a §11.3's weaker Debug-string regex
   (keep the regex too if free; the literal is the gate).
5. **Sqrt-warped ocean LUT row** (d3b §1.5): row 0 texels linear in
   `u = sqrt(clamp(−e/6000, 0, 1))`, shader computes `u` — matches
   `hypsometric`'s `t.sqrt()` (layers.rs:147) with proper resolution in the
   steep near-coast region, where d3a's linear-in-depth texels under-resolve.
   Manual two-texel `textureLoad` + `mix` stays (d3a §2.3, no sampler).
6. **Per-octave fade against pixel footprint** (d3b §3): attenuate each octave
   by `1 − smoothstep(0.25, 1.0, fwidth(length(q)))` — kills sub-pixel noise
   shimmer; d3a lacked any counterpart. Also: the sweep's first comparison row
   evaluates d3b's gradient noise against d3a's value noise at equal
   octaves/amplitude (d3b's "value noise reads blocky at coast scale" concern
   is plausible); the panel decides, both flavors use d3a's seed-lane integer
   hashing and §5.2 domain scheme.
7. **Screenshot-mode forced parity** (d3b §8.2): screenshot mode without
   explicit flags forces seed "cyrus" + Standard7 + detail 1.0 so the AFTER
   set matches the committed BEFORE set by default, not by checklist
   discipline; and the wrapper scripts fail loudly when the binary logs
   "ignoring unknown argument" (old-binary flag swallowing, d-report risk 7).
8. **Deterministic sweep coast crop** (d3b §10): the flat close-up centers on
   the max-slope cell with |elev| < 200 m, lowest id tie-break — reproducible
   per seed, better than reusing fixed screenshot stages for the judged crop.
9. **Headroom note** (d3b residual 1): an indexed scalar-layer fast path for
   the globe (scalar layers need no corner triple) is the documented fallback
   if Ultra9 globe fps disappoints on the PC — designed to be addable without
   buffer changes; record it in the work order, do not build it now.

## Judge-added requirements (from code verification)

- **R1 — walk step rule**: the GPU walk must replicate `nearest_cell`'s actual
  step: best-improvement over the ring with tie-break
  `(d > best_d) || (d == best_d && nb < best)` (grid.rs:131–148), not
  first-strictly-better. Then the §4.5 property test asserts bit-equality of
  winners, ties included.
- **R2 — barycentric conditioning**: solve the flat weights in differenced
  form (edge vectors `P_a − P_c`, `P_b − P_c`, `p − P_c`, or equivalent
  tangent-frame 2×2), not raw `[P_c P_a P_b]·β = p` triple products — the raw
  form loses percent-level accuracy in f32 at L9 spacing. Extend §4.5's CPU
  mirror test with a weight-accuracy bound (≤1e-4 vs an f64 reference) at
  L8/L9 sample points.

Everything else in d3a stands as written (WorldBundle §2, globe §3, flat §4,
shared WGSL + detail §5, uniforms §6, boundaries §8, Eckert IV §9,
presets/flags/perf §10, guard §11 as amended by graft 4, sweep §12 as amended
by grafts 6/8, risks §14).

## FROZEN A↔C interface (final — copy into feel-pass-design.md as the frozen contract)

Typed only over artifacts that survive C's rewrite: `worldmaker_core::Grid`,
`&mut [u32]`, and the io Stroke type. C ships both files (stub + constants) so
the merge order B→C→A compiles; the shapes below are frozen text — neither
track changes them until A's rebase (one decision-log line records the
shape-only partition exception).

```rust
// worldmaker-io/src/strokes.rs — shape frozen; behavior and impls are A's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StrokeTool { CratonPaint, CratonErase, Hotspot }

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum StrokePayload {
    /// Cell ids at the current grid level; sign = +1 paint continent, −1 force ocean.
    CratonPaint { cells: Vec<u32>, sign: i8 },
    /// Unit-vector position.
    HotspotAdd { pos: [f32; 3] },
    /// Unit-vector position the removal targets.
    HotspotRemove { pos: [f32; 3] },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Stroke { pub tool: StrokeTool, pub payload: StrokePayload }

// worldmaker-app/src/pending_edits.rs — module OWNED BY TRACK A.
// C creates it with exactly these items and a no-op apply_overlay body.

/// Overlay word layout (frozen):
///   bits 0..=3   tint code: 0 none, 1 craton +1, 2 craton −1,
///                3 hotspot existing marker, 4 pending hotspot add,
///                5 pending hotspot remove, 6..=15 reserved
///   bit  4       force-outline (outline this cell's region edge even where
///                the neighbor has the same tint code)
///   bits 8..=15  tint alpha 0..=255 (0 ⇒ renderer default 160)
///   bits 16..=31 reserved (zero)
pub const OVERLAY_TINT_MASK: u32 = 0xF;
pub const OVERLAY_FORCE_OUTLINE: u32 = 1 << 4;
pub const OVERLAY_ALPHA_SHIFT: u32 = 8;
pub const OVERLAY_ALPHA_MASK: u32 = 0xFF << OVERLAY_ALPHA_SHIFT;

pub struct OverlayInput<'a> {
    pub grid: &'a worldmaker_core::Grid,
    /// Pending stroke list, oldest first — passed EXPLICITLY; the function
    /// must not read tool state, history, or WorldApp.
    pub pending: &'a [worldmaker_io::Stroke],
    /// Base hotspot set for rendering pending hotspot deltas; None mid-run
    /// (history dropped at job start) — adds/removes must still render.
    pub generated_hotspots: Option<&'a [[f32; 3]]>,
}

/// Fill `out` (len == grid.cell_count(), pre-zeroed by the caller) with
/// per-cell overlay words, newest stroke winning per cell. Pure function of
/// its arguments; cell ids >= out.len() are skipped silently (stale ids
/// across a level switch never panic); hotspot positions resolve to marker
/// cells via grid.nearest_cell + neighbor ring (today's marker shape,
/// app.rs:416–421). No route to Pipeline, start_job, or TectonicsParams —
/// none of those types appear in this module's API or imports (Fix 1's
/// structural no-sim guard).
pub fn apply_overlay(input: &OverlayInput<'_>, out: &mut [u32]);
```

Call site (C's rebake, per d3a §2.4 step 3, amended): the overlay pass runs on
**every** rebake — including when `history` is `None` — with
`generated_hotspots: self.history.as_ref().map(|h| h.hotspots.as_slice())`
(shape per A's rebase); until A lands, `pending` is an empty `Vec<Stroke>`
field and the no-op body renders nothing. GPU side: `overlay: array<u32>`
bound FRAGMENT in both pipelines, uploaded on `overlay_gen` bumps; C
composites tint via LUT row 5 colors at the word's alpha and draws the
outline with the §3.3/§7.4 bisector-margin machinery wherever adjacent
candidates' words differ or FORCE_OUTLINE is set.

## Adversarial review (fix3)

Reviewer pass against main @ 9d5d272 (grid.rs, render.rs, shaders.wgsl,
app.rs, layers.rs, proj.rs, tectonics/mod.rs, main.rs, mapping tests) plus
d1-fix1-design.md for cross-track interface fit. Numeric claims below were
checked by computation, not by reading. Verdict: the d3a-plus-grafts plan
survives with **1 BLOCKER and 2 major amendments**, all fixable in the design
text; the winner choice itself is not disturbed.

### Verified (checked against real code — VERIFIED unless amended below)

- **R1 tie rule** — VERIFIED verbatim: grid.rs:138 is
  `d > best_d || (d == best_d && nb < best)` inside the loop at grid.rs:131–148.
  Tie-moves go only to a lower id, so the walk cannot cycle; the GPU cap-4
  mirror terminates. Judgement's citation is exact.
- **Code anchors** — VERIFIED: 4096×2048 R32Uint raster (render.rs:19–20,
  43–61); VERTEX-only globe binding 1 (render.rs:183); single shader module
  (render.rs:163–166); write-or-recreate upload (render.rs:399–416); both
  `prepare()`s rewrite whole uniforms every frame (render.rs:486–495,
  557–574) and the projection match at render.rs:564–567; `map_invert` +
  equirect strict gate (shaders.wgsl:79–109, comment 84–86); WGSL ROB table
  == core ROBINSON_TABLE; flat zoom clamp 0.5–80 (app.rs:631); CPU picking
  (app.rs:647–655); rebake early return (app.rs:399–401); marker shape
  (app.rs:416–421); sea-level needs_bake (app.rs slider); default Standard7
  (app.rs:220); layers.rs sqrt ocean warp (actually line 146, judgement said
  147 — harmless off-by-one), plate_rank :190–205, boundary priority
  :216–222; TectonicsParams = exactly 7 pub fields (mod.rs:73–90);
  Robinson 1.0001 (proj.rs:67); parse_args warn-and-ignore (main.rs:43);
  Grid CSR fields pub + CCW-documented (grid.rs:24–38) so d3a's storage
  bindings need no core changes; sim `fbm` is `pub(crate)`
  (noise_stage.rs:63) — unlinkable, as claimed.
- **Score arithmetic** — VERIFIED: 56 and 53 sum correctly.
- **Memory/limits** — VERIFIED: L9 worst ≈235 MB total; largest single
  buffer (CSR neighbors ≈62.9 MB, tri_ids ≈62.9 MB) < default 128 MB
  maxStorageBufferBindingSize; 5 fragment-stage storage buffers < default 8;
  downlevel-4 fallback (CSR concat → exactly 4) works.
- **Uniform layouts** — VERIFIED: ShadeParams 32 B lands 16-aligned in both
  structs (globe offset 80, flat offset 48); 112/80 B totals correct.
- **Frozen interface vs Track A's stated needs** — VERIFIED: satisfies d1 §7
  requirements 1–6 (explicit pending set, mid-run, not-colors, tint+outline,
  stale-id skip, deterministic). Mid-run overlay path (values Arc reuse +
  always-run overlay pass) is sound for seed regenerates. TectonicsHistory
  has `pub hotspots: Vec<[f32; 3]>` (keyframe.rs:242) so the call-site shape
  compiles. **Type-shape conflict with d1 itself: see A2.**
- **Eckert IV numerics** — VERIFIED by simulation: f32 Newton, θ₀ = φ/2,
  cap 8, tol 1e-7 round-trips with worst error 1.5e-6 rad over |lat| ≤ 89°
  — passes the suite's 1e-4 with margin; pole short-circuit covers exact
  ±90° cell centers; rejection anchor invert(0.9, 0.99) correct (pole-line
  half-width 0.5); aspect exactly 2. See A8 for the one pole-ring caveat.
- **Golden safety** — VERIFIED: every C-side change is app/render/proj
  territory; no sim-crate edit except the (test-only) graft-4 relocation to
  app-side worldgen.rs; nothing here can move a golden outside B's single
  sanctioned regeneration. Guard test (§11) is the strongest structural
  statement available and honestly labelled.
- **Determinism** — VERIFIED: no std-trig enters any committed metric from
  Track C (display-path only); rasterize_cell_ids rayon rows are disjoint +
  per-row serial (deterministic); bake_values rayon is UI-only and
  order-preserving; boundary extraction serial id-ordered; graft-8 crop rule
  is deterministic under a serial strict-`>` id-scan.

### Findings and fixes

- **B1 — BLOCKER (d3a §4.1 step 5): wedge containment test has an inverted
  sign and selects the wrong wedge.** As written, `dot(p, cross(pos_c,
  pos_ni)) ≥ 0 ∧ dot(p, cross(pos_ni1, pos_c)) ≤ 0` reduces to
  "g_i ≥ 0 ∧ g_{i+1} ≥ 0" (same sign twice) — not a bracketing test.
  Verified counterexample (c = pole, n_i = +x, n_{i+1} = +y, CCW): a point
  inside wedge i FAILS the test while a point in wedge i+1 PASSES it — every
  flat fragment gets a rotated wedge and garbage (often negative)
  barycentrics. **Fix (binding):** condition 2 becomes
  `dot(p, cross(pos_c, pos_ni1)) ≤ 0` (equivalently `dot(p, cross(pos_ni1,
  pos_c)) ≥ 0`). Additionally: compute `g_j = dot(p, cross(pos_c, pos_nj))`
  once per ring index and reuse it as wedge j's second test and wedge j+1's
  first, so shared boundaries evaluate bit-identically (exactly one wedge
  matches; on an exact 0.0 both match and first-in-ring-order wins); if the
  scan somehow exhausts the ring (cap-truncated walk on a future L>9 grid),
  fall back to the wedge with the largest min(g_i, −g_{i+1}) — never
  unreachable UB. AMENDED.
- **A2 — major (cross-track): the frozen Stroke surface contradicts
  d1-fix1-design.md, and both cannot land.** d1 ships
  `worldmaker-io/src/stroke.rs` with `Stroke { payload }` + a `tool()`
  accessor, `StrokeTool { Craton, Hotspot }`, and field name `unit`
  (d1 lines 18–61); the frozen text pins `strokes.rs`,
  `Stroke { tool, payload }`, `StrokeTool { CratonPaint, CratonErase,
  Hotspot }`, field `pos`. **Resolution (binding): the frozen text in this
  judgement wins** — the pinned Fix 1 contract says "Stroke = { tool,
  payload }" and the merge order has C shipping the file; A amends d1 at
  rebase. Three additive clarifications to the freeze: (1) C's strokes.rs
  ships with `pub mod strokes;` + `pub use strokes::{Stroke, StrokePayload,
  StrokeTool};` in worldmaker-io lib.rs — the frozen `worldmaker_io::Stroke`
  path in OverlayInput requires the root re-export the frozen text omitted;
  (2) consistency rule for the redundant pair: the **payload is
  authoritative** — `apply_overlay` and every renderer read only
  `stroke.payload`; `tool` is UI metadata (badge/undo labels), and A may
  debug_assert tool matches the payload discriminant; (3) d1's canonical
  form (cells sorted + deduped) remains A's internal invariant, not part of
  the frozen shape. AMENDED.
- **A3 — major (tests don't test the claim): R2's "≤1e-4-vs-f64" bound and
  §4.5's mirror test are circular as specified.** A CPU mirror of steps 3–6
  compared against an f64 run of the *same* mirror reproduces B1's wrong
  wedge on both sides and passes; §4.5 as written checks only winner
  equality and convergence, so neither test would have caught B1.
  **Fix (binding):** the f64 reference must be independent of the mirror —
  brute-force scan of the winner's fan (or all triangles at L6) with the
  standard three-half-space containment in f64, then an f64 solve; the test
  asserts (a) GPU-mirror wedge == reference wedge, (b) weights within 1e-4,
  (c) winner bit-equality per R1, at L6/L7/L8 (+ L9 spot samples) on
  PCG-seeded points. AMENDED (R2 + d3a §4.5).
- **A4 — minor (forgotten state): mid-run values reuse across a grid
  switch.** d3a §2.4 step 2 "keep the previous bundle's values Arcs" must
  never survive a preset switch — the old Arcs are sized to the old grid.
  Safe only because rebuild_grid publishes a fresh bundle first; the design
  never says so. **Fix:** rebuild_grid's successor explicitly publishes a
  placeholder bundle for the new grid (neutral values — precedent
  app.rs:299–305's 0xff40_4040 — zeroed overlay, empty BoundarySet, all
  three generations bumped), preserving the "values_gen/overlay_gen bump
  with grid_gen" invariant. AMENDED (d3a §2.4).
- **A5 — minor: globe nearest-of-3 tie-break.** d3a §3.3 ties "to the lowest
  k" (corner index); `nearest_cell` ties to the lowest **cell id** — corner
  order in `grid.triangles` is not id-sorted, so exact-tie fragments could
  disagree with CPU picking. Sub-pixel and cosmetic, but the fix is free:
  tie on the lower cid, matching R1's spirit on both canvases. AMENDED.
- **A6 — minor (internal contradiction): graft 5 vs d3a §2.3/§5.3.** Graft 5
  makes row 0 linear in `u = sqrt(clamp(−e/6000, 0, 1))` with the shader
  computing the sqrt, but "everything else stands as written" leaves d3a
  §2.3's row-0 bake (`hypsometric(−(i/255)·6000)`) and §5.3's "sqrt-free
  coordinate −e_render/6000" sentence in force — an implementer following
  d3a literally under-resolves the near-coast ramp. **Clarified:** graft 5
  supersedes both passages; row 0 texel u holds
  `lerp3(shallow, deep, u/255)`, shader computes u. AMENDED.
- **A7 — minor (frozen word gaps):** bits 5..=7 of the overlay word were
  unspecified — they are **reserved, must be zero** (renderer may assume
  so). The code-5 (pending hotspot remove) tint "HOTSPOT_MARK darkened
  ×0.55" has no LUT row-5 texel — pinned: the shader multiplies the row-5
  code-6 color by 0.55 for tint code 5 (no new texel). AMENDED (frozen
  interface doc comment + d3a §7.4).
- **A8 — minor (Eckert pole ring at L9):** f32 forward error grows as
  1/cos φ; at the L9 ring nearest the pole (~89.88°) residual noise maps to
  ~2e-4 rad of λ/φ wobble — inside a cell width (all committed tests still
  pass; measured, see Verified) but visible as sub-pixel readout jitter.
  Cheap insurance, adopted: compute the CPU forward's Newton internally in
  f64 with cap 12, tol 1e-9 (d3b §7.1's numerics), θ₀ = φ/2 and the pinned
  formulation unchanged; the closed-form inverse and WGSL arm are untouched.
  AMENDED (d3a §9.1 / D10 constants).
- **A9 — minor (spec gaps, one line each):** (1) d3a §8.2 says the ribbon
  VS "discards" back-hemisphere segments — a VS cannot discard; collapse the
  quad to zero area in the VS or pass camera-z and discard in FS (d3b §6.2's
  shape). (2) Closed boundary loops (junction-free — e.g. an enclave plate,
  or few-plate worlds) have no endpoints: Chaikin must run periodically
  (wrap), not "pin endpoints". (3) Graft 8's coast crop needs a CPU slope
  definition under d3a (which has no slope buffer): serial id-ordered scan,
  slope(c) = max over CSR neighbors |elev[n] − elev[c]|, strict `>` so ties
  go to the lowest id. AMENDED (d3a §8.2, §8.1 step 5, §12/graft 8).

All other design elements reviewed and left standing as written: WorldBundle
shape §2, unindexed globe §3.1–3.2, masked CrustAge §3.4, walk-cap geometry
§4.2 (hint ≤ ~0.5 cell spacing at L9 — the 4-cap has real margin), noise
scheme §5.2 (lattice coords ≈5.1e3 < 2^24 at L9/6 octaves — exact), slider
semantics §5.3, uniforms §6, boundaries-extraction determinism §8.1,
presets/flags/perf §10 (including the Standard7 screenshot-parity force),
guard §11 as amended by graft 4, sweep §12 as amended by grafts 6/8, and the
winner verdict with its scores.
