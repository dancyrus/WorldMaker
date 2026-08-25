//! The WorldMaker application: UI, canvases, input, the tectonic history
//! timeline (era picker), painting tools, and the scripted screenshot /
//! perf-harness modes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Instant;

use eframe::egui;

use worldmaker_core::grid::{latlon_to_unit, Grid};
use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Projection;
use worldmaker_sim::tectonics::{TectonicsHistory, TectonicsParams, TectonicsStage};
use worldmaker_sim::{Cancelled, Pipeline, Progress, StageContext, WorldState};

use crate::layers::{self, BakeOverlay, Layer};
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    None,
    CratonPaint,
    CratonErase,
    Hotspot,
}

/// Timeline playback speed while "Play" is on, in My per real second.
const PLAY_MY_PER_SECOND: f32 = 100.0;
/// Hotspot tool: clicking within this range of an existing hotspot removes it.
const HOTSPOT_REMOVE_KM: f32 = 300.0;
const EARTH_RADIUS_KM: f32 = 6371.0;

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

/// A tectonic simulation running on a worker thread.
struct SimJob {
    progress: Arc<Progress>,
    rx: mpsc::Receiver<anyhow::Result<(WorldState, f64)>>,
    started: Instant,
}

pub struct WorldApp {
    // World state.
    grid: Arc<Grid>,
    cell_ids: Arc<Vec<u32>>,
    history: Option<TectonicsHistory>,
    world_state: Option<WorldState>,
    job: Option<SimJob>,
    bundle: Arc<WorldBundle>,
    grid_gen: u64,
    field_gen: u64,
    needs_bake: bool,

    // Tectonics parameters (UI copies; Generate builds TectonicsParams).
    plate_count: u32,
    land_fraction: f32,
    tectonic_vigor: f32,
    span_my: f32,
    hotspot_count: u32,
    craton_paint: BTreeMap<u32, i8>,
    hotspot_overlay: Option<Vec<[f32; 3]>>,

    // Era picker.
    viewing_kf: usize,
    present_kf: usize,
    playing: bool,
    play_accum: f32,

