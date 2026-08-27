//! The WorldMaker application: UI, canvases, input, the tectonic history
//! timeline (era picker), painting tools, and the scripted screenshot /
//! perf-harness modes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Instant;

use eframe::egui;
use eframe::egui_wgpu::wgpu;

use worldmaker_core::grid::{latlon_to_unit, Grid};
use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Projection;
use worldmaker_io::pending::{self, ActiveBrush, PendingEdits, StrokeEvent};
use worldmaker_sim::tectonics::{TectonicsHistory, TectonicsParams};
use worldmaker_sim::{Cancelled, Progress, WorldState};

use crate::boundaries::BoundarySet;
use crate::layers::{self, Layer};
use crate::pending_edits;
use crate::render::{
    flat_base_half_extents, globe_radius_px, globe_rotation, layer_flags, pack_shade_params,
    rotate_inv, FlatCallback, FlatView, GlobeCallback, GlobeView, SceneResources, ShadeParams,
    WorldBundle,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Globe,
    Flat,
    Split,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Draft6,
    Standard7,
    High8,
    Ultra9,
}

impl Preset {
    const ALL: [Preset; 4] = [
        Preset::Draft6,
        Preset::Standard7,
        Preset::High8,
        Preset::Ultra9,
    ];
    fn level(self) -> u32 {
        match self {
            Preset::Draft6 => 6,
            Preset::Standard7 => 7,
            Preset::High8 => 8,
            Preset::Ultra9 => 9,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Preset::Draft6 => "Draft (L6, 41k cells)",
            Preset::Standard7 => "Standard (L7, 164k cells)",
            Preset::High8 => "High (L8, 655k cells)",
            Preset::Ultra9 => "Ultra (L9, 2.6M cells)",
        }
    }
    /// Parse a `--preset` value (case-insensitive), d3a §10.2.
    pub fn from_cli(text: &str) -> Option<Preset> {
        match text.to_ascii_lowercase().as_str() {
            "draft6" => Some(Preset::Draft6),
            "standard7" => Some(Preset::Standard7),
            "high8" => Some(Preset::High8),
            "ultra9" => Some(Preset::Ultra9),
            _ => None,
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
/// Render-detail fBm defaults, fixed by the WO-0003 leg-4 24-config sweep
/// (decision-log 2026-08-27). The Detail slider scales amplitude only:
/// 0 = exactly the no-noise image, 1 = the tuned default.
const DETAIL_DEFAULT_OCTAVES: u32 = 5;
const DETAIL_DEFAULT_AMP_M: f32 = 350.0;
/// Flat zoom for the deterministic coast crop (graft 8): ~30 degrees of
/// longitude across the 1600 pt scripted window — an L8 cell spans ~13 pt,
/// wide enough to judge coastline embellishment against facet size.
const COAST_CROP_ZOOM: f32 = 12.0;
/// Undo for pending strokes: ⌘Z on macOS, Ctrl+Z elsewhere (egui's COMMAND
/// virtual modifier maps to the platform's primary modifier).
const UNDO_STROKE: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);

/// Scripted modes and world flags, driven from the command line (d3a §10.2).
pub struct Script {
    pub screenshots_dir: Option<PathBuf>,
    pub perf_out: Option<PathBuf>,
    /// Grid-build timings measured in main() before the window opened.
    pub grid_build_ms: Vec<(u32, f64)>,
    /// `--seed`: hashed exactly like the seed box (`seed_from_text`).
    pub seed: Option<String>,
    /// `--preset`: draft6 | standard7 | high8 | ultra9.
    pub preset: Option<Preset>,
    /// `--detail`: Detail slider value 0..=1.
    pub detail: Option<f32>,
    /// Dev sweep override: `--detail-octaves`.
    pub detail_octaves: Option<u32>,
    /// Dev sweep override: `--detail-amp-m`.
    pub detail_amp_m: Option<f32>,
}

impl Script {
    /// Graft 7 (screenshot parity): screenshot mode with NO explicit world
    /// flags forces seed "cyrus" + Standard7 + Detail 1.0 so the AFTER set
    /// matches the committed BEFORE set by default, not by checklist
    /// discipline.
    fn forces_parity(&self) -> bool {
        self.screenshots_dir.is_some()
            && self.seed.is_none()
            && self.preset.is_none()
            && self.detail.is_none()
            && self.detail_octaves.is_none()
            && self.detail_amp_m.is_none()
    }
    /// Sweep capture mode (d3a §12): a screenshot run carrying the dev detail
    /// overrides captures only the two judged crops — the deterministic coast
    /// close-up (graft 8) and the mountains stage — keeping the 24-run sweep
    /// fast.
    fn sweep_shots(&self) -> bool {
        self.screenshots_dir.is_some()
            && (self.detail_octaves.is_some() || self.detail_amp_m.is_some())
    }
}

enum ScriptState {
    Idle,
    Shot {
        stage: usize,
        frames: u32,
        requested: bool,
    },
    /// Perf loop (d3a §10.3): presets Standard7 -> High8 -> Ultra9, and per
    /// preset the three views, 40 warmup + 240 sampled frames each.
    Perf {
        preset_idx: usize,
        /// Sim wall time for the current preset's world build was recorded.
        sim_recorded: bool,
        stage: usize,
        frames: u32,
        started: Option<Instant>,
        metrics: Vec<(String, f64)>,
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
    values_gen: u64,
    overlay_gen: u64,
    needs_bake: bool,

    /// Pending edit strokes (Fix 1): drawing only accumulates here — the
    /// world regenerates when Regenerate folds them into the applied
    /// overlays below.
    pending: PendingEdits,
    /// The hotspot set the last completed run actually used — the base for
    /// classifying, folding and displaying pending hotspot strokes. None
    /// only before the first run ever completes.
    hotspot_baseline: Option<Vec<[f32; 3]>>,

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
    /// Render-detail slider, 0..=1 (off -> tuned default amplitude). A pure
    /// live view control, like sea level: uniform-only, never a rebake.
    detail: f32,
    /// Render-detail fBm octave count; `DETAIL_DEFAULT_OCTAVES` unless the
    /// dev sweep flag `--detail-octaves` overrode it.
    detail_octaves: u32,
    /// Render-detail base amplitude in meters at Detail 1.0;
    /// `DETAIL_DEFAULT_AMP_M` unless `--detail-amp-m` overrode it.
    detail_amp_m: f32,
    view_mode: ViewMode,
    preset: Preset,
    projection: Projection,
    graticule: bool,
    layer: Layer,
    /// Debug toggle: true cell boundaries via the bisector-margin machinery.
    debug_cell_bounds: bool,
    /// Debug toggle: legacy one-cell boundary bands on the plates layer.
    debug_legacy_bands: bool,
    tool: Tool,
    brush_radius_km: f32,

    // Canvas view state.
    globe: GlobeView,
    flat_pan: [f32; 2],
    flat_zoom: f32,
    /// Deferred flat-canvas centering: (lat, lon) radians + zoom, resolved
    /// against the real canvas rect on the next flat frame (the coast-crop
    /// stage sets it; pan depends on rect size, unknown at stage setup).
    flat_center_target: Option<(f32, f32, f32)>,

    // Cursor readout: (canvas name, cell id, lat deg, lon deg).
    hover: Option<(&'static str, u32, f32, f32)>,
    pick_hint: Option<u32>,

    // FPS.
    frame_times: Vec<f32>,
    last_frame: Instant,
    /// Wall time of the most recent finished tectonics run, seconds (perf
    /// loop records it per preset).
    last_sim_wall_s: Option<f64>,
    /// Device handle for the perf loop's per-frame GPU sync.
    gpu_device: wgpu::Device,

    // Scripted modes.
    script: Script,
    script_state: ScriptState,
}

impl WorldApp {
    pub fn new(cc: &eframe::CreationContext<'_>, script: Script) -> Self {
        // Match egui's ctrl/cmd+scroll zoom rate (default 1/200) to the
        // canvases' plain-scroll rate so both zoom at the same speed.
        cc.egui_ctx
            .options_mut(|o| o.input_options.scroll_zoom_speed = 0.002);
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
                &render_state.queue,
                render_state.target_format,
            ));

        // ----- CLI world flags (d3a §10.2) + graft-7 screenshot parity -----
        let forces_parity = script.forces_parity();
        if forces_parity {
            log::info!("screenshot parity: no explicit flags — forcing seed cyrus + Standard7 + detail 1.0");
        }
        let seed_text = script.seed.clone().unwrap_or_else(|| "cyrus".to_string());
        let master_seed = seed_from_text(&seed_text);
        // Perf mode loops the pinned Standard7 -> High8 -> Ultra9 presets and
        // starts at the first; --preset is advisory there and ignored.
        let preset = if script.perf_out.is_some() {
            if script.preset.is_some() {
                log::warn!("perf mode loops Standard7->High8->Ultra9; ignoring --preset");
            }
            Preset::Standard7
        } else if forces_parity {
            Preset::Standard7
        } else {
            script.preset.unwrap_or(Preset::High8)
        };
        let detail = script.detail.map(|d| d.clamp(0.0, 1.0)).unwrap_or(1.0);
        let detail_octaves = script
            .detail_octaves
            .unwrap_or(DETAIL_DEFAULT_OCTAVES)
            .clamp(1, 8);
        let detail_amp_m = script
            .detail_amp_m
            .unwrap_or(DETAIL_DEFAULT_AMP_M)
            .clamp(0.0, 2000.0);
        let placeholder_grid = Arc::new(Grid::build(0));
        let placeholder_ids: Arc<Vec<u32>> = Arc::new(vec![
            0;
            (crate::render::CELL_ID_TEX_W * crate::render::CELL_ID_TEX_H)
                as usize
        ]);
        let mut app = WorldApp {
            grid: placeholder_grid.clone(),
            cell_ids: placeholder_ids.clone(),
            history: None,
            world_state: None,
            job: None,
            bundle: Arc::new(placeholder_bundle(
                &placeholder_grid,
                &placeholder_ids,
                0,
                0,
                0,
            )),
            grid_gen: 0,
            values_gen: 0,
            overlay_gen: 0,
            needs_bake: false,
            pending: PendingEdits::new(),
            hotspot_baseline: None,
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
            detail,
            detail_octaves,
            detail_amp_m,
            view_mode: ViewMode::Split,
            preset,
            projection: Projection::Equirectangular,
            graticule: true,
            layer: Layer::Elevation,
            debug_cell_bounds: false,
            debug_legacy_bands: false,
            tool: Tool::None,
            brush_radius_km: 600.0,
            globe: GlobeView {
                yaw: 0.0,
                pitch: 0.35,
                zoom: 1.0,
            },
            flat_pan: [0.0, 0.0],
            flat_zoom: 1.0,
            flat_center_target: None,
            hover: None,
            pick_hint: None,
            frame_times: Vec::with_capacity(240),
            last_frame: Instant::now(),
            last_sim_wall_s: None,
            gpu_device: render_state.device.clone(),
            // Perf runs first when both flags are given; its completion chains
            // into the screenshot script so neither output is silently lost.
            script_state: if script.perf_out.is_some() {
                ScriptState::Perf {
                    preset_idx: 0,
                    sim_recorded: false,
                    stage: 0,
                    frames: 0,
                    started: None,
                    metrics: Vec::new(),
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

    /// Base set for classifying, folding and displaying pending hotspot
    /// strokes: the applied overlay wins when present, else the last
    /// completed run's set (D2 fold rules).
    fn hotspot_fold_base(&self) -> Option<&[[f32; 3]]> {
        self.hotspot_overlay
            .as_deref()
            .or(self.hotspot_baseline.as_deref())
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
        // Pending craton strokes are per-level cell ids: discard them (and
        // any live stroke) before anything can observe the new grid. Pending
        // hotspot strokes are unit vectors: they stay (D2). First statement
        // by design, so every grid rebuild is covered.
        self.pending.discard_cratons();
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
        // values_gen and overlay_gen ALWAYS bump with grid_gen (renderer
        // invariant), and a fresh right-sized placeholder bundle is published
        // immediately: the old bundle's Arcs are sized to the old grid, and
        // rebake's mid-run values reuse must never see them (judgement A4).
        self.values_gen += 1;
        self.overlay_gen += 1;
        self.pick_hint = None;
        // Craton paint is per-grid (cell ids change with level); hotspot
        // positions are unit vectors and survive a preset switch.
        self.craton_paint.clear();
        self.bundle = Arc::new(placeholder_bundle(
            &grid,
            &cell_ids,
            self.grid_gen,
            self.values_gen,
            self.overlay_gen,
        ));
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
            let t0 = Instant::now();
            let result = crate::worldgen::build_world(grid, seed, params, Some(worker_progress))
                .map(|(world, _params_hash)| (world, t0.elapsed().as_secs_f64()));
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
                self.last_sim_wall_s = Some(seconds);
                self.history = world.history.take();
                // Capture the hotspot set this run actually used (generated
                // or overlay): the fold base survives the next job start
                // dropping the history. Only overwrite when a history
                // actually arrived (d1-F3).
                if let Some(h) = &self.history {
                    self.hotspot_baseline = Some(h.hotspots.clone());
                }
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

    /// Re-bake the per-cell shading values for the viewed keyframe, run the
    /// pending-edit overlay pass, and publish a bundle (d3a §2.4).
    ///
    /// Sea level and Detail are NOT inputs here — they are live uniforms
    /// (ShadeParams); a slider drag never rebakes.
    fn rebake(&mut self) {
        self.needs_bake = false;

        // Values pass — only when a history exists. Mid-run (start_job drops
        // history) the previous bundle's values/boundaries Arcs are reused so
        // pending strokes can still display over the current world; that is
        // safe across a grid switch because rebuild_grid publishes a fresh
        // right-sized placeholder bundle first (judgement A4).
        let (values, boundaries) = if let Some(history) = &self.history {
            let kf = &history.keyframes[self.viewing_kf.min(history.keyframes.len() - 1)];
            self.values_gen += 1;
            // Smoothed boundary polylines are Plates-layer styling (d3a §8):
            // extracted from this keyframe's plate assignment only there,
            // empty everywhere else.
            let boundaries = if self.layer == Layer::Plates {
                Arc::new(crate::boundaries::extract(
                    &self.grid,
                    &kf.plate_id,
                    &kf.flags,
                ))
            } else {
                Arc::new(BoundarySet::empty())
            };
            (Arc::new(layers::bake_values(self.layer, kf)), boundaries)
        } else {
            (self.bundle.values.clone(), self.bundle.boundaries.clone())
        };

        // Overlay pass — on EVERY rebake, including history == None, so
        // pending edits render mid-run (feel-pass-design.md § D1). The live
        // stroke displays too (display_strokes appends it last, so it wins
        // per cell), and the hotspot base is the same fold base that click
        // classification and Regenerate use, keeping display and fold
        // coherent (D2).
        let mut overlay = vec![0u32; self.grid.cell_count() as usize];
        let display = self.pending.display_strokes();
        pending_edits::apply_overlay(
            &pending_edits::OverlayInput {
                grid: &self.grid,
                pending: &display,
                generated_hotspots: self.hotspot_fold_base(),
            },
            &mut overlay,
        );
        self.overlay_gen += 1;

        self.bundle = Arc::new(WorldBundle {
            grid: self.grid.clone(),
            values,
            overlay,
            cell_ids: self.cell_ids.clone(),
            boundaries,
            grid_gen: self.grid_gen,
            values_gen: self.values_gen,
            overlay_gen: self.overlay_gen,
        });
    }

    /// The live shading uniforms for both canvases this frame.
    fn shade_params(&self) -> ShadeParams {
        pack_shade_params(
            self.master_seed,
            layer_flags(self.layer, self.debug_cell_bounds, self.debug_legacy_bands),
            self.detail_octaves,
            self.sea_level_m,
            self.detail * self.detail_amp_m,
            self.grid.cell_count(),
        )
    }

    fn fps(&self) -> f64 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        let sum: f32 = self.frame_times.iter().sum();
        self.frame_times.len() as f64 / sum.max(1e-6) as f64
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
        }
        // Fix 1: every tool frame funnels into the pending-edit machinery in
        // worldmaker-io — drawing accumulates strokes and can never launch a
        // run. Stroke end (release past the globe limb / projection outline
        // included) is handled inside handle_stroke_event, outside its hit
        // gate, so it stays hover-independent (review finding preserved).
        let brush = match self.tool {
            Tool::None => return,
            Tool::CratonPaint => ActiveBrush::Craton {
                sign: 1,
                radius_km: self.brush_radius_km,
            },
            Tool::CratonErase => ActiveBrush::Craton {
                sign: -1,
                radius_km: self.brush_radius_km,
            },
            Tool::Hotspot => ActiveBrush::Hotspot,
        };
        let ev = StrokeEvent {
            hit_cell: hit.map(|(cell, _, _)| cell),
            dragged: response.dragged(),
            clicked: response.clicked(),
            drag_stopped: response.drag_stopped(),
        };
        // Same base rule as hotspot_fold_base(), inlined so the borrows
        // split per field.
        let base = self
            .hotspot_overlay
            .as_deref()
            .or(self.hotspot_baseline.as_deref());
        if pending::handle_stroke_event(&mut self.pending, &self.grid, &brush, &ev, base) {
            self.needs_bake = true;
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
            // is_finite: two coincident touch points make zoom_delta() 0/0 = NaN,
            // and NaN would stick in the zoom state forever (clamp propagates it).
            if factor.is_finite() && factor > 0.0 && factor != 1.0 {
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
                    shade: self.shade_params(),
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
        // Resolve a deferred centering request now that the rect is known:
        // put the projected target point at the rect center at the given
        // zoom (mx at center = -pan_x/B_x, my at center = pan_y/B_y).
        if let Some((lat, lon, zoom)) = self.flat_center_target.take() {
            let base = flat_base_half_extents(self.projection, rect.width(), rect.height());
            let (fx, fy) = self.projection.project(lat, lon);
            self.flat_zoom = zoom;
            self.flat_pan = [-fx * base[0] * zoom, fy * base[1] * zoom];
        }
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
            if factor.is_finite() && factor > 0.0 && factor != 1.0 {
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
                    shade: self.shade_params(),
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
                    // Seed change keeps everything pending (D2); just close
                    // any half-drawn stroke defensively.
                    self.pending.end_stroke();
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
                        for p in Preset::ALL {
                            ui.selectable_value(&mut preset, p, p.label());
                        }
                    });
                if preset != self.preset {
                    self.preset = preset;
                    self.rebuild_grid(preset.level());
                }
                ui.separator();

                ui.label("Sea level:");
                // Offset around the solved sea level. A pure LIVE view
                // control since Fix 3: it rides the shading uniforms every
                // frame — dragging it never rebakes.
                ui.add(
                    egui::Slider::new(&mut self.sea_level_m, -4000.0..=4000.0)
                        .suffix(" m")
                        .fixed_decimals(0),
                );
                ui.separator();

                ui.label("Detail:");
                // Render-detail amplitude, off -> tuned default. Live uniform
                // like sea level (minimal slider; placement finalized leg 4).
                ui.add(egui::Slider::new(&mut self.detail, 0.0..=1.0).fixed_decimals(2));
                ui.separator();

                // Debug toggles (uniform bits; final top-bar layout in leg 4).
                ui.checkbox(&mut self.debug_cell_bounds, "Cell bounds");
                ui.checkbox(&mut self.debug_legacy_bands, "Legacy bands");

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
                    .add_enabled(self.job.is_none(), egui::Button::new("Regenerate"))
                    .clicked()
                {
                    // The fold point (D2): commit the live stroke, drain the
                    // pending list, fold it into the applied overlays, then
                    // run history off-thread with existing progress + cancel.
                    self.pending.end_stroke();
                    let strokes = self.pending.take_all();
                    let outcome = pending::fold(&strokes, self.hotspot_fold_base());
                    for (c, v) in outcome.craton_updates {
                        self.craton_paint.insert(c, v);
                    }
                    if let Some(hs) = outcome.hotspot_overlay {
                        self.hotspot_overlay = Some(hs);
                    }
                    // Clear the stale pending tint immediately (d1-F4).
                    self.needs_bake = true;
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
                // Badge: committed pending strokes, shown under every tool
                // (Navigate included) whenever any exist.
                ui.horizontal(|ui| {
                    ui.heading("Paint");
                    let n = self.pending.stroke_count();
                    if n > 0 {
                        ui.label(format!("{n} pending edit(s)"));
                    }
                });
                ui.horizontal(|ui| {
                    // Mirrors Cmd/Ctrl+Z: cancel the live stroke first, else
                    // pop the newest pending stroke. No redo.
                    if ui
                        .add_enabled(!self.pending.is_empty(), egui::Button::new("Undo"))
                        .clicked()
                        && self.pending.undo()
                    {
                        self.needs_bake = true;
                    }
                    // Discards pending strokes only; applied overlays stay.
                    if ui
                        .add_enabled(!self.pending.is_empty(), egui::Button::new("Discard edits"))
                        .clicked()
                    {
                        self.pending.discard_all();
                        self.needs_bake = true;
                    }
                });
                let before = self.tool;
                ui.selectable_value(&mut self.tool, Tool::None, "Navigate");
                ui.selectable_value(&mut self.tool, Tool::CratonPaint, "Craton brush");
                ui.selectable_value(&mut self.tool, Tool::CratonErase, "Craton eraser");
                ui.selectable_value(&mut self.tool, Tool::Hotspot, "Hotspots");
                if self.tool != before {
                    // A tool switch commits any half-drawn stroke (D2).
                    self.pending.end_stroke();
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
                    let clearable = !self.craton_paint.is_empty() || self.pending.has_craton();
                    if ui
                        .add_enabled(clearable, egui::Button::new("Clear craton paint"))
                        .clicked()
                    {
                        // Staged-param edit (D2): discards pending craton
                        // strokes (incl. live) AND clears the applied craton
                        // paint. No run; the world effect lands at the next
                        // Regenerate. Not undoable.
                        self.pending.discard_cratons();
                        self.craton_paint.clear();
                        self.needs_bake = true;
                    }
                    ui.label("Paint accumulates as pending strokes; Regenerate re-runs history with them.");
                }
                if self.tool == Tool::Hotspot {
                    if self.hotspot_fold_base().is_none() {
                        ui.label("Hotspots available after the first world finishes generating.");
                    } else {
                        ui.label("Click to add a hotspot; click an existing one to remove it.");
                    }
                    let resettable = self.hotspot_overlay.is_some() || self.pending.has_hotspot();
                    if ui
                        .add_enabled(resettable, egui::Button::new("Reset to generated"))
                        .clicked()
                    {
                        // Staged-param edit (D2): discards pending hotspot
                        // strokes AND drops the applied overlay; the next
                        // Regenerate (with no new strokes) re-generates
                        // hotspots from seed. A hotspot stroke drawn after
                        // this re-anchors to the still-displayed last-run
                        // set (d1-F2). Not undoable.
                        self.pending.discard_hotspots();
                        self.hotspot_overlay = None;
                        self.needs_bake = true;
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
            5 => {
                self.view_mode = ViewMode::Flat;
                self.layer = Layer::Elevation;
                self.viewing_kf = (kf_count * 3) / 5;
                "timeline"
            }
            _ => {
                // The deterministic coast crop (graft 8 / A9): flat close-up
                // centered on the max-slope near-coast cell of the final era.
                self.view_mode = ViewMode::Flat;
                self.layer = Layer::Elevation;
                self.viewing_kf = last;
                if let Some(h) = &self.history {
                    let kf = &h.keyframes[last];
                    if let Some(c) = coast_crop_cell(&self.grid, &kf.elev_m) {
                        let c = c as usize;
                        self.flat_center_target =
                            Some((self.grid.lat[c], self.grid.lon[c], COAST_CROP_ZOOM));
                    }
                }
                "coast"
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
        const NAMES: [&str; 7] = [
            "globe",
            "flat",
            "split",
            "plates",
            "mountains",
            "timeline",
            "coast",
        ];
        /// The normal documentation set.
        const NORMAL_SEQ: [usize; 6] = [0, 1, 2, 3, 4, 5];
        /// Sweep mode (d3a §12): only the two judged crops.
        const SWEEP_SEQ: [usize; 2] = [6, 4];
        let seq: &[usize] = if self.script.sweep_shots() {
            &SWEEP_SEQ
        } else {
            &NORMAL_SEQ
        };
        let (stage, frames, requested) = match &self.script_state {
            ScriptState::Shot {
                stage,
                frames,
                requested,
            } => (*stage, *frames, *requested),
            _ => return,
        };
        let shot = seq[stage];
        let frames = frames + 1;
        let mut requested = requested;
        if frames == 1 {
            self.setup_shot_stage(shot);
            self.needs_bake = true;
            log::info!("screenshot stage {stage}: {}", NAMES[shot]);
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
            let name = NAMES[shot];
            if let Err(e) = save_color_image(&image, &dir.join(format!("{name}.png"))) {
                log::error!("failed to save screenshot {name}: {e:#}");
            } else {
                log::info!("saved screenshot {name}.png");
            }
            self.script_state = if stage + 1 < seq.len() {
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
        /// The pinned preset loop (d3a §10.3): Standard7 -> High8 -> Ultra9.
        const PRESETS: [(Preset, &str); 3] = [
            (Preset::Standard7, "L7"),
            (Preset::High8, "L8"),
            (Preset::Ultra9, "L9"),
        ];
        const VIEWS: [(ViewMode, &str); 3] = [
            (ViewMode::Globe, "globe_fps"),
            (ViewMode::Flat, "flat_fps"),
            (ViewMode::Split, "split_fps"),
        ];
        const WARMUP: u32 = 40;
        const SAMPLE: u32 = 240;

        // GPU-sync the sampling loop: with vsync off (and macOS recycling
        // drawables instantly when the window is occluded) the CPU can spin
        // far ahead of the GPU, counting frames whose draws never finished.
        // Waiting for all submitted GPU work once per frame makes the
        // sampled fps the real per-frame cost.
        let _ = self.gpu_device.poll(wgpu::PollType::wait_indefinitely());

        // Copy the state out (drive_shot's pattern) so preset switches may
        // borrow all of self.
        let (mut preset_idx, mut sim_recorded, mut stage, mut frames, mut started, mut metrics) =
            match &mut self.script_state {
                ScriptState::Perf {
                    preset_idx,
                    sim_recorded,
                    stage,
                    frames,
                    started,
                    metrics,
                } => (
                    *preset_idx,
                    *sim_recorded,
                    *stage,
                    *frames,
                    *started,
                    std::mem::take(metrics),
                ),
                _ => return,
            };

        let (target_preset, tag) = PRESETS[preset_idx];
        // Preset switch: run the full world build at that level, exactly as
        // the UI combo would; drive_script's wait gate holds the loop until
        // the history lands. Elevation layer + Detail default = the pinned
        // "smooth shading + render detail on, smoothed boundaries off".
        if self.preset != target_preset {
            self.preset = target_preset;
            self.layer = Layer::Elevation;
            self.needs_bake = true;
            log::info!("perf: switching to preset {tag}");
            self.script_state = ScriptState::Perf {
                preset_idx,
                sim_recorded,
                stage,
                frames,
                started,
                metrics,
            };
            self.rebuild_grid(target_preset.level());
            return;
        }
        if !sim_recorded {
            // The wait gate guarantees the history for this preset is in.
            if let Some(w) = self.last_sim_wall_s {
                metrics.push((format!("sim_wall_s_{tag}_500my"), w));
            }
            sim_recorded = true;
        }

        let (mode, name) = VIEWS[stage];
        frames += 1;
        if frames == WARMUP {
            started = Some(Instant::now());
        }
        let mut finished: Option<Vec<(String, f64)>> = None;
        if frames == WARMUP + SAMPLE {
            let elapsed = started
                .take()
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or(1.0);
            metrics.push((format!("{name}_{tag}"), SAMPLE as f64 / elapsed));
            frames = 0;
            if stage + 1 < VIEWS.len() {
                stage += 1;
            } else if preset_idx + 1 < PRESETS.len() {
                preset_idx += 1;
                sim_recorded = false;
                stage = 0;
            } else {
                finished = Some(std::mem::take(&mut metrics));
            }
        }
        self.view_mode = mode;
        if let Some(metrics_taken) = finished {
            self.write_perf_results(&metrics_taken);
            // Chain into screenshots if both flags were given.
            self.script_state = if self.script.screenshots_dir.is_some() {
                if self.script.forces_parity() {
                    // Graft-7 parity survives the chained run: the perf loop
                    // left the app at Ultra9, so restore the forced trio and
                    // rebuild before the Shot stages start.
                    self.preset = Preset::Standard7;
                    self.detail = 1.0;
                    self.rebuild_grid(Preset::Standard7.level());
                }
                ScriptState::Shot {
                    stage: 0,
                    frames: 0,
                    requested: false,
                }
            } else {
                ScriptState::Closing
            };
        } else {
            self.script_state = ScriptState::Perf {
                preset_idx,
                sim_recorded,
                stage,
                frames,
                started,
                metrics,
            };
        }
    }

    fn write_perf_results(&self, collected: &[(String, f64)]) {
        let Some(out) = &self.script.perf_out else {
            return;
        };
        let mut metrics = serde_json::Map::new();
        for (level, ms) in &self.script.grid_build_ms {
            metrics.insert(format!("grid_build_ms_L{level}"), serde_json::json!(ms));
        }
        for (name, value) in collected {
            metrics.insert(
                name.clone(),
                serde_json::json!((value * 100.0).round() / 100.0),
            );
        }
        // fps_grid_level is retired: the per-preset loop suffixes every fps
        // and sim-wall key with its level (d3a §10.3).
        metrics.insert("detail".into(), serde_json::json!(self.detail));
        metrics.insert(
            "detail_octaves".into(),
            serde_json::json!(self.detail_octaves),
        );
        metrics.insert("detail_amp_m".into(), serde_json::json!(self.detail_amp_m));
        metrics.insert("fps_vsync_off".into(), serde_json::json!(true));
        // The loop waits for submitted GPU work every frame: fps counts only
        // frames the GPU actually finished (an occluded macOS window never
        // blocks on present, so unsynced counts measure CPU encode speed).
        metrics.insert("fps_gpu_synced".into(), serde_json::json!(true));
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

/// The deterministic coast-crop target (graft 8 as amended by A9): among
/// cells with |elev| < 200 m, the one maximizing
/// slope(c) = max over CSR neighbors |elev[n] - elev[c]|, found by a serial
/// id-ordered scan with strict `>` so ties go to the lowest cell id.
fn coast_crop_cell(grid: &Grid, elev_m: &[i16]) -> Option<u32> {
    let mut best: Option<(u32, i32)> = None;
    for c in 0..grid.cell_count() {
        let e = i32::from(elev_m[c as usize]);
        if e.abs() >= 200 {
            continue;
        }
        let mut slope = 0i32;
        for &nb in grid.neighbors_of(c) {
            let d = (i32::from(elev_m[nb as usize]) - e).abs();
            if d > slope {
                slope = d;
            }
        }
        if best.map(|(_, bs)| slope > bs).unwrap_or(true) {
            best = Some((c, slope));
        }
    }
    best.map(|(c, _)| c)
}

/// A neutral bundle for a freshly built grid: zero-elevation values, zeroed
/// overlay, empty boundary set, published with the caller's (just bumped)
/// generations. Keeps every buffer right-sized for the new grid before the
/// first real bake lands (judgement A4).
fn placeholder_bundle(
    grid: &Arc<Grid>,
    cell_ids: &Arc<Vec<u32>>,
    grid_gen: u64,
    values_gen: u64,
    overlay_gen: u64,
) -> WorldBundle {
    let n = grid.cell_count() as usize;
    WorldBundle {
        grid: grid.clone(),
        values: Arc::new(vec![[0f32.to_bits(), 0u32]; n]),
        overlay: vec![0u32; n],
        cell_ids: cell_ids.clone(),
        boundaries: Arc::new(BoundarySet::empty()),
        grid_gen,
        values_gen,
        overlay_gen,
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

        // Cmd/Ctrl+Z: cancel the live stroke first, else pop the newest
        // pending stroke (no redo). Gated so the seed field's own text undo
        // wins while it has keyboard focus.
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_shortcut(&UNDO_STROKE))
            && self.pending.undo()
        {
            self.needs_bake = true;
        }

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

#[cfg(test)]
mod tests {
    /// Fix 1's sanctioned-trigger tripwire (D2 guard c): the grid rebuild
    /// (preset switch / startup), the seed Generate/Enter branch, and
    /// Regenerate are the only places a simulation launches. Adding a call
    /// site is a design change — update this count deliberately, with a
    /// decision-log row, never to make a red test green.
    #[test]
    fn app_has_exactly_the_sanctioned_sim_triggers() {
        let src = include_str!("app.rs");
        // Needle assembled at runtime so this test's own source never
        // matches (and no comment in this file may quote it).
        let needle = ["self.start", "_job()"].concat();
        assert_eq!(src.matches(&needle).count(), 3);
    }
}
