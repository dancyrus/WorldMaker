# WO-0003 — Feel pass (v0.2.1)

**Status: CLOSED — v0.2.1 (2026-08-27, Daniels-MacBook-Air).** Started
2026-08-25 on Daniels-MacBook-Air (M1).
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
L8 default / L9 enabled (Track C). **Fix 4** = plates must never freeze
permanently — braking cap, drag floor, suture-gap close, liveliness gates
(sim; added 2026-08-25 from a v0.2.0 measurement, section below).
Merge order **B → C → Fix 4 → A**, each rebasing on the last. Two invariants
outrank everything: main stays green; same seed still means same world.

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
      (S4 note: Fix 4 later moved age-depth to 3.7% and Ashman to 9.17 —
      still passing; current-main numbers are in tectonics-fix4-*.json)
- [x] L9 cadence = 100 My implemented + logged (2 Gy ≈ 0.88 GB — a sizing
      projection from the measured 1 Gy row, not itself a results-JSON
      measurement); doc comment + CLAUDE.md updated; XL rows (measured):
      1 Gy L8 17.6 s / 535 MB, L9 109 s / 461 MB
- [x] harness.rs: plategen metric rows + WM_HARNESS_XL L8/L9 rows
- [x] Goldens regenerated exactly once (M3 f28e896): elevation
      0xf751…5b62 → 0x7b43_ec03_a6ef_ca2a, plates 0x70df…653d →
      0x1690_72d7_7080_3f71; phase-0 golden unmoved; decision-log rows
