// WorldMaker shaders: globe (orthographic 3D) and flat (projected map).
//
// Since WO-0003 Fix 3 both pipelines shade PER FRAGMENT from one per-cell
// VALUES buffer (f32 scalar bits + category word, layers::bake_values) and a
// 256x8 palette LUT baked from the Rust ramps. Sea level, Detail, the layer
// id and the debug toggles are uniforms (ShadeParams) - live view controls,
// never a rebake. Everything after "which cells, what weights" happens in the
// ONE resolve_fragment below, shared by both canvases by construction.
//
// All color math stays in sRGB-encoded space: LUT bytes in, manual mix in
// encoded space, Lambert multiply in encoded space (matching the old CPU
// bake's byte-through behavior).
//
// Binding-slot convention: module-scope vars are shared by both entry
// points, so SHARED resources (positions 1, cell_values 3, overlay 4,
// palette 5) use the same slot in both bind group layouts; per-canvas
// resources (uniform blocks at 0, tri_ids / cell_ids at 2, the flat CSR
// neighbor graph at 6/7) reuse or extend each other's slots - legal because
// no single entry point touches both canvases' resources.

// ---------- shared shading (both canvases) ----------

struct ShadeParams {
    // Master seed as two u32 lanes - never through f32.
    seed_lo: u32,
    seed_hi: u32,
    // bits 0..=3 layer id (0 elevation, 1 plates, 2 crust age, 3 thickness);
    // bit 8 debug true-cell boundaries; bit 9 debug legacy boundary bands.
    layer_flags: u32,
    // Render-detail fBm octaves.
    octaves: u32,
    // UI slider, live: subtracted from interpolated elevation per fragment.
    sea_level_m: f32,
    // Detail slider t x default amplitude, meters; 0 = detail off.
    detail_amp_m: f32,
    // Mean cell angular spacing sqrt(4*pi / cell_count), radians.
    detail_cell_rad: f32,
    _pad: f32,
}

const LF_LAYER_MASK: u32 = 0xFu;
const LF_DEBUG_CELLS: u32 = 256u; // 1 << 8
const LF_DEBUG_BANDS: u32 = 512u; // 1 << 9

// Category-word bits (layers::bake_values, d3a section 2.2).
const CAT_RANK_MASK: u32 = 0xFFu;
const CAT_BND_SHIFT: u32 = 8u;
const CAT_CONTINENT: u32 = 65536u; // 1 << 16

// Overlay-word bits (pending_edits.rs, frozen in feel-pass-design.md D1).
const OVERLAY_TINT_MASK: u32 = 0xFu;
const OVERLAY_FORCE_OUTLINE: u32 = 16u; // 1 << 4
const OVERLAY_ALPHA_SHIFT: u32 = 8u;

// Cell centers as tightly packed floats (array<f32>, indexed 3c..3c+2, so no
// vec3 16-byte stride padding); same buffer the globe VS fetches.
@group(0) @binding(1) var<storage, read> positions: array<f32>;
// Per-cell shading records: x = f32 scalar bits, y = category word.
@group(0) @binding(3) var<storage, read> cell_values: array<vec2<u32>>;
// Per-cell pending-edit overlay words.
@group(0) @binding(4) var<storage, read> overlay: array<u32>;
// Palette LUT, 256x8 Rgba8Unorm. No sampler anywhere: ramps are read with
// two textureLoads + an explicit mix so interpolation is deterministic and
// identical to the CPU piecewise-linear-in-sRGB ramps.
@group(0) @binding(5) var palette: texture_2d<f32>;

fn pos3(c: u32) -> vec3<f32> {
    let i = 3u * c;
    return vec3<f32>(positions[i], positions[i + 1u], positions[i + 2u]);
}

fn lut_texel(row: u32, i: u32) -> vec3<f32> {
    return textureLoad(palette, vec2<u32>(min(i, 255u), row), 0).rgb;
}

fn lut_ramp(row: u32, t: f32) -> vec3<f32> {
    let x = clamp(t, 0.0, 1.0) * 255.0;
    let i0 = u32(floor(x));
    let i1 = min(i0 + 1u, 255u);
    let c0 = textureLoad(palette, vec2<u32>(i0, row), 0).rgb;
    let c1 = textureLoad(palette, vec2<u32>(i1, row), 0).rgb;
    return mix(c0, c1, x - f32(i0));
}

