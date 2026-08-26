# Stage U reader (a) — app.rs input handling + UI state

Territory: `crates/worldmaker-app/src/app.rs` (1222 lines, as of commit 9d5d272).
All line numbers are against that file unless a path is given. No code changed.

## 0. Cast of types

| Type | Where | Notes |
|---|---|---|
| `WorldApp` | app.rs:100–157 | the one app struct; every field listed below |
| `Tool` | app.rs:55–61 | `enum Tool { None, CratonPaint, CratonErase, Hotspot }`, `Clone, Copy, PartialEq, Eq` |
| `Preset` | app.rs:31–53 | `enum Preset { Draft6, Standard7, High8 }`; `level()` → 6/7/8; `label()` strings |
| `ViewMode` | app.rs:24–29 | `Globe, Flat, Split` |
| `SimJob` | app.rs:93–98 | `{ progress: Arc<Progress>, rx: mpsc::Receiver<anyhow::Result<(WorldState, f64)>>, started: Instant }` |
| `WorldBundle` | **render.rs:24–37** (C territory) | `{ grid: Arc<Grid>, colors: Vec<u32>, cell_ids: Arc<Vec<u32>>, grid_gen: u64, field_gen: u64 }`; `rasterize_cell_ids(&Grid) -> Vec<u32>` at render.rs:43 |
| `BakeOverlay<'a>` | layers.rs:166–180 | `{ craton: Option<&'a [(u32, i8)]>, hotspot_cells: Option<&'a [u32]> }` + `BakeOverlay::NONE` |
| `TectonicsParams` | worldmaker-sim/src/tectonics/mod.rs:73–90 | `#[derive(Clone, Debug)]` — **no serde derives** |
| `TectonicsHistory` | worldmaker-sim/src/tectonics/keyframe.rs:237–244 | `{ dt_my, keyframe_interval_my, keyframes: Vec<Keyframe>, hotspots: Vec<[f32; 3]>, diagnostics }` |
| `Progress` / `Cancelled` | worldmaker-sim/src/pipeline.rs:34–63 | atomics: `set_fraction/fraction/request_cancel/cancel_requested` |

`WorldApp` fields relevant to Track A (app.rs:100–157):
`grid: Arc<Grid>`, `cell_ids: Arc<Vec<u32>>`, `history: Option<TectonicsHistory>`,
`world_state: Option<WorldState>`, `job: Option<SimJob>`, `bundle: Arc<WorldBundle>`,
`grid_gen: u64`, `field_gen: u64`, `needs_bake: bool`;
param copies `plate_count: u32`, `land_fraction: f32`, `tectonic_vigor: f32`,
`span_my: f32`, `hotspot_count: u32`, **`craton_paint: BTreeMap<u32, i8>`**,
**`hotspot_overlay: Option<Vec<[f32; 3]>>`**; era picker `viewing_kf/present_kf: usize`,
`playing: bool`, `play_accum: f32`; controls `seed_text: String`, `master_seed: u64`,
`sea_level_m: f32`, `view_mode`, `preset`, `projection`, `graticule`, `layer`,
`tool: Tool`, `brush_radius_km: f32`, `craton_stroke_dirty: bool`; canvas state
`globe: GlobeView`, `flat_pan: [f32; 2]`, `flat_zoom: f32`; `hover`, `pick_hint: Option<u32>`.

## 1. Event → tool-application path (both canvases)

Frame loop is `impl eframe::App for WorldApp::ui` (app.rs:1168): `poll_job` →
`drive_script` → `top_bar` → `side_panel` → `bottom_panel` → CentralPanel with
canvases (1192–1213) → `if self.needs_bake { self.rebake() }` (1215–1217) →
`ctx.request_repaint()`.

Globe (`globe_canvas`, app.rs:561–616):
1. `ui.allocate_rect(rect, egui::Sense::click_and_drag())` → `response`.
2. If `tool == Tool::None`, drag rotates (`yaw/pitch`, 564–569); scroll/pinch
   folds `smooth_scroll_delta.y` + `zoom_delta()` into `globe.zoom` (570–581).
3. Cursor→cell (584–599): unproject hover pos onto the sphere, rotate by
   `globe_rotation(yaw, pitch)` inverse, then
   **`self.grid.nearest_cell(world, self.pick_hint)`** (595) and
   `unit_to_latlon` → `hit: Option<(u32 cell, f32 lat_rad, f32 lon_rad)>`.
4. `self.canvas_common(&response, hit, "globe")` (600).

Flat (`flat_canvas`, app.rs:618–677): same shape. Cursor→cell (643–655):
pan/zoom math → `self.projection.invert(mx, my)` →
`self.grid.nearest_cell(latlon_to_unit(lat, lon), self.pick_hint)` (650–652) →
`self.canvas_common(&response, hit, "flat")` (656).

