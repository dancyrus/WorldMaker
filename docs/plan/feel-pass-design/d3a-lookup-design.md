# Stage D design — Fix 3 (Track C): rendering + resolution
## Candidate A: "lookup extension" — hint raster + per-fragment exact Voronoi

Designer A of two competing Stage-D designs for Fix 3. Assigned flat mechanism:
extend the cell-id lookup approach. Line references are against commit 9d5d272.
No code has been changed; this is the implementation contract for Track C if
this design wins.

---

## 0. Decision summary

| # | Decision |
|---|---|
| D1 | `WorldBundle.colors` is replaced by a per-cell **values** buffer (`[u32; 2]`/cell: f32 scalar bits + category word). All palette/threshold/detail work moves per-fragment. |
| D2 | Globe draws **unindexed** (3 verts/triangle, no vertex buffers): the vertex shader fetches corner ids from a triangle-list storage buffer and emits one-hot barycentrics + a flat `vec3<u32>` of the triangle's three cell ids. This gives every fragment barycentrics **and** all three corner cells without optional wgpu features. |
| D3 | Flat keeps the 4096×2048 R32Uint raster **as a hint only**; each fragment refines to the exact Voronoi winner with a fixed-cap greedy walk over the CSR neighbor graph (GPU mirror of `Grid::nearest_cell`), finds the containing icosphere wedge in the winner's CCW ring, and computes the same chord-plane barycentrics the globe rasterizer produces. The 0.088°/texel zoom cap disappears. |
| D4 | One `shaders.wgsl` module (as today, `include_str!` at render.rs:165) holds all shared functions; both fragment entry points call the **same** `resolve_fragment(...)` — noise + palette shared verbatim by construction. |
| D5 | Palettes ship as a 256×8 `Rgba8Unorm` LUT baked from layers.rs's ΔE-verified Rust anchors; one texture bound to both pipelines; all interpolation stays in sRGB-encoded space (matches today's CPU bake byte-for-byte in spirit). |
| D6 | Render detail = seeded u32-lattice value-noise fBm on the unit sphere, seed passed as two u32 uniform lanes, amplitude conditioned on candidate-triple slope + a smooth ocean-depth fade; applied to the Elevation layer only, before threshold + tint. |
| D7 | Sea level, Detail, debug toggles = uniform writes only; they no longer set `needs_bake`. |
| D8 | Frozen A↔C interface: `pending_edits::apply_overlay(pending: &[Stroke], hotspot_markers: &[[f32;3]], grid: &Grid, overlay: &mut [u32])` filling a per-cell u32 overlay word (tint code + flags); composited per fragment with an outline drawn on overlay-code changes using the same nearest-corner margin machinery. Works mid-run because the overlay pass never needs `history`. |
| D9 | Plate boundaries: CPU chain extraction over Voronoi vertices (mixed triangles), Chaikin ×2 on the sphere, GPU ribbon expansion in the vertex shader; three boundary-type colors from the LUT; legacy one-cell bands behind a debug flag bit. |
| D10 | Eckert IV: core forward (Newton, cap 8, tol 1e-7 rad) + closed-form inverse; WGSL gets the closed-form inverse as `misc.x = 2.0` with the equirect-style strict domain gate. |
| D11 | Presets: default **High8**, Draft6 and Standard7 stay, **Ultra9** added. Perf script loops Standard7→High8→Ultra9 with detail on; `--seed/--preset/--detail` flags. |
| D12 | Render-only guard: free `worldgen::build_world(...)` shared by `start_job` and an app `#[cfg(test)]` test; plus a Debug-string field-name check that `TectonicsParams` exposes nothing render-shaped. |

---

## 1. Architecture overview

Today, `layers::bake` (layers.rs:184) produces `Vec<u32>` RGBA8 colors consumed
by both pipelines (globe per-vertex, flat per-texel). This design replaces that
with:

```
CPU (rebake)                         GPU (per fragment, both canvases)
────────────                         ─────────────────────────────────
bake_values(layer, kf)               candidates = 3 cell ids + weights
  → values: [scalar, cat]/cell         (globe: tri corners + rasterized bary;
apply_overlay(pending, …)               flat: walk + wedge + bary solve)
  → overlay: u32/cell                scalar = Σ wᵢ·scalarᵢ  (masked where needed)
                                     e = scalar − sea_level + detail_noise
palette LUT (baked once from        color = LUT(layer, e | t | rank)
  layers.rs anchors)                 + boundary/debug lines (margin AA)
                                     + overlay tint + outline
```

Both pipelines compile from the single `shaders.wgsl` module (render.rs:163–166
today — unchanged); every shared function (`render_fbm`, `palette_*`,
`resolve_fragment`, `map_invert`) exists once and is called by both entry
points, which is what makes "noise + palette shared verbatim between canvases"
structural rather than aspirational.

---

## 2. WorldBundle, rebake, and what remains of layers.rs

### 2.1 New `WorldBundle` (render.rs, C-owned)

```rust
pub struct WorldBundle {
    pub grid: Arc<Grid>,
    /// Per-cell shading record for the active layer.
    /// x = f32 scalar bits (see §2.2), y = category word (see §2.2).
    pub values: Arc<Vec<[u32; 2]>>,
    /// A↔C overlay words, one per cell (§7). Rebuilt independently of values.
    pub overlay: Vec<u32>,
    /// Equirectangular *hint* raster, unchanged 4096×2048 R32Uint. Still built
    /// once per grid level by rasterize_cell_ids (render.rs:43) — its role is
    /// demoted from "truth" to "walk start" (§4).
    pub cell_ids: Arc<Vec<u32>>,
    /// Smoothed plate-boundary ribbons for the viewed keyframe (§8); empty
    /// when the layer draws none.
    pub boundaries: Arc<BoundarySet>,
    pub grid_gen: u64,
    /// Bumped when `values` changes (layer switch, scrub, new history).
    pub values_gen: u64,
    /// Bumped when `overlay` changes (any pending-stroke mutation).
    pub overlay_gen: u64,
}
```

`values` and `boundaries` are `Arc` so an overlay-only republish (painting
while a job runs) is O(cells) for the overlay Vec and free for everything else.
`sync_world` (render.rs:340) diffs three generations instead of two; the
existing "field_gen always bumps with grid_gen" invariant (render.rs:395–396)
becomes "values_gen and overlay_gen always bump with grid_gen".

### 2.2 `layers::bake_values` — the slimmed bake

```rust
/// Per-cell shading records for one keyframe. No sea level (GPU uniform),
/// no overlay (separate buffer), no colors (GPU LUT).
pub fn bake_values(layer: Layer, kf: &Keyframe) -> Vec<[u32; 2]>
```

Per-cell record, little-endian:

- `x` (scalar, f32 bits):
  - Elevation, Plates: `kf.elev_m[c] as f32` — raw keyframe-relative meters,
    **no neighbor averaging** (pinned). Plates carries elevation so the record
    layout is uniform; the plates shader path ignores it.
  - CrustAge: ramp coordinate `t = 1 − clamp(age/150, 0, 1)` (layers.rs:238
    math moved here); continent cells store `t = 0` (never read — see masked
    interpolation, §3.4).
  - Thickness: `t = clamp((ckm·0.01 − 5)/65, 0, 1)` (layers.rs:243).
