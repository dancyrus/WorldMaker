# WO-0009-S6: plate drag as boundary condition

CONTEXT. Phase 2 session 6. S5 merged. Dan's rule: edits are boundary conditions on the physics, never velocity decrees.

RULES. Single-track. Branch `feat/plate-drag` from `main`. Checkpoints 30–45 min. Sim hashes move only via the overlay participating in params_hash (no golden move — goldens are at default params with no overlay).

STEPS.

1. `git pull --ff-only origin main`. Create the branch.
2. Overlay: `drag_overlay: Vec<DragStroke>` in `TectonicsParams`, hashed. `DragStroke { cell: u32, target_v: [f32;3], ramp_my: f32, hard: bool }`.
3. Soft mode (default): in `motion_update()`, each stroke adds a driving torque toward `target_v` at its cell, decaying linearly over `ramp_my` (default 50 My) — the force balance still runs; slab pull and collisions still have their say.
4. Hard mode: pins `omega_target` for the plate while the stroke is active; unphysical badge.
5. UI: Drag tool beside Navigate; drag on either canvas previews an arrow through the pending-edit system; badge counts it; Regenerate applies via `run_history(ResumeFrom)` from the viewed keyframe.
6. Tests: resume-with-drag determinism (same drag + seed = same world); a soft drag on a slab-free plate moves it; the same drag against a locked collision does not tear the collision open.
7. `cargo test --workspace`. Screenshot: drag preview arrow, to `docs/media/wo-0009/`.
8. Commit, push, PR `WO-0009-S6: plate drag`. Merge when green. Delete the branch.
9. Report to Dan, under 250 words, and the S7 paste.

DONE WHEN. PR merged; determinism test green; soft/hard semantics tested; goldens untouched.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