`canvas_common(&mut self, response: &egui::Response, hit: Option<(u32, f32, f32)>, canvas: &'static str)`
(app.rs:528–559), the shared funnel:
- On hit: sets `pick_hint` + `hover`; if `tool != None` and
  `(response.dragged() || response.clicked())` →
  **`self.apply_tool(cell, clicked)`** (539); on `true` sets `needs_bake = true`
  and, for craton tools, `craton_stroke_dirty = true` (540–544).
- Outside the hit gate (551–558): for craton tools, on
  `response.drag_stopped() || response.clicked()` with `craton_stroke_dirty`
  → clear flag and **`self.start_job()`** — a review finding made this
  hover-independent so a release off the map still ends the stroke.
  `craton_stroke_dirty` is one bool shared by both canvases.

`apply_tool(&mut self, cell: u32, clicked: bool) -> bool` (app.rs:475–524):
- Craton arms: `v = +1` (paint) / `-1` (erase);
  `cells_within_radius(cell, self.brush_radius_km)` (453–472; BFS over
  `grid.neighbors_of` gated by `dot >= cos(radius_km / 6371.0)`; constants
  app.rs:66–67) → `self.craton_paint.insert(c, v)` per cell; returns whether
  anything changed. Note the eraser **inserts −1** (force ocean at setup); it
  never removes +1 entries.
- Hotspot arm (click-only): base set =
  `self.hotspot_overlay.clone().or_else(|| self.history.as_ref().map(|h| h.hotspots.clone()))`;
  bails (returns false) if neither exists — guard against replacing the
  generated set with a single click while a run is in flight (497–507).
  If an existing spot lies within `HOTSPOT_REMOVE_KM = 300.0` km (dot ≥
  `cos(300/6371)`, first match by Vec order, 509–512) it is removed, else the
  clicked cell's unit vector `grid.positions[cell]` is pushed. Then
  `self.hotspot_overlay = Some(spots)` and **`self.start_job()` (line 520)** —
  a second, direct stroke→sim route.

## 2. Where drawing triggers regeneration today (the thing Fix 1 removes)

Every route from a brush to `Pipeline::run`:

| # | Trigger | Call site | Chain |
|---|---|---|---|
| 1 | craton stroke end (release or click) | canvas_common app.rs:557 | `start_job()` |
| 2 | hotspot add/remove click | apply_tool app.rs:520 | `start_job()` |
| 3 | "Clear craton paint" button | side_panel app.rs:823–827 | `craton_paint.clear()` + `start_job()` |
| 4 | "Reset to generated" (hotspots) | side_panel app.rs:832–835 | `hotspot_overlay = None` + `start_job()` |

Non-stroke routes that also run sim (stay under Fix 1, become the fold points):
Generate/Enter on seed (top_bar 688–691), "Generate history" (side_panel
782–787), preset switch → `rebuild_grid` (top_bar 730–733 → app.rs:306),
initial `WorldApp::new` (258).

`start_job` (app.rs:312–342): cancels any live job via
`job.progress.request_cancel()`; **drops `self.history` and `self.world_state`
immediately** (keyframe memory budget: never two histories); spawns a
`std::thread` that builds `WorldState::new(grid)` + `Pipeline::new()` +
`TectonicsStage::new(params)` where `params = self.current_params()`
(app.rs:262–272), sets `ctx.progress`, and calls
`pipeline.run(&ctx, &mut world)` (signature: pipeline.rs:165,
`-> anyhow::Result<Vec<&'static str>>`); result goes back over
`mpsc::channel`. `poll_job` (345–385) installs the finished history each
frame, sets `present_kf` = last, keeps `viewing_kf = 0` while a craton tool is
active (362–367), calls `set_present`, sets `needs_bake = true`; `Cancelled`
is detected by `e.downcast_ref::<Cancelled>()`.

`current_params()` folds UI copies into `TectonicsParams`, with
`craton_overlay: self.craton_paint.iter().map(|(&c, &v)| (c, v)).collect()`
(BTreeMap ⇒ already id-sorted) and `hotspot_overlay: self.hotspot_overlay.clone()`.

## 3. How TectonicsParams stores the overlays (exact)

worldmaker-sim/src/tectonics/mod.rs:73–90, `#[derive(Clone, Debug)]`:
- `pub craton_overlay: Vec<(u32, i8)>` — "sorted by cell id; +1 paints a
  continental nucleus, −1 forces ocean at setup".
- `pub hotspot_overlay: Option<Vec<[f32; 3]>>` — "when set, **replaces the
  generated hotspot set entirely**".