- `y` (category word):
  - bits 0..=7: plate **rank** (the `plate_rank` logic of layers.rs:190–205
    moves into bake_values; ranks ≤ 23).
  - bits 8..=9: boundary code, priority pre-resolved exactly as layers.rs:216–222
    (0 none, 1 trench/convergent, 2 ridge/divergent, 3 transform).
  - bit 16: continent flag (kf.flags bit 15).
  - bits 24..=31: reserved.

Same rayon order-preserving `collect_into_vec` shape as today (UI-only, never
in a golden path). The serial overlay overwrite loop (layers.rs:251–267) is
**deleted** — replaced by the overlay buffer (§7).

### 2.3 Palette LUT — Rust stays the source of truth

```rust
/// 256 × LUT_ROWS RGBA8 texels, row-major, baked by evaluating the existing
/// Rust ramps. Deterministic, tested against ramp()/hypsometric() directly.
pub const LUT_W: u32 = 256;
pub const LUT_ROWS: u32 = 8;
pub fn bake_palette_lut() -> Vec<u8>   // LUT_W * LUT_ROWS * 4 bytes
```

| row | content | indexed by |
|---|---|---|
| 0 | hypsometric ocean: texel i = `hypsometric(−(i/255)·6000)` | `t = clamp(−e/6000, 0, 1)` |
| 1 | hypsometric land: texel i = `hypsometric((i/255)·5500)` | `t = clamp(e/5500, 0, 1)` |
| 2 | viridis (VIRIDIS anchors via `ramp`) | age t |
| 3 | batlow (BATLOW anchors via `ramp`) | thickness t |
| 4 | 24 plate colors in texels 0..24 (PLATE_COLORS), the oceanic-darkened ×0.55 variants in texels 32..56 | rank (+32 if oceanic) |
| 5 | fixed colors: 0 BOUNDARY_TRENCH, 1 BOUNDARY_RIDGE, 2 BOUNDARY_TRANSFORM, 3 AGE_CONTINENT, 4 PAINT_CONTINENT, 5 PAINT_OCEAN, 6 HOTSPOT_MARK, 7 outside-map background | code |
| 6–7 | reserved | |

- Texture: `Rgba8Unorm` (**not** `-srgb`), 2D, no sampler object anywhere
  (matching the current zero-sampler design): ramps are read with two
  `textureLoad`s + an explicit `mix` (manual linear interpolation), categorical
  rows with a single `textureLoad`. Manual lerp keeps interpolation math
  deterministic across GPUs, and identical in behavior to the CPU
  piecewise-linear-in-sRGB ramps the ΔE verification was done for. This is the
  resolution of the code map's sRGB risk: **all color math stays in
  sRGB-encoded space** — LUT bytes in, `mix` in encoded space, Lambert
  multiply in encoded space (exactly `fs_globe`'s current behavior,
  shaders.wgsl:40–44), target format untouched. No linear-light conversion
  anywhere, so the AFTER screenshots stay comparable to BEFORE.
- Uploaded once in `SceneResources::new`; the **same texture view is entered
  in both bind groups** — "one LUT bound to both pipelines", literally.
- New test `lut_rows_match_rust_ramps` in layers.rs `#[cfg(test)]`: every ramp
  texel equals `pack3(ramp(...))`/`pack3(hypsometric(...))` for its
  coordinate; categorical texels equal their constants. The existing anchor and
  distinctness tests (layers.rs:271–314) stay untouched.
- `layers::bake` (colors) is deleted; `pack`/`pack3`, `ramp`, `hypsometric`,
  the anchor tables, `Layer` all remain. `BakeOverlay` is deleted (replaced by
  §7). Re-log the reversal of the "palettes baked on CPU" decision in
  decision-log.md (pinned requirement).

### 2.4 rebake — new step list

`rebake` (app.rs:397) becomes:

1. `self.needs_bake = false`.
2. **Values pass** (only if `self.history` is Some): pick the viewed keyframe
   (clamped, as today), `values = Arc::new(layers::bake_values(self.layer, kf))`,
   `values_gen += 1`, and rebuild `boundaries` (§8) if the layer shows them.
   If `history` is None (mid-run — start_job dropped it, app.rs:316), **keep
   the previous bundle's `values`/`boundaries` Arcs and generation**. The old
   early-return (app.rs:399–401) is deleted; this is what makes mid-run
   pending-stroke display possible (resolves code-map friction a§10.3).
3. **Overlay pass** (always): `let mut overlay = vec![0u32; grid.cell_count() as usize];`
   `pending_edits::apply_overlay(&self.pending_strokes, &self.hotspot_marker_positions(), &self.grid, &mut overlay);`
   `overlay_gen += 1`. Until Track A lands, `pending_strokes` is an empty
   `Vec<Stroke>` field and `hotspot_marker_positions()` returns the same
   tool-gated set rebake computes today (app.rs:414–421) so current hotspot
   markers keep rendering; A replaces both call sites' inputs on rebase.
4. Publish `Arc<WorldBundle>` with the three generations.

`needs_bake` setters that **survive**: job completion, layer switch, tool
switch, paint/stroke mutation, timeline scrub, playback, script stages.
Setters that are **removed**: sea-level slider (app.rs:746) and the new Detail
slider — those write uniforms only (§6). Sea level and Detail thereby stay
pure live view controls (Fix 1 pinned list).

---

## 3. Globe pipeline

### 3.1 Geometry: unindexed corner-fetch draw

Why: crisp per-cell categorical layers need each fragment to know the
triangle's three corner cell ids and positions (code map b§7, pinned). WGSL has
no barycentric builtin, `@interpolate(flat)` alone paints whole triangles with
the provoking vertex (wrong shapes), and `@builtin(primitive_index)` is an
optional wgpu native feature with uneven backend support — not something to
hinge the design on. The dependency-free solution:

- **No vertex buffers at all** (`buffers: &[]`, like `vs_flat`). Draw call:
  `draw(0..3 * tri_count, 0..1)`.
- The vertex shader derives `tri = vi / 3`, `corner = vi % 3`, reads the
  corner ids from a `tri_ids: array<u32>` storage buffer (the existing
  `grid.triangles` bytes, bound as storage instead of an index buffer) and the
  position from `positions: array<f32>` (the existing `grid.positions` bytes;
  declared `array<f32>` and indexed `3c, 3c+1, 3c+2` to avoid WGSL's 16-byte
  `vec3` stride — zero repacking, buffers upload exactly as today's
  render.rs:341–359, just with `STORAGE` usage).
- Each of a triangle's three vertices outputs the **same** `vec3<u32>` of
  corner ids (flat interpolation is therefore provoking-vertex-safe) and a
  one-hot barycentric.

```wgsl
struct GlobeVsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal_cam: vec3<f32>,             // camera-space, as today
    @location(1) bary: vec3<f32>,                   // one-hot at corners
    @location(2) @interpolate(flat) cids: vec3<u32>,// the tri's 3 cell ids
}
```