    // Controls.
    seed_text: String,
    master_seed: u64,
    sea_level_m: f32,
    view_mode: ViewMode,
    preset: Preset,
    projection: Projection,
    graticule: bool,
    layer: Layer,
    tool: Tool,
    brush_radius_km: f32,
    /// The current craton stroke changed the overlay; re-run on stroke end.
    craton_stroke_dirty: bool,

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
        let placeholder_grid = Arc::new(Grid::build(0));
        let mut app = WorldApp {
            grid: placeholder_grid.clone(),
            cell_ids: Arc::new(vec![
                0;
                (crate::render::CELL_ID_TEX_W * crate::render::CELL_ID_TEX_H)
                    as usize
            ]),
            history: None,
            world_state: None,
            job: None,
            bundle: Arc::new(WorldBundle {
                grid: placeholder_grid,
                colors: vec![0xff40_4040; 12],
                cell_ids: Arc::new(vec![
                    0;
                    (crate::render::CELL_ID_TEX_W * crate::render::CELL_ID_TEX_H)
                        as usize
                ]),
                grid_gen: 0,
                field_gen: 0,
            }),
            grid_gen: 0,
            field_gen: 0,
            needs_bake: false,
            plate_count: 12,
            land_fraction: 0.29,
            tectonic_vigor: 1.0,
            span_my: 500.0,
            hotspot_count: 6,
            craton_paint: BTreeMap::new(),
            hotspot_overlay: None,
            viewing_kf: 0,
            present_kf: 0,
            playing: false,
            play_accum: 0.0,
            seed_text,
            master_seed,
            sea_level_m: 0.0,
            view_mode: ViewMode::Split,
            preset: Preset::Standard7,
            projection: Projection::Equirectangular,
            graticule: true,
            layer: Layer::Elevation,
            tool: Tool::None,
            brush_radius_km: 600.0,
            craton_stroke_dirty: false,
            globe: GlobeView {
                yaw: 0.0,
                pitch: 0.35,
                zoom: 1.0,
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

    fn current_params(&self) -> TectonicsParams {
        TectonicsParams {
            plate_count: self.plate_count,
            land_fraction: self.land_fraction,
            tectonic_vigor: self.tectonic_vigor,
            span_my: self.span_my,
            hotspot_count: self.hotspot_count,
            craton_overlay: self.craton_paint.iter().map(|(&c, &v)| (c, v)).collect(),
            hotspot_overlay: self.hotspot_overlay.clone(),
        }
    }

    /// Build (or rebuild) the grid at `level`, then launch a fresh history.
    fn rebuild_grid(&mut self, level: u32) {
        let t0 = Instant::now();
        let grid = Arc::new(Grid::build(level));
        log::info!(
            "grid L{level} built in {:.1} ms ({} cells)",
            t0.elapsed().as_secs_f64() * 1000.0,
            grid.cell_count()
        );

        let t1 = Instant::now();
        let cell_ids = Arc::new(WorldBundle::rasterize_cell_ids(&grid));
        log::info!(
            "cell-id raster built in {:.1} ms",
            t1.elapsed().as_secs_f64() * 1000.0
        );

        self.grid = grid.clone();
        self.cell_ids = cell_ids.clone();
        self.grid_gen += 1;
        self.field_gen += 1;
        self.pick_hint = None;
        // Craton paint is per-grid (cell ids change with level); hotspot
        // positions are unit vectors and survive a preset switch.
        self.craton_paint.clear();
        self.bundle = Arc::new(WorldBundle {
            grid,
            colors: vec![0xff40_4040; self.grid.cell_count() as usize],
            cell_ids,
            grid_gen: self.grid_gen,
            field_gen: self.field_gen,
        });
        self.start_job();
    }

    /// Launch the tectonic history on a worker thread. Cancels any running
    /// job and drops the old history immediately (keyframe memory budget:
    /// never hold two histories at once).
    fn start_job(&mut self) {
        if let Some(job) = &self.job {
            job.progress.request_cancel();
        }
        self.history = None;
        self.world_state = None;
        self.playing = false;
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(Progress::new());
        let worker_progress = progress.clone();
        let grid = self.grid.clone();
        let seed = self.master_seed;
        let params = self.current_params();
        std::thread::spawn(move || {
            let mut world = WorldState::new(grid);
            let mut pipeline = Pipeline::new();
            pipeline.push(Box::new(TectonicsStage::new(params)));
            let mut ctx = StageContext::new(seed);
            ctx.progress = Some(worker_progress);
            let t0 = Instant::now();
            let result = pipeline
                .run(&ctx, &mut world)
                .map(|_| (world, t0.elapsed().as_secs_f64()));
            let _ = tx.send(result);
        });
        self.job = Some(SimJob {
            progress,
            rx,
            started: Instant::now(),
        });
    }

    /// Poll the worker; install a finished history.
    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        match job.rx.try_recv() {
            Ok(Ok((mut world, seconds))) => {
                log::info!(
                    "tectonics run finished in {seconds:.2} s (seed {:#018x})",
                    self.master_seed
                );
                self.history = world.history.take();
                self.world_state = Some(world);
                self.job = None;
                let last = self
                    .history
                    .as_ref()
                    .map(|h| h.keyframes.len() - 1)
                    .unwrap_or(0);
                self.present_kf = last;
                // Keep painting context at t=0; otherwise show the present.
                self.viewing_kf = if matches!(self.tool, Tool::CratonPaint | Tool::CratonErase) {
                    0
                } else {
                    last
                };
                self.set_present(self.present_kf);
                self.needs_bake = true;
            }
            Ok(Err(e)) => {
                if e.downcast_ref::<Cancelled>().is_some() {
                    log::info!("tectonics run cancelled");
                } else {
                    log::error!("tectonics run failed: {e:#}");
                }
                self.job = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                log::error!("tectonics worker vanished without a result");
                self.job = None;
            }
        }
    }

    /// Pin a keyframe as "the present": decode it into the world fields so
    /// downstream stages and exports read that moment.
    fn set_present(&mut self, kf_index: usize) {
        self.present_kf = kf_index;
        if let (Some(history), Some(world)) = (&self.history, &mut self.world_state) {
            history.keyframes[kf_index].write_fields(&mut world.fields);
        }
    }

    /// Re-bake the layer colors for the viewed keyframe and publish a bundle.
    fn rebake(&mut self) {
        self.needs_bake = false;
        let Some(history) = &self.history else {
            return;
        };
        let kf = &history.keyframes[self.viewing_kf.min(history.keyframes.len() - 1)];

        let paint: Vec<(u32, i8)>;
        let hotspot_cells: Vec<u32>;
        let overlay = match self.tool {
            Tool::CratonPaint | Tool::CratonErase => {
                paint = self.craton_paint.iter().map(|(&c, &v)| (c, v)).collect();
                BakeOverlay {
                    craton: Some(&paint),
                    hotspot_cells: None,
                }
            }
            Tool::Hotspot => {
                let mut cells = Vec::new();
                let spots = self.hotspot_overlay.as_deref().unwrap_or(&history.hotspots);
                for h in spots {
                    let c = self.grid.nearest_cell(*h, None);
                    cells.push(c);
                    cells.extend_from_slice(self.grid.neighbors_of(c));
                }
                hotspot_cells = cells;
                BakeOverlay {
                    craton: None,
                    hotspot_cells: Some(&hotspot_cells),
                }
            }
            Tool::None => BakeOverlay::NONE,
        };

        let colors = layers::bake(self.layer, kf, self.sea_level_m, &overlay);
        self.field_gen += 1;
        self.bundle = Arc::new(WorldBundle {
            grid: self.grid.clone(),
            colors,
            cell_ids: self.cell_ids.clone(),
            grid_gen: self.grid_gen,
            field_gen: self.field_gen,
        });
    }

    fn fps(&self) -> f64 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        let sum: f32 = self.frame_times.iter().sum();
        self.frame_times.len() as f64 / sum.max(1e-6) as f64
    }

