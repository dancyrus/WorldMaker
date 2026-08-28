# WO-0009-S3: rivers, lakes, layers

CONTEXT. Phase 2 session 3. S2 merged. Benchmarks doc: `docs/plan/earth-benchmarks-v1.md`.

RULES. Single-track. Branch `feat/rivers-layers` from `main`. Checkpoints 30–45 min. App-side only; sim hashes must not move.

STEPS.

1. `git pull --ff-only origin main`. Create the branch.
2. Rivers: smoothed polylines (boundary-ribbon path) where discharge exceeds a threshold; width ∝ sqrt(Q) (Leopold & Maddock 1953); Strahler order stored per segment.
3. Lakes: fill at spill level, color distinct from ocean.
4. Layers + legends: `Rivers` (overlay toggle), `Discharge` (sequential colormap, no rainbow), `Sediment` (debug). Respect `viewing_kf` and the WO-0007 legend framework.
5. Stats to results JSON: drainage density (compare against benchmarks Table 6.4 coarse/medium bands — recorded, not gated), longest-river continuity, basin count, post-erosion hypsometry (still bimodal — gate).
6. Tests: no-uphill invariant on every suite world; a river polyline never crosses a lake without entering it.
7. `cargo test --workspace`. Screenshots: a dendritic network close-up and a lake district to `docs/media/wo-0009/`.
8. Commit, push, PR `WO-0009-S3: rivers and layers`. Merge when green. Delete the branch.
9. Report to Dan, under 250 words, with both screenshots and the S4 paste.

DONE WHEN. PR merged; invariants green; stats JSON committed; screenshots committed.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

