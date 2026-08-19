//! The WorldMaker application: UI, canvases, input, and the scripted
//! screenshot / perf-harness modes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;

use worldmaker_core::grid::{latlon_to_unit, Grid};
use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Projection;
use worldmaker_sim::noise_stage::ELEVATION_FIELD;
use worldmaker_sim::{NoiseElevationStage, Pipeline, StageContext, WorldState};

use crate::render::{
    flat_base_half_extents, globe_radius_px, globe_rotation, rotate_inv, FlatCallback, FlatView,
    GlobeCallback, GlobeView, SceneResources, WorldBundle,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Globe,
    Flat,
    Split,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Preset {
    Draft6,
    Standard7,
    High8,
}

impl Preset {
    fn level(self) -> u32 {
        match self {
            Preset::Draft6 => 6,
            Preset::Standard7 => 7,
            Preset::High8 => 8,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Preset::Draft6 => "Draft (L6, 41k cells)",
            Preset::Standard7 => "Standard (L7, 164k cells)",
            Preset::High8 => "High (L8, 655k cells)",
        }
    }
}

/// Scripted modes, driven from the command line.
pub struct Script {
    pub screenshots_dir: Option<PathBuf>,
    pub perf_out: Option<PathBuf>,
    /// Grid-build timings measured in main() before the window opened.
    pub grid_build_ms: Vec<(u32, f64)>,
}

enum ScriptState {
    Idle,
    Shot {
        stage: usize,
        frames: u32,
        requested: bool,
    },
    Perf {
        stage: usize,
        frames: u32,
        started: Option<Instant>,
        fps: Vec<(String, f64)>,
    },
    Closing,
}

pub struct WorldApp {
    // World state.
    grid: Arc<Grid>,
    pipeline: Pipeline,
    world_state: WorldState,
    bundle: Arc<WorldBundle>,
    grid_gen: u64,
    field_gen: u64,
    last_grid_build_ms: f64,

    // Controls.
    seed_text: String,
    master_seed: u64,
    sea_level_m: f32,
    view_mode: ViewMode,
    preset: Preset,
    projection: Projection,
    graticule: bool,
    layer: &'static str,

    // Canvas view state.
    globe: GlobeView,
    flat_pan: [f32; 2],
    flat_zoom: f32,

    // Cursor readout: (canvas name, cell id, lat deg, lon deg).
    hover: Option<(&'static str, u32, f32, f32)>,
    pick_hint: Option<u32>,

    // FPS.
    frame_times: Vec<f32>,
    last_frame: Instant,

    // Scripted modes.
    script: Script,
    script_state: ScriptState,
}

impl WorldApp {
    pub fn new(cc: &eframe::CreationContext<'_>, script: Script) -> Self {
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("wgpu render state missing — eframe was not started with the wgpu backend");
        render_state
            .renderer
            .write()
            .callback_resources
            .insert(SceneResources::new(
                &render_state.device,
                render_state.target_format,
            ));

        let seed_text = "cyrus".to_string();
        let master_seed = seed_from_text(&seed_text);
        let mut app = WorldApp {
            grid: Arc::new(Grid::build(0)), // replaced by rebuild_grid below
            pipeline: Pipeline::new(),
            world_state: WorldState::new(Arc::new(Grid::build(0))),
            bundle: Arc::new(WorldBundle {
                grid: Arc::new(Grid::build(0)),
                elevation: vec![0.0; 12],
                cell_ids: vec![
                    0;
                    (crate::render::CELL_ID_TEX_W * crate::render::CELL_ID_TEX_H)
                        as usize
                ],
                grid_gen: 0,
                field_gen: 0,
            }),
            grid_gen: 0,
            field_gen: 0,
            last_grid_build_ms: 0.0,
            seed_text,
            master_seed,
            sea_level_m: 0.0,
            view_mode: ViewMode::Split,
            preset: Preset::Standard7,
            projection: Projection::Equirectangular,
            graticule: true,
            layer: "Elevation",
            globe: GlobeView {
                yaw: 0.0,
                pitch: 0.35,
                zoom: 1.0,
                sea_level_m: 0.0,
            },
            flat_pan: [0.0, 0.0],
            flat_zoom: 1.0,
            hover: None,
            pick_hint: None,
            frame_times: Vec::with_capacity(240),
            last_frame: Instant::now(),
            // Perf runs first when both flags are given; its completion chains
            // into the screenshot script so neither output is silently lost.
            script_state: if script.perf_out.is_some() {
                ScriptState::Perf {
                    stage: 0,
                    frames: 0,
                    started: None,
                    fps: Vec::new(),
                }
            } else if script.screenshots_dir.is_some() {
                ScriptState::Shot {
                    stage: 0,
                    frames: 0,
                    requested: false,
                }
            } else {
                ScriptState::Idle
            },
            script,
        };
        app.rebuild_grid(app.preset.level());
        app
    }

    /// Build (or rebuild) the grid at `level`, then regenerate the world.
    fn rebuild_grid(&mut self, level: u32) {
        let t0 = Instant::now();
        let grid = Arc::new(Grid::build(level));
        self.last_grid_build_ms = t0.elapsed().as_secs_f64() * 1000.0;
        log::info!(
            "grid L{level} built in {:.1} ms ({} cells)",
            self.last_grid_build_ms,
            grid.cell_count()
        );

        let t1 = Instant::now();
        let cell_ids = WorldBundle::rasterize_cell_ids(&grid);
        log::info!(
            "cell-id raster built in {:.1} ms",
            t1.elapsed().as_secs_f64() * 1000.0
        );

        self.grid = grid.clone();
        self.world_state = WorldState::new(grid.clone());
        self.pipeline = Pipeline::new();
        self.pipeline.push(Box::new(NoiseElevationStage::default()));
        self.grid_gen += 1;
        self.pick_hint = None;

        // Seed the bundle with the new grid/raster; elevation follows.
        self.bundle = Arc::new(WorldBundle {
            grid,
            elevation: vec![0.0; self.world_state.fields.cell_count() as usize],
            cell_ids,
            grid_gen: self.grid_gen,
            field_gen: self.field_gen, // bumped by regenerate() below
        });
        self.regenerate();
    }

    /// Re-run the stage pipeline for the current seed and publish a new bundle.
    fn regenerate(&mut self) {
        let t0 = Instant::now();
        let ctx = StageContext {
            master_seed: self.master_seed,
        };
        if let Err(e) = self.pipeline.run(&ctx, &mut self.world_state) {
            log::error!("pipeline run failed: {e:#}");
            return;
        }
        let elevation = self
            .world_state
            .fields
            .get(ELEVATION_FIELD)
            .expect("noise stage must produce elevation")
            .to_vec();
        self.field_gen += 1;
        self.bundle = Arc::new(WorldBundle {
            grid: self.bundle.grid.clone(),
            elevation,
            cell_ids: self.bundle.cell_ids.clone(),
            grid_gen: self.grid_gen,
            field_gen: self.field_gen,
        });
        log::info!(
            "world regenerated (seed {:#018x}) in {:.1} ms",
            self.master_seed,
            t0.elapsed().as_secs_f64() * 1000.0
        );
    }

    fn fps(&self) -> f64 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        let sum: f32 = self.frame_times.iter().sum();
        self.frame_times.len() as f64 / sum.max(1e-6) as f64
    }

    // ----- canvases -----

    fn globe_canvas(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        if response.dragged() {
            let d = response.drag_delta();
            let k = 0.006 / self.globe.zoom;
            self.globe.yaw += d.x * k;
            self.globe.pitch = (self.globe.pitch + d.y * k).clamp(-1.55, 1.55);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.globe.zoom = (self.globe.zoom * (scroll * 0.002).exp()).clamp(0.4, 50.0);
            }
        }

        // Cursor -> cell.
        if let Some(pos) = response.hover_pos() {
            let c = rect.center();
            let r = globe_radius_px(rect.width(), rect.height(), self.globe.zoom);
            let vx = (pos.x - c.x) / r;
            let vy = -(pos.y - c.y) / r;
            let rr = vx * vx + vy * vy;
            if rr <= 1.0 {
                let cam = [vx, vy, (1.0 - rr).sqrt()];
                let rot = globe_rotation(self.globe.yaw, self.globe.pitch);
                let world = rotate_inv(&rot, cam);
                let cell = self.grid.nearest_cell(world, self.pick_hint);
                self.pick_hint = Some(cell);
                let (lat, lon) = worldmaker_core::grid::unit_to_latlon(world);
                self.hover = Some(("globe", cell, lat.to_degrees(), lon.to_degrees()));
            }
        }

        let view = GlobeView {
            sea_level_m: self.sea_level_m,
            ..self.globe
        };
        ui.painter()
            .add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                GlobeCallback {
                    world: self.bundle.clone(),
                    view,
                    rect_points: [
                        rect.center().x,
                        rect.center().y,
                        rect.width(),
                        rect.height(),
                    ],
                },
            ));
    }

    fn flat_canvas(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        if response.dragged() {
            let d = response.drag_delta();
            self.flat_pan[0] += d.x;
            self.flat_pan[1] += d.y;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old_zoom = self.flat_zoom;
                let new_zoom = (old_zoom * (scroll * 0.002).exp()).clamp(0.5, 80.0);
                if let Some(pos) = response.hover_pos() {
                    // Keep the map point under the cursor fixed while zooming.
                    let c = rect.center();
                    let scale = new_zoom / old_zoom;
                    self.flat_pan[0] = (self.flat_pan[0] - (pos.x - c.x)) * scale + (pos.x - c.x);
                    self.flat_pan[1] = (self.flat_pan[1] - (pos.y - c.y)) * scale + (pos.y - c.y);
                }
                self.flat_zoom = new_zoom;
            }
        }

        if let Some(pos) = response.hover_pos() {
            let c = rect.center();
            let base = flat_base_half_extents(self.projection, rect.width(), rect.height());
            let mx = (pos.x - c.x - self.flat_pan[0]) / (base[0] * self.flat_zoom);
            let my = -(pos.y - c.y - self.flat_pan[1]) / (base[1] * self.flat_zoom);
            if let Some((lat, lon)) = self.projection.invert(mx, my) {
                let cell = self
                    .grid
                    .nearest_cell(latlon_to_unit(lat, lon), self.pick_hint);
                self.pick_hint = Some(cell);
                self.hover = Some(("flat", cell, lat.to_degrees(), lon.to_degrees()));
            }
        }

        ui.painter()
            .add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                FlatCallback {
                    world: self.bundle.clone(),
                    view: FlatView {
                        projection: self.projection,
                        pan: self.flat_pan,
                        zoom: self.flat_zoom,
                        sea_level_m: self.sea_level_m,
                        graticule: self.graticule,
                    },
                    rect_points: [
                        rect.center().x,
                        rect.center().y,
                        rect.width(),
                        rect.height(),
                    ],
                },
            ));
    }

    // ----- scripted modes -----

    fn drive_script(&mut self, ctx: &egui::Context) {
        match &mut self.script_state {
            ScriptState::Idle => {}
            ScriptState::Closing => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ScriptState::Shot {
                stage,
                frames,
                requested,
            } => {
                const STAGES: [(ViewMode, &str); 3] = [
                    (ViewMode::Globe, "globe"),
                    (ViewMode::Flat, "flat"),
                    (ViewMode::Split, "split"),
                ];
                let (mode, name) = STAGES[*stage];
                self.view_mode = mode;
                *frames += 1;
                if *frames == 30 && !*requested {
                    *requested = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
                }
                // Collect a delivered screenshot.
                let image = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(image) = image {
                    let dir = self.script.screenshots_dir.clone().unwrap();
                    if let Err(e) = save_color_image(&image, &dir.join(format!("{name}.png"))) {
                        log::error!("failed to save screenshot {name}: {e:#}");
                    } else {
                        log::info!("saved screenshot {name}.png");
                    }
                    if *stage + 1 < STAGES.len() {
                        *stage += 1;
                        *frames = 0;
                        *requested = false;
                    } else {
                        self.script_state = ScriptState::Closing;
                    }
                }
            }
            ScriptState::Perf {
                stage,
                frames,
                started,
                fps,
            } => {
                const STAGES: [(ViewMode, &str); 3] = [
                    (ViewMode::Globe, "globe_fps"),
                    (ViewMode::Flat, "flat_fps"),
                    (ViewMode::Split, "split_fps"),
                ];
                const WARMUP: u32 = 40;
                const SAMPLE: u32 = 240;
                let (mode, name) = STAGES[*stage];
                self.view_mode = mode;
                *frames += 1;
                if *frames == WARMUP {
                    *started = Some(Instant::now());
                }
                if *frames == WARMUP + SAMPLE {
                    let elapsed = started
                        .take()
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(1.0);
                    fps.push((name.to_string(), SAMPLE as f64 / elapsed));
                    if *stage + 1 < STAGES.len() {
                        *stage += 1;
                        *frames = 0;
                    } else {
                        let fps_taken = std::mem::take(fps);
                        self.write_perf_results(&fps_taken);
                        // Chain into screenshots if both flags were given.
                        self.script_state = if self.script.screenshots_dir.is_some() {
                            ScriptState::Shot {
                                stage: 0,
                                frames: 0,
                                requested: false,
                            }
                        } else {
                            ScriptState::Closing
                        };
                    }
                }
            }
        }
    }

    fn write_perf_results(&self, fps: &[(String, f64)]) {
        let Some(out) = &self.script.perf_out else {
            return;
        };
        let mut metrics = serde_json::Map::new();
        for (level, ms) in &self.script.grid_build_ms {
            metrics.insert(format!("grid_build_ms_L{level}"), serde_json::json!(ms));
        }
        for (name, value) in fps {
            metrics.insert(
                name.clone(),
                serde_json::json!((value * 10.0).round() / 10.0),
            );
        }
        metrics.insert(
            "fps_grid_level".into(),
            serde_json::json!(self.preset.level()),
        );
        metrics.insert("fps_vsync_off".into(), serde_json::json!(true));
        metrics.insert(
            "render_mesh_decimated".into(),
            serde_json::json!(false), // full-resolution mesh; no decimation needed
        );
        let file = worldmaker_io::ResultsFile::new(
            &worldmaker_io::results::today_utc_iso(),
            serde_json::Value::Object(metrics),
        );
        match file.write(out) {
            Ok(()) => log::info!("perf results written to {}", out.display()),
            Err(e) => log::error!("failed to write perf results: {e:#}"),
        }
    }
}