    // ----- painting -----

    /// All cells within the brush radius of a hit cell (neighbor flood).
    fn cells_within_radius(&self, center: u32, radius_km: f32) -> Vec<u32> {
        let cos_thresh = (radius_km / EARTH_RADIUS_KM).cos();
        let cpos = self.grid.positions[center as usize];
        let dot = |c: u32| -> f32 {
            let p = self.grid.positions[c as usize];
            p[0] * cpos[0] + p[1] * cpos[1] + p[2] * cpos[2]
        };
        let mut visited = std::collections::HashSet::from([center]);
        let mut out = vec![center];
        let mut queue = std::collections::VecDeque::from([center]);
        while let Some(c) = queue.pop_front() {
            for &nb in self.grid.neighbors_of(c) {
                if dot(nb) >= cos_thresh && visited.insert(nb) {
                    out.push(nb);
                    queue.push_back(nb);
                }
            }
        }
        out
    }

    /// Apply the active tool at a hit cell. Returns true if state changed.
    fn apply_tool(&mut self, cell: u32, clicked: bool) -> bool {
        match self.tool {
            Tool::None => false,
            Tool::CratonPaint | Tool::CratonErase => {
                let v: i8 = if self.tool == Tool::CratonPaint {
                    1
                } else {
                    -1
                };
                let cells = self.cells_within_radius(cell, self.brush_radius_km);
                let mut changed = false;
                for c in cells {
                    if self.craton_paint.insert(c, v) != Some(v) {
                        changed = true;
                    }
                }
                changed
            }
            Tool::Hotspot => {
                if !clicked {
                    return false;
                }
                // Never edit blind: with no overlay AND no finished history
                // (a run is in flight), there is no hotspot set to add to —
                // falling through to an empty set would silently replace the
                // generated hotspots with just this click (review finding).
                let Some(mut spots) = self
                    .hotspot_overlay
                    .clone()
                    .or_else(|| self.history.as_ref().map(|h| h.hotspots.clone()))
                else {
                    return false;
                };
                let pos = self.grid.positions[cell as usize];
                let cos_remove = (HOTSPOT_REMOVE_KM / EARTH_RADIUS_KM).cos();
                let near = spots
                    .iter()
                    .position(|h| h[0] * pos[0] + h[1] * pos[1] + h[2] * pos[2] >= cos_remove);
                match near {
                    Some(i) => {
                        spots.remove(i);
                    }
                    None => spots.push(pos),
                }
                self.hotspot_overlay = Some(spots);
                self.start_job();
                true
            }
        }
    }

