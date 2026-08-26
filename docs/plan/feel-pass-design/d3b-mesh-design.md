# Stage D design — Fix 3 (Track C), candidate B: projected-mesh flat canvas

Design for rendering + resolution against main @ 9d5d272 and the Stage U code
map. No code changed. Assigned flat mechanism: rasterize the projected
triangle mesh. All line references match the code map.

Contents: §1 per-cell values + per-fragment shading (both canvases),
§2 flat-canvas mesh mechanism (the crux), §3 render-detail noise, §4 uniforms +
WorldBundle/rebake, §5 frozen A↔C overlay interface, §6 boundary polylines,
§7 Eckert IV, §8 presets + scripts, §9 render-only guard, §10 detail sweep,
§11 resolved risks + residual risks.

---

## 1. Per-cell values, per-fragment shading

### 1.1 What replaces `colors`

`WorldBundle.colors: Vec<u32>` is deleted. The renderer gets three per-cell
storage buffers, rebuilt by rebake (rayon order-preserving map, UI-only path):

| buffer | WGSL type | content per cell |
|---|---|---|
| `values` | `array<f32>` | layer scalar: Elevation → `elev_m[c] as f32` (relative to solved sea, slider NOT subtracted — that moves to a uniform); CrustAge → `crust_age_my[c] as f32`; Thickness → `thickness_ckm[c] as f32 * 0.01` (km); Plates → 0.0 |
| `cell_aux` | `array<u32>` | bits 0..7 plate rank (`plate_rank`, layers.rs:190 logic retained); bits 8..9 boundary type (0 none, 1 convergent, 2 divergent, 3 transform — priority conv > div > transform as layers.rs:216); bit 10 continent (flags bit 15); rest 0 |
| `slope` | `array<f32>` | Elevation layer only: `max over neighbors |elev[n] − elev[c]|` in metres (CSR walk, all-zero for other layers) |
| `overlay` | `array<u32>` | pending-edit words, §5 — uploaded independently of the three above |

All are FRAGMENT-visible (and VERTEX where noted below), fixing the
VERTEX-only visibility of today's globe binding 1 (render.rs:183).

### 1.2 Globe geometry: non-indexed vertex pulling

Crisp per-cell categorical layers need each fragment to know the containing
triangle's three corner cells. That is impossible with the current indexed
draw (a shared vertex cannot carry per-triangle data), so the globe switches
to a **non-indexed draw with vertex pulling** — no vertex buffer, no index
buffer:

- storage `tri_cells: array<u32>` = flattened triangle corner ids (3 per
  triangle; a per-grid-level permuted copy of `grid.triangles`, §2.3).
- storage `positions: array<f32>` = `grid.positions` flattened (3 f32 per
  cell; `array<f32>` with manual `3*c+i` indexing avoids vec3 stride padding).
- `draw(0..3*tri_count, 0..1)`; in `vs_globe`:
  `let t = vi / 3u; let k = vi % 3u;` fetch the triple
  `c0=tri_cells[3t], c1=tri_cells[3t+1], c2=tri_cells[3t+2]`, own corner
  `ck`, own position from `positions`.

Buffer cost at L9: `tri_cells` 63 MB + `positions` 31 MB — well under the
128 MB default `maxStorageBufferBindingSize`, and far below the 251 MB a
naive expanded vertex buffer would cost. Vertex cost: 3 VS invocations per
triangle with no reuse — 15.7 M pulls at L9, ~1 M at L7. The RTX 3080 (the
budget machine, rule 8) is untroubled; Air estimates in §8.4.

### 1.3 Varyings (both canvases — the pinned layout)

```wgsl
struct MeshVsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) sphere_pos: vec3<f32>,              // world-frame unit vector (pre-rotation)
    @location(1) value: f32,                          // own-corner layer scalar
    @location(2) slope: f32,                          // own-corner slope (m)
    @location(3) bary: vec3<f32>,                     // (1,0,0)/(0,1,0)/(0,0,1) by corner k
    @location(4) @interpolate(flat) cells: vec3<u32>, // the triangle's 3 corner cell ids
    @location(5) normal_cam: vec3<f32>,               // globe only (flat leaves it zero)
}
```

Because the draw is non-indexed, **every** vertex of a triangle emits the same
`cells` triple, so the flat-interpolation provoking-vertex convention is
irrelevant. `bary` interpolates to the fragment's barycentrics (orthographic
globe and 2D flat both have w = 1, so default perspective interpolation is
exactly linear). 6 locations, far under the inter-stage limit.

### 1.4 Per-fragment resolution — scalar vs categorical

One shared function in the single `shaders.wgsl` module, called by `fs_globe`
and `fs_flat` (both pipelines are created from the same
`include_str!("shaders.wgsl")` module today, render.rs:163 — that stays, which
is how "noise + palette shared verbatim" is enforced structurally):

```wgsl
fn shade_cell(sp: vec3<f32>, value: f32, slope: f32, bary: vec3<f32>,
              cells: vec3<u32>, p: ShadeParams) -> vec3<f32>
```

- **Nearest-of-3-corners (categorical machinery):**
  `n = normalize(sp)`; `d_i = dot(n, pos_of(cells[i]))` (positions buffer,
  FRAGMENT-visible); `near` = argmax with lowest-index tie-break. The
  icosphere is the Delaunay triangulation of the cell centers (cells are their
  Voronoi regions — grid.rs:102 doc), so nearest-of-3-corners equals
  `Grid::nearest_cell` for every fragment: rendering and CPU picking agree by
  construction, up to f32 rounding on the (sub-pixel) Voronoi edges.
- **Scalar layers** (Elevation, Thickness): use the interpolated `value`
  varying. Elevation: `e = value + detail(sp, slope, value, p)` (§3), then
  `e_rel = e − p.sea_level_m`, then the palette (below). Thickness: batlow
  ramp of `(value − 5.0)/65.0`.
