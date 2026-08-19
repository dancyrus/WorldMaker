# WO-0001 — Phase 0: walking skeleton

**Status: OPEN — in progress (2026-08-19)**

Goal: repo, docs, CI, and a seeded planet visible in both a 3D globe and a flat
projected map. No Phase 1 features (no real tectonics, no era picker function, no
brushes).

## Acceptance checklist

### Repository & workspace
- [x] Git repo initialized at C:\Claude\WorldMaker, main branch
- [ ] GitHub repo dancyrus/WorldMaker created, public, MIT
- [ ] Cargo workspace: worldmaker-core / -sim / -io / -app with correct dep edges
- [ ] Cargo.lock committed
- [x] README.md, CLAUDE.md, START-HERE.md
- [x] docs/plan/science-outline.md (verbatim), roadmap.md, decision-log.md (seeded)
- [x] docs/results/README.md (schema)
- [x] This work order

### worldmaker-core
- [ ] Icosphere grid L6–L9; vertex count = 10·4^L + 2
- [ ] Goldberg dual: exactly 12 pentagons, rest hexagons
- [ ] Cell centers as unit vectors + lat/lon; CSR neighbor table
- [ ] SoA f32 field storage keyed by cell id
- [ ] Seeded RNG: u64 master seed, PCG sub-streams by (seed, stage, purpose)
- [ ] Fixed iteration order; no HashMap iteration in sim paths; no fast-math

### worldmaker-app
- [ ] Globe canvas: orthographic 3D, drag rotate, scroll zoom
- [ ] Flat canvas: equirectangular + Robinson dropdown, pan/zoom, optional graticule
- [ ] Globe / Flat / Split view switcher, canvases synced
- [ ] Elevation from seeded fractal noise via worldmaker-sim placeholder stage
- [ ] Cursor reports cell id + lat/lon in both canvases; same ground position →
      same cell in both views; screen↔cell mapping is a tested core capability
- [ ] Disabled timeline strip at bottom, labeled for Phase 1
- [ ] Seed text field (any text hashes to a seed, never crashes) + Generate
- [ ] Sea-level slider recolors live in both canvases
- [ ] Layer dropdown: Elevation active; Plates, Climate greyed out
- [ ] Preset dropdown: draft L6 / standard L7 / high L8
- [ ] FPS readout
- [ ] No GPU → plain-language dialog naming the machine and likely fix, clean exit
- [ ] Rotating file log beside the executable, not the console

### Performance (on Dan's PC: i7-12700KF, RTX 3080)
- [ ] No per-cell heap allocation inside loops; rayon where parallel
- [ ] L7 grid build under 2 s (measured, committed to docs/results/)
- [ ] 60 fps in both canvases at L7 (measured; note if render mesh was decimated)

### Tests & CI
- [ ] Vertex count per level; pentagon count = 12; neighbor symmetry
- [ ] Fixed seed reproduces fixed elevation-field hash
- [ ] Generate L6–L8 without panic
- [ ] Projection round-trip both projections; globe/flat agree on cell at lat/lon
- [ ] Perf harness writes docs/results/perf-phase0-{machine}.json
- [ ] Determinism results in docs/results/determinism-phase0-{machine}.json
- [ ] GitHub Actions: fmt + clippy(-D warnings) + tests on ubuntu-latest, blocking
- [ ] macos-14 build job, continue-on-error, informational only
- [ ] Green blocking CI before phase close

### Wrap-up
- [ ] WorldMaker.bat builds release and runs
- [ ] Tag v0.1.0
- [ ] Screenshots: globe, flat, split → docs/media/phase0/
- [ ] Plain-English report to Dan: what exists, how to open, grid-build ms, fps,
      tests passing, open questions for Phase 1
