// WorldMaker Phase 0 shaders: globe (orthographic 3D) and flat (projected map).
//
// Both pipelines read the same per-cell elevation storage buffer, so a seed or
// sea-level change is just a buffer/uniform update. The flat pipeline resolves
// each fragment to a cell through an equirectangular cell-id lookup texture
// that depends only on the grid level.

// ---------- shared color ----------

fn palette(elevation_m: f32, sea_level_m: f32) -> vec3<f32> {
    let e = elevation_m - sea_level_m;
    if e <= 0.0 {
        // Ocean: deep abyss -> shallow shelf.
        let t = clamp(-e / 6000.0, 0.0, 1.0);
        let shallow = vec3<f32>(0.16, 0.40, 0.55);
        let deep = vec3<f32>(0.03, 0.10, 0.23);
        return mix(shallow, deep, sqrt(t));
    }
    // Land: lowland green -> upland tan -> rock -> snow.
    let t = clamp(e / 5500.0, 0.0, 1.0);
    if t < 0.35 {
        return mix(vec3<f32>(0.23, 0.44, 0.22), vec3<f32>(0.55, 0.50, 0.28), t / 0.35);
    } else if t < 0.7 {
        return mix(vec3<f32>(0.55, 0.50, 0.28), vec3<f32>(0.58, 0.55, 0.52), (t - 0.35) / 0.35);
    }
    return mix(vec3<f32>(0.58, 0.55, 0.52), vec3<f32>(0.95, 0.95, 0.97), (t - 0.7) / 0.3);
}

// ---------- globe ----------

struct GlobeUniforms {
    // Rotation of the planet into camera space (camera looks along -z, so
    // +z is toward the viewer after rotation).
    rot: mat4x4<f32>,
    // x, y: camera-space -> NDC scale. z: sea level (m). w: unused.
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globe_u: GlobeUniforms;
@group(0) @binding(1) var<storage, read> elevation: array<f32>;

struct GlobeVsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal_cam: vec3<f32>,
    @location(1) elev: f32,
}

@vertex
fn vs_globe(@builtin(vertex_index) vi: u32, @location(0) position: vec3<f32>) -> GlobeVsOut {
    let cam = (globe_u.rot * vec4<f32>(position, 1.0)).xyz;
    var out: GlobeVsOut;
    // Orthographic: depth maps front hemisphere (+z toward viewer) near 0.
    out.pos = vec4<f32>(cam.x * globe_u.params.x, cam.y * globe_u.params.y, 0.5 - cam.z * 0.4, 1.0);
    out.normal_cam = cam;
    out.elev = elevation[vi];
    return out;
}

@fragment
fn fs_globe(in: GlobeVsOut) -> @location(0) vec4<f32> {
    let base = palette(in.elev, globe_u.params.z);
    let l = normalize(vec3<f32>(0.35, 0.30, 0.90));
    let shade = 0.72 + 0.28 * max(dot(normalize(in.normal_cam), l), 0.0);
    return vec4<f32>(base * shade, 1.0);
}

// ---------- flat map ----------

struct FlatUniforms {
    // Map center and half-extents in framebuffer pixels.
    center_px: vec2<f32>,
    half_px: vec2<f32>,
    // x: projection (0 = equirectangular, 1 = Robinson), y: sea level (m),
    // z: graticule (0/1), w: unused.
    misc: vec4<f32>,
    // x, y: cell-id texture dimensions.
    tex: vec4<f32>,
}

@group(0) @binding(0) var<uniform> flat_u: FlatUniforms;
@group(0) @binding(1) var<storage, read> flat_elevation: array<f32>;
@group(0) @binding(2) var cell_ids: texture_2d<u32>;

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
        return vec4<f32>(0.10, 0.11, 0.13, 1.0); // outside the map outline
    }
    let lat = inv.x;
    let lon = inv.y;
    let w = flat_u.tex.x;
    let h = flat_u.tex.y;
    let tx = clamp((lon + PI) / (2.0 * PI) * w, 0.0, w - 1.0);
    let ty = clamp((0.5 - lat / PI) * h, 0.0, h - 1.0);
    let cell = textureLoad(cell_ids, vec2<i32>(i32(tx), i32(ty)), 0).r;
    let elev = flat_elevation[cell];
    var color = palette(elev, flat_u.misc.y);

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