// ----- render-detail noise (d3a section 5.2) -----
//
// Seeded u32-integer-lattice value noise on the unit sphere: display-only,
// deliberately an INDEPENDENT implementation from the sim's crate-private
// fBm, with different hash constants, so render detail can never correlate
// with world data. The master seed arrives as two u32 uniform lanes - it
// never passes through f32. Integer hashing is bit-exact everywhere; the
// float lerp chain is deterministic per GPU (display-only, guarded out of
// every golden by the worldgen.rs render-only guard test).

fn hash3(c: vec3<i32>, seed: vec2<u32>) -> u32 {
    var h = seed.x;
    h = (h ^ bitcast<u32>(c.x)) * 0x9E3779B1u;
    h = (h ^ bitcast<u32>(c.y)) * 0x85EBCA77u;
    h = (h ^ bitcast<u32>(c.z)) * 0xC2B2AE3Du;
    h = h ^ seed.y;
    h = h ^ (h >> 16u);
    h = h * 0x7FEB352Du;
    h = h ^ (h >> 15u);
    h = h * 0x846CA68Bu;
    return h ^ (h >> 16u);
}

// Lattice-corner value in [-1, 1), exact: top 24 bits -> f32.
fn corner_val(c: vec3<i32>, seed: vec2<u32>) -> f32 {
    return f32(hash3(c, seed) >> 8u) * (2.0 / 16777216.0) - 1.0;
}

