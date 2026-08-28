# WO-0009-S1: water inventory, dynamic sea level, control relocation

CONTEXT. Phase 2 session 1, per the approved plan (Project doc claude/phase2-plan-v1.md Rev B, items 1 and 1b; a copy of the relevant text is inline below — the repo is self-contained). v0.2.3 is the base. Later Phase 2 sessions wait on Dan's research dossier; this session stands alone. Sim hashes WILL move (the sea-level solve changes); mark the two golden tests `#[ignore]` ("WO-0009 in progress") and regenerate them at the END of this session as the fifth sanctioned move, with the decision-log entry.

RULES. Single-track. Branch `feat/water-inventory` from `main`. Checkpoint commits every 30–45 minutes. Keep determinism (f64 sums in cell-id order for the water solve). No subagents except a final verification pass capped at three lookups. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch. Commit this work order with message `docs: install WO-0009-S1`.
2. Append to `docs/plan/decision-log.md`: "Sea level: fixed water mass replaces constant ocean fraction (Dan 2026-08-28). Mass is conserved; volume is derived via seawater density. Hooks reserved for ice mass and thermal expansion (Phase 3+)."
3. In `keyframe.rs` / `SimState`: add `water_mass_kg: f64`, set once at t=0 in `elevation.rs::derive_and_solve()`: run the existing cell-fraction solve one final time, integrate flooded volume `Σ max(0, s − elev(c)) · cell_area_m2` in cell-id order (f64), multiply by `RHO_SEAWATER: f64 = 1027.0`, store. Add `water_mass_kg` to the keyframe encoding and round-trip test.
4. For every subsequent keyframe, replace the fraction solve and the PR #3 anchor-and-drift logic: bisect on s (40 fixed iterations, f64) so the integrated flooded mass equals `water_mass_kg - mass_in_ice`, where `fn mass_in_ice(&self) -> f64` returns 0.0 with a doc comment naming the Phase 3+ hook. Store the solved offset per keyframe exactly as today (0 = sea level downstream).
5. UI label beside the Land fraction slider: "start NN% → now MM%", start from the S0 achieved fraction, now from the viewed keyframe's solved land fraction.
6. Freeze-shoreline toggle: checkbox "Hold shoreline at present level" in the top bar. Display-only: when checked, every keyframe renders against the present era's solved offset. Add a hash-equality test proving the toggle changes no sim state.
7. Control relocation. Move the timeline strip — scrubber, ⏮ ▶ ⏸ ⏭, the speed ComboBox, the `t =` readout, `Set as present`, and the cell-inspect status line — from the bottom panel into a third top row under the existing two in `top_bar()`. Delete the bottom `TopBottomPanel`. Nothing may anchor to the bottom window edge. Screenshot at 1440 px width proving it; commit to `docs/media/wo-0009/top-controls.png`.
8. Gates. Add `water_gates.rs`: (a) water mass conservation — for every keyframe of a 2 Gy L6 run at seeds 42 and `cyrus`, the integrated flooded mass equals `water_mass_kg` exactly (f64, fixed order); (b) highstand sign test — a probe world whose ocean age is uniformly young solves a HIGHER sea level than the same world with ocean uniformly old (Hays & Pitman 1973 ridge-volume effect, sign only).
9. Re-baseline the Phase 1 hypsometry gate values against the new solve; record old and new values in the decision log as a sanctioned move tied to this WO.
10. Regenerate the two goldens once (fifth sanctioned move), remove `#[ignore]`, decision-log entry in the M3 style. Phase 0 noise golden unmoved.
11. Run `cargo test --workspace`. All green. Probe both seeds; commit `docs/results/water-solve-<seed>.json` with the solved level per keyframe.
12. Commit, push, PR titled `WO-0009-S1: water inventory and control relocation`. Merge when CI is green. Delete the branch.
13. Report to Dan in plain language, under 300 words: the t=0 water mass at each seed, how far the shoreline moves over 2 Gy (min/max solved level), the toggle, where the controls now live, and confirmation the goldens moved once.

DONE WHEN. PR merged; both water gates green in CI; no bottom-anchored controls (screenshot committed); goldens regenerated once; workspace green.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
