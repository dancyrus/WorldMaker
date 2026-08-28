# WO-0009-S7: calibrate, verify, close Phase 2

CONTEXT. Phase 2 session 7, the last. S6 merged. Benchmarks doc: `docs/plan/earth-benchmarks-v1.md` — the calibration quick sheet is the reference; respect its confidence notes (contested rows gate as ranges, never points).

RULES. Single-track. Branch `feat/phase2-close` from `main`. Calibration at L6. Verification fan-out at the end only: at most ten short subagents, one per acceptance area.

STEPS.

1. `git pull --ff-only origin main`. Create the branch.
2. Calibrate `K_LITH` inside the Stock & Montgomery span only: target a Himalaya-scale test orogen equilibrating at 6–9 km peaks under 30 My morphologic time; record every trial in `docs/results/terrain-calibration.json`.
3. Wire benchmark gates (quick-sheet rows, at default params, seeds 42 and cyrus): mean plate speed within 35–50 mm/yr equivalent (row 9); hypsometry anchors — land fraction 28–30% at start, mean land elevation within ±250 m of 840 m post-erosion (row 17, widened for grid resolution); erosion-rate span ≥ 3 orders of magnitude between orogen and craton cells (row 19); age-depth constants already gated (rows 12–13; assert unchanged). Record Wilson-cycle statistics (welds and openings per Gy vs rows 2–3) in results — gate stays the WO-0008 2–6/Gy band.
4. Re-run every existing gate suite: plate physics, water, terrain, liveliness, plategen, determinism.
5. Screenshots: generated world before/after erosion, Earth rivers, a delta close-up, the wide-orogen close-up, all with legends, to `docs/media/wo-0009/`.
6. Update `docs/plan/roadmap.md` status and `docs/plan/tectonics-design.md` cross-references; CLAUDE.md lessons if any.
7. Machine-labelled perf per the Air policy: erosion wall time L6/L7/L8.
8. Verification pass per RULES. Fix confirmed findings.
9. Commit, push, PR `WO-0009-S7: Phase 2 close`. Merge when green. Delete the branch. Tag `v0.3.0`.
10. Report to Dan, under 400 words: each benchmark gate with value and pass/fail, anything left failing and the physics it would take, the screenshot set, and open questions for Phase 3 (climate).

DONE WHEN. Tag `v0.3.0` exists; all gate suites green in CI; calibration JSON committed; screenshots committed; report delivered.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