// Trilinear value noise with a quintic fade.
fn vnoise(q: vec3<f32>, seed: vec2<u32>) -> f32 {
    let base = floor(q);
    let c = vec3<i32>(base);
    let f = q - base;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let v000 = corner_val(c + vec3<i32>(0, 0, 0), seed);
    let v100 = corner_val(c + vec3<i32>(1, 0, 0), seed);
    let v010 = corner_val(c + vec3<i32>(0, 1, 0), seed);
    let v110 = corner_val(c + vec3<i32>(1, 1, 0), seed);
    let v001 = corner_val(c + vec3<i32>(0, 0, 1), seed);
    let v101 = corner_val(c + vec3<i32>(1, 0, 1), seed);
    let v011 = corner_val(c + vec3<i32>(0, 1, 1), seed);
    let v111 = corner_val(c + vec3<i32>(1, 1, 1), seed);
    let x00 = mix(v000, v100, u.x);
    let x10 = mix(v010, v110, u.x);
    let x01 = mix(v001, v101, u.x);
    let x11 = mix(v011, v111, u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

// First-octave wavelength ~ 3 cell spacings (freq0 = this / cell spacing):
// bridges grid scale into sub-cell scale.
const DETAIL_FREQ_CELLS: f32 = 0.35;

// fBm: gain 0.5, lacunarity 2.0, normalized to [-1, 1]. Each octave fades
// against the pixel footprint (judgement graft 6) so sub-pixel octaves never
// shimmer; the fade does not renormalize (a faded octave must not boost the
// others). Footprint = length(fwidth(q)), how far the lattice coordinate
// moves per pixel: the graft's literal fwidth(length(q)) is identically ~0
// on the sphere (|q| = freq everywhere), so the intent is implemented, not
// the letter.
fn render_fbm(p: vec3<f32>, freq0: f32, octaves: u32, seed: vec2<u32>) -> f32 {
    var sum = 0.0;
    var norm = 0.0;
    var amp = 1.0;
    var freq = freq0;
    // Pixel footprint at the base frequency; doubles per octave with freq.
    var footprint = length(fwidth(p * freq0));
    for (var o = 0u; o < octaves; o = o + 1u) {
        let q = p * freq;
        let fade = 1.0 - smoothstep(0.25, 1.0, footprint);
        sum = sum + amp * fade * vnoise(q, seed);
        norm = norm + amp;
        amp = amp * 0.5;
        freq = freq * 2.0;
        footprint = footprint * 2.0;
    }
    return sum / max(norm, 1e-6);
}

// The one shading function (d3a section 5.1): everything after "which cells,
// what weights" is canvas-independent. cids = candidate cells (globe: the
// triangle's corners; flat: the winner's wedge triple), w = chord-plane
// barycentrics, p = unit-sphere position of the fragment, k_win = index of
// the nearest candidate.
fn resolve_fragment(
    cids: vec3<u32>,
    w_in: vec3<f32>,
    p: vec3<f32>,
    k_win: u32,
    sp: ShadeParams,
) -> vec3<f32> {
    let w = w_in / max(w_in.x + w_in.y + w_in.z, 1e-6);
    let v0 = cell_values[cids.x];
    let v1 = cell_values[cids.y];
    let v2 = cell_values[cids.z];
    let s = vec3<f32>(bitcast<f32>(v0.x), bitcast<f32>(v1.x), bitcast<f32>(v2.x));
    var cat_arr = array<u32, 3>(v0.y, v1.y, v2.y);
    var cid_arr = array<u32, 3>(cids.x, cids.y, cids.z);
    let cat_win = cat_arr[k_win];
    let cid_win = cid_arr[k_win];

    let layer = sp.layer_flags & LF_LAYER_MASK;
    var color: vec3<f32>;
    if layer == 1u {
        // Plates: categorical, winner cell only - crisp true-cell shapes.
        let rank = cat_win & CAT_RANK_MASK;
        let oceanic = (cat_win & CAT_CONTINENT) == 0u;
        color = lut_texel(4u, rank + select(0u, 32u, oceanic));
        if (sp.layer_flags & LF_DEBUG_BANDS) != 0u {
            // Legacy one-cell boundary bands, debug-only (pinned).
            let bnd = (cat_win >> CAT_BND_SHIFT) & 3u;
            if bnd != 0u {
                color = lut_texel(5u, bnd - 1u);
            }
        }
    } else if layer == 2u {
        // Crust age: continents crisp; the ocean ramp interpolates with
        // continent candidates masked out so no grey halo smears into young
        // ocean (d3a section 3.4).
        if (cat_win & CAT_CONTINENT) != 0u {
            color = lut_texel(5u, 3u);
        } else {
            let m = vec3<f32>(
                select(1.0, 0.0, (v0.y & CAT_CONTINENT) != 0u),
                select(1.0, 0.0, (v1.y & CAT_CONTINENT) != 0u),
                select(1.0, 0.0, (v2.y & CAT_CONTINENT) != 0u),
            );
            let wm = w * m;
            let t = dot(wm, s) / max(wm.x + wm.y + wm.z, 1e-6);
            color = lut_ramp(2u, t);
        }
    } else if layer == 3u {
        color = lut_ramp(3u, dot(w, s));
    } else {
        // Elevation: the rasterizer interpolates the VALUE (via w), render
        // detail adds sub-cell relief, then the live sea level thresholds
        // and the palette applies per fragment - so the threshold on
        // e_render is what makes coastlines fractal (d3a section 5.3).
        let e_base = dot(w, s) - sp.sea_level_m;
        var e = e_base;
        if sp.detail_amp_m > 0.0 {
            // Slope from the candidate triple (meters across ~one cell
            // spacing): calm plains stay calm, rough relief roughens. The
            // depth fade quiets the abyss but is smooth through e = 0, so
            // both sides of the coastline get equal noise.
            let slope_m = max(max(s.x, s.y), s.z) - min(min(s.x, s.y), s.z);
            let depth_t = smoothstep(-1500.0, -100.0, e_base);
            let amp = sp.detail_amp_m
                * mix(0.30, 1.0, clamp(slope_m / 800.0, 0.0, 1.0))
                * mix(0.25, 1.0, depth_t);
            let freq0 = DETAIL_FREQ_CELLS / sp.detail_cell_rad;
            let seed = vec2<u32>(sp.seed_lo, sp.seed_hi);
            e = e_base + amp * render_fbm(p, freq0, sp.octaves, seed);
        }
        if e <= 0.0 {
            // Row 0 is sqrt-warped (graft 5): texel coordinate u = sqrt(t).
            color = lut_ramp(0u, sqrt(clamp(-e / 6000.0, 0.0, 1.0)));
        } else {
            color = lut_ramp(1u, e / 5500.0);
        }
    }

    // Debug true-cell boundaries: bisector margin between the winner and the
    // best DISTINCT rival - the same machinery the overlay outline below
    // uses. A degenerate candidate triple draws none.
    if (sp.layer_flags & LF_DEBUG_CELLS) != 0u {
        var m_best = 1e30;
        let d_win = dot(p, pos3(cid_win));
        for (var k = 0u; k < 3u; k = k + 1u) {
            let cid = cid_arr[k];
            if cid != cid_win {
                m_best = min(m_best, d_win - dot(p, pos3(cid)));
            }
        }
        if m_best < 1e29 {
            let aa = max(1.5 * fwidth(m_best), 1e-9);
            let line = 1.0 - smoothstep(0.0, aa, m_best);
            color = mix(color, vec3<f32>(0.13, 0.13, 0.15), line * 0.6);
        }
    }

    // Pending-edit overlay tint, composited from the winner cell's word at
    // the word's alpha.
    let ow = overlay[cid_win];
    let code = ow & OVERLAY_TINT_MASK;
    if code != 0u {
        var tint = color;
        switch code {
            case 1u: { tint = lut_texel(5u, 4u); }        // craton paint +1
            case 2u: { tint = lut_texel(5u, 5u); }        // craton force-ocean
            case 3u, 4u: { tint = lut_texel(5u, 6u); }    // hotspot marker / add
            case 5u: { tint = lut_texel(5u, 6u) * 0.55; } // pending remove
            default: {}
        }
        var alpha = f32((ow >> OVERLAY_ALPHA_SHIFT) & 0xFFu);
        if alpha == 0.0 {
            alpha = 160.0; // frozen renderer default
        }
        color = mix(color, tint, alpha / 255.0);
    }

    // Pending-overlay region outline (feel-pass-design.md D1, d3a section
    // 7.4): the debug bisector-margin machinery above, keyed on overlay
    // words. For each non-winner candidate whose word differs from the
    // winner's (or where either sets FORCE_OUTLINE), a crisp ~1.8 px white
    // line on the true Voronoi edge, on both canvases. The margins and
    // their fwidths are computed unconditionally (smooth, uniform control
    // flow); only `line` is gated, so a zeroed overlay leaves `line` at 0
    // and mix(color, ., 0) - i.e. the exact leg-2 bytes.
    let d_win_o = dot(p, pos3(cid_win));
    var line_o = 0.0;
    for (var k = 0u; k < 3u; k = k + 1u) {
        let cid = cid_arr[k];
        let m_k = d_win_o - dot(p, pos3(cid));
        let aa = max(1.8 * fwidth(m_k), 1e-9);
        let ow_k = overlay[cid];
        if cid != cid_win && (ow_k != ow || ((ow_k | ow) & OVERLAY_FORCE_OUTLINE) != 0u) {
            line_o = max(line_o, 1.0 - smoothstep(0.0, aa, m_k));
        }
    }
    if line_o > 0.0 {
        color = mix(color, vec3<f32>(1.0, 1.0, 1.0), line_o * 0.9);
    }
    return color;
}

// ---------- globe ----------

struct GlobeUniforms {
    // Rotation of the planet into camera space (camera looks along -z, so
    // +z is toward the viewer after rotation).
    rot: mat4x4<f32>,
    // x, y: camera-space -> NDC scale. z, w: canvas rect size in
    // framebuffer pixels (the boundary ribbon VS needs pixel space).
    params: vec4<f32>,
    // x: boundary-ribbon half-width in framebuffer pixels. y, z, w: unused.
    bnd: vec4<f32>,
    shade: ShadeParams,
}

@group(0) @binding(0) var<uniform> globe_u: GlobeUniforms;
// The icosphere triangle list (3 cell ids per triangle): the old index
// buffer rebound as storage so the unindexed draw can fetch corner ids.
@group(0) @binding(2) var<storage, read> tri_ids: array<u32>;

struct GlobeVsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal_cam: vec3<f32>,
    // One-hot barycentrics; the rasterizer interpolates them (orthographic,
    // w = 1, so perspective-correct interpolation degenerates to linear).
    @location(1) bary: vec3<f32>,
    // The triangle's three cell ids - identical from all three vertices, so
    // flat interpolation is provoking-vertex-safe.
    @location(2) @interpolate(flat) cids: vec3<u32>,
}

@vertex
fn vs_globe(@builtin(vertex_index) vi: u32) -> GlobeVsOut {
    let tri = vi / 3u;
    let corner = vi % 3u;
    let cids = vec3<u32>(
        tri_ids[3u * tri],
        tri_ids[3u * tri + 1u],
        tri_ids[3u * tri + 2u],
    );
    var cid_arr = array<u32, 3>(cids.x, cids.y, cids.z);
    let position = pos3(cid_arr[corner]);
    let cam = (globe_u.rot * vec4<f32>(position, 1.0)).xyz;
    var out: GlobeVsOut;
    // Orthographic: depth maps front hemisphere (+z toward viewer) near 0.
    out.pos = vec4<f32>(cam.x * globe_u.params.x, cam.y * globe_u.params.y, 0.5 - cam.z * 0.4, 1.0);
    out.normal_cam = cam;
    var bary = vec3<f32>(0.0, 0.0, 0.0);
    bary[corner] = 1.0;
    out.bary = bary;
    out.cids = cids;
    return out;
}

@fragment
fn fs_globe(in: GlobeVsOut) -> @location(0) vec4<f32> {
    // Sphere position: rot is orthonormal, so its transpose is its inverse
    // (CPU precedent rotate_inv).
    let r = globe_u.rot;
    let rot3 = mat3x3<f32>(r[0].xyz, r[1].xyz, r[2].xyz);
    let p = normalize(transpose(rot3) * in.normal_cam);
    // Nearest of the 3 corners; exact ties go to the LOWEST cell id, matching
    // Grid::nearest_cell's tie rule (judgement A5).
    let d0 = dot(p, pos3(in.cids.x));
    let d1 = dot(p, pos3(in.cids.y));
    let d2 = dot(p, pos3(in.cids.z));
    var k_win = 0u;
    var best_d = d0;
    var best_cid = in.cids.x;
    if d1 > best_d || (d1 == best_d && in.cids.y < best_cid) {
        k_win = 1u;
        best_d = d1;
        best_cid = in.cids.y;
    }
    if d2 > best_d || (d2 == best_d && in.cids.z < best_cid) {
        k_win = 2u;
        best_d = d2;
        best_cid = in.cids.z;
    }
    let base = resolve_fragment(in.cids, in.bary, p, k_win, globe_u.shade);
    let l = normalize(vec3<f32>(0.35, 0.30, 0.90));
    let shade = 0.72 + 0.28 * max(dot(normalize(in.normal_cam), l), 0.0);
    return vec4<f32>(base * shade, 1.0);
}

// ---------- flat map ----------

struct FlatUniforms {
    // Map center and half-extents in framebuffer pixels.
    center_px: vec2<f32>,
    half_px: vec2<f32>,
    // x: projection (0 = equirectangular, 1 = Robinson, 2 = Eckert IV),
    // y: unused, z: graticule (0/1), w: unused.
    misc: vec4<f32>,
    // x, y: cell-id texture dimensions. z, w: canvas rect min in
    // framebuffer pixels (boundary ribbon VS).
    tex: vec4<f32>,
    // x: boundary-ribbon half-width px; y, z: canvas rect size px; w: unused.
    bnd: vec4<f32>,
    shade: ShadeParams,
}

@group(0) @binding(0) var<uniform> flat_u: FlatUniforms;
@group(0) @binding(2) var cell_ids: texture_2d<u32>;
// The grid's CSR neighbor graph (flat canvas only): offsets length cells+1,
// lists CCW-ordered viewed from outside - a verbatim copy of Grid's arrays.
@group(0) @binding(6) var<storage, read> nbr_offsets: array<u32>;
@group(0) @binding(7) var<storage, read> nbrs: array<u32>;

// Greedy-walk cap (d3a section 4.2): the raster hint is at most ~1.5 cell
// spacings from the true winner even at L9, needing <= 2 moves; 4 is a 2x
// margin. Mirrored by the CPU property test in render.rs.
const WALK_CAP: u32 = 4u;

// Robinson X (parallel length) and Y (parallel distance) every 5 degrees.
// Must match ROBINSON_TABLE in worldmaker-core (proj.rs).
var<private> ROB: array<vec2<f32>, 19> = array<vec2<f32>, 19>(
    vec2<f32>(1.0000, 0.0000), vec2<f32>(0.9986, 0.0620), vec2<f32>(0.9954, 0.1240),
    vec2<f32>(0.9900, 0.1860), vec2<f32>(0.9822, 0.2480), vec2<f32>(0.9730, 0.3100),
    vec2<f32>(0.9600, 0.3720), vec2<f32>(0.9427, 0.4340), vec2<f32>(0.9216, 0.4958),
    vec2<f32>(0.8962, 0.5571), vec2<f32>(0.8679, 0.6176), vec2<f32>(0.8350, 0.6769),
    vec2<f32>(0.7986, 0.7346), vec2<f32>(0.7597, 0.7903), vec2<f32>(0.7186, 0.8435),
    vec2<f32>(0.6732, 0.8936), vec2<f32>(0.6213, 0.9394), vec2<f32>(0.5722, 0.9761),
    vec2<f32>(0.5322, 1.0000)
);

const PI: f32 = 3.14159265358979;
const STEP_RAD: f32 = 0.08726646259971647; // 5 degrees

// Invert normalized map coords -> (lat, lon); w = 1 when inside the outline.
fn map_invert(proj: f32, mx: f32, my: f32) -> vec3<f32> {
    if abs(my) > 1.0 {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    if proj < 0.5 {
        // Strict gate, matching core Projection::invert exactly, so the
        // rendered map and the CPU cursor readout agree on every pixel.
        if abs(mx) > 1.0 {
            return vec3<f32>(0.0, 0.0, 0.0);
        }
        return vec3<f32>(my * PI * 0.5, mx * PI, 1.0);
    }
    if proj > 1.5 {
        // Eckert IV: closed form, mirroring core Projection::invert's arm
        // exactly (same rejection order, same 1.0001 hair tolerance, same
        // clamps - the equirect strict-gate discipline), so shader pixels
        // and the CPU cursor readout / brush picks agree on domain
        // membership per pixel. |my| > 1 was rejected above, matching the
        // CPU arm's y check coming first.
        let theta = asin(clamp(my, -1.0, 1.0));
        let s = sin(theta);
        let c = cos(theta);
        let lat = asin(clamp((theta + s * c + 2.0 * s) / (2.0 + PI * 0.5), -1.0, 1.0));
        let lon = 2.0 * PI * mx / (1.0 + c);
        if abs(lon) > PI * 1.0001 {
            return vec3<f32>(0.0, 0.0, 0.0);
        }
        return vec3<f32>(lat, clamp(lon, -PI, PI), 1.0);
    }
    // Robinson: |y| -> |lat| by inverse-interpolating the Y column.
    let ya = abs(my);
    var i: i32 = 0;
    loop {
        if i >= 17 || ROB[i + 1].y >= ya { break; }
        i = i + 1;
    }
    let y0 = ROB[i].y;
    let y1 = ROB[i + 1].y;
    var f: f32 = 0.0;
    if y1 > y0 { f = (ya - y0) / (y1 - y0); }
    let lat_abs = (f32(i) + f) * STEP_RAD;
    let px = ROB[i].x + (ROB[i + 1].x - ROB[i].x) * f;
    let lon = mx * PI / px;
    if abs(lon) > PI * 1.0001 {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    return vec3<f32>(lat_abs * sign(my), clamp(lon, -PI, PI), 1.0);
}

struct FlatVsOut {
    @builtin(position) pos: vec4<f32>,
}

@vertex
fn vs_flat(@builtin(vertex_index) vi: u32) -> FlatVsOut {
    // One large triangle; the render pass viewport/scissor is the canvas rect.
    var out: FlatVsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) & 1) * 4.0 - 1.0;
    out.pos = vec4<f32>(x, y, 0.5, 1.0);
    return out;
}

@fragment
fn fs_flat(in: FlatVsOut) -> @location(0) vec4<f32> {
    let mx = (in.pos.x - flat_u.center_px.x) / flat_u.half_px.x;
    let my = -(in.pos.y - flat_u.center_px.y) / flat_u.half_px.y;
    let inv = map_invert(flat_u.misc.x, mx, my);
    if inv.z < 0.5 {
        return vec4<f32>(lut_texel(5u, 7u), 1.0); // outside the map outline
    }
    let lat = inv.x;
    let lon = inv.y;
    // Unit-sphere position of the fragment - the same ground point the globe
    // shades, so the shared noise/palette functions see identical inputs.
    let cl = cos(lat);
    let p = vec3<f32>(cl * cos(lon), cl * sin(lon), sin(lat));
    let tw = flat_u.tex.x;
    let th = flat_u.tex.y;
    let tx = clamp((lon + PI) / (2.0 * PI) * tw, 0.0, tw - 1.0);
    let ty = clamp((0.5 - lat / PI) * th, 0.0, th - 1.0);
    let cell = textureLoad(cell_ids, vec2<i32>(i32(tx), i32(ty)), 0).r;

    // Exact Voronoi winner (d3a section 4, judgement R1): the raster is only
    // a walk HINT - refine with Grid::nearest_cell's exact step rule, best
    // improvement over the CSR ring with ties toward the lower id, so the
    // pixel winner bit-equals the CPU picking path at any zoom.
    var c = cell;
    var best_d = dot(p, pos3(c));
    for (var step = 0u; step < WALK_CAP; step = step + 1u) {
        var best = c;
        var bd = best_d;
        let lo = nbr_offsets[c];
        let hi = nbr_offsets[c + 1u];
        for (var i = lo; i < hi; i = i + 1u) {
            let nb = nbrs[i];
            let d = dot(p, pos3(nb));
            if d > bd || (d == bd && nb < best) {
                best = nb;
                bd = d;
            }
        }
        if best == c { break; }
        c = best;
        best_d = bd;
    }

    // Containing wedge of the winner's CCW ring (B1 corrected sign): inside
    // wedge i iff g_i >= 0 and g_{i+1} <= 0, with g_j = dot(p, cross(pos_c,
    // pos_nj)) computed ONCE per ring index so adjacent wedges share their
    // boundary test bit-identically (on an exact 0 both match; first in ring
    // order wins). If the scan somehow exhausts (cap-truncated walk on a
    // future L>9 grid), fall back to the wedge maximizing min(g_i, -g_{i+1})
    // - never unreachable UB.
    let rlo = nbr_offsets[c];
    let k = nbr_offsets[c + 1u] - rlo;
    let pc = pos3(c);
    var g: array<f32, 6>;
    for (var j = 0u; j < k; j = j + 1u) {
        g[j] = dot(p, cross(pc, pos3(nbrs[rlo + j])));
    }
    var wedge = 0xffffffffu;
    var fb = 0u;
    var fb_score = -1e30;
    for (var i = 0u; i < k; i = i + 1u) {
        var i1 = i + 1u;
        if i1 == k { i1 = 0u; }
        if wedge == 0xffffffffu && g[i] >= 0.0 && g[i1] <= 0.0 {
            wedge = i;
        }
        let score = min(g[i], -g[i1]);
        if score > fb_score {
            fb_score = score;
            fb = i;
        }
    }
    if wedge == 0xffffffffu { wedge = fb; }
    var w1 = wedge + 1u;
    if w1 == k { w1 = 0u; }
    let ia = nbrs[rlo + wedge];
    let ib = nbrs[rlo + w1];

    // Chord-plane barycentrics in differenced form (judgement R2): edge
    // vectors and the exact ray-plane point keep every term at cell-spacing
    // scale, so f32 stays accurate at L9 (the raw triple-product solve loses
    // percent-level precision there). These are exactly the weights the globe
    // rasterizer interpolates for the same ground position.
    let pa = pos3(ia);
    let pb = pos3(ib);
    let e1 = pa - pc;
    let e2 = pb - pc;
    let nrm = cross(e1, e2);
    // dot(p, nrm) ~ |nrm| for p inside the winner's cell (nrm is
    // near-parallel to pos_c): never near zero, no guard needed.
    let t = dot(pc, nrm) / dot(p, nrm);
    let dq = t * p - pc;
    let inv_nn = 1.0 / dot(nrm, nrm);
    let wa = dot(cross(dq, e2), nrm) * inv_nn;
    let wb = dot(cross(e1, dq), nrm) * inv_nn;
    var color = resolve_fragment(
        vec3<u32>(c, ia, ib),
        vec3<f32>(1.0 - wa - wb, wa, wb),
        p,
        0u,
        flat_u.shade,
    );

    if flat_u.misc.z > 0.5 {
        // Graticule every 15 degrees, anti-aliased via screen-space derivatives.
        let lat_deg = lat * 180.0 / PI;
        let lon_deg = lon * 180.0 / PI;
        let dl = abs(fract(lat_deg / 15.0 + 0.5) - 0.5) * 15.0;
        let dn = abs(fract(lon_deg / 15.0 + 0.5) - 0.5) * 15.0;
        let wl = fwidth(lat_deg);
        let wn = fwidth(lon_deg);
        let line = max(
            1.0 - smoothstep(0.0, wl * 0.9 + 1e-5, dl),
            1.0 - smoothstep(0.0, wn * 0.9 + 1e-5, dn)
        );
        color = mix(color, vec3<f32>(0.85, 0.88, 0.92), line * 0.35);
    }
    return vec4<f32>(color, 1.0);
}

// ---------- plate-boundary ribbons (d3a section 8) ----------
//
// Smoothed boundary chains arrive as ribbon vertices: two per polyline
// point (side = +/-1), each carrying its own position `p` and the next
// point `q` so the VS can expand a screen-space quad; 6 indices per
// segment. Globe vertices are unit vectors; flat vertices are projected
// normalized map coordinates (CPU-projected, antimeridian pre-split).
// Colors come from palette LUT row 5 (texel 0 trench, 1 ridge,
// 2 transform; velocity arrows arrive as btype 9 -> texel 8, white);
// alpha-blended over the fill with fwidth-AA edges.

struct BndVsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) side: f32,
    // Camera-space z of the point; the FS discards the back hemisphere
    // (< 0.02) on the globe (A9: FS discard, never "VS discard"). The flat
    // VS emits 1.0 - no culling on a map.
    @location(1) cam_z: f32,
    @location(2) @interpolate(flat) btype: u32,
}

