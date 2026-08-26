//! wgpu rendering for the globe and flat canvases, driven through
//! egui_wgpu paint callbacks.
//!
//! `SceneResources` lives in the egui renderer's callback resources. Each
//! frame the UI submits a `GlobeCallback` / `FlatCallback` carrying the
//! current `WorldBundle` (grid + per-cell shading values + overlay + cell-id
//! hint raster) and view parameters; `prepare` uploads whatever changed
//! (tracked by generation counters) and `paint` draws into egui's render
//! pass.
//!
//! Since WO-0003 Fix 3 (d3a) both canvases shade PER FRAGMENT: the CPU bakes
//! per-cell scalar VALUES (layers::bake_values), the palette lives in a GPU
//! LUT baked from the same Rust ramps, and sea level / Detail / debug flags
//! are pure uniform writes. The globe draws unindexed (corner-fetch from
//! storage) so every fragment knows its triangle's three corner cells.

use std::sync::Arc;

use eframe::egui_wgpu::{self, wgpu};
use eframe::wgpu::util::DeviceExt;

use worldmaker_core::Grid;

use crate::boundaries::BoundarySet;
use crate::layers::{self, Layer};

/// Width of the equirectangular cell-id lookup raster. Depends only on the
/// grid level, so seed and sea-level changes never touch it.
pub const CELL_ID_TEX_W: u32 = 4096;
pub const CELL_ID_TEX_H: u32 = 2048;

/// Immutable snapshot of the world the renderer draws. Republished (as a new
/// Arc) by rebake; the three generations tell `sync_world` what to re-upload.
pub struct WorldBundle {
    pub grid: Arc<Grid>,
    /// Per-cell shading record for the active layer (layers::bake_values):
    /// x = f32 scalar bits, y = category word (d3a §2.2). Arc'd so an
    /// overlay-only republish is free for the values.
    pub values: Arc<Vec<[u32; 2]>>,
    /// A↔C overlay words, one per cell (feel-pass-design.md § D1). Rebuilt
    /// independently of values on every rebake.
    pub overlay: Vec<u32>,
    /// Equirectangular *hint* raster, CELL_ID_TEX_W × CELL_ID_TEX_H R32Uint.
    /// Arc'd: depends only on the grid level. Since the flat exact walk
    /// (d3a §4) it is only the walk's starting hint, never the truth.
    pub cell_ids: Arc<Vec<u32>>,
    /// Smoothed plate-boundary ribbons for the viewed keyframe (d3a §8);
    /// empty when the layer draws none. Extraction + drawing land in leg 3.
    pub boundaries: Arc<BoundarySet>,
    /// Bumped when the grid (and cell_ids) change.
    pub grid_gen: u64,
    /// Bumped when `values` changes (layer switch, scrub, new history).
    /// Always bumps with grid_gen.
    pub values_gen: u64,
    /// Bumped when `overlay` changes (any pending-stroke mutation).
    /// Always bumps with grid_gen.
    pub overlay_gen: u64,
}

impl WorldBundle {
    /// Rasterize the cell-id lookup texture for a grid: for every texel,
    /// the Goldberg cell containing that lat/lon. Row-parallel; each row
    /// walks coherently from its west neighbor so lookups are O(1).
    pub fn rasterize_cell_ids(grid: &Grid) -> Vec<u32> {
        use rayon::prelude::*;
        let w = CELL_ID_TEX_W as usize;
        let h = CELL_ID_TEX_H as usize;
        let mut out = vec![0u32; w * h];
        out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let lat = std::f32::consts::FRAC_PI_2 * (1.0 - 2.0 * (y as f32 + 0.5) / h as f32);
            let (clat, slat) = (lat.cos(), lat.sin());
            let mut hint: Option<u32> = None;
            for (x, texel) in row.iter_mut().enumerate() {
                let lon = std::f32::consts::PI * (2.0 * (x as f32 + 0.5) / w as f32 - 1.0);
                let p = [clat * lon.cos(), clat * lon.sin(), slat];
                let cell = grid.nearest_cell(p, hint);
                hint = Some(cell);
                *texel = cell;
            }
        });
        out
    }
}

