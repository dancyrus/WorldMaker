//! wgpu rendering for the globe and flat canvases, driven through
//! egui_wgpu paint callbacks.
//!
//! `SceneResources` lives in the egui renderer's callback resources. Each
//! frame the UI submits a `GlobeCallback` / `FlatCallback` carrying the
//! current `WorldBundle` (grid + elevation + cell-id raster) and view
//! parameters; `prepare` uploads whatever changed (tracked by generation
//! counters) and `paint` draws into egui's render pass.

use std::sync::Arc;

use eframe::egui_wgpu::{self, wgpu};
use eframe::wgpu::util::DeviceExt;

use worldmaker_core::Grid;

/// Width of the equirectangular cell-id lookup raster. Depends only on the
/// grid level, so seed and sea-level changes never touch it.
pub const CELL_ID_TEX_W: u32 = 4096;
pub const CELL_ID_TEX_H: u32 = 2048;

/// Immutable snapshot of the world the renderer draws. Rebuilt (as a new Arc)
/// when the grid or the elevation field changes.
pub struct WorldBundle {
    pub grid: Arc<Grid>,
    pub elevation: Vec<f32>,
    /// Equirectangular raster of cell ids, CELL_ID_TEX_W × CELL_ID_TEX_H.
    pub cell_ids: Vec<u32>,
    /// Bumped when the grid (and cell_ids) change.
    pub grid_gen: u64,
    /// Bumped when the elevation field changes (includes grid changes).
    pub field_gen: u64,
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
    pub sea_level_m: f32,
}

/// View parameters for the flat canvas, in egui points.
#[derive(Clone, Copy)]
pub struct FlatView {
    pub projection: worldmaker_core::Projection,
    /// Pan offset of the map center relative to the rect center, in points.
    pub pan: [f32; 2],
    pub zoom: f32,
    pub sea_level_m: f32,
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlobeUniforms {
    rot: [[f32; 4]; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlatUniforms {
    center_px: [f32; 2],
    half_px: [f32; 2],
    misc: [f32; 4],
    tex: [f32; 4],
}

pub struct SceneResources {
    globe_pipeline: wgpu::RenderPipeline,
    flat_pipeline: wgpu::RenderPipeline,
    globe_bind_layout: wgpu::BindGroupLayout,
    flat_bind_layout: wgpu::BindGroupLayout,
    globe_uniform: wgpu::Buffer,
    flat_uniform: wgpu::Buffer,

    // World-dependent resources (recreated when generations change).
    vertex_buf: Option<wgpu::Buffer>,
    index_buf: Option<wgpu::Buffer>,
    index_count: u32,
    elevation_buf: Option<wgpu::Buffer>,
    cell_id_tex: Option<wgpu::Texture>,
    globe_bind: Option<wgpu::BindGroup>,
    flat_bind: Option<wgpu::BindGroup>,
    grid_gen: u64,
    field_gen: u64,
}

impl SceneResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("worldmaker-shaders"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });

        let globe_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let flat_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flat-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
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

        SceneResources {
            globe_pipeline,
            flat_pipeline,
            globe_bind_layout,
            flat_bind_layout,
            globe_uniform,
            flat_uniform,
            vertex_buf: None,
            index_buf: None,
            index_count: 0,
            elevation_buf: None,
            cell_id_tex: None,
            globe_bind: None,
            flat_bind: None,
            grid_gen: 0,
            field_gen: 0,
        }
    }

    /// Upload whatever parts of the world changed since the last frame.
    fn sync_world(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, world: &WorldBundle) {
        if self.grid_gen != world.grid_gen {
            let grid = &world.grid;
            let flat_positions: &[f32] = bytemuck::cast_slice(&grid.positions);
            self.vertex_buf = Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("globe-vertices"),
                    contents: bytemuck::cast_slice(flat_positions),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            );
            let indices: &[u32] = bytemuck::cast_slice(&grid.triangles);
            self.index_buf = Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("globe-indices"),
                    contents: bytemuck::cast_slice(indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
            );
            self.index_count = indices.len() as u32;

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
                bytemuck::cast_slice(&world.cell_ids),
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
            // Elevation buffer is recreated below (field_gen always bumps with
            // grid_gen); bind groups are rebuilt after that.
        }

        if self.field_gen != world.field_gen {
            self.elevation_buf = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("elevation-field"),
                    contents: bytemuck::cast_slice(&world.elevation),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                },
            ));
            let elev = self.elevation_buf.as_ref().unwrap();
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
                        resource: elev.as_entire_binding(),
                    },
                ],
            }));
            let tex_view = self
                .cell_id_tex
                .as_ref()
                .unwrap()
                .create_view(&wgpu::TextureViewDescriptor::default());
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
                        resource: elev.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&tex_view),
                    },
                ],
            }));
            self.grid_gen = world.grid_gen;
            self.field_gen = world.field_gen;
        }
    }
}

/// Paint callback for the globe canvas.
pub struct GlobeCallback {
    pub world: Arc<WorldBundle>,
    pub view: GlobeView,
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
                self.view.sea_level_m,
                0.0,
            ],
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
        let (Some(vb), Some(ib), Some(bind)) = (&r.vertex_buf, &r.index_buf, &r.globe_bind) else {
            return;
        };
        render_pass.set_pipeline(&r.globe_pipeline);
        render_pass.set_bind_group(0, bind, &[]);
        render_pass.set_vertex_buffer(0, vb.slice(..));
        render_pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..r.index_count, 0, 0..1);
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
                },
                self.view.sea_level_m,
                if self.view.graticule { 1.0 } else { 0.0 },
                0.0,
            ],
            tex: [CELL_ID_TEX_W as f32, CELL_ID_TEX_H as f32, 0.0, 0.0],
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
}
