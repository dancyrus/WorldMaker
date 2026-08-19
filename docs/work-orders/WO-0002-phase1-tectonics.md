# WO-0002 — Phase 1: plate tectonics and the era picker

**Status: OPEN.**

Goal: continents that drift, collide, and raise mountains over hundreds of
millions of years; a live bottom timeline to scrub that history and pick the
moment that becomes "today"; the first painting tools (craton brush, hotspot
placement, plate drag if it goes smoothly). Placeholder noise elevation is
replaced by elevation derived from crust physics; the noise generator survives
only as a low-amplitude detail texture.

Out of scope: erosion, rivers, climate, regional features, L9 in the preset
dropdown.

## Acceptance checklist

### Housekeeping & setup
- [x] Delete merged feat/phase0-app-shell branch from GitHub (and locally)
- [x] This work order written
- [x] Feature branch feat/phase1-tectonics created

### Fields & data model
- [x] Integer-capable field storage decision made: FieldStore gained a u32
      integer-field store (decision log)
- [x] New per-cell fields: plate_id, crust_type (ocean/continent),
      crust_age_my, crust_thickness_km, orogeny_age_my, feature bitmask
      (ridge, trench, arc, hotspot, rift) — plus rift_age_my and
      hotspot_buildup_km needed for full-state keyframes

### Tectonics stage (worldmaker-sim, on the Stage/Pipeline contract)
- [x] All randomness via sub_rng keyed on the stage id (purposes embed the
      absolute step index for bit-exact resume)
- [x] Plate setup: N plates (default 12, range 8–24) by farthest-point
      sampling + great-circle Voronoi flood fill
- [x] Cratons: per-plate continental nuclei (thickness 35–45 km, age
      1,500–3,500 My), sized so final land fraction ≈ parameter (default 0.29)
- [x] Oceanic init: thickness 7 km, age from smooth deterministic ramp
- [x] Euler poles + angular speeds (mean ~0.5 deg/My, scaled by tectonic
      vigor); poles random-walk slowly
- [x] Time stepping dt = 2 My; span default 500 My, param range 200–2,000 My
- [x] Motion update: slab pull scales speed with subducting-boundary fraction,
      continental-collision fraction damps it (saturating — colliding plates
      stall); clamp 0.1–1.2 deg/My; constants logged
- [x] Advection: per-plate rigid rotation, semi-Lagrangian sampling via
      Grid::nearest_cell on the fixed grid (forward-scatter candidate claims
      + pending sub-cell rotations, decision log)
- [x] Boundary classification by relative normal velocity: divergent > +0.4
      cm/yr, convergent < −0.4, transform between
- [x] Divergent: gap cells → new oceanic crust (age 0, 7 km, ridge flag);
      continental divergence > 20 My thins ~0.2 km/My; < 25 km converts to
      ocean (rifting)
- [x] Convergent with ocean: subduction — overriding plate advances, consumed
      cells vanish; trench flag, deepen toward −8,500 m; volcanic arc 150–250
      km inboard (2–4 cells at L7); oceanic overriders build island arcs
      (young continental crust ~20–25 km)
- [x] Convergent continent-continent: both sides thicken with convergence
      speed, cap 70 km, orogeny_age resets; after 30 My of slow (<0.5 cm/yr)
      collision, plates suture (plate-count floor 6)
- [x] Supercontinent breakup: plate > 1/3 of sphere (or of the world's
      continental crust — logged deviation, keeps the cycle alive) with
      youngest suture > 100 My nucleates a deterministic rift path
- [x] Hotspots: 6 fixed mantle points (param 0–12); shield volcanoes over
      5–10 My residence; age-progressive chains (13 emergent island cells in
      the acceptance run)
- [x] Aging: ocean age += dt; orogeny_age += dt; inactive orogens relax
      toward 38 km with 200 My time constant (primordial cratons exempt,
      logged)