/// View parameters for the globe canvas, in egui points.
#[derive(Clone, Copy)]
pub struct GlobeView {
    /// Yaw (around the planet's axis) and pitch, radians.
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

/// View parameters for the flat canvas, in egui points.
#[derive(Clone, Copy)]
pub struct FlatView {
    pub projection: worldmaker_core::Projection,
    /// Pan offset of the map center relative to the rect center, in points.
    pub pan: [f32; 2],
    pub zoom: f32,
    pub graticule: bool,
}

/// Rotation of the planet into camera space (column-major 4x4, matching
/// WGSL's `mat4x4` layout). Camera space: x right, y up, +z toward the viewer.
///
/// The camera looks at longitude `-yaw` (yaw spins the planet eastward) and
/// latitude `pitch`, with north up at pitch 0. Derivation: yaw about the
/// world z axis, then express in the camera basis at view center
/// c = (cos pitch, 0, sin pitch): cam_x = east = (0,1,0),
/// cam_y = c × cam_x = (-sin pitch, 0, cos pitch), cam_z = c.
pub fn globe_rotation(yaw: f32, pitch: f32) -> [[f32; 4]; 4] {
    let (cy, sy) = (yaw.cos(), yaw.sin());
    let (cp, sp) = (pitch.cos(), pitch.sin());
    // Columns are the images of the world basis vectors.
    [
        [sy, -sp * cy, cp * cy, 0.0],
        [cy, sp * sy, -cp * sy, 0.0],
        [0.0, cp, sp, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Apply the globe rotation to a vector (matches the shader's `rot * v`).
/// Used by tests as the oracle for `rotate_inv`; the live forward path is
/// the vertex shader.
#[cfg_attr(not(test), allow(dead_code))]
pub fn rotate(m: &[[f32; 4]; 4], v: [f32; 3]) -> [f32; 3] {
    // m is column-major: columns are m[0], m[1], m[2].
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2],
    ]
}

/// Inverse (transpose) of the globe rotation applied to a vector.
pub fn rotate_inv(m: &[[f32; 4]; 4], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

// ----- live shading parameters (uniform-only view controls) -----

/// `layer_flags` bit layout (shader must match): bits 0..=3 layer id
/// (0 elevation, 1 plates, 2 crust age, 3 thickness); bit 8 debug true-cell
/// boundaries; bit 9 debug legacy one-cell boundary bands.
pub const LF_DEBUG_CELL_BOUNDS: u32 = 1 << 8;
pub const LF_DEBUG_LEGACY_BANDS: u32 = 1 << 9;

/// Shared shading sub-struct embedded in both uniform blocks (d3a §6).
/// Sea level, Detail, layer and debug flags travel here — pure LIVE view
/// controls: writing them costs one uniform upload, never a rebake, and the
/// render-only guard test proves the world-building path cannot see them.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadeParams {
    /// Master seed low/high u32 lanes — the u64 never passes through f32.
    pub seed_lo: u32,
    pub seed_hi: u32,
    pub layer_flags: u32,
    /// Render-detail fBm octave count (sweep default; leg 4 finalizes).
    pub octaves: u32,
    pub sea_level_m: f32,
    /// Detail slider t × the default amplitude, meters. 0 = detail off.
    pub detail_amp_m: f32,
    /// Mean cell angular spacing sqrt(4π / cell_count), radians; the shader
    /// derives the base noise frequency from it.
    pub detail_cell_rad: f32,
    pub _pad: f32,
}

/// Pack the live view controls into the uniform sub-struct. This is the one
/// funnel through which Detail and sea level reach the GPU — the guard test
/// in worldgen.rs packs two extreme variants and proves the world-building
/// path is unaffected.
pub fn pack_shade_params(
    master_seed: u64,
    layer_flags: u32,
    octaves: u32,
    sea_level_m: f32,
    detail_amp_m: f32,
    cell_count: u32,
) -> ShadeParams {
    ShadeParams {
        seed_lo: (master_seed & 0xffff_ffff) as u32,
        seed_hi: (master_seed >> 32) as u32,
        layer_flags,
        octaves,
        sea_level_m,
        detail_amp_m,
        detail_cell_rad: (4.0 * std::f32::consts::PI / cell_count.max(1) as f32).sqrt(),
        _pad: 0.0,
    }
}

/// Build the `layer_flags` word for the active layer + debug toggles.
pub fn layer_flags(layer: Layer, debug_cell_bounds: bool, debug_legacy_bands: bool) -> u32 {
    let id = match layer {
        Layer::Elevation => 0u32,
        Layer::Plates => 1,
        Layer::CrustAge => 2,
        Layer::Thickness => 3,
    };
    id | if debug_cell_bounds {
        LF_DEBUG_CELL_BOUNDS
    } else {
        0
    } | if debug_legacy_bands {
        LF_DEBUG_LEGACY_BANDS
    } else {
        0
    }
}

/// Boundary-ribbon half-width in egui points (~1.8 pt full width); scaled by
/// pixels-per-point into the uniforms.
const BND_HALF_WIDTH_PT: f32 = 0.9;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlobeUniforms {
    rot: [[f32; 4]; 4],
    /// x, y: camera→NDC scale; z, w: canvas rect size in framebuffer px.
    params: [f32; 4],
    /// x: boundary-ribbon half-width in framebuffer px; y, z, w: spare.
    bnd: [f32; 4],
    shade: ShadeParams,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlatUniforms {
    center_px: [f32; 2],
    half_px: [f32; 2],
    misc: [f32; 4],
    /// x, y: cell-id texture dims; z, w: canvas rect min in framebuffer px.
    tex: [f32; 4],
    /// x: ribbon half-width px; y, z: canvas rect size px; w: spare.
    bnd: [f32; 4],
    shade: ShadeParams,
}

// ----- plate-boundary ribbons (d3a §8) -----

/// Globe ribbon vertex: this point, the next point (for the screen-space
/// direction), side (+/-1) and the boundary type. Stride 32.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BndVertex3 {
    p: [f32; 3],
    q: [f32; 3],
    side: f32,
    btype: f32,
}

/// Flat ribbon vertex in projected normalized map coordinates. Stride 24.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BndVertex2 {
    p: [f32; 2],
    q: [f32; 2],
    side: f32,
    btype: f32,
}

fn extrapolate3(last: [f32; 3], prev: [f32; 3]) -> [f32; 3] {
    [
        2.0 * last[0] - prev[0],
        2.0 * last[1] - prev[1],
        2.0 * last[2] - prev[2],
    ]
}

fn extrapolate2(last: [f32; 2], prev: [f32; 2]) -> [f32; 2] {
    [2.0 * last[0] - prev[0], 2.0 * last[1] - prev[1]]
}

/// Ribbon indices for one polyline starting at vertex `base`: two vertices
/// per point (side +1 then -1), 6 indices per segment; a closed polyline
/// adds the wrap segment.
fn ribbon_indices(indices: &mut Vec<u32>, base: u32, n: usize, closed: bool) {
    let segs = if closed { n } else { n - 1 };
    for s in 0..segs {
        let a = base + 2 * s as u32;
        let b = base + 2 * (((s + 1) % n) as u32);
        indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
    }
}

/// Ribbon geometry for the globe: chain points stay unit vectors; the VS
/// rotates, projects and expands them per frame.
fn build_globe_ribbons(set: &BoundarySet) -> (Vec<BndVertex3>, Vec<u32>) {
    let mut vs = Vec::new();
    let mut is = Vec::new();
    for ch in &set.chains {
        let n = ch.pts.len();
        if n < 2 {
            continue;
        }
        let base = vs.len() as u32;
        for j in 0..n {
            let p = ch.pts[j];
            let q = if ch.closed {
                ch.pts[(j + 1) % n]
            } else if j + 1 < n {
                ch.pts[j + 1]
            } else {
                extrapolate3(ch.pts[n - 1], ch.pts[n - 2])
            };
            let bt = ch.btype as f32;
            vs.push(BndVertex3 {
                p,
                q,
                side: 1.0,
                btype: bt,
            });
            vs.push(BndVertex3 {
                p,
                q,
                side: -1.0,
                btype: bt,
            });
        }
        ribbon_indices(&mut is, base, n, ch.closed);
    }
    (vs, is)
}

/// Ribbon geometry for the flat canvas: forward-project every chain point,
/// split any segment crossing the antimeridian at the projected map edge
/// (great-circle crossing latitude — projection-correct for the curved
/// Robinson / Eckert IV edges), then pack open/closed polylines in map
/// coordinates. Rebuilt only when the boundary set or projection changes.
fn build_flat_ribbons(
    set: &BoundarySet,
    proj: worldmaker_core::Projection,
) -> (Vec<BndVertex2>, Vec<u32>) {
    use std::f32::consts::PI;
    use worldmaker_core::grid::unit_to_latlon;
    let project = |u: [f32; 3]| -> [f32; 2] {
        let (lat, lon) = unit_to_latlon(u);
        let (x, y) = proj.project(lat, lon);
        [x, y]
    };
    let normalize = |v: [f32; 3]| -> [f32; 3] {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
        [v[0] / len, v[1] / len, v[2] / len]
    };

    let mut polylines: Vec<(u8, Vec<[f32; 2]>, bool)> = Vec::new();
    for ch in &set.chains {
        let n = ch.pts.len();
        if n < 2 {
            continue;
        }
        let mut pieces: Vec<Vec<[f32; 2]>> = Vec::new();
        let mut cur: Vec<[f32; 2]> = vec![project(ch.pts[0])];
        let mut split_any = false;
        let edges = if ch.closed { n } else { n - 1 };
        for e in 0..edges {
            let u = ch.pts[e];
            let v = ch.pts[(e + 1) % n];
            let (_, lon_u) = unit_to_latlon(u);
            let (_, lon_v) = unit_to_latlon(v);
            if (lon_u - lon_v).abs() > PI {
                // The longitudes wrap, so the chord crosses the y = 0 plane;
                // x < 0 there means the antimeridian (x >= 0 would be the
                // prime meridian near a pole — no split).
                let denom = u[1] - v[1];
                let t = if denom.abs() > 1e-12 {
                    u[1] / denom
                } else {
                    0.5
                };
                let c = normalize([
                    u[0] + (v[0] - u[0]) * t,
                    u[1] + (v[1] - u[1]) * t,
                    u[2] + (v[2] - u[2]) * t,
                ]);
                if c[0] < 0.0 {
                    let lat_c = c[2].clamp(-1.0, 1.0).asin();
                    let sign = if lon_u > 0.0 { 1.0 } else { -1.0 };
                    let (x_out, y_edge) = proj.project(lat_c, sign * PI);
                    let (x_in, _) = proj.project(lat_c, -sign * PI);
                    cur.push([x_out, y_edge]);
                    pieces.push(std::mem::take(&mut cur));
                    cur.push([x_in, y_edge]);
                    split_any = true;
                }
            }
            cur.push(project(v));
        }
        if ch.closed {
            if split_any {
                // The wrap edge ends back at the projection of pts[0], where
                // the first piece began: join last into first.
                pieces.push(std::mem::take(&mut cur));
                let first = pieces.remove(0);
                let last = pieces.last_mut().unwrap();
                last.extend(first.into_iter().skip(1));
                for pc in pieces {
                    polylines.push((ch.btype, pc, false));
                }
            } else {
                // cur = [p0, …, p_{n-1}, p0]: drop the duplicate and close.
                cur.pop();
                polylines.push((ch.btype, cur, true));
            }
        } else {
            pieces.push(std::mem::take(&mut cur));
            for pc in pieces {
                polylines.push((ch.btype, pc, false));
            }
        }
    }

    let mut vs = Vec::new();
    let mut is = Vec::new();
    for (bt, pts, closed) in polylines {
        let n = pts.len();
        if n < 2 {
            continue;
        }
        let base = vs.len() as u32;
        for j in 0..n {
            let p = pts[j];
            let q = if closed {
                pts[(j + 1) % n]
            } else if j + 1 < n {
                pts[j + 1]
            } else {
                extrapolate2(pts[n - 1], pts[n - 2])
            };
            let btf = bt as f32;
            vs.push(BndVertex2 {
                p,
                q,
                side: 1.0,
                btype: btf,
            });
            vs.push(BndVertex2 {
                p,
                q,
                side: -1.0,
                btype: btf,
            });
        }
        ribbon_indices(&mut is, base, n, closed);
    }
    (vs, is)
}

/// Upload one ribbon set; `None` when there is nothing to draw.
fn create_ribbon_buffers<V: bytemuck::Pod>(
    device: &wgpu::Device,
    label: &str,
    vs: &[V],
    is: &[u32],
) -> Option<(wgpu::Buffer, wgpu::Buffer, u32)> {
    if is.is_empty() {
        return None;
    }
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(vs),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(is),
        usage: wgpu::BufferUsages::INDEX,
    });
    Some((vb, ib, is.len() as u32))
}