`vs_globe`: identical transform to today (shaders.wgsl:29–37) with the
position fetched from storage. The rasterizer interpolates `bary` linearly
(orthographic, w = 1 — perspective-correct interpolation degenerates to
linear, so no `@interpolate(linear)` juggling is needed).

**"Rasterizer interpolates the value":** the fragment computes
`scalar = dot(bary, vec3(s0, s1, s2))` from the interpolated barycentrics and
the three per-corner scalars fetched from `cell_values`. This is arithmetically
identical to emitting the scalar as a per-vertex varying and letting the
rasterizer interpolate it — the rasterizer *is* interpolating the value,
expressed through `bary` — and the triple formulation is required anyway for
masked interpolation (§3.4) and slope (§5.3). The pinned "upload per-cell
scalar values, rasterizer interpolates the value, palette per fragment" is
satisfied; only the plumbing differs, and the design says so explicitly.

Cost: unindexed drawing loses vertex reuse (≈6× vertex invocations: L8 3.9M,
L9 15.7M per globe draw). The vertex shader is a fetch + one mat4 multiply.
Budget-gated on the PC (RTX 3080: trivial); on the Air this is the main Ultra9
fps risk — recorded by the perf script (§10), not gated (CLAUDE.md machine
note). Memory is unchanged versus today: the same positions (L9 31.5 MB) and
triangles (L9 62.9 MB) bytes, rebound as storage.

### 3.2 Globe bind group layout (replaces render.rs:168–192)

| binding | resource | visibility |
|---|---|---|
| 0 | `GlobeUniforms` (uniform, §6) | VERTEX_FRAGMENT |
| 1 | `positions: array<f32>` (RO storage) | VERTEX_FRAGMENT |
| 2 | `tri_ids: array<u32>` (RO storage) | VERTEX |
| 3 | `cell_values: array<vec2<u32>>` (RO storage) | FRAGMENT |
| 4 | `overlay: array<u32>` (RO storage) | FRAGMENT |
| 5 | `palette: texture_2d<f32>` | FRAGMENT |

This resolves the code map's VERTEX-only-visibility blocker (render.rs:183)
by construction. Fragment stage uses 3 storage buffers, vertex 2 — well under
the default limit of 8 per stage; if a backend ever reports downlevel limits
(4), positions+tri_ids concatenate into one buffer with a base-offset uniform
(documented fallback, not the default).

### 3.3 Fragment resolution — scalar, categorical, debug boundaries

`fs_globe`:

1. Sphere position: `p = normalize(rot3ᵀ · in.normal_cam)` where `rot3` is the
   upper-left 3×3 of `globe_u.rot` (orthonormal — transpose is the inverse;
   CPU precedent `rotate_inv`, render.rs:117). No extra varying needed.
2. Fetch the three corners: `Pk = pos3(cids[k])`, `Vk = cell_values[cids[k]]`.
3. Winner: `k_win = argmax_k dot(p, Pk)`, ties to the lowest k — same
   argmax-of-dot semantics as `Grid::nearest_cell` (grid.rs:109). Because every
   icosphere triangle is acute, the Voronoi cell of each vertex is contained in
   that vertex's triangle fan, so within a triangle the *global* nearest cell
   center is always one of its three corners: nearest-of-3 is exact, not an
   approximation.
4. Call the shared `resolve_fragment(cids, bary, p, k_win, ...)` (§5.1).

**Categorical layers** (Plates; the continent/ocean split of CrustAge): color
comes from the winner cell only — LUT row 4 at
`rank(win) + select(32, 0, continent(win))`, giving crisp true-cell-shaped
plates. Legacy one-cell boundary bands: if the `debug_bands` flag bit is set,
a nonzero boundary code on the winner overrides with LUT row 5 colors —
exactly today's look (layers.rs:216–222), debug-only (pinned).

**Scalar layers**: `scalar = dot(bary, s)` (masked for CrustAge, §3.4);
Elevation continues into sea-level threshold + detail (§5); CrustAge/Thickness
read their ramp rows directly.

**Debug true-cell-boundary toggle** (pinned "same machinery"): let
`m = dot(p, P_win) − dot(p, P_second)` (best minus second-best). Draw an
anti-aliased line where `m < 1.5 · fwidth(m)`, mixed 60% toward dark grey.
The identical expression renders the **overlay outline** (§7.4) and is shared
with the flat canvas via `resolve_fragment`.

### 3.4 Masked interpolation for CrustAge

Continent cells have no meaningful ocean age; naive interpolation would smear
grey halos into young ocean. Rule (deterministic, shared fn):

```
if continent(win): color = AGE_CONTINENT (LUT row 5, code 3)      // crisp
else:
    wk' = bary_k * select(0.0, 1.0, !continent(k))                 // mask
    t   = dot(w', s) / max(sum(w'), 1e-6)                          // renormalize
    color = viridis LUT row 2 at t
```

Elevation and Thickness interpolate unmasked (valid everywhere). Plates never
interpolates.

---

## 4. Flat canvas — the assigned lookup-extension mechanism

### 4.1 The scheme: hint raster + exact per-fragment Voronoi refinement

The 4096×2048 R32Uint raster is **kept, unchanged in size and construction**
(`rasterize_cell_ids`, render.rs:43–61) — but demoted to a *walk hint*. Per
fragment:

1. **Invert** the projection analytically, exactly as today
   (`map_invert`, shaders.wgsl:79; Eckert IV arm added, §9). Outside the
   outline → background (LUT row 5 code 7).
2. **Unit vector** `p` from lat/lon (`cos/sin`, as `rasterize_cell_ids` does
   CPU-side).
3. **Hint**: `c = textureLoad(cell_ids, texel(lat, lon)).r` — today's lookup,
   shaders.wgsl:135–139.
4. **Greedy walk to the exact winner** (GPU mirror of `Grid::nearest_cell`,
   grid.rs:109): up to `WALK_CAP = 4` iterations; per iteration scan the
   CSR neighbors of `c` in ring order and move to the first strictly-better
   `dot(p, pos)`; stop when no neighbor beats the current cell. Strict `>`
   with first-in-ring-order winning ties mirrors the CPU walk's tie behavior,
   so **GPU winner == `Grid::nearest_cell` result by construction** — the
   pinned "one true position→cell mapping" now holds per fragment, and cursor
   readout / brushes (which use the CPU path, app.rs:650–652) agree with the
   pixels exactly.
5. **Wedge**: the containing icosphere triangle has the winner as a corner
   (acute-fan argument, §3.3), so its other two corners are **consecutive
   neighbors in the winner's CCW ring** (grid.rs CSR is CCW-ordered). Scan the
   ring's ≤6 wedges `(c, n_i, n_{i+1 mod k})` and pick the first containing
   `p`: inside iff `dot(p, cross(pos_c, pos_ni)) ≥ 0 ∧ dot(p, cross(pos_ni1, pos_c)) ≤ 0`
   (CCW orientation; first match wins — deterministic).