fn save_color_image(image: &egui::ColorImage, path: &std::path::Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let [w, h] = image.size;
    let mut bytes = Vec::with_capacity(w * h * 4);
    for px in &image.pixels {
        bytes.extend_from_slice(&px.to_array());
    }
    let img = image::RgbaImage::from_raw(w as u32, h as u32, bytes)
        .ok_or_else(|| anyhow::anyhow!("screenshot buffer size mismatch"))?;
    img.save(path)?;
    Ok(())
}

impl eframe::App for WorldApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        // FPS bookkeeping.
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if self.frame_times.len() >= 120 {
            self.frame_times.remove(0);
        }
        self.frame_times.push(dt.max(1e-6));

        self.hover = None;
        self.drive_script(ctx);

        // ----- top controls -----
        egui::Panel::top("controls").show(root, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Seed:");
                let seed_edit =
                    ui.add(egui::TextEdit::singleline(&mut self.seed_text).desired_width(120.0));
                let enter = seed_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Generate").clicked() || enter {
                    self.master_seed = seed_from_text(&self.seed_text);
                    self.regenerate();
                }
                ui.separator();

                ui.selectable_value(&mut self.view_mode, ViewMode::Globe, "Globe");
                ui.selectable_value(&mut self.view_mode, ViewMode::Flat, "Flat");
                ui.selectable_value(&mut self.view_mode, ViewMode::Split, "Split");
                ui.separator();

                egui::ComboBox::from_label("Projection")
                    .selected_text(self.projection.name())
                    .show_ui(ui, |ui| {
                        for p in Projection::ALL {
                            ui.selectable_value(&mut self.projection, p, p.name());
                        }
                    });
                ui.checkbox(&mut self.graticule, "Graticule");
                ui.separator();

                egui::ComboBox::from_label("Layer")
                    .selected_text(self.layer)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.layer, "Elevation", "Elevation");
                        ui.add_enabled(false, egui::Button::selectable(false, "Plates (Phase 1)"));
                        ui.add_enabled(false, egui::Button::selectable(false, "Climate (Phase 3)"));
                    });

                let mut preset = self.preset;
                egui::ComboBox::from_label("Preset")
                    .selected_text(preset.label())
                    .show_ui(ui, |ui| {
                        for p in [Preset::Draft6, Preset::Standard7, Preset::High8] {
                            ui.selectable_value(&mut preset, p, p.label());
                        }
                    });
                if preset != self.preset {
                    self.preset = preset;
                    self.rebuild_grid(preset.level());
                }
                ui.separator();

                ui.label("Sea level:");
                if ui
                    .add(
                        egui::Slider::new(&mut self.sea_level_m, -4000.0..=4000.0)
                            .suffix(" m")
                            .fixed_decimals(0),
                    )
                    .changed()
                {
                    // Recoloring is a uniform update; nothing to recompute.
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.0} FPS", self.fps()));
                });
            });
        });

        // ----- bottom: cursor readout + timeline placeholder -----
        egui::Panel::bottom("timeline").show(root, |ui| {
            ui.horizontal(|ui| match self.hover {
                Some((canvas, cell, lat, lon)) => {
                    let ns = if lat >= 0.0 { "N" } else { "S" };
                    let ew = if lon >= 0.0 { "E" } else { "W" };
                    ui.monospace(format!(
                        "Cell {cell}  ·  {:.2}°{ns} {:.2}°{ew}  ({canvas})",
                        lat.abs(),
                        lon.abs()
                    ));
                }
                None => {
                    ui.monospace("Hover a canvas to inspect a cell");
                }
            });
            ui.add_enabled_ui(false, |ui| {
                let mut era = 0.0_f32;
                ui.horizontal(|ui| {
                    ui.label("Timeline");
                    let width = ui.available_width() - 260.0;
                    ui.spacing_mut().slider_width = width.max(100.0);
                    ui.add(egui::Slider::new(&mut era, 0.0..=100.0).show_value(false));
                    ui.label("era picker arrives in Phase 1");
                });
            });
        });

        // ----- canvases -----
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::from_rgb(20, 22, 26)))
            .show(root, |ui| {
                let full = ui.available_rect_before_wrap();
                match self.view_mode {
                    ViewMode::Globe => self.globe_canvas(ui, full),
                    ViewMode::Flat => self.flat_canvas(ui, full),
                    ViewMode::Split => {
                        let gap = 4.0;
                        let left = egui::Rect::from_min_max(
                            full.min,
                            egui::pos2(full.center().x - gap / 2.0, full.max.y),
                        );
                        let right = egui::Rect::from_min_max(
                            egui::pos2(full.center().x + gap / 2.0, full.min.y),
                            full.max,
                        );
                        self.globe_canvas(ui, left);
                        self.flat_canvas(ui, right);
                    }
                }
            });

        // Continuous repaint: keeps the FPS readout honest and drag smooth.
        ctx.request_repaint();
    }
}