pub struct SceneResources {
    globe_pipeline: wgpu::RenderPipeline,
    flat_pipeline: wgpu::RenderPipeline,
    bnd_globe_pipeline: wgpu::RenderPipeline,
    bnd_flat_pipeline: wgpu::RenderPipeline,
    globe_bind_layout: wgpu::BindGroupLayout,
    flat_bind_layout: wgpu::BindGroupLayout,
    globe_uniform: wgpu::Buffer,
    flat_uniform: wgpu::Buffer,
    /// 256×8 palette LUT baked once from layers.rs's Rust ramps; the same
    /// texture view is entered in BOTH bind groups.
    _palette_tex: wgpu::Texture,
    palette_view: wgpu::TextureView,

    // World-dependent resources (recreated when generations change).
    positions_buf: Option<wgpu::Buffer>,
    tri_ids_buf: Option<wgpu::Buffer>,
    /// CSR neighbor graph (flat walk, d3a §4): verbatim copies of
    /// `Grid::neighbor_offsets` / `Grid::neighbors`.
    nbr_offsets_buf: Option<wgpu::Buffer>,
    nbrs_buf: Option<wgpu::Buffer>,
    /// 3 × triangle count: the unindexed globe draw's vertex count.
    vertex_count: u32,
    values_buf: Option<wgpu::Buffer>,
    overlay_buf: Option<wgpu::Buffer>,
    cell_id_tex: Option<wgpu::Texture>,
    globe_bind: Option<wgpu::BindGroup>,
    flat_bind: Option<wgpu::BindGroup>,
    grid_gen: u64,
    values_gen: u64,
    overlay_gen: u64,
    /// Boundary-ribbon geometry (vertex buf, index buf, index count); None
    /// when the current boundary set draws nothing. Globe geometry depends
    /// only on the boundary set (keyed by values_gen — boundaries ride the
    /// values pass); flat geometry also depends on the projection.
    bnd_globe: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    bnd_globe_gen: u64,
    bnd_flat: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    bnd_flat_key: (u64, u32),
}