6. **Weights**: solve the 3×3 system `[P_c P_a P_b]·β = p` (closed-form via
   cross products / scalar triple products — no matrix inversion routine
   needed), then `w = β / (β.x + β.y + β.z)`. These are exactly the planar
   barycentrics of the chord-plane point the globe rasterizer would interpolate
   for the same ground position (the ray-plane scale folds out in the
   normalization), so flat interpolation is **numerically equivalent to the
   globe's**, not merely similar.
7. Call the **same** `resolve_fragment(cids = (c, a, b), w, p, k_win = 0, ...)`
   as the globe — palette, sea level, detail, masked interpolation, debug
   boundaries, overlay tint + outline: one code path.

### 4.2 Why the hint always converges within the cap

The hint cell is the nearest cell to the *texel center*; the fragment's true
point is at most half a texel diagonal away. Texel size is 0.088°; cell
spacing is ≈0.49° (L7) / 0.248° (L8) / 0.124° (L9). Worst case (L9) the hint
is ≤ ~1.5 cell spacings from the truth → the walk needs ≤2 moves; `WALK_CAP=4`
is a 2× margin. Near the poles equirect texels shrink in longitude, so hint
error only decreases. If a future level ever exceeded the cap, the failure
mode is a one-cell-off winner for a sliver of fragments — visible, not UB —
and the cap is a named constant to bump.

### 4.3 What this fixes, explicitly (the code-map risks)

- **>1 id per texel**: not needed. The texel stores one *hint*; the fragment
  derives the true candidate triple itself from CSR + positions. The candidate
  set is exact for every fragment, not quantized per texel.
- **~0.088°/texel zoom cap**: gone. At any zoom (clamp 0.5–80 kept,
  app.rs:631) the image is the analytic projection of the true Voronoi
  diagram + smooth value interpolation; the raster's footprint never reaches
  the screen. Cell edges are as crisp at zoom 80 as the globe's.
- **Memory at L8/L9**: the flat pipeline binds the *same* buffers the globe
  already uploads (positions, values, overlay) plus CSR (offsets + neighbors).
  Totals (GPU): L8 ≈ 7.9 + 5.2 + 2.6 + 18.4 = 34 MB (+15.7 MB tri_ids for the
  globe, +33.6 MB hint raster); L9 ≈ 31.5 + 21.0 + 10.5 + 73.4 = 136 MB
  (+62.9 + 33.6). Worst case ≈ 235 MB at L9 — fine on the RTX 3080 and within
  the Air's unified memory. Contrast: a k=3-ids+weights raster at the
  resolution needed to beat the zoom cap (≥16384×8192×16 B) would cost 2 GB
  and *still* be resolution-capped.
- **Seams at poles/antimeridian**: today's seam artifacts are raster
  quantization; the walk erases them (the hint may be slightly off across the
  seam, the refinement is seam-agnostic — `p` is a unit vector, the sphere has
  no seam). The equirect texel clamp at `x` edges (shaders.wgsl:137–138) stays
  as-is: a clamped hint is still a valid hint.
- **Rebuild cost on grid-level change**: identical to today — the same one
  raster per level (`rebuild_grid`, app.rs:285), plus uploading CSR
  (memcpy-sized; L9 ≈ 73 MB one-time per level). No new CPU rasterization.

### 4.4 Why this beats rasterizing the projected mesh

1. **No projection-dependent geometry.** A projected mesh must be forward-
   projected, split along the antimeridian, re-split when the projection
   changes, and its curved outline (Robinson, Eckert IV) approximated at the
   map edge. The single-triangle + analytic-inverse architecture keeps *all*
   of that as ~15 lines of WGSL per projection; adding Eckert IV touches one
   function (§9). With mesh rasterization, Eckert IV means new clipping code.
2. **Exact canvas equivalence.** Same candidate triple, same weights, same
   `resolve_fragment` — the two canvases cannot drift apart. A projected mesh
   has its own rasterizer interpolation but needs duplicated seam vertices
   whose values must be stitched; every seam is a bug surface.
3. **No second geometry copy.** The mesh approach uploads a second (projected,
   split) vertex/index set per projection state; this design reuses the
   globe's buffers plus CSR that is a verbatim copy of `Grid`'s arrays.
4. **The pinned cell-pick invariant holds by construction** ("same ground
   position resolves to same cell as other projections"): winner selection is
   a mirror of `Grid::nearest_cell`, per fragment, for every projection.
5. **Cost honesty:** the price is fragment work (walk ≤ 24 dot products worst
   case, wedge ≤ 6 tests, one 3×3 solve; neighboring fragments touch the same
   cells so loads are cache-coherent). Flat is currently the Air's slowest
   view (55.6 fps at L7); the perf script records the regression at
   L7/L8/L9 (§10). Budgets gate on the PC only.

### 4.5 CPU-side validation of the walk

A `#[cfg(test)]` module in render.rs adds `refine_from_hint(grid, raster, p)`
— a small Rust mirror of steps 3–5 — and property-tests it at L6/L7/L8: for a
few thousand deterministic sample points (PCG from a fixed literal seed),
`refine_from_hint == grid.nearest_cell(p, None)` and the walk converges within
`WALK_CAP`. This is the testable statement of §4.2's geometry argument.

### 4.6 Flat bind group layout (replaces render.rs:194–228)

| binding | resource | visibility |
|---|---|---|
| 0 | `FlatUniforms` (uniform, §6) | FRAGMENT |
| 1 | `positions: array<f32>` | FRAGMENT |
| 2 | `nbr_offsets: array<u32>` (CSR offsets) | FRAGMENT |
| 3 | `nbrs: array<u32>` (CSR lists) | FRAGMENT |
| 4 | `cell_values: array<vec2<u32>>` | FRAGMENT |
| 5 | `overlay: array<u32>` | FRAGMENT |
| 6 | `cell_ids: texture_2d<u32>` (hint) | FRAGMENT |
| 7 | `palette: texture_2d<f32>` | FRAGMENT |

Five fragment-stage storage buffers — within the default limit of 8; the
downlevel fallback is concatenating the two CSR arrays into one buffer.

---

## 5. Shared WGSL: render detail, shading, one source of truth

### 5.1 `resolve_fragment` — the one shading function

```wgsl
// Everything after "which cells, what weights" is canvas-independent.
// Called by fs_globe and fs_flat with identical semantics.
fn resolve_fragment(
    cids: vec3<u32>,   // candidate cells (globe: tri corners; flat: wedge)
    w: vec3<f32>,      // normalized chord-plane barycentrics, sum 1
    p: vec3<f32>,      // unit sphere position of the fragment
    sp: ShadeParams,   // §6
) -> vec3<f32>
```

Internals: per-corner fetches (`cell_values`, `positions`, `overlay` — all
bound identically in both groups), winner pick, layer dispatch on
`sp.layer_flags`, detail noise, sea-level threshold, LUT reads, debug lines,
overlay tint + outline. `fwidth` is used inside (legal in any function called
from a fragment entry point). `fs_globe` multiplies the result by its Lambert
term afterwards (unchanged, shaders.wgsl:41–42); `fs_flat` applies the
graticule mix afterwards (unchanged, shaders.wgsl:142–155).

Because both entry points live in the single `shaders.wgsl` module compiled
once (render.rs:163), "identical noise + palette functions shared verbatim"
is enforced by the compiler, not by review discipline.

### 5.2 Deterministic 3D noise on the sphere

Value-noise fBm over an integer lattice, all-integer hashing, **independent
implementation** from the sim's crate-private `fbm` (noise_stage.rs:63 —
`pub(crate)`, unlinkable from the app by design; different hash constants so
render detail can never accidentally correlate with world data):

