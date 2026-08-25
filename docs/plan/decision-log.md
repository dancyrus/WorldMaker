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
| 2026-08-19 | FieldStore gains a u32 integer-field store (plate_id, crust_type, feature bitmask) alongside f32; exact bit ops beat float-encoded ids. | default |
| 2026-08-19 | Sim path bans libm transcendentals: worldmaker-core::dmath supplies fixed-order Taylor sin/cos (small angles), Irwin–Hall gaussians, cube-rejection unit vectors, raw-bit uniforms; exp(-dt/200) is a precomputed literal. Cross-platform golden hashes depend on this. | default |
| 2026-08-19 | Tectonics RNG purposes embed the absolute step index (and plate id), so a run resumed from a keyframe replays identical randomness. | default |
| 2026-08-19 | Keyframes are exact full state: the sim round-trips its own per-cell state through the u16/i16 keyframe quantization at every keyframe (round-then-clamp, idempotent), and per-plate state carries pending sub-cell rotation plus boundary stats — resume-from-keyframe is bit-exact (tested). | default |
| 2026-08-19 | Advection ownership by forward-scatter claims (atomic OR bitmask) + per-cell coverage gather; handles multi-cell sweeps per step. Sub-cell motion banks into a per-plate pending rotation committed at ~0.75 cell so slow plates never freeze to the grid (design-review findings). | default |
| 2026-08-19 | Phase 1 tectonic constants (design review + tuning): slab-pull gain 1.0; collision damping 1.0 saturating at max(5% boundary, 4 cells) with speed floor collapsing to 0 — colliding plates stall (continental-area conservation); speed relax 0.15 up / 0.5 down; pole walk sigma 0.6 deg/step; arc growth 0.6 (ocean) / 0.15 (cont) km/My, cap 70 km, island-arc conversion at 20 km; collision thickening 0.12 km/My per cm/yr; hotspot buildup 0.8/0.4 km/My, cap 8 km, decays with the 200 My constant; trench blend 75% toward -8500 m; arc relief +2000 m; detail noise ±300 m; continental crust fraction = land fraction × 1.35. | default |
| 2026-08-19 | Continental crust thinner than 30 km is subductible (arc terranes recycle); continent ≥ 30 km never consumed — continent-continent overlaps jam in place instead. | default |
| 2026-08-19 | Rift timer: +dt on continent-continent divergence, decays at 2× otherwise (hysteresis vs classification noise); suture timer accrues on any slow (<0.5 cm/yr) continent contact and resets on fast convergence. | default |
| 2026-08-19 | Advection events gated by previous-step boundary class: transform-only cells make no ridge crust on gaps and no trench/subduction on overlaps (hex-zigzag artifact suppression). | default |
| 2026-08-19 | Breakup also triggers when a plate holds > 1/3 of the world's continental crust (not only 1/3 of the sphere, which a floor-6 stalled world never reaches); without it the Wilson cycle gridlocks. Deviation from WO wording, same intent. | default |
| 2026-08-19 | Orogenic relaxation exempts primordial crust (orogeny_age > 1200 My): cratons and painted continents keep their profiles; collision resets orogeny_age so real orogens still decay. | default |
| 2026-08-19 | Keyframe cadence 10 My (spec) at L6/L7, 20 My at L8 to keep a 2 Gy history inside the 1 GB budget. | default |
| 2026-08-19 | Pipeline marks a stage dirty before running it, so a cancelled/failed run can never serve a stale cache entry (design-review finding, regression-tested). | default |
| 2026-08-19 | "Set as present" is applied by the app decoding the chosen keyframe into the world fields — never a re-simulation; the stage itself always publishes the final keyframe. | default |
| 2026-08-19 | Rendering: per-cell layer colors baked on the CPU (layers.rs) into one RGBA8 storage buffer; shader palettes removed. A scrub/layer/sea-level change is a rebake (~1 ms at L7) plus one buffer write; palettes are testable Rust. | default |
| 2026-08-19 | Colormaps: crust age = viridis, thickness = batlow (verified perceptually-uniform anchor tables); elevation keeps the Phase 0 hypsometric ramp; plates use a 24-color categorical palette with a distinctness unit test. | default |
| 2026-08-19 | Plate-boundary "lines" render as one-cell bands colored by classified type (ridge red, trench navy, transform gold) — 56 km wide at L7; true polylines deferred until a phase needs them. | default |
| 2026-08-19 | Plate drag queued as the first item of Phase 2 (WO-0002 allows it): a correct version needs a hashed drag-overlay replayed through the stage cache contract, per the design review. The bit-exact resume machinery it needs (ResumeFrom) is built and tested. | default |
| 2026-08-19 | Arc acceptance metric reads as "a trench on the arc's own (overriding) plate within 400 km" via local search — the globally nearest trench can belong to a neighboring subduction zone without invalidating the arc. | default |
| 2026-08-19 | Craton paint is stored per grid level (cell ids are level-specific): preset changes clear it; seed changes keep it, so painted continents ride along on a reroll. Hotspot placements are unit vectors and survive preset changes (code-review finding). | default |
| 2026-08-19 | Code-review pass (6 finder + 9 verifier agents; 7 confirmed findings, all fixed): dead hover readout, hotspot click during a run could wipe the generated set, craton stroke released off-map skipped its re-run, stability gate was tautological, resume/cancel tests under-asserted. | default |
| 2026-08-19 | Stability acceptance additionally gates continental-crust inventory drift over the run (≤5% relative) — the land-fraction gate alone is pinned by the sea-level solve and proves nothing about the sim. | default |
| 2026-08-19 | Elevation is derived at keyframe steps only: it is a pure function of crust state and nothing reads it between keyframes ("each step" in the WO wording is honored in effect, cheaper in practice). | default |
| 2026-08-19 | Age-depth metric also excludes cells with >0.1 km hotspot buildup (the sub-flag skirt of a shield) — "hotspot cells" by relief, not only by flag. | default |
| 2026-08-19 | The simulated span rounds to whole keyframes, so no steps are simulated past the last snapshot. | default |
| 2026-08-19 | Plates layer colors index by the rank of alive plate ids in the keyframe (≤24 alive → never a collision), not raw id mod 24 — breakup-minted ids no longer alias colors. | default |
| 2026-08-19 | Sea level: solved once at the t=0 anchor (land fraction exact there), then the datum stays fixed so sea level drifts with the hypsometry — coastlines flood and drain through history. Measured: land 0.290→0.341 over 2 Gy at L6, stabilizing; dynamics bit-identical (plate golden unchanged); elevation golden regenerated. Stability gate now checks the anchor land fraction + continental-inventory drift; the over-run land range is recorded as data. | Dan |
| 2026-08-25 | Mac interim: Dan's PC is packed for a move until ~mid-September 2026; the M1 MacBook Air is the primary dev machine. Numbers rule 8 or open orders benchmark "on Dan's PC" are recorded on the Air instead, machine-labelled (Daniels-MacBook-Air), as records not pass/fail gates; PC-gated numbers are re-measured when the PC returns. WO-0003 preset decisions stand; its session records Air timings. macOS CI stays informational. Verified: full suite + all goldens pass bit-for-bit on Apple Silicon; sim wall times ~15% faster than the PC, rendering 7–25x slower in fps but ≥55 fps everywhere. | Dan |