- **CrustAge (mixed):** continent mask is categorical — `cont_i` from
  `cell_aux[cells[i]]` bit 10; if the nearest corner is continental →
  `AGE_CONTINENT` grey. Otherwise ocean age is interpolated with
  continent-masked weights so grey cells never pollute the ramp:
  `w_i = bary_i * (1 − cont_i)`, `age = Σ w_i*age_i / Σ w_i` (per-corner
  `age_i = values[cells[i]]`), viridis of `1 − age/150`.
- **Plates (categorical):** `aux = cell_aux[cells[near]]`; normally the plate
  fill only — `plate_lut[rank] * select(0.55, 1.0, continent)`; boundary
  bits are used **only** when the debug "raw boundary bands" flag is set
  (bands are replaced by polylines, §6).
- **Debug true-cell-boundary toggle — same machinery:** sort `d` to get
  `d1 ≥ d2`; `edge = 1.0 − smoothstep(0.75, 1.5, (d1 − d2) / max(fwidth(d1 − d2), 1e-7))`;
  mix a dark line in when the debug-cells flag is set. This is the exact
  Voronoi boundary, anti-aliased ~1 px at any zoom, both canvases.
- **Overlay compositing** (per §5) runs last, before the globe's Lambert term.

### 1.5 Palette as a GPU LUT baked from layers.rs

The ΔE-verified anchors stay in Rust as the source of truth (layers.rs tests
untouched in spirit). New `layers::bake_palette_lut() -> PaletteLut`
evaluates the existing Rust ramp code into one **256 × 8, Rgba8Unorm**
texture (NOT `-Srgb` — see below), built once at startup:

| row | content | addressing |
|---|---|---|
| 0 | hypsometric ocean: texel u holds `lerp3(shallow, deep, u)`; fragment computes `u = sqrt(clamp(−e_rel/6000, 0, 1))` so the sqrt warp is exact, matching layers.rs:143–146 | linear sample |
| 1 | hypsometric land: `hypsometric(t*5500)` for t = texel center | linear sample |
| 2 | viridis (7 anchors) | linear sample |
| 3 | batlow (7 anchors) | linear sample |
| 4 | categorical: texels 0..23 `PLATE_COLORS`, 24..26 boundary trench/ridge/transform, 27 `PAINT_CONTINENT`, 28 `PAINT_OCEAN`, 29 `HOTSPOT_MARK`, 30 hotspot-remove mark, 31 `AGE_CONTINENT` | `textureLoad` (texelFetch, no filtering) |

The **same texture + one filtering sampler are bound to both pipelines** —
palette "shared verbatim" is one GPU object, not two copies. The crisp
sea-level threshold is the row-0/row-1 split selected by `sign(e_rel)`; no
filtering ever crosses the coastline discontinuity. Piecewise-linear ramps
resampled at 256 texels deviate by well under one 8-bit color quantum. A new
layers.rs test pins LUT texels against `ramp()`/`hypsometric()` at sample
points.

**sRGB handling:** today's path writes sRGB-encoded bytes straight through
(code map §5), and the ΔE verification was done "under piecewise-linear sRGB
interpolation" (layers.rs header). So the design keeps ALL shader color math
in sRGB-encoded space: LUT format Rgba8Unorm (filtering interpolates encoded
values, identical to the CPU `lerp3`), Lambert multiply on encoded values as
`fs_globe` does now, output unchanged. No `-Srgb` view, no gamma code —
pixel-for-pixel continuity with the current look. Re-log the reversed
"palettes baked on CPU" decision in decision-log.md when this lands.

---

## 2. Flat canvas: rasterize the projected mesh (the crux)

### 2.1 Mechanism

The fullscreen-triangle + per-fragment inverse + 4096×2048 cell-id texture
pipeline is deleted. The flat canvas draws the same icosphere triangles the
globe draws, with vertices placed by the **forward projection evaluated on the
CPU**, pulled from storage buffers exactly like the globe. Fragments then run
the identical `shade_cell` machinery (interpolated values, corner Voronoi,
noise, LUT).

**Pinned: forward projection on CPU, not in the vertex shader.** Rationale:
(a) the per-frame VS stays a trivial affine — no Robinson table walk and no
Eckert IV Newton run 15.7 M times per frame at L9; (b) projection math then
lives in exactly one place (core `proj.rs`) instead of three — the WGSL keeps
only a ~10-line outline test (§2.5), killing the triplication risk the code
map flags (§b risk 6); (c) the cost lands on projection switch only: one
rayon pass over N cells + a ≤21 MB upload at L9, tens of ms, a one-off hitch
on a dropdown click.

### 2.2 Per-projection geometry (`FlatGeom`)

Built in `rebuild_grid` and on projection switch (`refresh_flat_geom`),
stored in the bundle as `Arc<FlatGeom>`:

- `flat_pos: Vec<f32>` — 2 f32 per cell: normalized map coords
  `(x_n, y_n) = proj.project(lat[c], lon[c])` (core forward, y up). Used by
  every **interior** triangle.
- `ex_verts: Vec<u32>` — the exception mesh (§2.3): packed non-indexed
  triples of `{ cell: u32, x_bits: u32, y_bits: u32 }` (f32 positions stored
  as bit patterns; WGSL `bitcast<f32>`). A few thousand vertices at L9.

### 2.3 Topology: one permuted triangle list, seam + pole exceptions

Per grid level (projection-independent — all three projections are
pseudocylindrical with the seam at lon ±π and pole rows at lat ±π/2), the
bundle carries `tris: Arc<Vec<u32>>`, a permuted copy of `grid.triangles`
partitioned `[interior… | seam… | pole…]` with counts in `TriPartition`.
The core `Grid` is not touched. The globe draws the whole list (order is
irrelevant to it); the flat interior draw covers only the interior range.

**Classification (CPU, once per grid build):**
- *pole triangle*: touches a pole vertex. The golden-rectangle icosahedron
  (grid.rs:172) has base vertices (0, ±b, ±a); the edge between (0,−b,a) and
  (0,b,a) midpoint-normalizes to exactly (0,0,1), so at L≥1 both poles are
  exact vertices with a 6-triangle fan.