- [x] Track B merged green (PR #6, all CI checks)

### Track C — rendering + resolution (merges second, rebases on B) — DONE, merged as PR #10 (d432b51)
- [x] Globe: per-cell scalar upload; palette + sea-level threshold + render
      detail per fragment; Plates layer crisp per-cell; CPU-bake decision re-logged
- [x] Flat: chosen smoothing mechanism; same per-fragment palette/threshold/
      detail; only noise + palette fns shared verbatim between canvases
- [x] Render detail: deterministic 3D sphere noise from master seed, slope +
      land/ocean conditioned amplitude; octave/amplitude sweep on 2 seeds;
      screenshot panel picks default; Detail slider off→full; params logged
      (default octaves 5 / A0 350 m; decision-log 2026-08-27, incl. the
      slope-floor visibility caveat and the 1-in-24 capture-pan flake)
- [x] Plate boundaries: smoothed type-colored polylines both canvases; old bands
      under debug toggle only
- [x] Debug toggle showing true cell boundaries
- [x] Eckert IV in core projections (Newton fwd, closed-form inverse, fixed
      iteration cap); round-trip tests; dropdown; brushes + cursor readout +
      graticule + smooth rendering work; same ground position → same cell
- [x] L8 default preset (~28 km); Draft L6 stays; Ultra L9 enabled
- [x] main.rs/scripts: --seed, --preset, --detail flags; perf script records fps
      at L7/L8/L9 smooth+detail → docs/results/perf-feelpass-Daniels-MacBook-Air.json
- [x] Render-only guard test: Detail 0 vs max ⇒ identical params_hash + committed
      field hashes; check sim exposes no render-detail param
- [x] Sim tests re-run green after C merges (golden hashes unmoved; verified on
      main @ d432b51: tectonic goldens + phase-0 noise golden reproduce)
- [x] Track C merged green (PR #10, all CI checks)

### Fix 4 — plates must never freeze for good (sim; after C, before A)

Session 2 owns the instructions, the measurement record, and the diagnosis:
[WO-0003-S2.md](WO-0003-S2.md). All changes in worldmaker-sim.

- [x] Cap collision braking below a full stop; a jammed plate keeps a small
      residual convergence (SPEED_FLOOR_JAMMED 0.05 deg/My at f_coll = 1)
- [x] Mantle-drag floor: an alive plate's speed relaxes toward a small
      nonzero base when boundary forces vanish, so a plate whose partner
      rifts away wakes up (verified by trace: recovery well within 100 My)
- [x] Close the classification gap: a continent-continent boundary below the
      classify threshold for longer than the suture timer sutures anyway;
      motionless welded continents are one plate (SUTURE_SLOW_CMYR 1.2 above
      the jam-creep ceiling + 2× timer decay instead of hard reset + floor
      gridlock breaker in maybe_breakup; S2 record §5)
- [x] Soften or widen the speed clamp so vigor spreads speeds instead of
      pegging them (SPEED_MAX 2.0, shared with setup's base-speed draw)
- [x] Liveliness gates in harness JSON + fast CI test (the gate class Phase 1
      missed): no alive plate of ≥50 cells keeps ownership overlap ≥ 0.985
      across a 300 My window unless inside an active collision younger than
      the suture timer; no alive plate holds speed < 0.05 deg/My for more
      than 200 My (tectonics::metrics::liveliness; exemptions logged in S2)
- [x] Echo run at Dan's exact recorded settings (seed "dan", 8 plates, 0.40,
      1.73, 2 Gy) passes both liveliness gates (CI test + harness rows)
- [x] Full Phase 1 acceptance + plate-generator gates + seed-42 2 Gy
      stability gate re-run green; if stability breaks, retune inside the
      pinned area bands using the documented breakup↔suture diagnosis and
      log it — do not weaken the gate (no retune needed: drift 2.1%, alive
      6–12; tectonics-fix4-Daniels-MacBook-Air.json)
- [x] SECOND sanctioned golden regeneration (Fix 2's "exactly once"
      bookkeeping closed at M3): tectonic goldens move once more, as the
      final commit of the Fix 4 session, decision-log entry in the same
      style
- [x] Fix 4 merged green (PR #15, all CI checks; branch deleted; merge
      commit 0650dee. Note: Track A merged before Fix 4 in practice —
      PR #13/#14 landed first and Fix 4 rebased onto them; the sim goldens
      were unaffected by A, and the Fix 4 golden move is the branch's final
      substantive commit as ordered)

### Track A — pending edits (merges last, rebases on C)

Session 3 owns the instructions: [WO-0003-S3.md](WO-0003-S3.md).
- [x] pending-edits module: ordered stroke list (CratonPaint cells ±1 /
      HotspotAdd / HotspotRemove unit vectors) — worldmaker-io/src/pending.rs
- [x] Structural guard: stroke path has no route to Pipeline::run, enforced by test
      (crate wall + io-manifest test + lexical scans + start_job count == 3 tripwire)
- [x] Badge counts strokes; Cmd/Ctrl+Z pops newest (live first, Undo button
      mirrors it); Discard clears; Regenerate folds into TectonicsParams
      overlays, clears, runs history off-thread w/ existing progress+cancel
- [x] Preset switch discards pending craton strokes, keeps hotspot strokes; seed
      change keeps everything; logged in decision-log contract (2026-08-25 rows)
- [x] Pending overlay (tint+outline) rendered via frozen interface, world behind
      (live stroke included; works mid-run off the hotspot fold base)
- [x] Stroke type serde Serialize/Deserialize in worldmaker-io beside save stubs
      (stubs stay stubs; JSON shape pinned by test)
- [x] View controls stay live (sea level, layers, projections, timeline, Detail)
- [x] Sim tests re-run green after A merges (golden hashes unmoved; verified on
      main @ PR #13 merge, 2026-08-27, tectonics_reproduces_committed_goldens ok)
- [x] Track A merged green (PR #13, all CI checks)

## Stage V — verify + close

Session 4 owns the instructions: [WO-0003-S4.md](WO-0003-S4.md).

- [x] Bounded verification pass: exactly ten agents (2026-08-27), one per
      area — pending-edit guard, plate-generator gates, liveliness gates,
      render-only guard, Eckert IV, presets/CLI, golden integrity, CI config,
      checklist truth, report accuracy. Every area verified; confirmed
      findings fixed in the close commit (see decision-log 2026-08-27 S4
      row: perf re-measured at A0 350 m, crust-type golden pinned, Eckert
      polar band tested, guard needles + CLI-value warnings added, stale
      comment fixed). Accepted residuals, deliberately not "fixed": the
      render-only guard is structural (build_world takes no render argument
      — the equality assertions prove determinism, the signature proves
      independence); Eckert IV is std-float view math, allowed because no
      projection value enters any golden-hashed path; the re-judging panel's
      retuned PNGs live in the judge record + JSON rows, not committed
      images; main has no GitHub branch protection (blocked by session
      permissions; see decision-log note).
- [x] AFTER screenshots via Track C's parity machinery (parity forced seed
      cyrus, Standard7, Detail 1.0; framing sanity-checked against BEFORE —
      plates pair matches at t = 250 My) plus coast-high8.png (High8
      defaults, octaves 5 / A0 350 m) → docs/media/feel-pass/after/
- [x] Sim suite re-run at close (2026-08-27, 15/15 binaries green): Tracks C
      and A (Sessions 1 and 3) moved NO golden hash after Fix 4's sanctioned
      move — elevation 0x857b_8233_0e24_2c03, plates 0xa8c4_9d9b_f779_59e8,
      phase-0 noise 0xa86a_7471_79a3_5a46 all byte-identical, verified both
      by the suite and by a commit-by-commit git audit of the golden file
- [x] Interaction contract (2026-08-25 standing row) + all decisions in
      docs/plan/decision-log.md
- [x] CI green on every WO-0003 PR (#6, #10, #13, #15 all 4/4 checks);
      branches merged and deleted; tag v0.2.1; roadmap row 1½ added
- [x] Plain-English report to Dan delivered in the S4 session (fixes, how to
      see each at home, headline numbers, Phase 2 open questions)

Scope guard: no erosion, rivers, plate drag, climate — Phase 2.
If context runs short: commit, push, update the boxes, stop cleanly with a
note to Dan.
