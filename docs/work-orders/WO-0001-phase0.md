# WO-0001 — Phase 0: walking skeleton

**Status: OPEN — in progress (2026-08-19). All local work done and verified;
remaining items blocked on GitHub sign-in (Dan's browser approval).**

Goal: repo, docs, CI, and a seeded planet visible in both a 3D globe and a flat
projected map. No Phase 1 features (no real tectonics, no era picker function, no
brushes).

## Acceptance checklist

### Repository & workspace
- [x] Git repo initialized at C:\Claude\WorldMaker, main branch
- [ ] GitHub repo dancyrus/WorldMaker created, public, MIT
- [x] Cargo workspace: worldmaker-core / -sim / -io / -app with correct dep edges
- [x] Cargo.lock committed
- [x] README.md, CLAUDE.md, START-HERE.md
- [x] docs/plan/science-outline.md (verbatim), roadmap.md, decision-log.md (seeded)
- [x] docs/results/README.md (schema)
- [x] This work order

### worldmaker-core
- [x] Icosphere grid L6–L9; vertex count = 10·4^L + 2 (tested per level)
- [x] Goldberg dual: exactly 12 pentagons, rest hexagons (tested)
- [x] Cell centers as unit vectors + lat/lon; CSR neighbor table (CCW, symmetric)
- [x] SoA f32 field storage keyed by cell id
- [x] Seeded RNG: u64 master seed, PCG sub-streams by (seed, stage, purpose)
- [x] Fixed iteration order; no HashMap iteration in sim paths; no fast-math

### worldmaker-app
- [x] Globe canvas: orthographic 3D, drag rotate, scroll zoom
- [x] Flat canvas: equirectangular + Robinson dropdown, pan/zoom, optional graticule
- [x] Globe / Flat / Split view switcher, canvases synced
- [x] Elevation from seeded fractal noise via worldmaker-sim placeholder stage
- [x] Cursor reports cell id + lat/lon in both canvases; same ground position →
      same cell in both views (mapping_tests.rs); nearest_cell is the single
      tested core mapping both canvases use
- [x] Disabled timeline strip at bottom, labeled for Phase 1
- [x] Seed text field (any text hashes to a seed, never crashes — fuzz-ish test) + Generate
- [x] Sea-level slider recolors live in both canvases (uniform-only update)
- [x] Layer dropdown: Elevation active; Plates, Climate greyed out
- [x] Preset dropdown: draft L6 / standard L7 / high L8
- [x] FPS readout
- [x] No GPU → plain-language dialog naming the machine and likely fix, clean exit
- [x] Rotating file log beside the executable, not the console

### Performance (on Dan's PC: DESKTOP-VKD81C6, i7-12700KF, RTX 3080)
- [x] No per-cell heap allocation inside loops; rayon on noise, raster, lat/lon
- [x] L7 grid build under 2 s — measured 19.6 ms (docs/results/perf-phase0-DESKTOP-VKD81C6.json)
- [x] 60 fps in both canvases at L7 — measured 1,884 (globe) / 2,040 (flat) /
      1,994 (split) fps with vsync off; full-resolution mesh, no decimation

### Tests & CI
- [x] Vertex count per level; pentagon count = 12; neighbor symmetry
- [x] Fixed seed reproduces fixed elevation-field hash (golden committed)
- [x] Generate L6–L8 without panic
- [x] Projection round-trip both projections; globe/flat agree on cell at lat/lon
- [x] Perf harness writes docs/results/perf-phase0-{machine}.json
- [x] Determinism results in docs/results/determinism-phase0-{machine}.json
- [x] GitHub Actions workflow: fmt + clippy(-D warnings) + tests on ubuntu-latest,
      blocking; all pass locally (31 tests)
- [x] macos-14 build job, continue-on-error, informational only (in ci.yml)
- [ ] Green blocking CI on GitHub before phase close

### Wrap-up
- [x] WorldMaker.bat builds release and runs
- [ ] Tag v0.1.0 (after merge to main)
- [x] Screenshots: globe, flat, split → docs/media/phase0/
- [ ] Plain-English report to Dan: what exists, how to open, grid-build ms, fps,
      tests passing, open questions for Phase 1

## Open questions queued for Phase 1
- Timeline scrubbing granularity: keyframes every 10 My are the plan; decide how
  draft-quality interpolation between keyframes should look in the UI.
- Craton painting: which brush ships first — craton seed placement or plate-drag?
- L9 stays out of the preset dropdown until tectonics needs it (build measured
  457.8 ms, fine; the cost is elsewhere: memory and per-frame updates).