- *seam triangle*: `max(lon_i) − min(lon_i) > π` over its (non-pole) corners
  — reliable because triangles are small. This also catches triangles that
  merely touch the seam through a vertex at lon = +π exactly.

**Seam handling — draw twice, clip analytically.** Each seam triangle is
emitted into `ex_verts` twice: an *east copy* (west corners projected with
lon + 2π) and a *west copy* (east corners with lon − 2π). Copies overhang the
map edge; the per-fragment outline test (§2.5) discards the overhang exactly
at the seam, because for all three projections the outline at a given y is
precisely lon = ±π. No new vertices, no triangle clipping code, exact
coverage.

**Pole handling — quadify the fan.** For each pole-incident triangle
(P, A, B), P's projection is ambiguous (the pole is a line in all three
projections). Replace it with the quad A, B, P_B, P_A (two `ex_verts`
triangles), where `P_A = proj(±π/2, lon_A)` and `P_B = proj(±π/2, lon_B)` in
the triangle's wrap frame. The top/bottom map edge is then covered exactly to
y = ±1 with no notches. Both P copies carry the pole cell's id and sphere
position, so values, Voronoi and noise behave; distortion is confined to the
sub-cell polar cap. Pole quads whose A/B straddle the seam get the seam
treatment too (12 quads per pole rowend, trivial count).

The flat VS handles both fetch paths in **one pipeline and one draw**:
`draw(0..3*(n_interior + n_ex))`; if `vi < counts.x` (= 3·n_interior) pull
`tri_cells`/`flat_pos[cell]`, else pull `ex_verts[vi − counts.x]`.

### 2.4 Camera model (pan/zoom) and picking

The existing affine survives; only its consumer moves from FS to VS.

- CPU (unchanged semantics): `center_px`, `half_px` from
  `flat_base_half_extents` + pan/zoom/ppp (render.rs:557–562).
- VS: framebuffer px `fb = center_px + vec2(x_n * half_px.x, −y_n * half_px.y)`;
  NDC via the canvas viewport rect (new uniform `rect_px` = min.xy, size.zw in
  fb px — egui_wgpu sets the viewport to the callback rect, which the current
  globe VS already relies on):
  `ndc = vec2( (fb.x − cx)/hw, −(fb.y − cy)/hh )`.
- FS: map coords reconstructed from `@builtin(position)` with
  `center_px`/`half_px` exactly as today — used only for the outline test.
- **Pan/zoom are pure uniform writes** — zero re-upload, any zoom level. The
  flat zoom clamp rises from 0.5–80 to **0.5–200** (at L9, ~47 px per cell at
  zoom 200 on a 1400 px map; f32 px coordinates remain comfortably exact).
- **Picking and cursor readout do not change at all**: app.rs:643–655 already
  goes canvas pt → affine → `Projection::invert` (CPU) → `latlon_to_unit` →
  `nearest_cell`. With the mesh, the rendered cell at a pixel *is*
  `nearest_cell` (§1.4 Delaunay argument), so brush/readout/render agreement
  is exact by construction rather than quantized by a 0.088° texel. The
  cell-id texture, its 33 MB CPU raster, and `rasterize_cell_ids` are
  **deleted**.

### 2.5 Outline (Eckert IV's curved boundary), background, graticule

- **Outline test** (runs for every flat fragment, a few ALU): with
  `(mx, my)` reconstructed map coords, discard when
  `|mx| > outline_half_x(my, proj)` or `|my| > 1`.
  `outline_half_x`: equirect → 1.0; Robinson → the existing `ROB` table
  inverse-y interpolation (table stays, must-match comment stays); Eckert IV
  → `θ = asin(clamp(my, −1, 1)); return 0.5 * (1.0 + cos(θ))` (from the
  normalized forward, §7 — note `1 + cosθ ∈ [1, 2]`, no singularity).
  This is **where `misc.x = 2.0` slots in** — the projection selector's only
  remaining WGSL consumer. Interior fragments are never discarded (all three
  outlines are convex, so chords of boundary points lie inside); the test
  exists to clip seam copies and to keep fragment coverage identical to the
  CPU `invert` domain (the per-pixel-agreement contract, transferred: the
  strict-gate comment at shaders.wgsl:84–86 moves onto `outline_half_x`).
- `map_invert` and the WGSL Robinson **inverse** path are deleted; there is
  no WGSL Newton anywhere (the Eckert Newton is forward-only and CPU-only,
  §7), so the "identical cap + tolerance in WGSL" requirement is satisfied
  vacuously and the b-report's triplication risk closes.
- **Background:** the old fullscreen triangle is kept as a 3-line pipeline
  (`vs_flat_bg`/`fs_flat_bg`) painting today's outside color (0.10, 0.11,
  0.13) under the mesh — pass order: background → mesh → polylines (§6).
- **Graticule** moves from inverted lat/lon to the interpolated sphere
  position: `lat = asin(n.z)`, `lon = atan2(n.y, n.x)`; the 15° distance
  fields `dl`, `dn` are computed first and the AA widths are
  `fwidth(dl)` / `fwidth(dn)` — the periodic distance is continuous across
  the antimeridian (the ±180° jump is an integer number of 15° periods), so
  the seam shows no artifact even though `lon` itself jumps. Toggle stays
  `misc.z`. (The globe still has no graticule — unchanged scope.)

### 2.6 Draw cost at L9, and why this beats extending the lookup texture

