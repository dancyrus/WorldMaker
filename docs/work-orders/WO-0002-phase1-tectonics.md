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
- [ ] Feature branch feat/phase1-tectonics created

### Fields & data model
- [ ] Integer-capable field storage decision made (integer variant vs exact
      small floats) and logged in docs/plan/decision-log.md
- [ ] New per-cell fields: plate_id, crust_type (ocean/continent),
      crust_age_my, crust_thickness_km, orogeny_age_my, feature bitmask
      (ridge, trench, arc, hotspot, rift)

### Tectonics stage (worldmaker-sim, on the Stage/Pipeline contract)
- [ ] All randomness via sub_rng keyed on the stage id
- [ ] Plate setup: N plates (default 12, range 8–24) by farthest-point
      sampling + great-circle Voronoi flood fill
- [ ] Cratons: per-plate continental nuclei (thickness 35–45 km, age
      1,500–3,500 My), sized so final land fraction ≈ parameter (default 0.29)
- [ ] Oceanic init: thickness 7 km, age from smooth deterministic ramp
- [ ] Euler poles + angular speeds (mean ~0.5 deg/My, scaled by tectonic
      vigor); poles random-walk slowly
- [ ] Time stepping dt = 2 My; span default 500 My, param range 200–2,000 My
- [ ] Motion update: slab pull scales speed with subducting-boundary fraction,
      continental-collision fraction damps it; clamp 0.1–1.2 deg/My;
      constants logged
- [ ] Advection: per-plate rigid rotation, semi-Lagrangian sampling via
      Grid::nearest_cell on the fixed grid
- [ ] Boundary classification by relative normal velocity: divergent > +0.4
      cm/yr, convergent < −0.4, transform between
- [ ] Divergent: gap cells → new oceanic crust (age 0, 7 km, ridge flag);
      continental divergence > 20 My thins ~0.2 km/My; < 25 km converts to
      ocean (rifting)
- [ ] Convergent with ocean: subduction — overriding plate advances, consumed
      cells vanish; trench flag, deepen toward −8,500 m; volcanic arc 150–250
      km inboard (2–4 cells at L7); oceanic overriders build island arcs
      (young continental crust ~20–25 km)
- [ ] Convergent continent-continent: both sides thicken with convergence
      speed, cap 70 km, orogeny_age resets; after 30 My of slow (<0.5 cm/yr)
      collision, plates suture (plate-count floor 6)
- [ ] Supercontinent breakup: plate > 1/3 of sphere with youngest suture >
      100 My nucleates a deterministic rift path through continental interior
- [ ] Hotspots: 6 fixed mantle points (param 0–12); shield volcanoes over
      5–10 My residence; age-progressive chains
- [ ] Aging: ocean age += dt; orogeny_age += dt; inactive orogens relax
      toward 38 km with 200 My time constant
- [ ] Elevation derived from state each step (never integrated): continents
      150 m per km of thickness above 35 km; ocean −(2600 + 365·√age) m
      flattening at −5,600 m; trench/arc/hotspot reliefs; low-amplitude noise
      detail texture
- [ ] Sea-level offset solved by bisection on the hypsometric curve so ocean
      fraction matches the parameter; existing slider becomes an offset
      around that solution
- [ ] Elevation goldens regenerated deliberately and logged (never bend the
      test)

### Keyframes & era picker
- [ ] Snapshot every 10 My: elevation, plate_id, crust_age, thickness, flags
      in compact encoding (i16/u16 per field)
- [ ] Keyframe memory ≤ 1 GB at L7 for a 2 Gy run; actual usage recorded in
      results JSON
- [ ] Bottom timeline: drag to scrub with snap-to-keyframe, epoch readout in
      My, play/pause at comfortable speed
- [ ] "Set as present" pins the chosen keyframe as world state for downstream
      stages and exports; default present = final keyframe
- [ ] Scrubbing feels instant (keyframe display is a buffer swap)
- [ ] Simulation runs off the UI thread; progress bar; working cancel button;
      window responsive throughout

### Layers
- [ ] Plates layer: categorical colors distinguishable at 24 plates; boundary
      lines styled by type (ridge, trench, transform)
- [ ] Crust age layer: perceptually uniform sequential colormap (no rainbow)
- [ ] Elevation layer: hypsometric tint (ocean blues below sea level, terrain
      ramp above)
- [ ] Thickness layer (debug view)
- [ ] Climate stays greyed out

### Painting (in this order)
- [ ] 1. Craton brush: paint/erase continental nuclei on the initial state,
      adjustable radius, both canvases via nearest_cell; applying re-runs
      history from t=0 with the same seed
- [ ] 2. Hotspot placement: click to add/remove hotspots on a marker layer;
      re-run
- [ ] 3. Plate drag (only if 1 and 2 are done and green): drag sets a
      surface-velocity target, recompute the plate's Euler pole, re-run
      forward from the currently viewed keyframe — OR queued as first item of
      Phase 2 with a note

### Acceptance metrics (committed to docs/results/ as machine-labelled JSON)
- [ ] Age-depth: mean ocean depth per 10 My age bin (0–80 My, excluding
      trench/arc/hotspot cells) within ±10% of the cooling curve
- [ ] Hypsometry: bimodal elevation distribution (one mode deep ocean, one
      near sea level) with a defensible bimodality metric, recorded
- [ ] Arcs on the overriding side: ≥95% of arc cells have nearest trench
      across the boundary within 400 km
- [ ] Determinism: same seed → identical final elevation and plate_id hashes;
      goldens committed
- [ ] Stability: 2 Gy at L6 keeps plate count in 6–24 and land fraction
      within ±5% of the parameter
- [ ] Performance on Dan's PC: 1 Gy at L7 ≤ 60 s wall clock; default 500 My
      time recorded too, plus keyframe memory
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
