# WO-0003 — Feel pass (v0.2.1)

**Status: OPEN** — started 2026-08-25 on Daniels-MacBook-Air (M1).
Baseline: main @ 9d5d272 (4 commits past tag v0.2.0 — sea-level drift + Mac
baseline; this is what Dan plays with, so it is the "before" state).

Dan's three notes, verbatim:

1. "Whenever I draw something, it starts to immediately regenerate. It should
   let me draw, then I choose when to regenerate."
2. "The default tectonic plate shapes make it look like a soccer ball, which
   just carries over to the final part."
3. "The cell size looks too coarse. It feels too arcady."

Fixes: **Fix 1** = pending-edits / explicit Regenerate (Track A).
**Fix 2** = new plate generator with measured gates (Track B).
**Fix 3** = smooth per-fragment rendering + render detail + Eckert IV +
L8 default / L9 enabled (Track C). Merge order **B → C → A**, each rebasing
on the last. Two invariants outrank everything: main stays green; same seed
still means same world.

Full specification (contracts, metrics, gates, track file partition) lives in
the order text, mirrored into docs/plan/feel-pass-design.md during Stage U/D.
This file is the resume point: a new session continues from the first
unchecked box.

## Stage U — understand (no code changes)

- [ ] Reader (a): app.rs input handling + UI state mapped
- [ ] Reader (b): render.rs / shaders.wgsl / layers.rs, incl. CPU palette bake
- [ ] Reader (c): tectonics/ incl. setup.rs seeding + ±300 m fBm in elevation.rs
- [ ] Reader (d): goldens, determinism tests, harness.rs, results-JSON schema
- [ ] docs/plan/feel-pass-design.md assembled from reader findings
- [ ] BEFORE screenshots from main @ 9d5d272 → docs/media/feel-pass/before/
      (Standard L7, app default seed, final era)

## Stage D — design

- [ ] Fix 1 state-machine design + adversarial design review
- [ ] Frozen overlay interface published in feel-pass-design.md
      (exact signature; A codes against it, C implements; frozen until A's rebase)
- [ ] Fix 2: competing generator designs scoped (3 candidates behind one trait)
- [ ] Fix 3: flat-canvas smoothing mechanism chosen + logged; render-detail
      sweep plan set

## Stage I — implement (isolated worktrees)

### Track B — plates + harness (merges first)
- [ ] Incumbent metrics (area CV, sinuosity) measured on 5 fixed seeds, committed
      to docs/results/tectonics-feelpass-Daniels-MacBook-Air.json BEFORE replacement
- [ ] Three candidates implemented behind common trait; plate-map PNGs via
      dev-only #[ignore] test; judge panel scores metrics + PNGs; decision logged;
      losers deleted
- [ ] Final gates strictly exclude incumbent scores (provisional CV ≥ 0.5,
      sinuosity ≥ 1.15; adjustments logged); fast CI gate test in worldmaker-sim
      (L7 seed 42 + two L6 seeds)
- [ ] Determinism: dmath only, id-ordered PQ w/ deterministic tie-breaks, serial
      id-ordered metric aggregation, stage sub-stream randomness
- [ ] Phase 1 acceptance re-passes end to end (age-depth, hypsometry, arcs,
      2 Gy stability: plates 6–24, land ±5%) → new results file
- [ ] L9 keyframe-cadence decision implemented + logged; keyframe_interval_my
      doc comment updated; CLAUDE.md key facts updated
- [ ] harness.rs: new metrics + optional L8/L9 rows wired
- [ ] Goldens (GOLDEN_TECTONIC_ELEVATION, GOLDEN_TECTONIC_PLATES) regenerated
      exactly once, final commit on B's branch, decision-log entry
- [ ] Track B merged green

### Track C — rendering + resolution (merges second, rebases on B)
- [ ] Globe: per-cell scalar upload; palette + sea-level threshold + render
      detail per fragment; Plates layer crisp per-cell; CPU-bake decision re-logged
- [ ] Flat: chosen smoothing mechanism; same per-fragment palette/threshold/
      detail; only noise + palette fns shared verbatim between canvases
- [ ] Render detail: deterministic 3D sphere noise from master seed, slope +
      land/ocean conditioned amplitude; octave/amplitude sweep on 2 seeds;
      screenshot panel picks default; Detail slider off→full; params logged
- [ ] Plate boundaries: smoothed type-colored polylines both canvases; old bands
      under debug toggle only
- [ ] Debug toggle showing true cell boundaries
- [ ] Eckert IV in core projections (Newton fwd, closed-form inverse, fixed
      iteration cap); round-trip tests; dropdown; brushes + cursor readout +
      graticule + smooth rendering work; same ground position → same cell
- [ ] L8 default preset (~28 km); Draft L6 stays; Ultra L9 enabled
- [ ] main.rs/scripts: --seed, --preset, --detail flags; perf script records fps
      at L7/L8/L9 smooth+detail → docs/results/perf-feelpass-Daniels-MacBook-Air.json
- [ ] Render-only guard test: Detail 0 vs max ⇒ identical params_hash + committed
      field hashes; check sim exposes no render-detail param
- [ ] Sim tests re-run green after C merges (golden hashes unmoved)
- [ ] Track C merged green

### Track A — pending edits (merges last, rebases on C)
- [ ] pending-edits module: ordered stroke list (CratonPaint cells ±1 /
      HotspotAdd / HotspotRemove unit vectors)
- [ ] Structural guard: stroke path has no route to Pipeline::run, enforced by test
- [ ] Badge counts strokes; Ctrl+Z pops newest; Discard clears; Regenerate folds
      into TectonicsParams overlays, clears, runs history off-thread w/ progress+cancel
- [ ] Preset switch discards pending craton strokes, keeps hotspot strokes; seed
      change keeps everything; logged in decision-log contract
- [ ] Pending overlay (tint+outline) rendered via frozen interface, world behind
- [ ] Stroke type serde Serialize/Deserialize in worldmaker-io beside save stubs
      (stubs stay stubs)
- [ ] View controls stay live (sea level, layers, projections, timeline, Detail)
- [ ] Sim tests re-run green after A merges (golden hashes unmoved)
- [ ] Track A merged green

## Stage V — verify + close

- [ ] One adversarial agent per acceptance line vs the real build
- [ ] Multi-agent review of merged diff: correctness, determinism, performance,
      UX-contract, repo-hygiene; confirmed findings fixed
- [ ] AFTER screenshots: Standard L7 same seed/era; + coastline close-up at L8
      default with render detail on → docs/media/feel-pass/
- [ ] Interaction contract + all decisions in docs/plan/decision-log.md
- [ ] CI green; branches merged and deleted; tag v0.2.1
- [ ] Plain-English report to Dan (what changed, how to compare at home,
      headline numbers in plain terms, open questions for Phase 2)

Scope guard: no erosion, rivers, plate drag, climate — Phase 2.
If context runs short: commit, push, update the boxes, stop cleanly with a
note to Dan.
