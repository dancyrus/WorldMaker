# Stage U reader (b): render.rs / shaders.wgsl / layers.rs + WorldBundle/rebake

Code map as of commit 9d5d272. Line numbers refer to that state. No code changed.

Files: `crates/worldmaker-app/src/render.rs` (638 lines), `shaders.wgsl` (157),
`layers.rs` (315), plus `app.rs` (1222) for WorldBundle plumbing and `rebake()`,
and `crates/worldmaker-core/src/proj.rs` / `grid.rs` for the types they consume.

## 1. Palette baking today (CPU)

One function does all coloring:

```rust
pub fn bake(layer: Layer, kf: &Keyframe, sea_level_m: f32, overlay: &BakeOverlay) -> Vec<u32>
```
— `layers.rs:184`. Output is one packed RGBA8 `u32` per cell
(`pack` at layers.rs:40: little-endian `r | g<<8 | b<<16 | 0xff000000`; test
`pack_is_rgba8_little_endian` layers.rs:288). The body is a rayon
`into_par_iter().map(...).collect_into_vec` over cell ids (layers.rs:208–248) —
order-preserving, so deterministic despite rayon (and it is UI-only, never in a
golden path). Overlay decoration (craton tint, hotspot markers) is applied
**after** the parallel bake as serial per-cell overwrites (layers.rs:251–267).

The result becomes `WorldBundle.colors: Vec<u32>` (render.rs:29) and is uploaded
as a **read-only storage buffer** `"cell-colors"` in
`SceneResources::sync_world` (render.rs:399–416; reuses the buffer via
`queue.write_buffer` when the size matches and the grid didn't change,
render.rs:404).

Globe consumption: `vs_globe` (shaders.wgsl:29–37) does
`out.color = unpack4x8unorm(cell_colors[vi]).rgb` — **`vertex_index` == cell
id** — and the color is a plain `@location(1)` varying, so the rasterizer
Gouraud-interpolates *colors* (not values) across each triangle. `fs_globe`
(shaders.wgsl:40–44) only multiplies by a fixed-light Lambert term
(`0.72 + 0.28·max(dot(n,l),0)`). This is exactly the "palettes baked on CPU"
decision Fix 3 reverses.

Globe vertex layout: a single attribute, `Float32x3` position, `array_stride:
12` (render.rs:248–252). No per-vertex color attribute — color comes from the
storage buffer indexed by `vertex_index`.

## 2. Flat canvas pipeline, end to end

- **Geometry**: `vs_flat` (shaders.wgsl:116–123) emits one oversized triangle
  from `vertex_index` (draw call `draw(0..3, 0..1)`, render.rs:588); the
  canvas rect is enforced by egui's paint-callback viewport/scissor. No vertex
  buffer (`buffers: &[]`, render.rs:283).
- **Fragment → map coords**: `fs_flat` (shaders.wgsl:126–157) converts
  `@builtin(position)` framebuffer pixels to normalized map coords via
  `FlatUniforms.center_px`/`half_px` (computed CPU-side in
  `FlatCallback::prepare`, render.rs:543–576, from `flat_base_half_extents`
  render.rs:532 + pan/zoom/ppp).
- **Projection inverse is evaluated IN THE SHADER**: `map_invert(proj, mx, my)`
  (shaders.wgsl:79–109). `proj` is `misc.x` — a float switch: 0.0 =
  equirectangular, 1.0 = Robinson (set at render.rs:564–567). The Robinson
  5°-table is **duplicated** in WGSL as `ROB` (shaders.wgsl:65–73) with a
  must-match comment referencing `ROBINSON_TABLE` in core proj.rs:80. The
  equirect branch deliberately gates `|mx|>1` exactly like
  `Projection::invert` (core proj.rs:48–73) so shader pixels and the CPU
  cursor readout agree per-pixel (comment shaders.wgsl:84–86).
- **CPU side of projections**: core `Projection` (proj.rs:11) has
  `ALL: [Projection; 2]` (:17), `name()` (:19), `aspect()` (:27),
  `project(lat, lon) -> (f32, f32)` (:36), `invert(x, y) -> Option<(f32,
  f32)>` (:48); normalized coords in [-1,1]², y up. The CPU inverse is used
  only for cursor picking / brushes (`flat_canvas`, app.rs:643–655). Forward
  `project` is currently used nowhere in the app render path.
