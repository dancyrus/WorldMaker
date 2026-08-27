# WorldMaker — rules for Claude Code sessions

Fantasy world maker / map painter grounded in real geology and climatology.
New session? Read START-HERE.md, then take the next open order in docs/work-orders/.

## Who this is for

Dan does not code and does not use git. Report in plain language. Never ask him to
read code, run commands, or decide merges. Claude owns the repo end to end,
including GitHub (repo: dancyrus/WorldMaker).

## Ground rules

1. `main` must always build. Feature work on branches; merge only when CI is green.
   Conventional commits.
2. Keep this file under 150 lines. Route detail into docs/. Add lessons learned
   here as you find them.
3. Every test/benchmark number is committed as machine-labelled JSON under
   docs/results/ (schema in docs/results/README.md). Chat-only numbers don't count.
4. Use subagents for research and verification work that can run in parallel.
5. No network calls, no telemetry in the app. Cargo.lock is committed.
6. Approved deps: eframe/egui, wgpu, rayon, rand, rand_pcg, serde, serde_json,
   image, log + a file logger, anyhow, thiserror. Anything else needs one line of
   why in docs/plan/decision-log.md (current: flexi_logger, pollster, bytemuck).
7. Determinism: one u64 master seed; PCG sub-streams keyed by (seed, stage, purpose);
   fixed iteration order; no HashMap iteration in sim paths; no fast-math.
8. Performance budgets benchmark to Dan's PC only (i7-12700KF, RTX 3080).
   Measure, don't guess.

## Machine note — 2026-08-25

Dan's PC (i7-12700KF/RTX 3080) is packed for a move until ~mid-September 2026.
Until it returns, this MacBook Air (M1, 16 GB) is the primary dev machine:
- Wherever rule 8 or an open order says numbers are measured "on Dan's PC",
  record them on this Mac instead, machine-labelled (Daniels-MacBook-Air).
  They are records, not pass/fail gates; re-measure PC-gated numbers when the
  PC is back.
- WO-0003's preset decisions stand; its session records Air timings for them.
- The macOS CI job stays informational.
- Proven 2026-08-25: full test suite + all goldens pass on Apple Silicon
  bit-for-bit; sim wall times ~15% faster than the PC; rendering 7–25x slower
  in raw fps but ≥55 fps in every view. Dan opens the app by double-clicking
  WorldMaker.command.

## Layout

- crates/worldmaker-core — grid, RNG, fields, projections. No internal deps.
- crates/worldmaker-sim — Stage trait, cache/dirty skeleton, placeholder noise stage.
- crates/worldmaker-io — results-JSON writer, save/export stubs.
- crates/worldmaker-app — eframe/egui + wgpu shell (globe + flat canvases).
- docs/plan/ — science-outline.md (spec), roadmap.md (phases), decision-log.md.
- docs/work-orders/ — numbered work orders with acceptance checklists.
- docs/results/ — committed benchmark/test JSON.
- docs/media/ — screenshots per phase.

## Key technical facts

- Grid: subdivided icosahedron, dual Goldberg cells. Levels L6–L9;
  cells = 10·4^L + 2; exactly 12 pentagons. Neighbors in CSR, CCW-ordered.
- Cell ids are u32, stable for a given level. Fields are SoA `Vec<f32>` per cell.
- `Grid::nearest_cell(unit_vec, hint)` is the one true position→cell mapping;
  every canvas and every future brush must go through it.
- Projections implemented in core (pure, testable): equirectangular, Robinson
  (5°-table interpolation). Round-trip tests must pass for both.
- App renders the icosphere triangle mesh (globe) and a cell-id lookup texture
  (flat) — the flat texture depends only on grid level, so seed/sea-level changes
  are just buffer updates.
- Keyframes: 16 B/cell; cadence 10 My (L6/L7), 20 My (L8), 100 My (L9); a
  history stores its own interval. The 1 GB / 2 Gy budget is defined at L7.
- App builds: `cargo run --release -p worldmaker-app`. Perf harness:
  `cargo run --release -p worldmaker-app -- --perf-out <file>`. Screenshots:
  `-- --screenshots <dir>`.

## Lessons learned

- winget needs `--source winget` on this machine (msstore source is blocked).
- eframe must be used with `default-features = false` + the `wgpu` feature so the
  glow/OpenGL path never links; use `eframe::egui_wgpu::wgpu` re-export to keep
  wgpu versions matched.
- Ubuntu CI needs libxkbcommon/wayland dev packages before `cargo build` of the
  app crate; keep the apt-get step in ci.yml.
- Windows PowerShell 5.1: no `&&`; `Set-Content` defaults to ANSI — pass
  `-Encoding utf8`.
- egui 0.36 renamed things: `App::ui(&mut self, ui, frame)` replaces `update`;
  `egui::Panel::top/bottom(...).show(ui, ...)` replaces TopBottomPanel;
  `Button::selectable` replaces SelectableLabel;
  `Context::egui_wants_keyboard_input` replaces `wants_keyboard_input`.
  wgpu 30: bind-group layouts
  are `&[Option<&BindGroupLayout>]`, vertex buffers
  `&[Option<VertexBufferLayout>]`, pipeline layouts use `immediate_size`,
  `RenderPipelineDescriptor` uses `multiview_mask`.
- Changing anything in the noise/elevation path changes the committed golden
  hash — regenerate it deliberately and log the change; never "fix" the test.
- PowerShell 5.1 Get-Content|Set-Content pipelines mangle UTF-8 source files
  (em-dashes become mojibake) — use the Edit tool for file changes, always.
- The sim path must stay libm-free (use worldmaker-core::dmath); Rust f32::sin
  etc. differ between MSVC and glibc in the last ulp and break the goldens.
- rayon float reductions are order-nondeterministic: cross-cell aggregates in
  sim code must be integer counts or serial id-ordered loops.
- The workspace built and passed every test on Apple Silicon unmodified — the
  libm-free dmath + serial-reduction design holds: Windows-made goldens
  reproduce bit-for-bit on ARM.
- macOS sets neither COMPUTERNAME nor HOSTNAME for non-interactive processes;
  machine_name() falls back to `hostname -s`, else results files say
  "unknown-machine".
- macOS trackpad pinch reaches egui as zoom_delta (ctrl+scroll maps there
  too), never smooth_scroll_delta — canvases must fold both into zoom.
- A .command Finder launcher runs in a fresh Terminal that may lack cargo on
  PATH: export ~/.cargo/bin explicitly, and detach the app with `&` + disown
  so closing the Terminal window doesn't kill it.
- Scripted screenshot/perf runs hang at their first stage if the Mac's
  display is asleep (the window never gets redraw events): wrap them in
  `caffeinate -dimsu <command>`.
- zsh: `status` is a read-only special variable (`$?` alias) — assigning it
  aborts the script; use `rc=$?` in .command/script files.
- fps with vsync off is only real if something blocks on the GPU: an
  occluded macOS window recycles drawables instantly, so an unsynced frame
  counter measures CPU encode speed (6000+ "fps"). Perf loops must wait for
  submitted GPU work each frame (fps_gpu_synced in results JSON).
