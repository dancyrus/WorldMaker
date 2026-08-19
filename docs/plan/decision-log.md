# Decision log

One line per decision: date, the decision, why/scope, and who made it
([Dan] or [default] = Claude's call under standing authority).

| Date | Decision | By |
|---|---|---|
| 2026-08-19 | Core loop is staged simulation with painting at any stage; downstream stages recompute around edits. | Dan |
| 2026-08-19 | Science approach: grounded approximations validated against Earth. | Dan |
| 2026-08-19 | Tech stack: Rust + wgpu + egui. | Dan |
| 2026-08-19 | Globe and flat canvases from day one, with selectable projections. | Dan |
| 2026-08-19 | Timeline at the bottom of the window as the era picker. | Dan |
| 2026-08-19 | Direct plus intent brushes; each stroke hard or soft. | Dan |
| 2026-08-19 | Regions in high detail are the real goal; regional refinement is Phase 5. | Dan |
| 2026-08-19 | Named branches with side-by-side compare. | Dan |
| 2026-08-19 | Three switchable map styles: atlas, satellite, parchment. | Dan |
| 2026-08-19 | Auto-generated editable names. | Dan |
| 2026-08-19 | Magic toolkit stays on the backlog. | Dan |
| 2026-08-19 | Performance benchmarked to the PC only (i7-12700KF, RTX 3080); M1 Air requirement excluded for now. | Dan |
| 2026-08-19 | Repo name WorldMaker, public, MIT license. | default |
| 2026-08-19 | Geodesic grid presets L6/L7/L8/L9. | default |
| 2026-08-19 | Earth-test regression gate from Phase 3. | default |
| 2026-08-19 | Dependency addition: flexi_logger — the approved list allows "log plus a file logger"; flexi_logger provides rotating file logs beside the executable. | default |
| 2026-08-19 | Dependency addition: pollster — one-function futures executor needed to call wgpu's async adapter probe synchronously at startup; no runtime, no transitive weight. | default |
| 2026-08-19 | Dependency addition: bytemuck — safe byte-casting for GPU vertex/uniform buffers; already in the dependency tree via egui/wgpu, standard for wgpu apps. | default |
| 2026-08-19 | Noise stage reseeded through sub_rng(seed, stage-id, purpose) instead of an ad-hoc XOR constant, honoring the Stage RNG contract before Phase 1 copies it; elevation golden hash regenerated (review finding). | default |
