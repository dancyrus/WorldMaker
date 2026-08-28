# WO-0009-S5: terrain brushes

CONTEXT. Phase 2 session 5. S4 merged. All brushes ride the pending-edit system (worldmaker-io pending.rs); drawing never simulates; Regenerate applies (standing contract).

RULES. Single-track. Branch `feat/terrain-brushes` from `main`. Checkpoints 30–45 min.

STEPS.

1. `git pull --ff-only origin main`. Create the branch.
2. Direct brushes: Raise, Lower, Smooth; radius + strength controls. Soft mode = pre-erosion elevation bias folded into the terrain stage params; hard mode = painted cells locked against erosion, badged when physics checks fail on them. Overlays hashed into the terrain stage params hash.
3. Intent stamps: Mountain range (stroke adds crust thickness + young orogeny age to the tectonic-output snapshot; isostasy + erosion realize it); Island chain (hotspot-style buildup line, lithology vb).
4. River assist: user draws a course; carve a least-cost channel, hard-locked. Lake stamp: depression + fill.
5. Fast path: direct strokes re-run terrain only; intent strokes re-run from tectonic output. Measure and record both latencies at L6/L7.
6. Undo, badge counting, discard, preset-change semantics per the standing interaction contract; tests for each brush's fold and for hard-lock survival through a terrain re-run.
7. `cargo test --workspace`. Screenshots: a painted range realized by the sim, a locked hard stroke, to `docs/media/wo-0009/`.
8. Commit, push, PR `WO-0009-S5: terrain brushes`. Merge when green. Delete the branch.
9. Report to Dan, under 250 words, with the two screenshots, the two latencies, and the S6 paste.

DONE WHEN. PR merged; all six brush tools work through pending edits; hard/soft tests green; latencies recorded.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