fn storage_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl SceneResources {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("worldmaker-shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });

        // Binding slots are shared between the two pipelines wherever the
        // WGSL module shares the resource (positions 1, values 3, overlay 4,
        // palette 5): module-scope vars serve both entry points, so shared
        // resources must agree on slots; per-canvas resources (uniforms at 0,
        // tri_ids / cell_ids at 2, the flat CSR graph at 6/7) may reuse or
        // extend the slot space because no single entry point touches both.
        let globe_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe-bind-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                storage_entry(1, wgpu::ShaderStages::VERTEX_FRAGMENT), // positions
                storage_entry(2, wgpu::ShaderStages::VERTEX),          // tri_ids
                storage_entry(3, wgpu::ShaderStages::FRAGMENT),        // cell_values
                storage_entry(4, wgpu::ShaderStages::FRAGMENT),        // overlay
                texture_entry(5, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });

        let flat_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flat-bind-layout"),
            entries: &[
                // VERTEX visibility for the boundary-ribbon VS.
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                storage_entry(1, wgpu::ShaderStages::FRAGMENT), // positions
                texture_entry(2, wgpu::TextureSampleType::Uint), // cell_ids hint
                storage_entry(3, wgpu::ShaderStages::FRAGMENT), // cell_values
                storage_entry(4, wgpu::ShaderStages::FRAGMENT), // overlay
                texture_entry(5, wgpu::TextureSampleType::Float { filterable: false }),
                storage_entry(6, wgpu::ShaderStages::FRAGMENT), // CSR offsets
                storage_entry(7, wgpu::ShaderStages::FRAGMENT), // CSR neighbors
            ],
        });

        let globe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globe-pipeline-layout"),
            bind_group_layouts: &[Some(&globe_bind_layout)],
            immediate_size: 0,
        });
        let flat_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flat-pipeline-layout"),
            bind_group_layouts: &[Some(&flat_bind_layout)],
            immediate_size: 0,
        });

        let globe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("globe-pipeline"),
            layout: Some(&globe_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_globe"),
                compilation_options: Default::default(),
                // Unindexed corner-fetch draw: no vertex buffers at all — the
                // vertex shader pulls corner ids + positions from storage.
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_globe"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let flat_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flat-pipeline"),
            layout: Some(&flat_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_flat"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_flat"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Boundary-ribbon pipelines (d3a §8): reuse each canvas's bind group
        // layout (the ribbon shaders statically use only the uniforms and
        // the palette), add a vertex buffer, and alpha-blend over the fill.
        let bnd_targets = [Some(wgpu::ColorTargetState {
            format: target_format,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let bnd_primitive = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        };
        let vec_attr = |dims: wgpu::VertexFormat, floats: u64| -> [wgpu::VertexAttribute; 4] {
            [
                wgpu::VertexAttribute {
                    format: dims,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: dims,
                    offset: 4 * floats,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 8 * floats,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 8 * floats + 4,
                    shader_location: 3,
                },
            ]
        };
        let globe_attrs = vec_attr(wgpu::VertexFormat::Float32x3, 3);
        let flat_attrs = vec_attr(wgpu::VertexFormat::Float32x2, 2);
        let bnd_globe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("boundary-globe-pipeline"),
            layout: Some(&globe_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_bnd_globe"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<BndVertex3>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &globe_attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_bnd"),
                compilation_options: Default::default(),
                targets: &bnd_targets,
            }),
            primitive: bnd_primitive,
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let bnd_flat_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("boundary-flat-pipeline"),
            layout: Some(&flat_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_bnd_flat"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<BndVertex2>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &flat_attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_bnd"),
                compilation_options: Default::default(),
                targets: &bnd_targets,
            }),
            primitive: bnd_primitive,
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let globe_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globe-uniforms"),
            size: std::mem::size_of::<GlobeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let flat_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flat-uniforms"),
            size: std::mem::size_of::<FlatUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Palette LUT: baked once from the Rust ramps, uploaded once, bound
        // to both pipelines. Rgba8Unorm (NOT -srgb): bytes pass through, all
        // color math stays in sRGB-encoded space (d3a §2.3 / D4).
        let palette_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("palette-lut"),
            size: wgpu::Extent3d {
                width: layers::LUT_W,
                height: layers::LUT_ROWS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &palette_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &layers::bake_palette_lut(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(layers::LUT_W * 4),
                rows_per_image: Some(layers::LUT_ROWS),
            },
            wgpu::Extent3d {
                width: layers::LUT_W,
                height: layers::LUT_ROWS,
                depth_or_array_layers: 1,
            },
        );
        let palette_view = palette_tex.create_view(&wgpu::TextureViewDescriptor::default());

        SceneResources {
            globe_pipeline,
            flat_pipeline,
            bnd_globe_pipeline,
            bnd_flat_pipeline,
            globe_bind_layout,
            flat_bind_layout,
            globe_uniform,
            flat_uniform,
            _palette_tex: palette_tex,
            palette_view,
            positions_buf: None,
            tri_ids_buf: None,
            nbr_offsets_buf: None,
            nbrs_buf: None,
            vertex_count: 0,
            values_buf: None,
            overlay_buf: None,
            cell_id_tex: None,
            globe_bind: None,
            flat_bind: None,
            grid_gen: 0,
            values_gen: 0,
            overlay_gen: 0,
            bnd_globe: None,
            bnd_globe_gen: 0,
            bnd_flat: None,
            bnd_flat_key: (0, u32::MAX),
        }
    }

    /// Upload whatever parts of the world changed since the last frame.
    fn sync_world(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, world: &WorldBundle) {
        let grid_changed = self.grid_gen != world.grid_gen;
        if grid_changed {
            let grid = &world.grid;
            self.positions_buf = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("cell-positions"),
                    contents: bytemuck::cast_slice(&grid.positions),
                    usage: wgpu::BufferUsages::STORAGE,
                },
            ));
            self.tri_ids_buf = Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("globe-tri-ids"),
                    contents: bytemuck::cast_slice(&grid.triangles),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
            );
            self.nbr_offsets_buf = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("csr-neighbor-offsets"),
                    contents: bytemuck::cast_slice(&grid.neighbor_offsets),
                    usage: wgpu::BufferUsages::STORAGE,
                },
            ));
            self.nbrs_buf = Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("csr-neighbors"),
                    contents: bytemuck::cast_slice(&grid.neighbors),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
            );
            self.vertex_count = (grid.triangles.len() * 3) as u32;

            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cell-id-texture"),
                size: wgpu::Extent3d {
                    width: CELL_ID_TEX_W,
                    height: CELL_ID_TEX_H,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(world.cell_ids.as_slice()),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(CELL_ID_TEX_W * 4),
                    rows_per_image: Some(CELL_ID_TEX_H),
                },
                wgpu::Extent3d {
                    width: CELL_ID_TEX_W,
                    height: CELL_ID_TEX_H,
                    depth_or_array_layers: 1,
                },
            );
            self.cell_id_tex = Some(tex);
            self.grid_gen = world.grid_gen;
            // values_gen and overlay_gen always bump with grid_gen, so the
            // right-sized buffers are recreated below and bind groups rebuilt.
        }

        let mut rebind = grid_changed;
        if self.values_gen != world.values_gen {
            let bytes: &[u8] = bytemuck::cast_slice(world.values.as_slice());
            match &self.values_buf {
                // Reuse the buffer when only contents changed (scrub, layer
                // switch): a queue write, no reallocation.
                Some(buf) if buf.size() == bytes.len() as u64 && !grid_changed => {
                    queue.write_buffer(buf, 0, bytes);
                }
                _ => {
                    self.values_buf = Some(device.create_buffer_init(
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("cell-values"),
                            contents: bytes,
                            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                        },
                    ));
                }
            }
            self.values_gen = world.values_gen;
            rebind = true;
        }

        if self.overlay_gen != world.overlay_gen {
            let bytes: &[u8] = bytemuck::cast_slice(world.overlay.as_slice());
            match &self.overlay_buf {
                Some(buf) if buf.size() == bytes.len() as u64 && !grid_changed => {
                    queue.write_buffer(buf, 0, bytes);
                }
                _ => {
                    self.overlay_buf = Some(device.create_buffer_init(
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("cell-overlay"),
                            contents: bytes,
                            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                        },
                    ));
                }
            }
            self.overlay_gen = world.overlay_gen;
            rebind = true;
        }

        if rebind {
            let positions = self.positions_buf.as_ref().unwrap();
            let tri_ids = self.tri_ids_buf.as_ref().unwrap();
            let values = self.values_buf.as_ref().unwrap();
            let overlay = self.overlay_buf.as_ref().unwrap();
            let cell_id_view = self
                .cell_id_tex
                .as_ref()
                .unwrap()
                .create_view(&wgpu::TextureViewDescriptor::default());
            self.globe_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("globe-bind"),
                layout: &self.globe_bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.globe_uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: positions.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: tri_ids.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: values.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: overlay.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&self.palette_view),
                    },
                ],
            }));
            self.flat_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("flat-bind"),
                layout: &self.flat_bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.flat_uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: positions.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&cell_id_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: values.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: overlay.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&self.palette_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: self.nbr_offsets_buf.as_ref().unwrap().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: self.nbrs_buf.as_ref().unwrap().as_entire_binding(),
                    },
                ],
            }));
        }
    }
}