- **Cell resolution**: fragment lat/lon → equirect texel → `textureLoad` of the
  **R32Uint 4096×2048** cell-id texture (`CELL_ID_TEX_W/H`, render.rs:19–20;
  texture created render.rs:361–394; sampled with integer coords — no sampler
  object anywhere) → `flat_colors[cell]` (shaders.wgsl:135–140). So flat is
  hard nearest-cell, resolution-capped by the 4096×2048 raster regardless of
  grid level or zoom.
- **Raster construction**: `WorldBundle::rasterize_cell_ids(grid)`
  (render.rs:43–61), rayon row-parallel, each texel via
  `grid.nearest_cell(p, hint)` with a west-neighbor hint. Built once per grid
  level in `rebuild_grid` (app.rs:285), held as `Arc<Vec<u32>>` and shared
  across every rebake (render.rs:32).

## 3. Layers and their coloring (layers.rs)

`Layer` enum (layers.rs:14): `Elevation, Plates, CrustAge, Thickness`;
`Layer::ALL` (:23), `name()` (:29). Per-cell inputs come from
`worldmaker_sim::tectonics::Keyframe` (keyframe.rs:67): `elev_m: Vec<i16>`,
`plate_id: Vec<u16>`, `crust_age_my: Vec<u16>`, `thickness_ckm: Vec<u16>`,
`flags: Vec<u16>` (bit 15 = continent, `KF_CONTINENT_BIT` keyframe.rs:63;
feature bits 0..=7), plus `plates: Vec<PlateState>`.

- **Elevation** (layers.rs:214): `hypsometric(kf.elev_m[c] as f32 -
  sea_level_m)` — Phase 0 ramp ported from old WGSL (layers.rs:140–156).
- **Plates** (layers.rs:215–231): boundary cells first, priority
  convergent > divergent > transform, using flag masks re-exported from sim:
  `F_BND_DIVERGENT = 1<<5`, `F_BND_CONVERGENT = 1<<6`, `F_BND_TRANSFORM =
  1<<7` (tectonics/mod.rs:51–53). Colors: `BOUNDARY_TRENCH` rgb(15,20,60),
  `BOUNDARY_RIDGE` rgb(235,60,40), `BOUNDARY_TRANSFORM` rgb(255,210,60)
  (layers.rs:130–132). These are the **one-cell-wide bands**: a boundary is
  any cell whose flag bit is set — no polyline exists anywhere; smoothing them
  (Chaikin) means creating boundary polylines from scratch (sim flags → chains
  of cell centers), there is no current geometry to reuse. Non-boundary fill:
  alive plate ids ranked (`plate_rank`, layers.rs:190–205, avoids id%24
  collisions across breakups) into the 24-entry `PLATE_COLORS`
  (layers.rs:102), oceanic darkened ×0.55.
- **CrustAge** (layers.rs:233–241): continent flag → flat grey
  `AGE_CONTINENT`; ocean → viridis of `1 − age/150 My` (`VIRIDIS` 7 anchors,
  layers.rs:79; `AGE_MAX_MY` :137).
- **Thickness** (layers.rs:242–245): batlow of `(ckm·0.01 − 5)/65`
  (`BATLOW`, layers.rs:90).
- **Overlay preview** (`BakeOverlay`, layers.rs:166–179): craton paint cells
  overwritten with `PAINT_CONTINENT`/`PAINT_OCEAN`, hotspot center+ring cells
  with magenta `HOTSPOT_MARK`. Plain color replacement — **no outline, no
  alpha tint** (Fix 1 wants "tint + outline").

Palette tests (layers.rs:271–314) pin ramp anchors and pairwise distinctness —
this testability is an argument for keeping palettes in Rust and shipping them
to the GPU as a small LUT, rather than porting ramp math to WGSL by hand.

## 4. Sea-level threshold

