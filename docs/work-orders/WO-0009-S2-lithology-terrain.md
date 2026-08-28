# WO-0009-S2: lithology field and terrain stage

CONTEXT. Phase 2 session 2. WO-0009-S1 is merged. Benchmarks: `docs/plan/earth-benchmarks-v1.md` (cite table numbers in gates). Lithology joins the TECTONIC keyframe, so the tectonic goldens move once here (seventh sanctioned move; the sixth is WO-0011 S1+S2, which runs first); the terrain stage gets its own new goldens.

RULES. Single-track. Branch `feat/terrain-stage` from `main`. Checkpoint commits every 30–45 minutes. Determinism rules apply. No subagents except a final verification pass capped at three lookups. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch. (This work order, its siblings, and the benchmarks are already committed.)
2. Lithology. Per-cell `lithology: u8` in the tectonic keyframe, the 16-class GLiM enum (su, ss, sm, sc, py, ev, mt, pa, pi, pb, va, vi, vb, ig, wb, nd; Hartmann & Moosdorf 2012). Written by crust events: setup cratons → pa; non-craton continent at setup → sm; collision-thickened belts → mt; arc conversion → vi; hotspot buildup and ridge fill → vb; rift-shoulder exposure → pb. Classes sc, ev, py, ig stay unwritten (documented Phase 3+ gaps). Keyframe encoding + round-trip test. Regenerate the tectonic goldens once (seventh sanctioned move, decision-log entry).
3. Layer `Lithology` in `layers.rs` + `Layer::ALL`: categorical colors, legend shows only classes present.
4. Terrain stage. New stage id `"phase2-terrain"` in `pipeline.rs`, running on the pinned era's keyframe, own `sub_rng` stream, cached by params hash. Sub-steps in fixed order:
   4.1 Uplift: `U = U0_MM_YR * exp(-orogeny_age / 50 My)` on orogen cells, `U0_MM_YR = 5.0`; a 0.5 mm/yr term on active arc and hotspot cells.
   4.2 Fluvial erosion: `dh/dt = U - K_LITH[lith] * A^0.5 * S` (m=0.5, n=1), implicit O(n) on the flow tree (Braun & Willett 2013). `K_LITH` values inside the Stock & Montgomery 1999 span (benchmarks Table 5.2): pa hardest ~1e-6, mt 2e-6, vi/vb 3e-6, pb 4e-6, ss/sm 2e-5, su 1e-4 (all at m=0.5, n=1; final values are S7 calibration inside the published span).
   4.3 Hillslope diffusion + 33 degree talus limit.
   4.4 Priority-flood with epsilon drainage (Barnes et al. 2014); closed basins become lakes at spill level.
   4.5 Flow + discharge: steepest descent on the dual mesh, id-ordered; precipitation = smoothed latitude bands (wet equator, dry 25 deg, moderate midlatitudes, dry poles).
   4.6 Sediment: capacity-limited routing, capacity ∝ Q·S; deposition writes lithology su and returns thickness through the WO-0008 crust-volume ledger. Transport only: terrain creates and destroys no rock; ledger residual gate stays zero.
5. Morphologic time slider: `morpho_my` 5–100, default 30, in the World panel; participates in the terrain stage params hash.
6. Gates (`terrain_gates.rs`): active-orogen cells erode 0.5–10 mm/yr and craton cells 0.0003–0.02 mm/yr at defaults (benchmarks Table 5.1); ledger residual zero; no-river-flows-uphill on the standard suite.
7. New terrain goldens at L6 seed 42 (post-erosion elevation, discharge). Prove Phase 0/noise goldens unmoved.
8. Run `cargo test --workspace`. Screenshots: before/after erosion at matched seed to `docs/media/wo-0009/`.
9. Commit, push, PR `WO-0009-S2: lithology and terrain stage`. Merge when green. Delete the branch.
10. Report to Dan, under 300 words: what the mountains look like after 30 My of rain, erosion-rate numbers vs the benchmark table, and the S3 paste.

DONE WHEN. PR merged; gates green; terrain goldens committed; tectonic goldens moved exactly once; workspace green.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