Doc comment at mod.rs:71: all params are hashed into `params_hash`
(`fn params_hash` mod.rs:136), so any change re-runs the stage.
App-side staging: `craton_paint: BTreeMap<u32, i8>` (app.rs:118) and
`hotspot_overlay: Option<Vec<[f32; 3]>>` (app.rs:119).

## 4. Side-panel paint section

`side_panel` (app.rs:756–838), egui `Panel::left("world-params")`. Paint block
starts at 802: four `ui.selectable_value(&mut self.tool, …)` rows — Navigate
(`Tool::None`), "Craton brush", "Craton eraser", "Hotspots". Tool switch
(809–816): `needs_bake = true`; craton tools also force `viewing_kf = 0` and
stop playback ("cratons are painted on the initial state"). Craton tools show
the `brush_radius_km` slider (150–2000 km, 817–822) and the
"Clear craton paint" button; hotspot tool shows help text + "Reset to
generated". Selection state is just `self.tool: Tool` + `self.brush_radius_km:
f32`; there is no per-tool struct and no badge/pending-count UI today.

## 5. Job triggering, progress, cancel, guards

- Off-thread: plain `std::thread::spawn` per run (app.rs:325), result over
  `mpsc`; no thread pool, no job queue. One `Option<SimJob>` slot.
- Progress: `Arc<Progress>` (pipeline.rs:34–57), lock-free;
  UI reads `job.progress.fraction()` into a `ProgressBar` + elapsed-seconds
  label; Cancel button calls `job.progress.request_cancel()` (side_panel
  789–800).
- Guards are **per-widget, not structural**: "Generate history" is
  `add_enabled(self.job.is_none(), …)` (783); the era picker is
  `add_enabled_ui(kf_count > 1 && self.job.is_none(), …)` (bottom_panel 879).
  `start_job` itself may be called at any time (it cancels-and-replaces), and
  strokes/preset/seed routes are not gated on `job.is_none()`.
- Scripted modes wait on `self.job.is_some() || self.history.is_none()`
  (drive_script 999–1002).

## 6. Preset switch and seed change — what is cleared/kept

- Preset switch (top_bar 730–733) → `rebuild_grid(level)` (app.rs:275–307):
  rebuilds grid + cell-id raster, bumps `grid_gen`/`field_gen`, clears
  `pick_hint`, **`self.craton_paint.clear()` (line 298)** — comment: craton
  paint is per-grid, "hotspot positions are unit vectors and survive a preset
  switch" — publishes a placeholder-colored bundle, then `start_job()`.
  `hotspot_overlay` is untouched. This matches the pinned discard/keep rule,
  but note the switch **also immediately regenerates**.
- Seed change (top_bar 685–691): editing `seed_text` does nothing until
  Generate/Enter, which sets `master_seed = seed_from_text(&seed_text)` then
  `start_job()`. Keeps `craton_paint`, `hotspot_overlay`, tool, view state.
- `WorldApp::new` (160–260) seeds with "cyrus", `Standard7`, then
  `rebuild_grid` → first job.

## 7. rebake() — what it consumes (A↔C freeze input)

`rebake(&mut self)` (app.rs:397–440), called from exactly one place: end of
frame (app.rs:1215–1217) when `needs_bake` is set. Consumes:
`self.history` (**early-returns if `None`, after clearing `needs_bake`** —
399–401), `self.viewing_kf` (clamped), **`self.tool`** (the overlay is built
per active tool, 406–429):
- Craton tools → `BakeOverlay { craton: Some(&Vec<(u32,i8)> from craton_paint), hotspot_cells: None }`.
- Hotspot tool → collects marker cells: for each spot in
  `self.hotspot_overlay.as_deref().unwrap_or(&history.hotspots)`,
  `grid.nearest_cell(*h, None)` + its `neighbors_of` ring.
- `Tool::None` → `BakeOverlay::NONE`.
Then `layers::bake(self.layer, kf, self.sea_level_m, &overlay) -> Vec<u32>`
(layers.rs:184), bumps `field_gen`, publishes a fresh
`Arc<WorldBundle> { grid, colors, cell_ids, grid_gen, field_gen }`. Renderers
diff the gens to decide buffer uploads. Overlay visuals today are **tint only**
(layers.rs:159–163: `PAINT_CONTINENT` orange, `PAINT_OCEAN` blue, magenta
`HOTSPOT_MARK`), blended inside `layers::bake`; there is no outline mechanism.

`needs_bake` setters today: layer switch (719), sea-level slider (746), tool
switch (810), craton/hotspot paint via canvas_common (540), timeline scrub
(901) and playback (934), poll_job completion (369), Clear-paint button (825),
script stages (1028).

## 8. Undo state

**None exists.** No undo stack, no stroke history, no `egui::Undoer` usage.
`craton_paint` is a flat coverage map (a second stroke over the same cells is
absorbed — `insert` returning the same value reports "unchanged"), so stroke
boundaries are not recoverable from current state. The only keyboard handling
in the whole app is Enter on the seed field (app.rs:687); nothing consumes
Ctrl/Cmd+Z today.

## 9. Ctrl+Z on macOS (egui 0.36.1, eframe 0.36.1 wgpu backend)

- egui's virtual modifier `egui::Modifiers::COMMAND` maps to **⌘ Cmd on macOS**
  and Ctrl on Windows/Linux (`Modifiers::command` is set from `mac_cmd` on
  mac, from `ctrl` elsewhere). The contract's "Ctrl+Z" should be implemented as
  `egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z)` and
  consumed via `ui.input_mut(|i| i.consume_shortcut(&SHORTCUT))` so it is
  Cmd+Z for Dan. Matching raw `Modifiers { ctrl: true, .. }` would be the
  wrong key on the Mac.
- Focus caveat: the seed `TextEdit` consumes Cmd+Z for its own text undo while
  focused; gate the stroke-undo on `!ctx.wants_keyboard_input()` (or check
  `ctx.memory(|m| m.focused())`) so painting undo never fights text undo.

## 10. Contract frictions to design around (flags)

1. **Four stroke→sim routes to sever** (table in §2). The structural guard
   ("no route to Pipeline::run from stroke handling") must also cover
   `apply_tool`'s hotspot arm (app.rs:520) and the two side-panel buttons —
   not just canvas_common's stroke-end branch.
2. **Hotspot strokes are deltas; `hotspot_overlay` is an absolute replacement
   set.** Folding `HotspotAdd/Remove` at Regenerate needs a base set:
   today that is `hotspot_overlay.or(history.hotspots)`, and `start_job`
   drops the history immediately, so after a cancelled/failed run there can be
   **no base to fold onto** (apply_tool already bails in that case, 497–507).
   The fold rule and the remove-matching rule (nearest within 300 km — first
   match in Vec order today, not nearest; app.rs:509–512) must be pinned
   deterministically, including removes that target pending adds.
3. **Overlay display is tool-gated and history-gated.** rebake shows craton
   paint only while a craton tool is active and early-returns with no history
   (start_job sets `history = None`, so mid-run the last bundle freezes and a
   `needs_bake` request is silently dropped, 398–401). Pending strokes that
   must render "with current world behind" whenever they exist — including
   during a run and with Navigate selected — do not fit the current
   `match self.tool` shape; the frozen `apply_overlay` interface needs to take
   the pending set explicitly, not infer it from `tool`.
4. **Tint + outline**: only per-cell tint exists (layers.rs); outlines are a
   new mechanism and land in C-owned code — the frozen interface must carry
   enough data for outlines (e.g. stroke cell sets, not pre-blended colors).
5. **Undo granularity**: `craton_paint: BTreeMap<u32, i8>` cannot reproduce
   stroke boundaries or restore overwritten values; the pending-strokes list
   must record per-stroke cell payloads itself. A drag today applies
   incrementally every frame (`response.dragged()`), so "stroke" = frames
   between drag start and `drag_stopped()`/`clicked()` — `craton_stroke_dirty`
   (one shared bool, both canvases) is the only stroke-boundary state today.
   `Response::drag_started()` exists but is unused.
6. **`TectonicsParams` has no serde derives** (`Clone, Debug` only) and every
   field feeds `params_hash` (mod.rs:71, 136). The serde `Stroke` type goes in
   worldmaker-io per the contract (serde already a dep there;
   worldmaker-app already depends on worldmaker-io — Cargo.toml:12 — for
   `ResultsFile`, so the app can use the type with no new edges).
7. **Preset switch regenerates unconditionally** (`rebuild_grid` →
   `start_job`). Under Fix 1, keeping pending hotspot strokes across a preset
   switch means the discard/keep logic runs before that rebuild; craton
   discard is currently modeled by `craton_paint.clear()` at app.rs:298.
8. **A/C seam inside functions A doesn't own**: `rebuild_grid`, `rebake`,
   `WorldBundle` are C's; A's pending-edit rendering enters only through the
   frozen `pending_edits::apply_overlay(...)` call C publishes. Today's
   equivalent seam is the `BakeOverlay` argument of `layers::bake`
   (layers.rs:184) built inside `rebake` (app.rs:404–429) — freezing something
   shaped like "pending set → BakeOverlay-equivalent" matches the code as it
   stands.
9. Era picker is disabled while a job runs (879) — "timeline scrubbing stays
   live" is already limited by history being dropped during a run; Fix 1's
   Regenerate keeps this behavior (contract says existing progress + cancel).