Applied in exactly one place: the `e <= 0.0` branch inside `hypsometric`
(layers.rs:142), after subtracting the UI slider `sea_level_m` from the stored
elevation (layers.rs:214). Keyframe elevations are already relative to the
solved sea level (`Keyframe.sea_offset_m`, keyframe.rs:70), so slider 0 = "as
solved". The GPU currently never sees elevation or sea level; a sea-level drag
is `needs_bake = true` (app.rs:746) → full CPU rebake + buffer write. Under
Fix 3 the threshold moves to the fragment shader; `sea_level_m` becomes a
uniform and the slider becomes a uniform write (no rebake) — note it must keep
counting as a pure view control (Fix 1).

## 5. wgpu specifics: layouts, formats, uniforms — and where new state goes

`SceneResources` (render.rs:141–159) lives in egui's `CallbackResources`
(inserted in `WorldApp::new`, app.rs:169–176), created by
`SceneResources::new(device, target_format)` (render.rs:162) with egui's
surface `target_format` — color math today is effectively "sRGB bytes in, same
bytes out"; a per-fragment palette must keep interpolation in the same space
or every color shifts subtly.

- **Globe bind group layout** (render.rs:168–192):
  binding 0 uniform `GlobeUniforms` (visibility VERTEX_FRAGMENT), binding 1
  read-only storage `cell_colors` — **visibility VERTEX only**
  (render.rs:183). Per-fragment palettes need FRAGMENT visibility on
  whatever value buffers replace it.
- **Flat bind group layout** (render.rs:194–228): binding 0 uniform
  `FlatUniforms` (FRAGMENT), binding 1 storage colors (FRAGMENT), binding 2
  `texture_2d<u32>` cell ids (FRAGMENT, `Uint` sample type).
- **Uniform structs** (render.rs:125–139, WGSL mirrors shaders.wgsl:11–17 and
  48–57):
  `GlobeUniforms { rot: [[f32;4];4], params: [f32;4] }` — `params.zw` unused.
  `FlatUniforms { center_px: [f32;2], half_px: [f32;2], misc: [f32;4],
  tex: [f32;4] }` — `misc.y`, `misc.w`, `tex.zw` unused.
  Free lanes exist but not enough for detail params + sea level + seed on both
  pipelines; extending both structs is trivial (buffers sized with
  `size_of`, written whole each frame in the two `prepare()`s,
  render.rs:486–495 and 557–574). **Seed caveat**: master seed is `u64`;
  uniforms are f32 — pass it as two `u32` lanes (`vec4<u32>` field or
  bitcast), not as f32.
- **New bindings** (palette LUT texture or per-cell value buffers): add
  entries to both layouts; bind groups are already rebuilt every `field_gen`
  bump inside `sync_world` (render.rs:418–454), so re-binding is free
  plumbing. Generation bookkeeping: `grid_gen`/`field_gen` copied at
  render.rs:455–456 *inside* the field branch only — sound because
  `field_gen` always bumps with `grid_gen` (comment render.rs:395–396); keep
  that invariant if adding buffers.
- **Pipelines** (render.rs:241–305): both `BlendState::REPLACE`, no MSAA, **no
  depth buffer** (`depth_stencil: None`); globe hidden hemisphere handled by
  back-face culling only (front CCW, cull Back). Flat: cull None. wgpu-30
  idioms already in place (`bind_group_layouts: &[Some(..)]`,
  `immediate_size: 0`, `multiview_mask: None`).

## 6. Graticule

Flat only, fully in `fs_flat` (shaders.wgsl:142–155): 15° lines from the
fragment's analytic lat/lon, anti-aliased with `fwidth`, mixed 35% toward
light grey. Toggle = `misc.z` from `FlatView.graticule` (render.rs:569). Since
it runs on the *inverted* lat/lon, any new projection whose inverse is added
to `map_invert` gets the graticule for free. The globe has **no graticule**.

## 7. Globe mesh construction

