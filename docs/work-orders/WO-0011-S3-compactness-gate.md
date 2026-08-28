# WO-0011-S3: compactness gate and census measurement

CONTEXT. Plate-shape fix, session 3 of 3. S1+S2 are merged; the shape defects are fixed. This WO commits the regression gate at Dan's ruled thresholds, measures the plate census, and closes the fix. No goldens move.

RULES. Single-track. Branch `feat/plate-shape-gate` from `main`. Checkpoint commits every 30–45 minutes. Determinism rules apply. No subagents except a final verification pass capped at three lookups. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch.
2. Gate file `crates/worldmaker-sim/tests/plate_shape_gates.rs`, built on the S1 metrics functions (`plate_shape_probe.rs` stays as the diagnostic, unchanged):
   2.1 Two 2 Gy L6 runs, 24 plates, land 0.40, vigor 1.0, seeds cyrus and 42. ARMED clauses: compact (mean boundary/area) at 2 Gy <= 1.15 x its 100 My value; finger fraction <= 0.5% at every sample after 100 My; `craton_transfer_violations == 0` over the whole run; largest-plate share <= 40% (hard).
   2.2 Neck clause on the S2 dumbbell probe world at L7: no plate holds a neck narrower than 170 km joining two masses each above 2% of the sphere (Dan's ruled bound; 170 km is 3 cells at L7 — state it in km so the clause is level-independent).
   2.3 RECORDED clauses (measured and reported, not armed): alive count and largest share at 2 Gy, both seeds. Dan's ruled healthy band: alive 8–20 from a 24-plate start, largest share <= 35% target. Outside the band, the gate stays green but the report carries a census finding recommending a separate break-up/rift balance item (`check_rift_splits`, `grow_rifts`, `link_rifts`, micro thresholds) — that item is out of scope here.
3. Screenshots: L7, seed cyrus, Dan's recording settings, at 53, 600 and 2000 My, to `docs/media/wo-0011/`. Visual pass criteria: no parallel strips, no dumbbells, margins keep natural irregularity (not over-straightened). The Earth-preset margin check happens in WO-0009-S7 — the preset does not exist yet.
4. Decision-log rows: gates armed with the ruled thresholds; census measurement result and finding.
5. Run `cargo test --workspace`: green. Commit, push, PR `WO-0011-S3: compactness gate + census`. Merge when green. Delete the branch.
6. Report to Dan, under 300 words: gate results at both seeds, screenshot verdict against the original recording, the census numbers against the 8–20 / 35% band with a break-up recommendation if outside it, and the WO-0009-S2 paste line (Phase 2 resumes).

DONE WHEN. PR merged; armed clauses green at both seeds; census recorded; screenshots committed; decision-log rows written; report delivered.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