fn ribbon_dir(px_q: vec2<f32>, px_p: vec2<f32>) -> vec2<f32> {
    let d = px_q - px_p;
    let len = length(d);
    if len > 1e-6 {
        return d / len;
    }
    return vec2<f32>(1.0, 0.0); // degenerate segment: any direction works
}

@vertex
fn vs_bnd_globe(
    @location(0) p: vec3<f32>,
    @location(1) q: vec3<f32>,
    @location(2) side: f32,
    @location(3) btype: f32,
) -> BndVsOut {
    let cam_p = (globe_u.rot * vec4<f32>(p, 1.0)).xyz;
    let cam_q = (globe_u.rot * vec4<f32>(q, 1.0)).xyz;
    let half_vp = max(0.5 * vec2<f32>(globe_u.params.z, globe_u.params.w), vec2<f32>(1.0, 1.0));
    let px_p = vec2<f32>(cam_p.x * globe_u.params.x, cam_p.y * globe_u.params.y) * half_vp;
    let px_q = vec2<f32>(cam_q.x * globe_u.params.x, cam_q.y * globe_u.params.y) * half_vp;
    let dir = ribbon_dir(px_q, px_p);
    let perp = vec2<f32>(-dir.y, dir.x);
    let px = px_p + perp * side * globe_u.bnd.x;
    var out: BndVsOut;
    // Drawn after the globe fill in the same pass (no depth buffer), so z
    // only needs to be inside the clip volume.
    out.pos = vec4<f32>(px / half_vp, 0.1, 1.0);
    out.side = side;
    out.cam_z = cam_p.z;
    out.btype = u32(btype + 0.5);
    return out;
}