```wgsl
fn hash3(c: vec3<i32>, seed: vec2<u32>) -> u32 {
    var h = seed.x;
    h = (h ^ bitcast<u32>(c.x)) * 0x9E3779B1u;
    h = (h ^ bitcast<u32>(c.y)) * 0x85EBCA77u;
    h = (h ^ bitcast<u32>(c.z)) * 0xC2B2AE3Du;
    h = h ^ seed.y;
    h = h ^ (h >> 16u); h = h * 0x7FEB352Du;
    h = h ^ (h >> 15u); h = h * 0x846CA68Bu;
    return h ^ (h >> 16u);
}
// corner value in [-1, 1), exact: top 24 bits -> f32
fn corner_val(c: vec3<i32>, seed: vec2<u32>) -> f32 {
    return f32(hash3(c, seed) >> 8u) * (2.0 / 16777216.0) - 1.0;
}
fn vnoise(q: vec3<f32>, seed: vec2<u32>) -> f32   // trilinear, quintic fade
fn render_fbm(p: vec3<f32>, freq0: f32, octaves: u32, seed: vec2<u32>) -> f32
    // sum gain 0.5, lacunarity 2.0, normalized to [-1, 1]
```

- **Seed**: the u64 master seed enters as `seed_lo`/`seed_hi` u32 uniform
  lanes (§6) — never through f32 (pinned; code map b§8). Both canvases receive
  the same lanes, so the noise field is identical on globe and flat at the
  same ground point.
- **Domain**: `q = p * freq0 * 2^octave`, `p` the unit sphere position — a
  true 3D field, so no projection seams and no pole pinching, on either canvas.
- **Base frequency**: `freq0 = DETAIL_FREQ_CELLS / θ_cell`, with
  `θ_cell = sqrt(4π / cell_count)` computed CPU-side into the uniform
  (`detail_cell_rad`), `DETAIL_FREQ_CELLS = 0.35` (first octave wavelength
  ≈ 3 cell spacings, bridging grid scale into sub-cell scale). Lattice
  coordinates stay < ~10⁴ at L9 with 6 octaves — comfortably inside exact
  i32/f32 territory.
- **Determinism scope**: integer hashing is bit-exact everywhere; the float
  lerp chain is deterministic for a given GPU but not contractually
  bit-identical across GPU vendors. That is acceptable and stated: render
  detail is display-only, guarded out of every golden by §11.

### 5.3 Slope per fragment, amplitude conditioning, slider semantics

- **Slope** (defined; pinned "how slope is obtained per fragment"): from the
  candidate triple already in hand —
  `slope_m = max(s0, s1, s2) − min(s0, s1, s2)` (meters across one cell
  spacing). No extra fetches; identical formula on both canvases because both
  hold a candidate triple.
- **Amplitude** (meters), Elevation layer only:

```
e_base   = dot(w, s) − sea_level_m
depth_t  = smoothstep(-1500.0, -100.0, e_base)         // abyss fades detail
amp      = detail_amp_m                                 // = slider t × A0 (§12)
         · mix(0.30, 1.0, clamp(slope_m / 800.0, 0, 1)) // calm plains, rough relief
         · mix(0.25, 1.0, depth_t)                      // full strength near coasts
e_render = e_base + amp · render_fbm(p, freq0, sp.octaves, seed)
```

  The depth fade is smooth through e = 0, so coastline fragments on both sides
  of the isoline get equal noise — that is what makes the sea-level threshold
  on `e_render` produce **fractal coastlines** (the pinned effect). The
  0.30/0.25 floors, 800 m and (−1500, −100) band are sweep-adjustable
  constants (§12); `A0` and octaves are the sweep's outputs.
- **Threshold + palette**: `e_render ≤ 0` → ocean LUT row 0 at
  `sqrt`-free coordinate `−e_render/6000` (the sqrt lives in the baked row,
  §2.3); else land row 1 at `e_render/5500`.
- **Detail slider**: app field `detail: f32 ∈ [0, 1]`, default 1.0.
  `detail_amp_m = detail × A0` where `A0` is the sweep-chosen default
  amplitude — slider 0 = off (uniform amp 0; the shader skips the fBm loop
  when `amp == 0.0`, making "off" exactly the no-noise image), slider 1 =
  the tuned default (pinned "off→full"). Octave count is fixed at the sweep's
  choice; the slider scales amplitude only (one knob, monotone, no popping).
- Detail applies to the Elevation layer only (categorical layers have no
  elevation semantics; age/thickness ramps portray data, not terrain).

---

## 6. Uniforms: live view controls, seed lanes

One shared sub-struct embedded in both uniform blocks (offsets 16-aligned in
both, satisfying WGSL uniform layout rules):

```wgsl
struct ShadeParams {            // 32 bytes
    seed_lo: u32,               // master seed low 32 bits
    seed_hi: u32,               // master seed high 32 bits
    layer_flags: u32,           // bits 0..3 layer id (0 elev, 1 plates,
                                // 2 age, 3 thickness); bit 8 debug true-cell
                                // boundaries; bit 9 debug legacy bands;
                                // bits 10.. reserved
    octaves: u32,               // render-detail octaves (sweep default)
    sea_level_m: f32,           // UI slider, live
    detail_amp_m: f32,          // slider t × A0, live
    detail_cell_rad: f32,       // sqrt(4π / cell_count), set per grid level
    _pad: f32,
}
struct GlobeUniforms { rot: mat4x4<f32>, params: vec4<f32>, shade: ShadeParams }  // 112 B
struct FlatUniforms  { center_px: vec2<f32>, half_px: vec2<f32>,
                       misc: vec4<f32>, tex: vec4<f32>, shade: ShadeParams }      // 80 B
```

Rust mirrors are `#[repr(C)]` bytemuck Pod structs with the same field order
(u32 lanes for the seed — `(seed & 0xffff_ffff) as u32`, `(seed >> 32) as u32`;
no f32 round-trip anywhere). Both `prepare()`s already write the whole uniform
every frame (render.rs:486–495, 557–574), so sea level, Detail, layer id and
debug flags become pure uniform writes — a slider drag costs one 112/80-byte
`write_buffer` instead of today's full L8 rebake (app.rs:746). `misc` keeps
its current meaning (`misc.x` gains value 2.0 for Eckert IV; `misc.z`
graticule; `misc.y/w` stay spare).

---

## 7. The frozen A↔C overlay interface

### 7.1 The frozen artifacts (types survive C's rewrite)

The interface is deliberately typed over things Fix 3 does **not** rewrite:
`worldmaker_core::Grid`, plain `&mut [u32]`, and the serde `Stroke` type in
worldmaker-io (pinned home; worldmaker-app already depends on worldmaker-io).
It is **not** typed over colors, `WorldBundle`, `BakeOverlay`, `Layer`, or any
render type — resolving the code map's biggest flagged risk (b§Contradictions 1).

