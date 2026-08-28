# WO-0009-S4: Earth preset

CONTEXT. Phase 2 session 4. S3 merged. Benchmarks doc: `docs/plan/earth-benchmarks-v1.md` Tables 6.1, 6.2, 7.1.

RULES. Single-track. Branch `feat/earth-preset` from `main`. Checkpoints 30–45 min. The ETOPO download happens once, in a dev tool, never in the app.

STEPS.

1. `git pull --ff-only origin main`. Create the branch.
2. Data: download ETOPO 2022 60-arc-second ice-surface elevation (NOAA NCEI, public domain). Dev-only converter in `worldmaker-io`: per-cell mean elevation for L6, L7, AND L8. Commit binaries under `data/earth/` (~31 MB total) with a README recording source, version, license, processing steps, and checksums.
3. World menu: Generated (seed) | Earth (present day). Earth injects elevation and land/sea crust type, bypasses tectonics, hides the timeline. Synthesize GLiM lithology by documented rules: shields → pa, young ranges → mt, island arcs → vi, sedimentary basins → ss, ocean floor → vb.
4. Water: `water_mass_kg = 1.4e21`. GATE: the S1 solver lands within ±100 m of the real shoreline on L7 Earth (benchmarks Table 7.1 anchors: land 29%, mean land elevation ~840 m — record both).
5. Hydrology routing-only (stage flag; no erosion loop on Earth). Run depression fill, flow, discharge, rivers, lakes.
6. Checks: no-uphill invariant on Earth (gate); basin count sanity range (gate); Mississippi, Amazon, Congo, Nile basins assemble at L7 (qualitative — screenshot for Dan, compare areas to benchmarks Table 6.1, recorded not gated).
7. `cargo test --workspace`. Screenshot: Earth with rivers on, to `docs/media/wo-0009/earth-rivers.png`.
8. Commit, push, PR `WO-0009-S4: Earth preset`. Merge when green. Delete the branch.
9. Report to Dan, under 300 words: the solved Earth sea level, basin comparison table, the rivers screenshot, and the S5 paste.

DONE WHEN. PR merged; Earth loads offline from committed data; ±100 m gate green; no-uphill green on Earth; screenshot committed.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