    // ----- canvases -----

    fn canvas_common(
        &mut self,
        response: &egui::Response,
        hit: Option<(u32, f32, f32)>,
        canvas: &'static str,
    ) {
        if let Some((cell, lat, lon)) = hit {
            self.pick_hint = Some(cell);
            self.hover = Some((canvas, cell, lat.to_degrees(), lon.to_degrees()));
            if self.tool != Tool::None {
                let clicked = response.clicked();
                if (response.dragged() || clicked) && self.apply_tool(cell, clicked) {
                    self.needs_bake = true;
                    if matches!(self.tool, Tool::CratonPaint | Tool::CratonErase) {
                        self.craton_stroke_dirty = true;
                    }
                }
            }
        }
        // Stroke end must fire even when the release lands off the map (past
        // the globe's limb or the projection outline): drag_stopped() is
        // hover-independent, so check it outside the hit gate (review
        // finding). Only re-run when the stroke actually changed the overlay.
        if matches!(self.tool, Tool::CratonPaint | Tool::CratonErase)
            && (response.drag_stopped() || response.clicked())
            && self.craton_stroke_dirty
        {
            self.craton_stroke_dirty = false;
            // Stroke finished: re-run history from t=0, same seed.
            self.start_job();
        }
    }

    fn globe_canvas(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let painting = self.tool != Tool::None;
        if response.dragged() && !painting {
            let d = response.drag_delta();
            let k = 0.006 / self.globe.zoom;
            self.globe.yaw += d.x * k;
            self.globe.pitch = (self.globe.pitch + d.y * k).clamp(-1.55, 1.55);
        }
        if response.hovered() {
            // Trackpad pinch (and ctrl+scroll) arrives as zoom_delta, wheel /
            // two-finger scroll as smooth_scroll_delta; egui never reports the
            // same gesture in both, so combining them is safe.
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll * 0.002).exp() * pinch;
            if factor != 1.0 {
                self.globe.zoom = (self.globe.zoom * factor).clamp(0.4, 50.0);
            }
        }