/// Paint callback for the globe canvas.
pub struct GlobeCallback {
    pub world: Arc<WorldBundle>,
    pub view: GlobeView,
    pub shade: ShadeParams,
    /// Canvas rect in points: (center x, center y, width, height).
    pub rect_points: [f32; 4],
}

impl egui_wgpu::CallbackTrait for GlobeCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let r: &mut SceneResources = resources.get_mut().expect("SceneResources missing");
        r.sync_world(device, queue, &self.world);

        let ppp = screen.pixels_per_point;
        let w_px = self.rect_points[2] * ppp;
        let h_px = self.rect_points[3] * ppp;
        let radius_px =
            globe_radius_px(self.rect_points[2], self.rect_points[3], self.view.zoom) * ppp;
        let uniforms = GlobeUniforms {
            rot: globe_rotation(self.view.yaw, self.view.pitch),
            params: [
                2.0 * radius_px / w_px.max(1.0),
                2.0 * radius_px / h_px.max(1.0),
                w_px,
                h_px,
            ],
            bnd: [BND_HALF_WIDTH_PT * ppp, 0.0, 0.0, 0.0],
            shade: self.shade,
        };
        queue.write_buffer(&r.globe_uniform, 0, bytemuck::bytes_of(&uniforms));

        // Boundary ribbons follow the values pass (empty set → None).
        if r.bnd_globe_gen != self.world.values_gen {
            r.bnd_globe_gen = self.world.values_gen;
            let (vs, is) = build_globe_ribbons(&self.world.boundaries);
            r.bnd_globe = create_ribbon_buffers(device, "boundary-globe", &vs, &is);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::epaint::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let r: &SceneResources = resources.get().expect("SceneResources missing");
        let Some(bind) = &r.globe_bind else { return };
        if r.vertex_count == 0 {
            return;
        }
        render_pass.set_pipeline(&r.globe_pipeline);
        render_pass.set_bind_group(0, bind, &[]);
        render_pass.draw(0..r.vertex_count, 0..1);
        // Smoothed plate-boundary ribbons over the fill (same pass, no
        // depth buffer; the FS discards the back hemisphere).
        if let Some((vb, ib, n)) = &r.bnd_globe {
            render_pass.set_pipeline(&r.bnd_globe_pipeline);
            render_pass.set_bind_group(0, bind, &[]);
            render_pass.set_vertex_buffer(0, vb.slice(..));
            render_pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..*n, 0, 0..1);
        }
    }
}

/// Globe radius in points for a given canvas size and zoom.
pub fn globe_radius_px(w: f32, h: f32, zoom: f32) -> f32 {
    0.45 * w.min(h) * zoom
}

/// Paint callback for the flat map canvas.
pub struct FlatCallback {
    pub world: Arc<WorldBundle>,
    pub view: FlatView,
    pub shade: ShadeParams,
    /// Canvas rect in points: (center x, center y, width, height).
    pub rect_points: [f32; 4],
}

/// Base map half-extents in points for a rect and projection at zoom 1
/// (fit-to-rect).
pub fn flat_base_half_extents(
    proj: worldmaker_core::Projection,
    rect_w: f32,
    rect_h: f32,
) -> [f32; 2] {
    let aspect = proj.aspect();
    let h = rect_h.min(rect_w / aspect);
    [h * aspect / 2.0, h / 2.0]
}