**Frozen `Stroke` shape** (worldmaker-io, new `strokes.rs` beside the save
stubs; matches the Fix 1 pinned payloads exactly):

```rust
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Stroke {
    /// Craton brush stroke: cell ids at the current grid level; sign = +1
    /// (paint continent) or −1 (force ocean).
    CratonPaint { cells: Vec<u32>, sign: i8 },
    /// Hotspot added at a unit-vector position.
    HotspotAdd { pos: [f32; 3] },
    /// Hotspot removal targeting a unit-vector position.
    HotspotRemove { pos: [f32; 3] },
}
```

Per the order, C ships the interface (including this type skeleton and a
no-op default) so the frozen signature compiles on C's branch; the type's
*shape* is frozen here, its behavior and all impls are Track A's. One
decision-log line records this shape-only exception to the file partition.

**Frozen function** (module `crates/worldmaker-app/src/pending_edits.rs` —
A's module; C's branch creates it with the constants + a no-op body):

```rust
/// Overlay word bit layout (frozen):
///   bits 0..=2  tint code: 0 none, 1 craton +1, 2 craton −1,
///               3 hotspot marker, 4 pending hotspot add,
///               5 pending hotspot remove, 6..7 reserved
///   bit  3      force-outline (outline this cell's region edge even where
///               the neighbor has the same tint code)
///   bits 8..=31 reserved (zero)
pub const OVERLAY_TINT_MASK: u32 = 0b111;
pub const OVERLAY_FORCE_OUTLINE: u32 = 1 << 3;

/// Fill `overlay` (len == grid.cell_count(), pre-zeroed by the caller) from
/// the pending strokes, newest stroke winning per cell, plus hotspot marker
/// positions to display. Pure function of its arguments: no tool state, no
/// history, no sim types — callable mid-run and from any future tool.
pub fn apply_overlay(
    pending: &[worldmaker_io::Stroke],
    hotspot_markers: &[[f32; 3]],
    grid: &worldmaker_core::Grid,
    overlay: &mut [u32],
);
```

Pinned semantics C's compositor relies on (A implements): cell ids ≥
`overlay.len()` are skipped silently (stale ids across a level switch can
never panic); hotspot positions resolve to marker cells via
`grid.nearest_cell` + `neighbors_of` ring (today's shape, app.rs:416–421);
determinism is A's concern (iteration in stroke order).

### 7.2 Why this satisfies every pinned property

- **Takes the pending set explicitly** — no `self.tool`, no `history`
  (resolves code-map friction a§10.3: overlay display today is tool- and
  history-gated).
- **Works mid-run**: rebake's overlay pass runs even when `history` is None
  (§2.4 step 3), and the bundle retains the last values Arc, so pending
  strokes render "with current world behind" during a job.
- **Survives C's rewrite**: no argument or constant mentions colors or any
  type C changes. A codes against it from day one; C's no-op default
  (`fn apply_overlay(..) { let _ = (pending, hotspot_markers, grid, overlay); }`)
  keeps rebake wired and rendering exactly nothing until A's rebase.

### 7.3 GPU side

`overlay: array<u32>` per cell, bound FRAGMENT in both pipelines (§3.2, §4.6),
uploaded on `overlay_gen` bumps via the existing write-or-recreate pattern
(render.rs:402–416). 4 B/cell: L8 2.6 MB, L9 10.5 MB.

### 7.4 Compositing: tint + outline per fragment (both canvases)

In `resolve_fragment`, after the base color:

- `code = overlay[cids[k_win]] & OVERLAY_TINT_MASK`; if nonzero, tint:
  `color = mix(color, tint_color(code), 0.45)` with tint colors from LUT row 5
  (codes 1→PAINT_CONTINENT, 2→PAINT_OCEAN, 3/4→HOTSPOT_MARK, 5→HOTSPOT_MARK
  darkened ×0.55) — single Rust source of truth for overlay colors too.
- **Outline** ("tint + outline" is unexpressible today — layers.rs overlay is
  opaque replacement; this is the new mechanism): for each non-winner
  candidate k, if `overlay_word(k) != overlay_word(win)` (or either has
  FORCE_OUTLINE), compute the bisector margin
  `m_k = dot(p, P_win) − dot(p, P_k)` and draw
  `line = 1 − smoothstep(0, 1.8 · fwidth(m_k), m_k)` in white mixed at 0.9 —
  a crisp ~1.8-px outline lying exactly on the true Voronoi edge of the
  stroke's region, on both canvases, derived purely from per-cell words. This
  is the same margin machinery as the debug cell-boundary toggle (§3.3):
  one mechanism, three uses (debug boundaries, overlay outlines, and — see
  §8 — nothing else; plate boundaries get real polylines).

---

## 8. Plate-boundary smoothed polylines (both canvases)

### 8.1 CPU chain extraction (app crate, new module `boundaries.rs`, C-owned)

From the viewed keyframe's `plate_id` + grid CSR + `triangles` — no sim
changes, nothing shared with B's metrics code (B owns sim-side structures;
this is render-only and lives in the app):

1. **Nodes**: triangles with ≥2 distinct corner plate ids ("mixed"); node
   point = normalized triangle centroid (≈ Voronoi vertex of the cell
   diagram). Triangles with 3 distinct ids are **junction** nodes.
2. **Links**: each Delaunay edge (a, b), a < b, with `plate_id[a] ≠ plate_id[b]`
   links its two flanking mixed triangles (the boundary crosses that edge).
   Enumerate edges in (a, b) id order — deterministic.
3. **Chains**: walk links, starting first from junction nodes in ascending
   triangle-index order, then remaining closed loops from their lowest
   triangle index. Serial, id-ordered — deterministic (and display-only
   regardless).
4. **Type**: each link inherits a boundary type from its generating edge's
   two cells' `F_BND_*` bits with the existing priority (convergent >
   divergent > transform, layers.rs:216–222); chains split where the type
   changes, so each chain has one color.
5. **Chaikin**: 2 corner-cutting iterations on the sphere (each new point
   normalized back to the unit sphere); junction endpoints pinned. Output:
   `BoundarySet { chains: Vec<BoundaryChain { btype: u8, pts: Vec<[f32; 3]> }> }`.

Rebuilt in rebake's values pass when the layer is Plates (and only then —
boundaries are Plates-layer styling, as the bands are today). Cost: boundary
cells are O(√n); at L8 a few thousand segments ×4 after Chaikin — sub-ms.

### 8.2 Globe drawing

New small pipeline `boundary_globe`: vertex buffer of ribbon vertices
`{ a: [f32;3], b: [f32;3], side: f32, btype: f32 }` (stride 32; two verts per
polyline point, 6 indices per segment). VS transforms `a` and `b` by
`globe_u.rot`, projects to NDC, offsets by
`side · half_width_px · normalize(perp(b_ndc − a_ndc))` in pixel space
(width uniform, ~1.75 pt × ppp); discards when the segment midpoint's
camera-space z < 0.02 (back hemisphere; no depth buffer exists — draw order
after the globe fill in the same egui render pass handles occlusion,
matching the current no-depth design, render.rs:270). FS: flat color by
`btype` from LUT row 5 with fwidth-AA edges; `BlendState::ALPHA` for this
pipeline only.