@vertex
fn vs_bnd_flat(
    @location(0) p: vec2<f32>,
    @location(1) q: vec2<f32>,
    @location(2) side: f32,
    @location(3) btype: f32,
) -> BndVsOut {
    let px_p = vec2<f32>(
        flat_u.center_px.x + p.x * flat_u.half_px.x,
        flat_u.center_px.y - p.y * flat_u.half_px.y,
    );
    let px_q = vec2<f32>(
        flat_u.center_px.x + q.x * flat_u.half_px.x,
        flat_u.center_px.y - q.y * flat_u.half_px.y,
    );
    let dir = ribbon_dir(px_q, px_p);
    let perp = vec2<f32>(-dir.y, dir.x);
    let px = px_p + perp * side * flat_u.bnd.x;
    // Framebuffer pixels -> viewport NDC (the viewport is the canvas rect).
    let rect_min = vec2<f32>(flat_u.tex.z, flat_u.tex.w);
    let sz = max(vec2<f32>(flat_u.bnd.y, flat_u.bnd.z), vec2<f32>(1.0, 1.0));
    var out: BndVsOut;
    out.pos = vec4<f32>(
        (px.x - rect_min.x) / sz.x * 2.0 - 1.0,
        1.0 - (px.y - rect_min.y) / sz.y * 2.0,
        0.1,
        1.0,
    );
    out.side = side;
    out.cam_z = 1.0;
    out.btype = u32(btype + 0.5);
    return out;
}

@fragment
fn fs_bnd(in: BndVsOut) -> @location(0) vec4<f32> {
    if in.cam_z < 0.02 {
        discard; // back hemisphere on the globe
    }
    let color = lut_texel(5u, in.btype - 1u);
    let aa = max(fwidth(in.side) * 1.5, 1e-4);
    let alpha = (1.0 - smoothstep(1.0 - aa, 1.0, abs(in.side))) * 0.9;
    // Premultiplied alpha over the already-drawn fill.
    return vec4<f32>(color * alpha, alpha);
}
