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
    /// Arc'd: depends only on the grid level. Leg 3 demotes it from flat
    /// truth to walk hint (d3a §4).
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlobeUniforms {
    rot: [[f32; 4]; 4],
    params: [f32; 4],
    shade: ShadeParams,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlatUniforms {
    center_px: [f32; 2],
    half_px: [f32; 2],
    misc: [f32; 4],
    tex: [f32; 4],
    shade: ShadeParams,
}

pub struct SceneResources {
    globe_pipeline: wgpu::RenderPipeline,
    flat_pipeline: wgpu::RenderPipeline,
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
        // tri_ids / cell_ids at 2) may reuse each other's slots because no
        // single entry point touches both.
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
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                storage_entry(1, wgpu::ShaderStages::FRAGMENT), // positions
                texture_entry(2, wgpu::TextureSampleType::Uint), // cell_ids hint
                storage_entry(3, wgpu::ShaderStages::FRAGMENT), // cell_values
                storage_entry(4, wgpu::ShaderStages::FRAGMENT), // overlay
                texture_entry(5, wgpu::TextureSampleType::Float { filterable: false }),
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
            globe_bind_layout,
            flat_bind_layout,
            globe_uniform,
            flat_uniform,
            _palette_tex: palette_tex,
            palette_view,
            positions_buf: None,
            tri_ids_buf: None,
            vertex_count: 0,
            values_buf: None,
            overlay_buf: None,
            cell_id_tex: None,
            globe_bind: None,
            flat_bind: None,
            grid_gen: 0,
            values_gen: 0,
            overlay_gen: 0,
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
                0.0,
                0.0,
            ],
            shade: self.shade,
        };
        queue.write_buffer(&r.globe_uniform, 0, bytemuck::bytes_of(&uniforms));
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
        let uniforms = FlatUniforms {
            center_px: [(cx + self.view.pan[0]) * ppp, (cy + self.view.pan[1]) * ppp],
            half_px: [
                (base[0] * self.view.zoom * ppp).max(1.0),
                (base[1] * self.view.zoom * ppp).max(1.0),
            ],
            misc: [
                match self.view.projection {
                    worldmaker_core::Projection::Equirectangular => 0.0,
                    worldmaker_core::Projection::Robinson => 1.0,
                    worldmaker_core::Projection::EckertIv => 2.0,
                },
                0.0,
                if self.view.graticule { 1.0 } else { 0.0 },
                0.0,
            ],
            tex: [CELL_ID_TEX_W as f32, CELL_ID_TEX_H as f32, 0.0, 0.0],
            shade: self.shade,
        };
        queue.write_buffer(&r.flat_uniform, 0, bytemuck::bytes_of(&uniforms));
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
        // WGSL struct sizes the Rust mirrors must match (d3a §6).
        assert_eq!(std::mem::size_of::<ShadeParams>(), 32);
        assert_eq!(std::mem::size_of::<GlobeUniforms>(), 112);
        assert_eq!(std::mem::size_of::<FlatUniforms>(), 80);
    }
}