### 8.3 Flat drawing

Chains are projected CPU-side in `FlatCallback::prepare` **only when the
projection or the boundary set changed** (cached by `(values_gen, projection)`):
`Projection::project(lat, lon)` per point (forward projection's first use in
the app render path — it exists and is tested, proj.rs:36).
**Antimeridian**: split any segment whose endpoint longitudes differ by > π:
compute the crossing latitude by great-circle interpolation of the two unit
vectors to lon = ±π, emit `…→(x_edge(lat), y)` and `(−x_edge(lat), y)→…`,
where `x_edge` is the projected map-edge x at that latitude (projection-
correct for Robinson/Eckert IV's curved edges). Same ribbon expansion in a
`boundary_flat` VS using `center_px`/`half_px`. Pole caps need no special
casing (chains never pass through the exact pole point; a chain touching the
pole cell renders its centroid points normally).

### 8.4 Bands demoted

The one-cell F_BND bands remain computable from the category word (bits 8..9,
§2.2) and render only when `layer_flags` bit 9 (debug legacy bands) is set —
pinned "old one-cell bands only under debug toggle". The debug UI is two
checkboxes in a "Debug" section of C's top bar: "True cell boundaries"
(bit 8), "Legacy boundary bands" (bit 9).

---

## 9. Eckert IV

### 9.1 Core (worldmaker-core proj.rs — display path, std trig allowed; the
dmath rule binds committed sim metrics only, per code map c§dmath)

Add `Projection::EckertIv` to the enum, `ALL` (len 3), `name()` ("Eckert IV"),
`aspect() = 2.0` (exact: x_range/y_range = [4π/√(π(4+π))]/[2√(π/(4+π))] = 2).

Normalized forward (Snyder, unit sphere, pinned):
```
solve  θ + sinθcosθ + 2sinθ = (2 + π/2)·sinφ
  Newton from θ₀ = φ/2:  θ ← θ − (f(θ) − c) / (2cosθ(1 + cosθ))
  ITER_CAP = 8, TOL = 1e-7 rad (early-out when |Δθ| ≤ TOL; deterministic —
  same input ⇒ same iterate sequence); |sinφ| ≥ 1 − 1e-7 short-circuits to
  θ = sign(φ)·π/2 (derivative → 0 at the pole).
x_n = λ(1 + cosθ) / (2π)          // λ=π, θ=0 ⇒ 1; pole line = half equator
y_n = sinθ
```

Inverse (closed form, pinned):
```
θ = asin(clamp(y, −1, 1))          // reject |y| > 1 first
φ = asin(clamp((θ + sinθcosθ + 2sinθ) / (2 + π/2), −1, 1))
λ = 2π·x / (1 + cosθ)
reject |λ| > π·1.0001, then clamp to ±π    // same hair-tolerance pattern as
                                            // Robinson (proj.rs:62–70)
```