`Grid` (core grid.rs:24–39): `positions: Vec<[f32;3]>` are **cell centers**
(icosphere vertices; cells are the dual Goldberg polygons), `triangles:
Vec<[u32;3]>` are the icosphere faces (CCW). `sync_world` uploads positions
verbatim as the vertex buffer and `triangles` cast to `&[u32]` as the u32
index buffer (render.rs:341–359). So: **vertices = cell centers, 1:1 with cell
ids; one indexed TriangleList draw** (`draw_indexed(0..index_count, 0, 0..1)`,
render.rs:513). Each triangle spans 3 *different* cells — which is why colors
smear across cell boundaries and why "crisp per-cell plates on the globe"
cannot come from `@interpolate(flat)` (that would paint whole triangles with
one cell's color). Crisp cells need the fragment to know all three corner cell
positions/ids and pick the nearest (barycentrics + per-corner data), which
also yields the debug "true cell boundaries" view nearly free.

## 8. Zoom / DPI relevant to "no visible facets at default zoom"

- egui `pixels_per_point` is applied in both `prepare()`s (render.rs:481,
  554) — everything internal is framebuffer pixels; UI-side units are points.
- Globe: `globe_radius_px(w, h, zoom) = 0.45·min(w,h)·zoom` (render.rs:518);
  zoom clamp **0.4–50** (app.rs:579), default 1.0; NDC scale via
  `params.xy = 2·radius_px / canvas_px` (render.rs:489–490). At L7, icosphere
  edge ≈ 0.5° ≈ 0.0086 rad; on a ~700 pt canvas at default zoom a triangle
  edge is ~3 pt — facet shading is what's visible, and at zoom 50 an edge is
  ~150 pt. "No visible facets at default zoom" is about color interpolation +
  per-fragment detail, not geometry (mesh stays coarse; no depth buffer to
  fight).
- Flat: zoom clamp **0.5–80** (app.rs:631) with cursor-anchored zoom; at high
  zoom the 4096×2048 id raster's texels (~0.088°/texel) are the visible
  quantum, not the grid — any flat mechanism chosen in Stage D must fix or
  bypass this cap (blend-of-nearest-centers needs more than one id per texel;
  rasterizing the projected triangle mesh bypasses the raster entirely but
  must handle projection seams/antimeridian triangle splitting).
- For render-detail noise both fragments can reconstruct the sphere position:
  globe `fs` already receives camera-space unit normal (`normal_cam`) and can
  apply the inverse rotation (rot is orthonormal; `rotate_inv` exists CPU-side
  render.rs:117); flat `fs` has analytic lat/lon.

## 9. rebake(): exact shape, call sites, and where the frozen overlay call sits

Signature: `fn rebake(&mut self)` — private inherent method, app.rs:397–440.
Steps today:

1. `self.needs_bake = false` (:398).
2. Early-return if `self.history` is None (:399–401).
3. `kf = &history.keyframes[viewing_kf.min(len-1)]` (:402).
4. Build `BakeOverlay` from live tool state (:404–429): craton →
   `self.craton_paint` (a `BTreeMap<u32, i8>`) flattened to sorted pairs;
   hotspot → for each unit-vector hotspot (`self.hotspot_overlay` or
   `history.hotspots`), `grid.nearest_cell` + `neighbors_of` ring.
5. `colors = layers::bake(self.layer, kf, self.sea_level_m, &overlay)` (:431).
6. `self.field_gen += 1`; publish a fresh
   `Arc<WorldBundle> { grid, colors, cell_ids, grid_gen, field_gen }`
   (:432–439).

**Exactly one call site**: end of `WorldApp::ui`, `if self.needs_bake {
self.rebake(); }` (app.rs:1215–1217), after panels and canvases, on the UI
thread. `needs_bake` setters: job completion (app.rs:369), layer switch
(:719), sea-level slider (:746), tool switch (:810), paint during
drag/click via `canvas_common` (:540), timeline slider (:901), playback
(:934), screenshot script (:1028). `rebuild_grid` (app.rs:275–307) does NOT
set it — it publishes a placeholder-grey bundle directly and relies on
`start_job` → `poll_job` to bake later.

**Where `pending_edits::apply_overlay(...)` would sit**: the natural seam is
steps 4–5 — delete the inline `BakeOverlay` construction and have rebake end:
bake base colors (or, post-Fix-3, bake/upload base *values*), then
`pending_edits::apply_overlay(...)`, then publish the bundle. Equivalent to
today because the overlay is already a pure post-bake per-cell overwrite
(layers.rs:251–267 could move behind the new call unchanged).

## Contradictions / complications for the pinned contracts

1. **The frozen A↔C interface must not be defined over `Vec<u32>` colors.**
   Fix 3 removes CPU-baked colors from `WorldBundle` (per-cell *values* +
   per-fragment palette), while Fix 1 has Track A code against
   `apply_overlay(...)` called at the end of rebake. If the interface takes
   `&mut Vec<u32>` colors, C's rewrite deletes its substrate mid-flight.
   Freeze it over an overlay artifact that survives C's change — e.g. A
   produces a per-cell overlay buffer (RGBA tint + outline/flag bits) or
   stroke geometry that C's shaders composite. This is the single biggest
   coordination risk in my territory.
2. **"Tint + outline" overlay cannot be expressed today**: the current overlay
   is opaque per-cell color replacement, and on the globe even that smears
   across triangles (per-vertex color). The outline needs either the
   per-fragment nearest-corner-cell machinery (item 7) or polyline draws.
3. **Crisp per-cell plates on the globe** is *not* just "interpolate the value
   per fragment": plate id is categorical. Fragments need the triangle's three
   corner cell ids + positions and a nearest-corner pick (`@interpolate(flat)`
   alone gives wrong, whole-triangle shapes). Plan the vertex/varying layout
   for this from the start; it doubles as the debug cell-boundary view.
4. **Storage-buffer visibility**: globe binding 1 is VERTEX-only
   (render.rs:183); every per-fragment plan requires FRAGMENT-visible value
   buffers on both pipelines.
5. **Flat resolution cap**: the 4096×2048 R32Uint raster (~0.088°/texel)
   quantizes the flat view regardless of Stage-D palette work; "blend nearest
   cell centers" needs per-texel neighbor data, "rasterize projected mesh"
   needs antimeridian/outline handling that the single-triangle+inverse
   approach currently gets free. Also the raster costs ~33 MB CPU-side per
   level and is equirect — reusable by any projection via lat/lon.
6. **Projection triplication**: Robinson already lives twice (core
   proj.rs:80 table, WGSL shaders.wgsl:65 `ROB`). Eckert IV adds core
   forward+inverse *and* a WGSL inverse (Newton in `map_invert`, `misc.x =
   2.0`), and the shader/CPU pair must agree per-pixel like the equirect
   strict-gate comment demands (shaders.wgsl:84–86) or cursor-readout/brush
   acceptance fails. Fixed Newton cap + tolerance identical in both.
7. **Palette testability**: layers.rs's ΔE-verified anchor tables and tests
   are an asset; prefer baking palettes into a small LUT texture (CPU Rust
   stays the source of truth and testable, GPU applies per fragment) over
   hand-porting ramp math to WGSL. Same LUT bound to both pipelines satisfies
   "noise + palette shared verbatim".
8. **Seed uniform**: master seed is u64; pass as two u32 uniform lanes, don't
   round-trip through f32.
9. **Sea level / Detail sliders become uniform writes** (no rebake): today the
   sea-level drag path sets `needs_bake` (app.rs:746) and costs a full L8
   rebake per change; after Fix 3 route them to `prepare()` uniforms and keep
   them out of the strokes/edits path (Fix 1 "pure view controls stay live").
10. **Default preset**: `Preset` (app.rs:32–53) is Draft6/Standard7/High8 with
    default `Standard7` (app.rs:220); contract wants default High8 + new
    Ultra9 row (`Preset::level/label`, the combo at app.rs:722–733, and the
    perf script's `fps_grid_level` metric app.rs:1136 all touch it).
11. **Perf script shape**: perf mode currently measures 40 warmup + 240 sample
    frames per view at the *current* preset only (app.rs:1068–1119), vsync off
    via `PresentMode::AutoNoVsync` (main.rs:228). The contract's
    L7/L8/L9-with-detail matrix needs a stage loop over presets (each preset
    change triggers `rebuild_grid` + a full sim run — budget script wall time
    for L9). CLI parsing is a hand-rolled `while let` in `parse_args`
    (main.rs:29+) — `--seed/--preset/--detail` slot in trivially.
