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

- [x] Reader (a): app.rs input handling + UI state mapped
- [x] Reader (b): render.rs / shaders.wgsl / layers.rs, incl. CPU palette bake
- [x] Reader (c): tectonics/ incl. setup.rs seeding + ±300 m fBm in elevation.rs
- [x] Reader (d): goldens, determinism tests, harness.rs, results-JSON schema
- [x] docs/plan/feel-pass-design.md assembled from reader findings
- [x] BEFORE screenshots from main @ 9d5d272 → docs/media/feel-pass/before/
      (Standard L7, seed "cyrus", final era 500 My; 6 views committed)

## Stage D — design

- [x] Fix 1 state-machine design + adversarial design review
- [x] Frozen overlay interface published in feel-pass-design.md § D1
      (exact signature; A codes against it, C implements; frozen until A's rebase)
- [x] Fix 2: competing generator designs scoped (3 candidates behind one trait;
      d2-fix2-design.md + adversarial amendments; commit protocol M1→M3)
- [x] Fix 3: flat-canvas mechanism chosen — d3a lookup extension (judged 56 v 53,
      9 grafts from d3b binding); render-detail sweep plan set (D4)
      Headroom note (graft 9): indexed scalar-layer globe fast path is the
      documented fallback if Ultra9 globe fps disappoints on the PC — designed
      addable without buffer changes; not built in this order.

## Stage I — implement (isolated worktrees)

### Track B — plates + harness (merges first) — DONE, merged as PR #6 (b0ec0ff)
- [x] Incumbent metrics (area CV ~0.08, sinuosity ~1.11) measured on 5 fixed
      seeds × L6/L7, committed BEFORE replacement (M1, plategen-feelpass JSON)
- [x] Three candidates behind PlateGenerator trait; 20-PNG panel; 3-judge panel
      → hybrid won 3–0; stability failure at P2 diagnosed (suture-driven engine
      seizure, not R1 consumption) → C9 retune (giant 22–25%, smallest 3%)
      → re-judging panel confirmed 3–0; losers deleted at M3
- [x] Final gates CV ≥ 0.50 / sinuosity ≥ 1.18 strictly exclude every incumbent
      score on all 10 pairs; always-on gate test 0.29 s (triple L7:42 + L6:7 +
      L6:cyrus, connectivity + enclave checks)
- [x] Determinism: dmath arc_len3, id-ordered PQ, serial metric aggregation,
      stage sub-stream; goldens reproduce ARM↔x86 bit-for-bit in CI
- [x] Phase 1 acceptance re-passes end to end (all_acceptance_pass = true:
      drift 4.1%, alive 6–12, age-depth 2.77%, Ashman 6.53, arcs 100%)
      → tectonics-feelpass-Daniels-MacBook-Air.json
- [x] L9 cadence = 100 My implemented + logged (2 Gy ≈ 0.88 GB); doc comment +
      CLAUDE.md updated; XL rows: 1 Gy L8 17.6 s / 535 MB, L9 109 s / 461 MB
- [x] harness.rs: plategen metric rows + WM_HARNESS_XL L8/L9 rows
- [x] Goldens regenerated exactly once (M3 f28e896): elevation
      0xf751…5b62 → 0x7b43_ec03_a6ef_ca2a, plates 0x70df…653d →
      0x1690_72d7_7080_3f71; phase-0 golden unmoved; decision-log rows
- [x] Track B merged green (PR #6, all CI checks)

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
