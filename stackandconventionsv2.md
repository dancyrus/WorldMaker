# WorldMaker: stack and conventions v2

2026-08-19, revised to benchmark performance to the PC only (M1 Air requirement excluded for now). Supersedes v1. Audience: Claude Code and future planning sessions.

## Stack decision

Rust + wgpu + egui (eframe), the same stack as the CFD repo. One language end to end, and wgpu reaches Vulkan/D3D12 on the RTX 3080 when compute kernels move to the GPU. The Air requirement is excluded for now; the identical stack reaches Metal if it returns, which is why dropping the requirement costs nothing architecturally. The conventions below already work in dancyrus/CFD, so nothing has to be reinvented.

Alternatives considered. A TypeScript web app with WebGPU iterates fast and shares easily, but it adds a second toolchain next to the Rust projects and browser memory caps the ultra preset. Godot reaches a pretty globe quickly, but heavy custom simulation in engine scripting is slow and headless testing is clumsy. Both lose to consistency with the existing repos.

## Workspace layout

Four crates, same shape as the CFD project:

- worldmaker-core: grid (icosphere levels 6–9, dual cells, CSR neighbors), seeded RNG (PCG sub-streams), SoA field storage, math helpers. No dependencies on the other crates.
- worldmaker-sim: stages 0–4 as pure functions behind a common Stage trait, plus the cache/dirty-propagation and edit-overlay machinery. Depends only on core.
- worldmaker-io: save/load, exports (PNG, heightmap, GeoJSON, fact sheet), ETOPO ingest for the Earth test, results-JSON writer. 
- worldmaker-app: eframe/egui shell, wgpu globe and 2D renderers, brushes, timeline scrubber, presets.

Approved dependencies: eframe/egui, wgpu, rayon, rand + rand_pcg, serde + serde_json, image, log plus a file logger, anyhow/thiserror. Anything else gets a line in the decision log explaining why. For ETOPO ingest, evaluate the netcdf crate against NOAA's ERDDAP subsetting service (which can serve a pre-downsampled grid over HTTP once, checked into data/ as a small binary); pick one and record it.

## Determinism rules

Same seed and parameters must reproduce the same world.

- Single u64 master seed; PCG sub-streams keyed by (seed, stage, purpose).
- Fixed iteration order everywhere in sim code; no HashMap iteration in simulation paths.
- f32 fields with f64 accumulators for reductions (the CFD lesson); no fast-math flags.
- Bitwise-identical reruns required on the same binary. Cross-machine equivalence testing is deferred along with the Air requirement.
- Every export embeds seed, parameters, and app version.

## Performance strategy

CPU first with rayon until the science is right, GPU compute later, which is the pattern that worked for the CFD proof of concept. Field memory is small (a full stage set at ultra L9 is a few hundred MB), so compute, not memory, is the constraint. Budgets live in the roadmap per phase; measured numbers get committed, never guessed. Live editing runs the downstream pipeline at draft L6 while the full-resolution pass refines in the background; the draft/refine split is the interactivity contract.

All performance budgets benchmark to the PC (i7-12700KF, RTX 3080). Presets: draft for live edit re-runs, standard as the default generation preset, high and ultra available; raise the defaults when Phase 1 benchmarks show the headroom. The app still records which machine produced every results file.

## Repository conventions

Mirrors dancyrus/CFD:

- CLAUDE.md at root, under 150 lines, routing to docs/ rather than holding detail; lessons learned land there as they happen.
- START-HERE.md tells any new Claude Code session how to pick up: read CLAUDE.md, read the work-order queue, take the next open order.
- docs/plan/ holds the science outline, roadmap mirror, and decision log; docs/work-orders/ is the queue, one file per order with status; docs/results/ holds machine-labelled JSON for every test and benchmark; docs/media/ holds per-phase screenshots for Dan.
- .claude/settings.json grants the permissions the session needs so runs don't stall on prompts.

## Git and GitHub

Claude Code owns the repository completely; Dan never touches git. Repo dancyrus/WorldMaker, public, MIT (defaults, changeable in the decision log). main always builds. One feature branch per work order, merged when CI is green, conventional commits. CI on GitHub Actions: fmt check, clippy with warnings denied, tests on ubuntu-latest, blocking. A macos-14 build job runs continue-on-error, informational only: macOS is out of scope for now but stays one config line from returning. Tag the end of each phase. A launcher script (WorldMaker.bat) exists so Dan opens the app with a double-click.

## Session workflow

Build-and-run phases run in local Claude Code sessions on the PC, which has the GPU and the GitHub auth. Research and documentation work orders can run in cloud sessions. Single track through the end of Phase 2, because parallel sessions on a green-field codebase spend their time merging; from Phase 3 onward, split into a simulation track and a UI/painting track if pace demands it. Dan's involvement per phase: paste one kickoff prompt, look at the result, answer the open questions the report raises.