Tests in core (mapping_tests.rs per the test-inventory precedent): join the
existing `dense_latlon_roundtrip` / anchor / rejection loops (Newton TOL 1e-7
sits far inside the suite's 1e-4 tolerance); plus Eckert-specific anchors
(pole line x = ±0.5 at |lat| = 90°, equator y = 0, antimeridian accepted).

### 9.2 WGSL (`misc.x = 2.0`)

`map_invert` (shaders.wgsl:79) gains a third arm implementing exactly the
closed-form inverse above with the same rejection order, the same `1.0001`
tolerance and the same clamps — the equirect strict-gate discipline
(shaders.wgsl:84–86) applied to Eckert IV, so shader pixels and the CPU
cursor readout/brush picks agree on domain membership per pixel. Note the
pinned "same cap/tolerance in WGSL" concern lands here as: **the Newton
iteration exists only on the CPU forward path** (used by brushes' inverse?
no — by boundary projection §8.3 and future exports); the *inverse* both
sides share is closed-form, so cap/tolerance matching reduces to matching the
clamps and the 1.0001 gate, which this design pins. `FlatCallback::prepare`'s
projection match (render.rs:564–567) adds `EckertIv => 2.0`. Graticule works
free (computed from inverted lat/lon, shaders.wgsl:142); cursor readout and
brushes work through the existing `self.projection.invert` path (app.rs:649)
the moment the enum variant exists; `flat_base_half_extents` needs nothing
(aspect() covers it).

---

## 10. Presets, flags, perf script

### 10.1 Presets

`enum Preset { Draft6, Standard7, High8, Ultra9 }`; `level()` → 6/7/8/9;
labels gain "Ultra (L9, 2.6M cells)". **Default becomes `High8`**
(app.rs:220). The combo (app.rs:722–733) lists all four. L9 keyframe cadence
and its doc comment are **B's** (pinned); C only enables the preset. Budget
note recorded with the change: at the current 20 My cadence a 500 My L9 run
holds ≈ 26 × 42 MB ≈ 1.1 GB of keyframes and an L9 sim wall time in the
minutes range on the Air — acceptable for perf runs, and it shrinks if B picks
a sparser L9 cadence. C must not touch `keyframe_interval_my` or harness.rs.

### 10.2 CLI flags (`parse_args`, main.rs:29)

```
--seed <text>        seed exactly as typed in the seed box (seed_from_text) —
                     identical hashing keeps script/UI parity
--preset <draft6|standard7|high8|ultra9>   (case-insensitive)
--detail <0..1>      Detail slider value
--detail-octaves <n> --detail-amp-m <x>    dev-only sweep overrides (§12)
```

Slotted into the existing `while let` match; carried in `Script` and applied
in `WorldApp::new` **before** `rebuild_grid`. Because unknown args are
warn-and-ignore today (main.rs:43), the scripts that use the new flags first
assert the running binary supports them (e.g. `--flag-check` … simplest: the
capture scripts are updated in the same commit as the flags, and the work
order notes old binaries silently ignore them — code-map risk d§7
acknowledged and mitigated by same-commit delivery).

### 10.3 Perf script preset loop

`ScriptState::Perf` gains a preset index over `[Standard7, High8, Ultra9]`
(pinned L7/L8/L9). Per preset: switch preset (`rebuild_grid` + full sim, as
the UI would), wait for history (existing `drive_script` wait), then per view
Globe→Flat→Split: 40 warmup + 240 sampled frames (unchanged), **with
`layer = Elevation`, Detail = default (1.0 × sweep A0), smoothed boundaries
off (elevation layer), vsync off** (pinned "smooth shading + render detail
on"). Metrics per preset: `globe_fps_L{7,8,9}`, `flat_fps_L{7,8,9}`,
`split_fps_L{7,8,9}`, plus `sim_wall_s_L{7,8,9}_500my` (measured while
waiting — free, useful for the L9 budget conversation), `detail` (slider),
`fps_vsync_off: true`, `grid_build_ms_L6..9` (unchanged). The old
`fps_grid_level` key is retired (superseded by the suffixed keys). Output:
`docs/results/perf-feelpass-Daniels-MacBook-Air.json` via `ResultsFile`
(schema d§4; phase-1 files untouched).

### 10.4 Screenshot parity

The BEFORE set (commit 8381450) used seed "cyrus", **Standard7**, 500 My.
Since the default preset changes to High8, the AFTER capture must pass
`--preset standard7 --seed cyrus` explicitly — recorded in the work-order
checklist so the comparison stays honest (code-map risk d§9 resolved).

---

## 11. Render-only guard, honestly testable

### 11.1 Shared world-building function

New module `crates/worldmaker-app/src/worldgen.rs` (C territory — app
plumbing):

```rust
/// The one world-building recipe. `start_job` calls this on its worker
/// thread; the guard test calls it headlessly. Render-only state (Detail,
/// sea level, layer, projection, debug flags) is structurally absent from
/// the signature — that absence IS the guard.
pub fn build_world(
    grid: Arc<Grid>,
    master_seed: u64,
    params: TectonicsParams,
    progress: Option<Arc<Progress>>,
) -> anyhow::Result<(WorldState, u64 /* params_hash */)>
```

Body: `WorldState::new` + `Pipeline::new` + `push(TectonicsStage::new(params))`
(capturing `stage.params_hash()` first) + `StageContext::new(master_seed)`
with optional progress + `run` — i.e., exactly `start_job`'s closure
(app.rs:325–335) moved out. `start_job` shrinks to channel plumbing around
`worldgen::build_world`, so "the same path Regenerate uses" is one function,
not a convention (resolves code-map risk d§Contradictions 5).

### 11.2 The guard test (`#[cfg(test)]` in worldgen.rs — binary crate rules,
d§2)

```
detail_and_sea_level_cannot_touch_the_world:
  grid = Grid::build(5)                    // fast; L5 ≈ 10k cells
  (w1, h1) = build_world(grid, 42, TectonicsParams::default(), None)
  u_lo = pack_shade_params(detail = 0.0, sea_level_m = −4000.0, seed = 42, …)
  u_hi = pack_shade_params(detail = 1.0, sea_level_m = +4000.0, seed = 42, …)
  assert u_lo != u_hi                      // the render path DID change
  (w2, h2) = build_world(grid, 42, TectonicsParams::default(), None)
  assert h1 == h2                                          // params_hash
  assert hash_f32_slice(ELEVATION_M) equal across w1/w2    // committed fields
  assert hash_u32_slice(PLATE_ID)   equal across w1/w2
  assert hash_u32_slice(CRUST_TYPE) equal across w1/w2
```

`pack_shade_params` is the *real* uniform-packing helper the two `prepare()`s
use (factored for this purpose), so the test demonstrates the Detail path
terminates in uniforms while the world path is a pure function of
(grid, seed, params). Rust has no call-graph reflection; this is the
strongest structural statement available, and the design says so.

### 11.3 "worldmaker-sim exposes no render-detail parameter"

Same test module, no sim-crate edits (respecting B's territory):

```
assert TectonicsParams::default() Debug output lists exactly the 7 known
field names (plate_count, land_fraction, tectonic_vigor, span_my,
hotspot_count, craton_overlay, hotspot_overlay) and matches none of
/detail|render|octav|noise|amp/i.
```

Debug derives print field names (mod.rs:73 `#[derive(Clone, Debug)]`), so a
render-shaped field cannot be added to sim params without failing this test.
Not to be confused with `SimState.noise_seed` (sim fBm, golden-feeding) —
untouched, unlinkable (`pub(crate)`), and unrelated to `ShadeParams.seed_*`.

---

## 12. Octave/amplitude sweep plan

- **Matrix**: octaves {3, 4, 5, 6} × A0 {120, 220, 350} m — 12 combos —
  × 2 fixed seeds ("cyrus" and "feelpass") = 24 runs, all at High8,
  Elevation layer, Detail 1.0, driven by:
  `worldmaker --screenshots out/o{O}_a{A}_{seed} --seed {seed} --preset high8
   --detail 1 --detail-octaves {O} --detail-amp-m {A}` from a small
  `scripts/detail-sweep.sh` (Air) reusing the existing screenshot stages —
  stage 4 (mountains close-up, zoom 1.6) and stage 1 (flat) are the judged
  crops; other stages are skipped in sweep mode to keep it under ~15 min.
- **Panel**: the script writes `index.md` with a 12×2-per-seed image table
  into the sweep dir; a judging session views it and scores each combo on:
  coastline fractality at default zoom, absence of visible cell facets,
  absence of "noise soup" on plains, mountain texture at zoom 1.6.
- **Pick + log**: the chosen (octaves, A0) become `DETAIL_DEFAULT_OCTAVES` /
  `DETAIL_DEFAULT_AMP_M` consts in app.rs; the decision, scores, and both
  seeds' winning crops go to decision-log.md and
  `docs/media/feel-pass/detail-sweep/` (winning crops + index only — not all
  24 runs; media budget). Sweep runs before the AFTER screenshot set so the
  AFTER set shows the tuned default.

---

## 13. Ownership, merge order, decision-log entries

- All changes above sit in C's pinned territory: render.rs, shaders.wgsl,
  layers.rs, main.rs, core proj.rs, and app.rs's WorldBundle plumbing/rebake/
  top-bar/preset/debug-toggle/Detail-slider surface, plus new C-created
  files `worldgen.rs`, `boundaries.rs`, `pending_edits.rs` (stub for A),
  `worldmaker-io/src/strokes.rs` (frozen shape for A). harness.rs untouched
  (B's); goldens untouched (render-only by §11; C merges after B and re-runs
  sim tests to prove hashes unmoved, per the pinned merge order B → C → A).
- Decision-log entries C adds: (1) reversal of "palettes baked on CPU" →
  per-fragment GPU palette with Rust-baked LUT; (2) frozen A↔C interface +
  Stroke shape shipped by C (shape-only partition exception); (3) default
  preset High8 + Ultra9 enabled (cadence deferred to B); (4) sweep-chosen
  detail defaults; (5) Eckert IV Newton cap/tolerance constants.

## 14. Residual risks (explicit)

1. **Air fps at Ultra9**: unindexed globe (15.7M VS invocations) + heavier
   flat fragment could dip below 55 fps on the Air at L9. Recorded, not
   gated (machine note); documented fallback: none needed for correctness —
   perf work would target the walk cap or an indexed scalar-only fast path
   later.
2. **Cross-GPU float identity of noise/interp**: not contractually
   bit-identical between vendors; display-only by §11's guard. Screenshot
   diffs across machines may differ at the pixel level — the acceptance
   comparisons are same-machine BEFORE/AFTER.
3. **Storage-buffer limits on exotic backends**: 5 fragment-stage storage
   buffers exceed downlevel-default 4; fallback (CSR concatenation) specified
   in §4.6 but not built by default.
4. **`--flag` support on old binaries** is warn-and-ignore; mitigated by
   same-commit script updates (§10.2), not eliminated.
5. **Walk cap** is proven adequate for L≤9 by §4.2's geometry argument and
   §4.5's test, but is an assumption for any future L>9 grid.