Cost: interior + exceptions ≈ 5.24 M triangles → 15.7 M VS pulls per flat
frame at L9 (identical to the globe's). On the RTX 3080 this is a few ms. On
this Air, extrapolating from the measured 214.9 fps globe at L7 (~1 M tris ⇒
~210 M tris/s raster-bound): **~25–40 fps for L9 full-map views** — recorded,
not gated (CLAUDE.md machine note: Air numbers are records; rule 8 budgets to
the PC). L8 (1.3 M tris) stays ~100+ fps on the Air; the default preset (§8)
is L8. Sub-pixel-triangle shimmer at far zoom is comparable to today's
point-sampled texture shimmer; scalar layers interpolate smoothly so it is
mild, and the detail-noise octave fade (§3) removes the worst of it.

Why not extend the cell-id texture:
1. **The texel cap is structural.** 4096×2048 ≈ 0.088°/texel quantizes every
   projection at high zoom regardless of grid level. Matching L9 cells
   (~0.06°) needs ≥3× per axis, and "blend nearest centers" needs ≥3 ids per
   texel: ~12288×6144×3×u32 ≈ 900 MB GPU + a proportionally longer raster
   build. The mesh is exact at every zoom for 94 MB of level-shared buffers.
2. **CPU cost deleted, not grown**: `rasterize_cell_ids` (hundreds of ms per
   grid rebuild, 33 MB heap) goes away entirely.
3. **One machinery, two canvases**: flat gets interpolated values, crisp
   Voronoi categoricals, the debug boundary view, and noise input (sphere
   varying) as the *same* code path as the globe — the texture route would
   re-implement per-fragment neighbor blending as a second, different
   mechanism that must be kept visually in sync.
4. **Exact picking coherence**: nearest-of-3 = `nearest_cell` (Delaunay/
   Voronoi duality) — the render agrees with the brush at f32 precision, not
   at texel precision.
5. **No WGSL inverse projections**: Eckert IV lands with zero shader Newton
   and no third copy of projection math; adding future projections costs one
   CPU forward + ~3 WGSL lines of outline.

Costs accepted: seam/pole exception builder (bounded, projection-independent,
unit-testable on small levels), projection-switch re-projection (~tens of ms,
one-off), VS pull with no vertex reuse (bounded by §1.2 buffer scheme).

---

## 3. Render detail: deterministic 3D noise on the sphere

Renderer-only; the sim fBm (±300 m, elevation.rs:34–35) is world data and is
not touched or linked (`fbm` is `pub(crate)` in sim — the renderer cannot
name it).

- **Seed:** the master seed enters as `seed: vec2<u32>` (lo, hi) in
  `ShadeParams` — u64 → two u32 lanes, **never through f32**. It is the raw
  master seed, deliberately distinct from the sim's `sub_rng`-derived
  `noise_seed`; renderer output feeds nothing hashed.
- **Hash** (integer, exact in WGSL):
  `h = (u32(px)*0x8da6b343u) ^ (u32(py)*0xd8163841u) ^ (u32(pz)*0xcb1ab31fu) ^ seed.x;`
  `h ^= seed.y;` then lowbias32 finalize
  (`h ^= h>>16; h *= 0x7feb352du; h ^= h>>15; h *= 0x846ca68bu; h ^= h>>16;`).
- **Noise:** 3D **gradient noise** (Perlin-style; value noise reads blocky at
  coast scale): lattice corner gradients from the classic 16-entry edge
  table indexed by `h & 15u`, quintic fade, trilinear blend. Domain is the
  unit-sphere position scaled by frequency — continuous everywhere, no
  projection seams (the interpolated-then-normalized sphere varying is
  continuous across triangles, and seam/pole duplicates carry identical
  sphere positions).
- **fBm:** `octaves` (sweep-chosen, §10; slider-independent), lacunarity 2.0,
  gain 0.5, each octave's domain pre-rotated by a fixed constant orthonormal
  mat3 to break axis alignment; normalized by the gain sum. Base frequency
  `detail_base_freq` (sweep; order 48–96 — first octave ~2–4 cell widths at
  L8, finest octaves sub-cell). Per-octave **fade against pixel footprint**:
  `oct_amp *= 1 − smoothstep(0.25, 1.0, fwidth(length(pos_scaled)))` — kills
  shimmer when an octave is sub-pixel; render-only, so view dependence is
  fine.
- **Conditioning (slope + land/ocean), per fragment:** slope arrives as the
  interpolated per-cell `slope` varying (§1.1 — CPU-computed from the CSR,
  stable and cheap; no screen-space derivative tricks needed for it).
  ```
  land   = step(0.0, value − p.sea_level_m);
  amp_m  = p.detail_amp_m
         * mix(p.ocean_detail_k, 1.0, land)          // ocean_detail_k ≈ 0.35
         * clamp(slope / p.slope_ref_m, 0.15, 1.0);  // slope_ref_m ≈ 400
  e      = value + amp_m * fbm(normalize(sphere_pos), p);
  ```
  `e` then feeds the sea-level threshold and the hypsometric LUT — noise is
  added to interpolated elevation **before** tint/threshold, which is what
  makes coastlines fractal. Elevation layer only.
- **Detail slider (0..1):** `detail_amp_m = t * DETAIL_AMP_DEFAULT_M` with
  octaves and frequency fixed at the sweep-chosen constants; t = 0 branches
  the noise out entirely (off), t = 1 is the sweep-tuned full look, which is
  also the default slider position and the screenshot/perf setting.
- **Sharing:** one `fn detail_m(...)` in the single shaders.wgsl module,
  called from `shade_cell` — the same compiled code in both pipelines.
- Cross-GPU bit-exactness is not claimed (render-only; goldens untouched);
  per-seed visual stability on a given machine is.

---

## 4. Uniforms, WorldBundle, rebake

### 4.1 Uniform structs (WGSL + Rust `#[repr(C)]` bytemuck mirrors)

```wgsl
struct ShadeParams {                       // embedded in BOTH uniform structs
    seed: vec2<u32>,                       // master seed lo, hi
    layer: u32,                            // 0 elev, 1 plates, 2 age, 3 thickness
    flags: u32,                            // bit0 debug cell borders, bit1 debug raw bands,
                                           // bit2 graticule (flat), bit3 detail enabled
    sea_level_m: f32,
    detail_amp_m: f32,
    detail_base_freq: f32,
    detail_octaves: u32,
    slope_ref_m: f32,
    ocean_detail_k: f32,
    _pad: vec2<f32>,
}                                          // 48 B

struct GlobeUniforms {
    rot: mat4x4<f32>,                      // unchanged
    params: vec4<f32>,                     // xy NDC scale (unchanged), zw spare
    shade: ShadeParams,
}                                          // 128 B

struct FlatUniforms {
    center_px: vec2<f32>, half_px: vec2<f32>,  // map-plane affine, unchanged semantics
    rect_px: vec4<f32>,                    // canvas viewport min.xy, size.zw (fb px) for VS NDC
    misc: vec4<f32>,                       // x proj id (0/1/2 — outline fn), yzw spare
    counts: vec4<u32>,                     // x = 3*n_interior (VS fetch-path threshold), y = 3*n_ex
    shade: ShadeParams,
}                                          // 112 B
```

Both `prepare()`s already write their whole uniform each frame — so the
sea-level slider and Detail slider become **pure uniform writes**: their UI
handlers stop setting `needs_bake` (app.rs:746 path removed) and just mutate
`WorldApp` state read in `prepare`. They remain view controls, never edits
(Fix 1). Live cost of a slider drag at L8: zero uploads beyond the 128 B
uniform.

### 4.2 Bind group layouts

Globe (group 0): b0 uniform `GlobeUniforms` VERTEX_FRAGMENT; b1 `tri_cells`
VERTEX; b2 `positions` VERTEX_FRAGMENT; b3 `values` VERTEX_FRAGMENT;
b4 `cell_aux` FRAGMENT; b5 `slope` VERTEX; b6 `overlay` FRAGMENT; b7 LUT
`texture_2d<f32>` FRAGMENT; b8 filtering sampler FRAGMENT.

Flat (group 0): b0 uniform `FlatUniforms` VERTEX_FRAGMENT; b1–b8 the **same
buffer/texture objects** as globe (b1 `tri_cells` VERTEX, etc.); plus
b9 `flat_pos` VERTEX; b10 `ex_verts` VERTEX.

Storage buffers per stage: flat VERTEX 6, flat FRAGMENT 4, globe ≤4 — within
the default limit of 8. Pipelines: globe-mesh, flat-mesh, flat-bg,
globe-lines, flat-lines (§6) — all from the one shader module.

### 4.3 WorldBundle (new shape) and generations

```rust
pub struct WorldBundle {
    pub grid: Arc<Grid>,
    pub tris: Arc<Vec<u32>>,          // permuted corner-id list  [interior | seam | pole]
    pub tri_counts: TriPartition,     // { interior: u32, seam: u32, pole: u32 }
    pub values: Arc<Vec<f32>>,        // §1.1 — Arc so overlay-only rebakes reuse them
    pub cell_aux: Arc<Vec<u32>>,
    pub slope: Arc<Vec<f32>>,
    pub overlay: Arc<Vec<u32>>,       // §5
    pub chains: Arc<ChainSet>,        // §6 (sphere-space polylines + types)
    pub flat_geom: Arc<FlatGeom>,     // §2.2 (per grid × projection)
    pub palette: Arc<PaletteLut>,     // §1.5 (per build)
    pub grid_gen: u64,                // grid + tris (+ implies all below)
    pub value_gen: u64,               // values/cell_aux/slope/chains
    pub overlay_gen: u64,             // overlay
    pub flat_gen: u64,                // flat_geom (projection switch)
}
```

`colors` and `cell_ids` are gone. `sync_world` tracks the four generations:
grid_gen → recreate `tris`/`positions`; value_gen → write values/aux/slope +
line-segment buffers; overlay_gen → write overlay; flat_gen → write
`flat_pos`/`ex_verts` + flat line buffer; any change → rebuild both bind
groups (cheap, as today). The "field_gen always bumps with grid_gen"
invariant generalizes to: grid_gen bump implies all other gens bump.

### 4.4 rebake — new step list

`layers::bake` is replaced by `layers::bake_values(layer, kf) -> LayerBake
{ values, cell_aux, slope }` (rayon order-preserving) plus the startup-only
`bake_palette_lut()`. `BakeOverlay` is deleted. rebake becomes:

1. `needs_bake = false`.
2. If `history` is `Some`: pick the viewed keyframe; run `bake_values`;
   run `boundaries::extract(kf, &grid)` + Chaikin when `layer == Plates`
   (else reuse/empty); `value_gen += 1`.
   If `None` (**mid-run**): reuse the previous bundle's `values`/`cell_aux`/
   `slope`/`chains` Arcs unchanged — the world stays behind, no early return.
3. `let mut ov = vec![0u32; n]; pending_edits::apply_overlay(&OverlayInput {
   grid: &self.grid, pending: &self.pending_strokes,
   generated_hotspots: ... }, &mut ov);` `overlay_gen += 1`. (C ships the
   module with a no-op body and an empty `pending` default until A lands.)
4. Publish a fresh `Arc<WorldBundle>` (flat_geom/palette Arcs carried over).

Sea level and detail do not appear — they are uniforms (§4.1). Triggers for
`needs_bake` shrink to: job completion, layer switch, timeline scrub/playback,
overlay changes, screenshot script. `rebuild_grid` additionally builds
`tris`/`tri_counts` and `flat_geom` (and drops the cell-id raster step).

---

## 5. FROZEN A↔C overlay interface

Frozen over artifacts that survive C's rewrite: a per-cell **overlay word
buffer** plus the pinned Stroke type. Not typed over colors (b-report risk 1).

### 5.1 The frozen Rust surface (to be copied into feel-pass-design.md as FROZEN)

```rust
// worldmaker-io/src/strokes.rs — pinned by the Fix 1 contract; shipped on
// C's branch (merge order B→C→A requires it to exist for rebake to compile);
// A codes against it and does not change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StrokeTool { CratonPaint, CratonErase, Hotspot }

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum StrokePayload {
    CratonPaint { cells: Vec<u32>, sign: i8 },  // +1 continent / −1 ocean; per-level ids
    HotspotAdd { pos: [f32; 3] },               // unit vector
    HotspotRemove { pos: [f32; 3] },            // unit vector
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Stroke { pub tool: StrokeTool, pub payload: StrokePayload }

// worldmaker-app/src/pending_edits.rs — module OWNED BY TRACK A. C creates
// the file with exactly this signature and a no-op body, wired into rebake.
pub struct OverlayInput<'a> {
    pub grid: &'a worldmaker_core::Grid,
    /// The pending stroke list, oldest first — passed EXPLICITLY: the
    /// function must not read tool state, history, or WorldApp.
    pub pending: &'a [worldmaker_io::Stroke],
    /// Base hotspot set for rendering pending hotspot deltas; None mid-run
    /// (history dropped) — the function must still render adds/removes.
    pub generated_hotspots: Option<&'a [[f32; 3]]>,
}

/// Fill `out` (cell-count-long, zeroed by the caller) with per-cell overlay
/// words. Pure; must not allocate world state; has no route to Pipeline,
/// start_job, or TectonicsParams (structural no-sim guard: none of those
/// types appear in this module's API or imports).
pub fn apply_overlay(input: &OverlayInput<'_>, out: &mut [u32]);
```

### 5.2 The frozen word layout (render contract C guarantees)

| bits | meaning |
|---|---|
| 0..3 | tint class: 0 none, 1 craton +1, 2 craton −1, 3 hotspot-add, 4 hotspot-remove, 5 hotspot-existing-marker |
| 8..15 | tint alpha 0..255 (A picks; C suggests 160) |
| others | reserved, must be 0 |

C composites per fragment, after the base layer color, using the
nearest-corner machinery (§1.4): `w = overlay[cells[near]]`; if class ≠ 0,
`color = mix(color, tint_lut[class], alpha/255)` (tint colors = LUT row 4
texels 27..30, baked from the layers.rs constants). **Outline:** where the
Voronoi edge factor is high AND the nearest and second-nearest corners have
*different* tint classes, C draws the tint color opaque — a crisp ~1 px
outline tracing the true region boundary of every pending stroke, derived
entirely from the words (A supplies which cells; C supplies how drawn).

Properties demanded by the contract, checked: takes the pending set
explicitly (not inferred from tool) — yes; tool- and history-independent —
yes (mid-run rebake reuses value Arcs, §4.4 step 2, so tint + outline render
with the frozen world behind); typed over artifacts that survive Fix 3 — yes
(words + grid + the io Stroke type, no colors); uploads independently of
world bakes — yes (`overlay_gen`); no-op default shipped by C — yes.
Ownership note: `pending_edits` is A's module; C only creates the stub and
never edits it after A lands. The Stroke type is frozen text from this
design; C ships it verbatim so the merge order works, and A treats it as
pinned.

---

## 6. Plate-boundary smoothed polylines (both canvases)

### 6.1 CPU extraction (display-only; serial, deterministic for reproducible screenshots)

From the viewed keyframe's `plate_id` + grid CSR + `grid.triangles` (no
sim-side structure exists — built from scratch, code map §b):