impl egui_wgpu::CallbackTrait for FlatCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let r: &mut SceneResources = resources.get_mut().expect("SceneResources missing");
        r.sync_world(device, queue, &self.world);

        let ppp = screen.pixels_per_point;
        let [cx, cy, w, h] = self.rect_points;
        let base = flat_base_half_extents(self.view.projection, w, h);
        let proj_code = match self.view.projection {
            worldmaker_core::Projection::Equirectangular => 0u32,
            worldmaker_core::Projection::Robinson => 1,
            worldmaker_core::Projection::EckertIv => 2,
        };
        let uniforms = FlatUniforms {
            center_px: [(cx + self.view.pan[0]) * ppp, (cy + self.view.pan[1]) * ppp],
            half_px: [
                (base[0] * self.view.zoom * ppp).max(1.0),
                (base[1] * self.view.zoom * ppp).max(1.0),
            ],
            misc: [
                proj_code as f32,
                0.0,
                if self.view.graticule { 1.0 } else { 0.0 },
                0.0,
            ],
            tex: [
                CELL_ID_TEX_W as f32,
                CELL_ID_TEX_H as f32,
                (cx - w / 2.0) * ppp,
                (cy - h / 2.0) * ppp,
            ],
            bnd: [BND_HALF_WIDTH_PT * ppp, w * ppp, h * ppp, 0.0],
            shade: self.shade,
        };
        queue.write_buffer(&r.flat_uniform, 0, bytemuck::bytes_of(&uniforms));

        // Boundary ribbons: chains are projected CPU-side, so the geometry
        // depends on (boundary set, projection); cached on that key.
        if r.bnd_flat_key != (self.world.values_gen, proj_code) {
            r.bnd_flat_key = (self.world.values_gen, proj_code);
            let (vs, is) = build_flat_ribbons(&self.world.boundaries, self.view.projection);
            r.bnd_flat = create_ribbon_buffers(device, "boundary-flat", &vs, &is);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::epaint::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let r: &SceneResources = resources.get().expect("SceneResources missing");
        let Some(bind) = &r.flat_bind else { return };
        render_pass.set_pipeline(&r.flat_pipeline);
        render_pass.set_bind_group(0, bind, &[]);
        render_pass.draw(0..3, 0..1);
        // Smoothed plate-boundary ribbons over the map fill.
        if let Some((vb, ib, n)) = &r.bnd_flat {
            render_pass.set_pipeline(&r.bnd_flat_pipeline);
            render_pass.set_bind_group(0, bind, &[]);
            render_pass.set_vertex_buffer(0, vb.slice(..));
            render_pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..*n, 0, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5 && (a[2] - b[2]).abs() < 1e-5
    }

    #[test]
    fn rotation_at_identity_view_puts_north_up_and_lon0_forward() {
        let m = globe_rotation(0.0, 0.0);
        // North pole (world +z) points up on screen (+y).
        assert!(close(rotate(&m, [0.0, 0.0, 1.0]), [0.0, 1.0, 0.0]));
        // Lat 0 / lon 0 (world +x) faces the viewer (+z).
        assert!(close(rotate(&m, [1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]));
        // East of it (world +y) is screen right (+x).
        assert!(close(rotate(&m, [0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]));
    }

    #[test]
    fn rotate_inv_is_the_inverse() {
        let m = globe_rotation(0.73, -0.41);
        for v in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.6, -0.64, 0.48],
        ] {
            assert!(close(rotate_inv(&m, rotate(&m, v)), v));
            assert!(close(rotate(&m, rotate_inv(&m, v)), v));
        }
    }

    #[test]
    fn pitch_looks_at_that_latitude() {
        // At pitch p, the view center (screen center, toward viewer) must be
        // the ground point at latitude p, longitude -yaw.
        let (yaw, pitch) = (0.9f32, 0.5f32);
        let m = globe_rotation(yaw, pitch);
        let ground = [
            pitch.cos() * (-yaw).cos(),
            pitch.cos() * (-yaw).sin(),
            pitch.sin(),
        ];
        assert!(close(rotate(&m, ground), [0.0, 0.0, 1.0]));
    }

    #[test]
    fn shade_params_pack_seed_as_u32_lanes_and_uniform_sizes_match_wgsl() {
        let sp = pack_shade_params(0xdead_beef_1234_5678, 1, 5, -120.0, 220.0, 163_842);
        assert_eq!(sp.seed_lo, 0x1234_5678);
        assert_eq!(sp.seed_hi, 0xdead_beef);
        // WGSL struct sizes the Rust mirrors must match (d3a §6; the bnd
        // vec4 for boundary ribbons grew both blocks in leg 3).
        assert_eq!(std::mem::size_of::<ShadeParams>(), 32);
        assert_eq!(std::mem::size_of::<GlobeUniforms>(), 128);
        assert_eq!(std::mem::size_of::<FlatUniforms>(), 96);
    }

    // ----- flat exact-walk CPU mirror (d3a §4.5 as amended by A3) -----
    //
    // `walk_from_hint` / `wedge_and_weights` transliterate fs_flat's WGSL in
    // f32: the hint walk with the R1 tie rule, the shared-g wedge scan with
    // B1's corrected sign, and the R2 differenced barycentric solve. The
    // reference is a deliberately INDEPENDENT f64 formulation — brute-force
    // three-half-space containment over the winner's fan plus a raw Cramer
    // solve of [P_c P_a P_b]·β = p — so a sign or wedge error in the mirror
    // cannot reproduce on both sides and cancel out of the comparison (the
    // circularity A3 flagged in the original test spec).
    mod flat_walk {
        use super::super::{CELL_ID_TEX_H, CELL_ID_TEX_W};
        use worldmaker_core::grid::{unit_to_latlon, Grid};
        use worldmaker_core::hash::splitmix64;

        /// Must match WALK_CAP in shaders.wgsl.
        const WALK_CAP: u32 = 4;

        fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
            a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
        }
        fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        }
        fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
            [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
        }
        fn tod(a: [f32; 3]) -> [f64; 3] {
            [a[0] as f64, a[1] as f64, a[2] as f64]
        }
        fn cross3d(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        }
        /// Scalar triple product a · (b × c) in f64.
        fn trip(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
            let x = cross3d(b, c);
            a[0] * x[0] + a[1] * x[1] + a[2] * x[2]
        }

        /// What the hint raster stores in `p`'s texel: the nearest cell to
        /// the texel CENTER. nearest_cell's greedy ascent maximizes a linear
        /// function over the vertex graph of a convex polytope, so its result
        /// is hint-independent — this equals the committed raster value
        /// without building 8.4M-texel rasters per level in a test.
        fn raster_hint(grid: &Grid, p: [f32; 3]) -> u32 {
            let (lat, lon) = unit_to_latlon(p);
            let w = CELL_ID_TEX_W as f32;
            let h = CELL_ID_TEX_H as f32;
            // Texel pick exactly as fs_flat computes tx/ty (truncation).
            let pi = std::f32::consts::PI;
            let x = ((lon + pi) / (2.0 * pi) * w).clamp(0.0, w - 1.0) as i32 as f32;
            let y = ((0.5 - lat / pi) * h).clamp(0.0, h - 1.0) as i32 as f32;
            // Texel center exactly as rasterize_cell_ids computes it.
            let tlat = std::f32::consts::FRAC_PI_2 * (1.0 - 2.0 * (y + 0.5) / h);
            let tlon = pi * (2.0 * (x + 0.5) / w - 1.0);
            let (cl, sl) = (tlat.cos(), tlat.sin());
            grid.nearest_cell([cl * tlon.cos(), cl * tlon.sin(), sl], None)
        }

        /// f32 mirror of fs_flat's hint→winner walk (R1 tie rule, cap 4).
        fn walk_from_hint(grid: &Grid, hint: u32, p: [f32; 3]) -> u32 {
            let mut c = hint;
            let mut best_d = dot3(p, grid.positions[c as usize]);
            for _ in 0..WALK_CAP {
                let mut best = c;
                let mut bd = best_d;
                for &nb in grid.neighbors_of(c) {
                    let d = dot3(p, grid.positions[nb as usize]);
                    if d > bd || (d == bd && nb < best) {
                        best = nb;
                        bd = d;
                    }
                }
                if best == c {
                    break;
                }
                c = best;
                best_d = bd;
            }
            c
        }

        /// f32 mirror of fs_flat's wedge scan + differenced barycentrics.
        /// Returns (wedge index, weights ordered (c, n_i, n_{i+1})).
        fn wedge_and_weights(grid: &Grid, c: u32, p: [f32; 3]) -> (usize, [f32; 3]) {
            let ring = grid.neighbors_of(c);
            let k = ring.len();
            let pc = grid.positions[c as usize];
            let mut g = [0f32; 6];
            for (j, &nj) in ring.iter().enumerate() {
                g[j] = dot3(p, cross3(pc, grid.positions[nj as usize]));
            }
            let mut wedge = usize::MAX;
            let mut fb = 0usize;
            let mut fb_score = f32::NEG_INFINITY;
            for i in 0..k {
                let i1 = (i + 1) % k;
                if wedge == usize::MAX && g[i] >= 0.0 && g[i1] <= 0.0 {
                    wedge = i;
                }
                let score = g[i].min(-g[i1]);
                if score > fb_score {
                    fb_score = score;
                    fb = i;
                }
            }
            if wedge == usize::MAX {
                wedge = fb;
            }
            let pa = grid.positions[ring[wedge] as usize];
            let pb = grid.positions[ring[(wedge + 1) % k] as usize];
            let e1 = sub3(pa, pc);
            let e2 = sub3(pb, pc);
            let n = cross3(e1, e2);
            let t = dot3(pc, n) / dot3(p, n);
            let dq = sub3([t * p[0], t * p[1], t * p[2]], pc);
            let inv_nn = 1.0 / dot3(n, n);
            let wa = dot3(cross3(dq, e2), n) * inv_nn;
            let wb = dot3(cross3(e1, dq), n) * inv_nn;
            (wedge, [1.0 - wa - wb, wa, wb])
        }

        /// Independent f64 wedge reference (A3): standard three-half-space
        /// containment over the winner's fan, first match in ring order.
        fn reference_wedge(grid: &Grid, c: u32, p: [f32; 3]) -> usize {
            let ring = grid.neighbors_of(c);
            let k = ring.len();
            let pd = tod(p);
            let pc = tod(grid.positions[c as usize]);
            let mut wedge = usize::MAX;
            let mut fb = 0usize;
            let mut fb_score = f64::NEG_INFINITY;
            for i in 0..k {
                let pa = tod(grid.positions[ring[i] as usize]);
                let pb = tod(grid.positions[ring[(i + 1) % k] as usize]);
                let h1 = trip(pd, pc, pa);
                let h2 = trip(pd, pa, pb);
                let h3 = trip(pd, pb, pc);
                if wedge == usize::MAX && h1 >= 0.0 && h2 >= 0.0 && h3 >= 0.0 {
                    wedge = i;
                }
                let score = h1.min(h2).min(h3);
                if score > fb_score {
                    fb_score = score;
                    fb = i;
                }
            }
            if wedge == usize::MAX {
                wedge = fb;
            }
            wedge
        }

        /// Independent f64 weight reference (A3): raw Cramer solve of
        /// [P_c P_a P_b]·β = p for a GIVEN wedge, normalized so the
        /// ray-plane scale folds out — a different formulation from the
        /// mirror's differenced solve, in f64 where the raw form is exact
        /// to ~1e-10 even at L9.
        fn reference_weights(grid: &Grid, c: u32, wedge: usize, p: [f32; 3]) -> [f64; 3] {
            let ring = grid.neighbors_of(c);
            let k = ring.len();
            let pd = tod(p);
            let pc = tod(grid.positions[c as usize]);
            let pa = tod(grid.positions[ring[wedge] as usize]);
            let pb = tod(grid.positions[ring[(wedge + 1) % k] as usize]);
            let det = trip(pc, pa, pb);
            let bc = trip(pd, pa, pb) / det;
            let ba = trip(pc, pd, pb) / det;
            let bb = trip(pc, pa, pd) / det;
            let s = bc + ba + bb;
            [bc / s, ba / s, bb / s]
        }

        /// Deterministic unit vectors from a fixed splitmix64 stream.
        fn sample_unit(stream: u64, i: u64) -> [f32; 3] {
            let a = splitmix64(stream.wrapping_add(2 * i + 1));
            let b = splitmix64(stream.wrapping_add(2 * i + 2));
            let z = (a >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            let lon = ((b >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * std::f64::consts::PI;
            let r = (1.0 - z * z).max(0.0).sqrt();
            [(r * lon.cos()) as f32, (r * lon.sin()) as f32, z as f32]
        }

        /// Full per-point property: winner bit-equality vs nearest_cell,
        /// convergence within the cap, wedge equality vs the independent
        /// f64 containment reference (except inside the f64-measured
        /// boundary band, where two adjacent wedges are equally valid), and
        /// weights within 1e-4 of the independent f64 Cramer solve for the
        /// mirror's own triple (the R2 conditioning claim).
        fn check_point(grid: &Grid, p: [f32; 3], on_bisector: bool) {
            let truth = grid.nearest_cell(p, None);
            let hint = raster_hint(grid, p);
            let win = walk_from_hint(grid, hint, p);
            assert_eq!(win, truth, "walk winner != nearest_cell at p = {p:?}");
            // The cap did not truncate: no ring neighbor beats the winner
            // under the R1 step rule.
            let wd = dot3(p, grid.positions[win as usize]);
            for &nb in grid.neighbors_of(win) {
                let d = dot3(p, grid.positions[nb as usize]);
                assert!(
                    d < wd || (d == wd && nb > win),
                    "walk terminated on a non-optimal cell at p = {p:?}"
                );
            }
            let (wedge, w32) = wedge_and_weights(grid, win, p);
            let rwedge = reference_wedge(grid, win, p);
            // Wedge selection: strict equality, except where the f64
            // reference itself measures the point inside the f32 noise band
            // of the SHARED boundary plane between the two (adjacent) picks
            // — there both wedges are equally valid and the interpolant is
            // continuous across the plane. The f64 gate means a real sign
            // error (whose wrong wedge shows up at LARGE |g|) can never
            // hide in this exception. Cell-pair midpoints sit exactly on
            // such a plane by construction.
            if wedge != rwedge && !on_bisector {
                let ring = grid.neighbors_of(win);
                let k = ring.len();
                let shared = if (rwedge + 1) % k == wedge {
                    Some(wedge)
                } else if (wedge + 1) % k == rwedge {
                    Some(rwedge)
                } else {
                    None
                };
                let in_band = shared.is_some_and(|s| {
                    let pn = tod(grid.positions[ring[s] as usize]);
                    trip(tod(p), tod(grid.positions[win as usize]), pn).abs() <= 1e-6
                });
                assert!(
                    in_band,
                    "wedge mismatch outside the boundary band at p = {p:?}: \
                     mirror {wedge}, reference {rwedge}"
                );
            }
            // Weight accuracy (R2): the mirror's f32 differenced solve vs
            // the independent f64 raw-Cramer solve of the SAME triple.
            let wref = reference_weights(grid, win, wedge, p);
            for (j, (&wv, &rv)) in w32.iter().zip(wref.iter()).enumerate() {
                let err = (wv as f64 - rv).abs();
                assert!(
                    err <= 1e-4,
                    "weight {j} off by {err} at p = {p:?} (wedge {wedge})"
                );
            }
            // Barycentric sanity: the point is inside (or within noise of)
            // its wedge, so no weight may be strongly negative — a wrong
            // wedge scan (the B1 sign error) extrapolates hard and fails
            // this immediately.
            for (j, &wv) in w32.iter().enumerate() {
                assert!(
                    (-0.05..=1.05).contains(&wv),
                    "weight {j} = {wv} out of range at p = {p:?}"
                );
            }
        }

        /// Adjacent-pair midpoints sit exactly on Voronoi bisectors; in f32
        /// many produce EXACT dot ties, exercising the R1 tie rule for real.
        /// Returns how many exact ties were seen.
        fn check_midpoints(grid: &Grid, pairs: u64, stream: u64) -> u64 {
            let n = grid.cell_count() as u64;
            let mut ties = 0;
            for i in 0..pairs {
                let a = (splitmix64(stream.wrapping_add(3 * i)) % n) as u32;
                let ring = grid.neighbors_of(a);
                let pick = splitmix64(stream.wrapping_add(3 * i + 1)) % ring.len() as u64;
                let b = ring[pick as usize];
                let pa = grid.positions[a as usize];
                let pb = grid.positions[b as usize];
                let m = [pa[0] + pb[0], pa[1] + pb[1], pa[2] + pb[2]];
                let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
                let p = [m[0] / len, m[1] / len, m[2] / len];
                if dot3(p, pa) == dot3(p, pb) {
                    ties += 1;
                }
                check_point(grid, p, true);
            }
            ties
        }

        #[test]
        fn flat_walk_matches_nearest_cell_and_f64_reference() {
            let mut ties = 0;
            for (level, samples, pairs) in [(6u32, 3000u64, 800u64), (7, 2000, 400), (8, 1200, 200)]
            {
                let grid = Grid::build(level);
                for i in 0..samples {
                    check_point(&grid, sample_unit(0x5eed_0000 + level as u64, i), false);
                }
                ties += check_midpoints(&grid, pairs, 0x0bad_cafe + level as u64);
            }
            assert!(ties > 0, "no exact f32 dot tie was exercised (R1 untested)");
        }

        #[test]
        fn flat_walk_l9_spot_samples() {
            let grid = Grid::build(9);
            for i in 0..200 {
                check_point(&grid, sample_unit(0x5eed_1009, i), false);
            }
            check_midpoints(&grid, 100, 0x0bad_f00d);
        }
    }

    /// The WGSL Eckert IV inverse arm (shaders.wgsl `map_invert`, proj > 1.5)
    /// transliterated in f32, asserted against core `Projection::invert` to
    /// the strict-gate standard: identical accept/reject decisions on a dense
    /// grid over and beyond the outline, values within f32 trig tolerance.
    #[test]
    fn wgsl_eckert_inverse_arm_matches_cpu_invert() {
        use std::f32::consts::PI;
        let wgsl_arm = |mx: f32, my: f32| -> Option<(f32, f32)> {
            if my.abs() > 1.0 {
                return None;
            }
            let theta = my.clamp(-1.0, 1.0).asin();
            let s = theta.sin();
            let c = theta.cos();
            let lat = ((theta + s * c + 2.0 * s) / (2.0 + PI * 0.5))
                .clamp(-1.0, 1.0)
                .asin();
            let lon = 2.0 * PI * mx / (1.0 + c);
            if lon.abs() > PI * 1.0001 {
                return None;
            }
            Some((lat, lon.clamp(-PI, PI)))
        };
        let proj = worldmaker_core::Projection::EckertIv;
        for iy in -110..=110 {
            for ix in -110..=110 {
                let mx = ix as f32 * 0.01;
                let my = iy as f32 * 0.01;
                let cpu = proj.invert(mx, my);
                let gpu = wgsl_arm(mx, my);
                match (cpu, gpu) {
                    (None, None) => {}
                    (Some((la, lo)), Some((lb, lob))) => {
                        assert!(
                            (la - lb).abs() < 1e-6 && (lo - lob).abs() < 1e-6,
                            "value drift at ({mx}, {my}): cpu ({la}, {lo}) wgsl ({lb}, {lob})"
                        );
                    }
                    _ => panic!(
                        "accept/reject mismatch at ({mx}, {my}): cpu {:?} wgsl {:?}",
                        cpu, gpu
                    ),
                }
            }
        }
    }
}