- [x] Elevation derived from state each step (never integrated): continents
      150 m per km of thickness above 35 km; ocean −(2600 + 365·√age) m
      flattening at −5,600 m; trench/arc/hotspot reliefs; low-amplitude noise
      detail texture
- [x] Sea-level offset solved by bisection so ocean fraction matches the
      parameter; the slider is an offset around that solution
- [x] Elevation goldens: noise-stage golden unchanged (stage still unit-
      tested); new tectonic elevation + plate_id goldens committed and logged

### Keyframes & era picker
- [x] Snapshot every 10 My (20 My at L8, logged): elevation, plate_id,
      crust_age, thickness, flags (+ rift age, buildup, per-plate state) in
      i16/u16 encoding, 16 B/cell; quantization round-trip makes resume from
      any keyframe bit-exact (tested)
- [x] Keyframe memory ≤ 1 GB at L7 for a 2 Gy run: measured 527 MB
      (tectonics-phase1 results JSON)
- [x] Bottom timeline: drag to scrub with snap-to-keyframe, epoch readout in
      My, play/pause at 100 My/s
- [x] "Set as present" pins the chosen keyframe (decoded into world fields);
      default present = final keyframe
- [x] Scrubbing is a rebake + one buffer write (~1 ms at L7)
- [x] Simulation on a worker thread; progress bar; working cancel button
      (pipeline cache stays clean on cancel — regression-tested); window
      responsive throughout

### Layers
- [x] Plates layer: 24 categorical colors (distinctness unit test); boundary
      cells styled by classified type (ridge red / trench navy / transform
      gold; one-cell bands, logged)
- [x] Crust age layer: viridis (perceptually uniform, verified anchors)
- [x] Elevation layer: hypsometric tint (Phase 0 ramp, now testable Rust)
- [x] Thickness layer (batlow, debug view)
- [x] Climate stays greyed out

### Painting (in this order)
- [x] 1. Craton brush: paint/erase continental nuclei on the initial state,
      adjustable radius (150–2,000 km), both canvases via nearest_cell;
      stroke end re-runs history from t=0 with the same seed (plate layout
      repeats — sim-tested)
- [x] 2. Hotspot placement: click to add/remove hotspots on a marker layer;
      re-run
- [x] 3. Plate drag: QUEUED as the first item of Phase 2 (decision log) — a
      correct version needs a hashed drag-overlay through the stage cache
      contract; the bit-exact ResumeFrom machinery it needs is built and
      tested

### Acceptance metrics (docs/results/tectonics-phase1-DESKTOP-VKD81C6.json)
- [x] Age-depth: max error 2.4% across the 0–80 My bins (gate ±10%)
- [x] Hypsometry: bimodal, ocean mode −5,628 m / land mode +174 m, Ashman's
      D = 8.52 (gate > 2)
- [x] Arcs on the overriding side: 100% of 2,878 arc cells within 400 km of
      a same-plate trench (gate ≥95%)
- [x] Determinism: double-run hashes identical; goldens committed
      (determinism_tests.rs)
- [x] Stability: 2 Gy at L6 — plate count 6–12, land fraction
      0.2901–0.2905 (parameter 0.29), 23 sutures, 17 breakups
- [x] Performance: 1 Gy L7 in 4.2 s (gate ≤60 s); 500 My L7 in 2.2 s;
      keyframes 265 MB (1 Gy) / 527 MB (2 Gy) at L7
- [ ] All Phase 0 tests still pass; CI green (ubuntu blocking, macOS
      informational)

### Wrap-up
- [ ] CI green on the branch; merged to main; branch deleted
- [ ] Tag v0.2.0
- [ ] Roadmap status updated
- [ ] Screenshots to docs/media/phase1/: plates layer mid-run,
      continent-continent mountain range, timeline mid-scrub
- [ ] Plain-English report to Dan: what exists, how to try the timeline and
      craton brush, headline numbers, open questions for Phase 2

## Notes
- Per CLAUDE.md rule 4: subagents/workflows for research and verification.
- If context runs short: finish the current commit, push, update these boxes,
  stop cleanly with a note to Dan.