1. A *mixed edge* is an icosphere edge (a<b) with `plate_id[a] ≠ plate_id[b]`.
   A triangle with ≥2 distinct corner plate ids is a *node*; its point is the
   normalized triangle centroid. A triangle with 3 distinct ids is a
   **junction** (chain endpoint).
2. Chains: iterate triangles in id order; from each junction (then from the
   lowest-id unvisited 2-distinct triangle, for junction-free loops) walk
   across mixed edges to the neighboring node until a junction or the start
   is reached. Serial, id-ordered — deterministic.
3. Segment type: for the mixed edge crossed, take the higher-priority
   boundary class of its two cells' F_BND_* bits (convergent > divergent >
   transform, matching layers.rs:216–222); split chains where the type
   changes, so each chain has one of the three colors.
4. **Chaikin ×2** corner cutting on the node list (junction endpoints
   pinned), each new point normalized back to the sphere.

Std trig is fine here (display path — the dmath rule binds committed metrics
only, code map §c). Cost at L8: tens of thousands of boundary triangles,
trivial per rebake; runs only when `layer == Plates`.

### 6.2 Rendering

`ChainSet` flattens to line segments. Two small pipelines, one shared
`fs_line` (flat color from a 3-entry uniform array filled from the layers.rs
boundary constants — same three colors):