        // Cursor -> cell.
        let mut hit = None;
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
                let (lat, lon) = worldmaker_core::grid::unit_to_latlon(world);
                hit = Some((cell, lat, lon));
            }
        }
        self.canvas_common(&response, hit, "globe");

        ui.painter()
            .add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                GlobeCallback {
                    world: self.bundle.clone(),
                    view: self.globe,
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
        let painting = self.tool != Tool::None;
        if response.dragged() && !painting {
            let d = response.drag_delta();
            self.flat_pan[0] += d.x;
            self.flat_pan[1] += d.y;
        }
        if response.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll * 0.002).exp() * pinch;
            if factor != 1.0 {
                let old_zoom = self.flat_zoom;
                let new_zoom = (old_zoom * factor).clamp(0.5, 80.0);
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

        let mut hit = None;
        if let Some(pos) = response.hover_pos() {
            let c = rect.center();
            let base = flat_base_half_extents(self.projection, rect.width(), rect.height());
            let mx = (pos.x - c.x - self.flat_pan[0]) / (base[0] * self.flat_zoom);
            let my = -(pos.y - c.y - self.flat_pan[1]) / (base[1] * self.flat_zoom);
            if let Some((lat, lon)) = self.projection.invert(mx, my) {
                let cell = self
                    .grid
                    .nearest_cell(latlon_to_unit(lat, lon), self.pick_hint);
                hit = Some((cell, lat, lon));
            }
        }
        self.canvas_common(&response, hit, "flat");

        ui.painter()
            .add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                FlatCallback {
                    world: self.bundle.clone(),
                    view: FlatView {
                        projection: self.projection,
                        pan: self.flat_pan,
                        zoom: self.flat_zoom,
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

    // ----- panels -----

    fn top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("controls").show(root, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Seed:");
                let seed_edit =
                    ui.add(egui::TextEdit::singleline(&mut self.seed_text).desired_width(120.0));
                let enter = seed_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Generate").clicked() || enter {
                    self.master_seed = seed_from_text(&self.seed_text);
                    self.start_job();
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

                let old_layer = self.layer;
                egui::ComboBox::from_label("Layer")
                    .selected_text(self.layer.name())
                    .show_ui(ui, |ui| {
                        for l in Layer::ALL {
                            ui.selectable_value(&mut self.layer, l, l.name());
                        }
                        ui.add_enabled(false, egui::Button::selectable(false, "Climate (Phase 3)"));
                    });
                if self.layer != old_layer {
                    self.needs_bake = true;
                }

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
                    // Offset around the solved sea level; recolor only.
                    self.needs_bake = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.0} FPS", self.fps()));
                });
            });
        });
    }

    fn side_panel(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("world-params")
            .resizable(false)
            .show(root, |ui| {
                ui.add_space(4.0);
                ui.heading("World");
                ui.add(
                    egui::Slider::new(&mut self.plate_count, 8..=24).text("Plates"),
                );
                ui.add(
                    egui::Slider::new(&mut self.land_fraction, 0.05..=0.7)
                        .text("Land fraction")
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
                ui.add(
                    egui::Slider::new(&mut self.tectonic_vigor, 0.25..=2.0).text("Tectonic vigor"),
                );
                ui.add(
                    egui::Slider::new(&mut self.span_my, 200.0..=2000.0)
                        .text("History span")
                        .suffix(" My")
                        .step_by(100.0),
                );
                ui.add(
                    egui::Slider::new(&mut self.hotspot_count, 0..=12).text("Hotspots"),
                );
                if ui
                    .add_enabled(self.job.is_none(), egui::Button::new("Generate history"))
                    .clicked()
                {
                    self.start_job();
                }

                // Progress + cancel while a run is live.
                if let Some(job) = &self.job {
                    let frac = job.progress.fraction();
                    ui.add(egui::ProgressBar::new(frac).show_percentage());
                    ui.label(format!(
                        "simulating… {:.1} s",
                        job.started.elapsed().as_secs_f64()
                    ));
                    if ui.button("Cancel").clicked() {
                        job.progress.request_cancel();
                    }
                }

                ui.separator();
                ui.heading("Paint");
                ui.selectable_value(&mut self.tool, Tool::None, "Navigate");
                let before = self.tool;
                ui.selectable_value(&mut self.tool, Tool::CratonPaint, "Craton brush");
                ui.selectable_value(&mut self.tool, Tool::CratonErase, "Craton eraser");
                ui.selectable_value(&mut self.tool, Tool::Hotspot, "Hotspots");
                if self.tool != before {
                    self.needs_bake = true;
                    if matches!(self.tool, Tool::CratonPaint | Tool::CratonErase) {
                        // Cratons are painted on the initial state.
                        self.viewing_kf = 0;
                        self.playing = false;
                    }
                }
                if matches!(self.tool, Tool::CratonPaint | Tool::CratonErase) {
                    ui.add(
                        egui::Slider::new(&mut self.brush_radius_km, 150.0..=2000.0)
                            .text("Radius")
                            .suffix(" km"),
                    );
                    if !self.craton_paint.is_empty() && ui.button("Clear craton paint").clicked() {
                        self.craton_paint.clear();
                        self.needs_bake = true;
                        self.start_job();
                    }
                    ui.label("Paint on the t = 0 map; history re-runs from the start with the same seed.");
                }
                if self.tool == Tool::Hotspot {
                    ui.label("Click to add a hotspot; click an existing one to remove it.");
                    if self.hotspot_overlay.is_some() && ui.button("Reset to generated").clicked() {
                        self.hotspot_overlay = None;
                        self.start_job();
                    }
                }
            });
    }

    fn bottom_panel(&mut self, root: &mut egui::Ui, frame_dt: f32) {
        egui::Panel::bottom("timeline").show(root, |ui| {
            // Cursor readout with the active layer's value.
            ui.horizontal(|ui| match self.hover {
                Some((canvas, cell, lat, lon)) => {
                    let ns = if lat >= 0.0 { "N" } else { "S" };
                    let ew = if lon >= 0.0 { "E" } else { "W" };
                    let value = self
                        .history
                        .as_ref()
                        .map(|h| {
                            let kf = &h.keyframes[self.viewing_kf.min(h.keyframes.len() - 1)];
                            let c = cell as usize;
                            format!(
                                "  ·  plate {}  ·  {} m  ·  {} My  ·  {:.1} km",
                                kf.plate_id[c],
                                kf.elev_m[c],
                                kf.crust_age_my[c],
                                kf.thickness_ckm[c] as f32 * 0.01,
                            )
                        })
                        .unwrap_or_default();
                    ui.monospace(format!(
                        "Cell {cell}  ·  {:.2}°{ns} {:.2}°{ew}  ({canvas}){value}",
                        lat.abs(),
                        lon.abs()
                    ));
                }
                None => {
                    ui.monospace("Hover a canvas to inspect a cell");
                }
            });

            // The era picker.
            let (kf_count, interval) = self
                .history
                .as_ref()
                .map(|h| (h.keyframes.len(), h.keyframe_interval_my))
                .unwrap_or((0, 10.0));
            ui.add_enabled_ui(kf_count > 1 && self.job.is_none(), |ui| {
                ui.horizontal(|ui| {
                    let icon = if self.playing { "⏸" } else { "▶" };
                    if ui.button(icon).clicked() {
                        self.playing = !self.playing;
                        if self.playing && self.viewing_kf + 1 >= kf_count {
                            self.viewing_kf = 0; // replay from the start
                        }
                        self.play_accum = 0.0;
                    }

                    let mut idx = self.viewing_kf.min(kf_count.saturating_sub(1));
                    let width = ui.available_width() - 330.0;
                    ui.spacing_mut().slider_width = width.max(120.0);
                    let slider = ui.add(
                        egui::Slider::new(&mut idx, 0..=kf_count.saturating_sub(1))
                            .show_value(false),
                    );
                    if slider.changed() {
                        self.playing = false;
                        if idx != self.viewing_kf {
                            self.viewing_kf = idx;
                            self.needs_bake = true;
                        }
                    }

                    let t_my = self.viewing_kf as f32 * interval;
                    let present_my = self.present_kf as f32 * interval;
                    ui.monospace(format!("t = {t_my:5.0} My"));
                    if ui
                        .add_enabled(
                            self.viewing_kf != self.present_kf,
                            egui::Button::new("Set as present"),
                        )
                        .clicked()
                    {
                        self.set_present(self.viewing_kf);
                    }
                    ui.weak(format!("present: {present_my:.0} My"));
                });
            });

            // Playback advance.
            if self.playing && kf_count > 1 {
                self.play_accum += frame_dt * PLAY_MY_PER_SECOND / interval;
                let steps = self.play_accum as usize;
                if steps > 0 {
                    self.play_accum -= steps as f32;
                    let next = self.viewing_kf + steps;
                    if next >= kf_count - 1 {
                        self.viewing_kf = kf_count - 1;
                        self.playing = false;
                    } else {
                        self.viewing_kf = next;
                    }
                    self.needs_bake = true;
                }
            }
        });
    }

    // ----- scripted modes -----

    /// Screenshot stages: the Phase 0 trio plus the Phase 1 documentation
    /// shots (plates layer mid-run, a mountain range, timeline mid-scrub).
    fn setup_shot_stage(&mut self, stage: usize) -> &'static str {
        let kf_count = self
            .history
            .as_ref()
            .map(|h| h.keyframes.len())
            .unwrap_or(1);
        let last = kf_count - 1;
        match stage {
            0 => {
                self.view_mode = ViewMode::Globe;
                self.layer = Layer::Elevation;
                self.viewing_kf = last;
                "globe"
            }
            1 => {
                self.view_mode = ViewMode::Flat;
                "flat"
            }
            2 => {
                self.view_mode = ViewMode::Split;
                "split"
            }
            3 => {
                self.view_mode = ViewMode::Split;
                self.layer = Layer::Plates;
                self.viewing_kf = kf_count / 2;
                "plates"
            }
            4 => {
                // Center the globe on the highest continental cell: the
                // continent-continent mountain range.
                self.view_mode = ViewMode::Globe;
                self.layer = Layer::Elevation;
                self.viewing_kf = last;
                if let Some(h) = &self.history {
                    let kf = &h.keyframes[last];
                    if let Some((c, _)) = kf.elev_m.iter().enumerate().max_by_key(|&(_, &e)| e) {
                        self.globe.yaw = -self.grid.lon[c];
                        self.globe.pitch = self.grid.lat[c];
                        self.globe.zoom = 1.6;
                    }
                }
                "mountains"
            }
            _ => {
                self.view_mode = ViewMode::Flat;
                self.layer = Layer::Elevation;
                self.viewing_kf = (kf_count * 3) / 5;
                "timeline"
            }
        }
    }

    fn drive_script(&mut self, ctx: &egui::Context) {
        // Scripted runs wait for the initial history to finish.
        if !matches!(self.script_state, ScriptState::Idle | ScriptState::Closing)
            && (self.job.is_some() || self.history.is_none())
        {
            return;
        }
        match self.script_state {
            ScriptState::Idle => {}
            ScriptState::Closing => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ScriptState::Shot { .. } => self.drive_shot(ctx),
            ScriptState::Perf { .. } => self.drive_perf(),
        }
    }

    fn drive_shot(&mut self, ctx: &egui::Context) {
        const NAMES: [&str; 6] = ["globe", "flat", "split", "plates", "mountains", "timeline"];
        let (stage, frames, requested) = match &self.script_state {
            ScriptState::Shot {
                stage,
                frames,
                requested,
            } => (*stage, *frames, *requested),
            _ => return,
        };
        let frames = frames + 1;
        let mut requested = requested;
        if frames == 1 {
            self.setup_shot_stage(stage);
            self.needs_bake = true;
            log::info!("screenshot stage {stage}: {}", NAMES[stage]);
        }
        if frames == 30 && !requested {
            requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
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
            let name = NAMES[stage];
            if let Err(e) = save_color_image(&image, &dir.join(format!("{name}.png"))) {
                log::error!("failed to save screenshot {name}: {e:#}");
            } else {
                log::info!("saved screenshot {name}.png");
            }
            self.script_state = if stage + 1 < NAMES.len() {
                ScriptState::Shot {
                    stage: stage + 1,
                    frames: 0,
                    requested: false,
                }
            } else {
                ScriptState::Closing
            };
        } else {
            self.script_state = ScriptState::Shot {
                stage,
                frames,
                requested,
            };
        }
    }

    fn drive_perf(&mut self) {
        let ScriptState::Perf {
            stage,
            frames,
            started,
            fps,
        } = &mut self.script_state
        else {
            return;
        };
        const VIEWS: [(ViewMode, &str); 3] = [
            (ViewMode::Globe, "globe_fps"),
            (ViewMode::Flat, "flat_fps"),
            (ViewMode::Split, "split_fps"),
        ];
        const WARMUP: u32 = 40;
        const SAMPLE: u32 = 240;
        let (mode, name) = VIEWS[*stage];
        *frames += 1;
        let frames_now = *frames;
        if frames_now == WARMUP {
            *started = Some(Instant::now());
        }
        let mut finished_fps: Option<Vec<(String, f64)>> = None;
        if frames_now == WARMUP + SAMPLE {
            let elapsed = started
                .take()
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or(1.0);
            fps.push((name.to_string(), SAMPLE as f64 / elapsed));
            if *stage + 1 < VIEWS.len() {
                *stage += 1;
                *frames = 0;
            } else {
                finished_fps = Some(std::mem::take(fps));
            }
        }
        self.view_mode = mode;
        if let Some(fps_taken) = finished_fps {
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
        metrics.insert("layer".into(), serde_json::json!("elevation"));
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

        self.poll_job();
        self.drive_script(ctx);

        self.top_bar(root);
        self.side_panel(root);
        // The readout consumes the hover the canvases wrote LAST frame (the
        // canvases render after the panels); clear it only afterwards, or the
        // Some arm is unreachable (review finding).
        self.bottom_panel(root, dt);
        self.hover = None;

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

        if self.needs_bake {
            self.rebake();
        }

        // Continuous repaint: keeps progress, playback and drags live.
        ctx.request_repaint();
    }
}