- **Globe:** storage buffer of `{ p0: [f32;3], p1: [f32;3], ty: u32, pad }`
  segments; `vs_line_globe` pulls `seg = vi / 6`, corner = vi % 6, rotates
  endpoints, extrudes a screen-space quad ±`line_width_px/2` (uniform,
  ~1.75 pt × ppp) perpendicular to the projected segment. Back-hemisphere
  culling: pass the camera-space z of the fragment's endpoint mix as a
  varying; `discard` when < 0 (no depth buffer exists; painter's order —
  lines draw after the mesh in the same pass).
- **Flat:** segments pre-projected on CPU (with the chains, at value_gen; and
  re-projected at flat_gen — a few thousand points, trivial): `{ p0: [f32;2],
  p1: [f32;2], ty, pad }` in normalized map coords; VS extrudes in px space
  after the §2.4 affine. **Antimeridian:** while flattening a chain, when
  consecutive points jump |Δlon| > π, interpolate the seam crossing on the
  sphere and emit two chain endpoints at lon = +π and −π — the chain splits
  into two strips meeting the map edges exactly.
- Old one-cell bands: preserved verbatim behind the debug "raw bands" flag
  (§1.4) — no polylines drawn while it is on.

---

## 7. Eckert IV

### 7.1 Core (worldmaker-core/src/proj.rs) — pinned math and constants

`Projection::EckertIV` joins the enum; `ALL: [Projection; 3]`;
`name() = "Eckert IV"`; **`aspect() = 2.0` exactly** (Snyder unit-sphere
extents: x_max = 4π/√(π(4+π)), y_max = 2√(π/(4+π)); their ratio is exactly
2). In the normalized [-1,1]² frame the Snyder constants cancel:

- **Forward** (`project`): solve `θ + sinθ·cosθ + 2·sinθ = (2 + π/2)·sinφ`
  by Newton, `θ₀ = φ/2`, `Δθ = −f(θ)/f'(θ)` with
  `f'(θ) = 2·cosθ·(1 + cosθ)`; **iteration cap 12, tolerance |Δθ| ≤ 1e-9,
  computed internally in f64** (display-path; f64 keeps the near-pole
  round-trip inside the existing 1e-4 test tolerance), returned as f32.
  Special case first: if `|sinφ| ≥ 1 − 1e-12`, `θ = sign(φ)·π/2` directly
  (f' → 0 there; deterministic, no slow tail). Then
  `x_n = λ·(1 + cosθ)/(2π)`, `y_n = sinθ` — algebraically identical to the
  pinned `x = (2/√(π(4+π)))·λ·(1+cosθ)`, `y = 2√(π/(4+π))·sinθ` divided by
  the map half-extents.
- **Inverse** (`invert`) — closed form, no Newton: reject `|y| > 1`;
  `θ = asin(y)`; `φ = asin( (θ + sinθ·cosθ + 2·sinθ) / (2 + π/2) )` (clamp
  the asin argument to ±1); `λ = 2π·x/(1 + cosθ)` — well-defined everywhere
  since `1 + cosθ ∈ [1, 2]` for θ ∈ [−π/2, π/2]; reject `|λ| > π·1.0001` and
  clamp, mirroring the Robinson hair-tolerance (proj.rs:67).
- Tests in the existing suites: `dense_latlon_roundtrip` extends to
  `ALL: [Projection; 3]` unchanged (1e-4 rad); outside-rejection cases
  (corner beyond the pole line, e.g. invert(0.9, 0.99) is None since the
  pole line half-width is 0.5); a new cross-projection test: for a fixed
  lat/lon sample set, `nearest_cell(latlon_to_unit(invert(project(·))))` is
  the same cell for all three projections (the "same ground position → same
  cell" acceptance).

### 7.2 WGSL and the flat canvas

Under the mesh mechanism there is **no WGSL inverse** (the fragment never
inverts a projection). Eckert IV's entire shader footprint is the
`outline_half_x` arm — `0.5·(1 + cos(asin(clamp(my,−1,1))))` — selected by
**`misc.x = 2.0`**, plus the CPU `FlatGeom` projector arm. The pinned
"identical iteration cap + tolerance in WGSL" clause is void by construction
(the Newton is forward-only and CPU-only); the per-pixel CPU/GPU agreement it
protected is carried by (a) fragment coverage = mesh ∪ outline test, matching
`invert`'s domain gate formula-for-formula, and (b) picking running through
the same core `invert` as always. Graticule (§2.5), brushes and cursor
readout (§2.4) work with no Eckert-specific code. The projection dropdown
gains the third entry; a switch costs one `refresh_flat_geom` (§2.2).

---

## 8. Presets, flags, perf script

### 8.1 Presets

`Preset { Draft6, Standard7, High8, Ultra9 }` — Draft6 stays, Standard7 stays
(the committed BEFORE screenshots use it), **default becomes High8**
(app.rs:220), new `Ultra9` = `("Ultra (L9, 2.6M cells)", level 9)`. Combo
(app.rs:722–733) iterates all four. The L9 keyframe cadence is B's decision
and lands in `keyframe_interval_my`; C's preset only selects level 9.
Preset-switch behavior (`rebuild_grid`) is unchanged; Fix 1's pending
discard/keep runs before it (A's remit). Update the CLAUDE.md key-facts line
and the doc comment reference when B's cadence lands.

### 8.2 CLI flags (`parse_args`, main.rs:29)

Three new arms: `--seed <text>` (via `seed_from_text`, identical to typing in
the seed box), `--preset <draft6|standard7|high8|ultra9>` (case-insensitive),
`--detail <0..=1 f32>`. Carried in `Script` and applied in `WorldApp::new`
before `rebuild_grid`. Defaults for scripted runs: **screenshot mode without
explicit flags forces seed "cyrus" + Standard7 + detail 1.0** so the AFTER
set matches the committed BEFORE set's seed/preset/eras (d-report risk 9);
interactive default is High8. Old binaries warn-and-ignore unknown flags, so
the perf/screenshot wrapper scripts must check the built binary's log for
"ignoring unknown argument" and fail loudly (d-report risk 7).

### 8.3 Perf preset loop

`ScriptState::Perf` gains a preset dimension: for each of
`[Standard7, High8, Ultra9]`: set preset → `rebuild_grid` (sim runs;
`drive_script`'s existing wait-for-history gate covers the wait) → for each
view Globe/Flat/Split: 40 warmup + 240 sampled frames (unchanged), with
**detail 1.0 and smooth shading on** (they are always on now). Metrics →
`perf-feelpass-{machine}.json`: `globe_fps_L{7,8,9}`, `flat_fps_L{7,8,9}`,
`split_fps_L{7,8,9}`, `grid_build_ms_L{6..9}`, `sim_wall_s_L{7,8,9}` (from
SimJob timing), `detail_amp_m`, `detail_octaves_count`, `fps_vsync_off`,
`layer`. `fps_grid_level` is superseded by the per-level keys.

### 8.4 Budget note (L9 during perf runs)

Sim wall for the L9 stage at the default 500 My span: L7/500 My measured
1.80 s on this Air → ×16 cells ≈ 30–60 s; total script wall ≈ 3–5 min —
acceptable, logged. Memory: at the current 20 My cadence, 500 My of L9
keyframes ≈ 26 × 42 MB ≈ 1.09 GB transient — already over the 1 GB budget,
which is exactly B's cadence decision; the perf script runs under whatever
cadence B ships and records actual bytes via B's harness rows. Expected fps
(Air, estimates to verify): L7 ≥ 150, L8 ≥ 90, L9 ~25–40 full-map — recorded,
not gated (machine note); the PC re-measures when it returns.

---

## 9. Render-only guard test

### 9.1 Shared free function (new C-owned module `worldmaker-app/src/worldgen.rs`)

```rust
/// The one world-building recipe. start_job's worker thread calls this;
/// so does the guard test. Detail/sea-level/view state are not parameters —
/// render settings cannot reach the sim by construction.
pub(crate) fn build_world(
    grid: Arc<worldmaker_core::Grid>,
    master_seed: u64,
    params: worldmaker_sim::tectonics::TectonicsParams,
    progress: Option<Arc<worldmaker_sim::Progress>>,
) -> anyhow::Result<(worldmaker_sim::WorldState, u64 /* params_hash */)>
```

Body: `WorldState::new(grid)` → `Pipeline::new()` → `TectonicsStage::new(
params)` (capture `stage.params_hash()`) → `StageContext::new(master_seed)` +
progress → `run`. `start_job` (app.rs:312) shrinks to: cancel/drop as today,
then spawn a thread that calls `build_world(self.grid.clone(),
self.master_seed, self.current_params(), Some(progress))`. Headless-reachable
— resolves d-report risk 5 ("same path" no longer by-convention).

### 9.2 The tests

- App crate, `#[cfg(test)] mod tests` in worldgen.rs (binary-crate
  precedent: layers.rs/render.rs): build L6/seed 42/`TectonicsParams::
  default()` twice, with the app-side detail value set to 0.0 for the first
  run and 1.0 for the second (a local `ViewSettings { detail, sea_level_m,
  .. }` — the type start_job provably does not read, since `build_world`'s
  signature cannot receive it); assert both runs return the **same
  params_hash** and identical `hash_f32_slice(ELEVATION_M)`,
  `hash_u32_slice(PLATE_ID)`, `hash_u32_slice(CRUST_TYPE)`. Wall ≈ 1 s.
- worldmaker-sim, `tests/determinism_tests.rs`, new
  `params_expose_no_render_fields`: constructs `TectonicsParams` with an
  **exhaustive struct literal** of the 7 known fields (no `..Default`) —
  adding any field (render or otherwise) breaks the build and forces review —
  and asserts its `params_hash` equals `TectonicsParams::default()`'s. C adds
  only this test fn; golden constants in the same file remain B's (merge
  order B → C means C rebases over B's regenerated constants).

---

## 10. Octave/amplitude sweep

- **Matrix:** octaves {4, 5, 6} × amplitude {150, 300, 450 m} at base
  frequency 64 (revisit frequency only if all 9 fail), × 2 seeds: "cyrus"
  and "feelpass". 36 PNGs.
- **Harness:** dev flag `--detail-sweep <dir>` (C-owned main.rs + a script
  stage reusing the screenshot machinery): per combo, force the uniforms and
  capture (a) globe at default zoom, (b) flat coast close-up at zoom 6
  centered on the max-slope cell with |elev| < 200 m (lowest id tie-break —
  deterministic per seed). Files
  `o{oct}_a{amp}_{seed}_{globe|coast}.png` → `docs/media/feel-pass/
  detail-sweep/`.
- **Judging:** a panel session scores each combo: coastline fractality
  visible at default zoom; no speckle on plains/abyssal flats (slope
  conditioning working); no visible tiling/repetition; no shimmer at zoom 8.
  Default = best total, ties to fewer octaves (cheaper).
- **Recording:** chosen constants land as `DETAIL_OCTAVES_DEFAULT` /
  `DETAIL_AMP_DEFAULT_M` / `DETAIL_BASE_FREQ` in render.rs, a decision-log
  row with the scores, keys `detail_amp_m` + `detail_octaves_count` in
  `perf-feelpass-*.json`, and the sweep grid committed under docs/media (the
  screenshot AFTER set is captured only after this default is fixed).

---

## 11. Risk resolution and residual risks

Code-map risks resolved: frozen interface not over colors (§5, overlay words
+ io Stroke); tint + outline expressible (§5.2 FS outline from words); crisp
plates via corner varyings planned from the start (§1.3–1.4); FRAGMENT
visibility on all value buffers (§4.2); flat texel cap removed by deleting
the raster (§2); projection triplication ended — no WGSL inverses, outline
only (§2.5, §7.2); palettes as LUT from tested Rust anchors, one object both
pipelines (§1.5); seed as two u32 lanes (§4.1); sea-level/detail as uniform
writes, no rebake (§4.1); presets default High8 + Ultra9 (§8.1); perf preset
loop + flags (§8.2–8.3); mid-run overlay display via Arc-reused values
(§4.4); "same path" guard made structural (§9).

Residual risks, owned and stated:
1. **L9 raster cost on the Air** (~25–40 fps full-map estimate, §2.6/§8.4):
   accepted — Air numbers are records, PC is the budget machine. Headroom
   option if the PC also disappoints: an indexed fast path for scalar layers
   (they need no corner triple), halving pulls; designed to be addable
   without changing buffers.
2. **Non-indexed pulls triple VS invocations** at every level; mitigated by
   the tiny VS and pull buffers; measured by the perf loop before merge.
3. **Voronoi ties in f32** at cell edges can flicker sub-pixel; cosmetic,
   bounded by the lowest-index tie-break.
4. **Stroke type ships on C's branch though A owns the pending-set type** —
   forced by merge order B→C→A; de-risked by freezing the exact type text in
   §5.1 (A must not alter it).
5. **Seam/pole exception builder** is the one genuinely new geometry code
   path; unit-test it at L2/L3 (assert: union of interior + exceptions
   covers all triangles exactly once per wrap frame; every ex vertex's x
   within the outline; pole rows reach y = ±1).
6. **Noise is not bit-exact across GPUs** (render-only; documented, §3).
7. **L9 keyframe memory during perf** depends on B's cadence (§8.4) —
   sequencing dependency, not a design gap.
