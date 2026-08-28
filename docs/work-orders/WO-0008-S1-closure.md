# WO-0008-S1: relic-basin closure, rift linkage, seam fix, continental balance

CONTEXT. Dan's rulings after the WO-0006-S3 report (2026-08-28). Runs after WO-0008-S0 (whole-plate crust setup). Fixes the four S3 misses that are closure- and bookkeeping-related. Session 2 (`WO-0008-S2-orogens-arcs.md`) reworks orogen width and island arcs. Sim hashes WILL move; the goldens are already `#[ignore]` since S0.

RULES. Single-track. Branch `feat/closure-s1` from `main`. Checkpoint commits every 30–45 minutes. Keep determinism (master seed, PCG sub-streams, fixed order, no libm, id-ordered reductions). No subagents except a final verification pass capped at three lookups. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch. Commit this work order and its sibling with message `docs: install WO-0008 S1–S2`.
2. Update `docs/plan/plate-physics-model.md` with a dated addendum: relic-basin closure (§3), rift linkage (§5), continental-area balance (§9 note), and the S2 items (orogen width, discrete island arcs) marked "S2". Append the matching decision-log entry.
3. Relic-basin closure. Add to the suture path: when a plate pair is in a locked continental collision (contact ≥ 30% of the smaller perimeter, relative velocity < `SUTURE_LOCK_CMYR`) but oceanic cells remain near the contact, identify each enclosed oceanic basin (connected oceanic region whose border cells belong ≥ 80% to the two colliding plates). While the collision stays locked, consume that basin at its margin cells at the plate pair's convergence-equivalent rate, as internal subduction feeding the overriding plate's slab ledger. A basin consumed below `RELIC_BASIN_KEEP_CELLS = 12` cells survives as a relic sea and stops blocking suture condition 3 — condition 3 now reads: no oceanic region larger than `RELIC_BASIN_KEEP_CELLS` within 2 rings of the contact. Basis: Mediterranean-style terminal closure; Caspian/Black Sea relics.
4. Rift linkage. In `grow_rifts()`: when two active rift tips on the same plate come within 3 cells, connect them along the least-strength path and merge the rifts into one system. Basis: East Africa–Red Sea–Gulf of Aden linkage.
5. Seam fix. In `advect()`, change the scatter/gather tie-breaking so seam cells resolve to the owner whose back-rotated candidate wins by the same rule in both passes (record the exact rule in the work order). Target: `connectivity_reassigned` ≤ 10 cells per 100 My at L6, measured by the probe. `enforce_connectivity()` stays as the backstop.
6. Continental balance. Diagnose with the probe: per 100 My, continental cells created (arcs, thickening above sea entry) vs destroyed (margin consumption). Adjust only the consumption eligibility — continental crust above `SUBDUCTIBLE_CONT_KM` must never be consumed at a margin — and verify arc creation rates against Earth (continental crust grows slowly; net change over 2 Gy within ±15% of start). If balance cannot be reached without a new mechanic, stop, record why, and report.
7. Gates. Update `plate_physics_gates.rs`: weld target becomes 2–6 per Gy; add: zero enclosed basins larger than `RELIC_BASIN_KEEP_CELLS` inside locked collisions older than 60 My; `connectivity_reassigned` ≤ 10 per 100 My; continental area at 2 Gy within ±15% of t=0. Leave the orogen and arc gates to S2.
8. Tests: relic-basin consumption on a hand-built enclosed basin; rift linkage on two converging tips; the seam rule on a two-plate synthetic world.
9. Run `cargo test --workspace` (green except ignored goldens). Probe both seeds; commit `docs/results/plate-physics-probe-s4-<seed>.json`.
10. Commit, push, PR titled `WO-0008-S1: closure, linkage, seams, balance`. Merge when green. Delete the branch.
11. Report to Dan in plain language, under 300 words: welds per Gy now, continental area at 2 Gy vs t=0, seam reassignment count, and the one-line paste for S2.

SEAM RULE AS IMPLEMENTED (step 5 record). Three parts, all in `advect()`:

1. **Coverage union.** A plate covers a cell when EITHER rasterization of
   its rigid motion says so: its back-rotated sample lands on its own
   crust (gather half), OR one of its cells mapped exactly onto the cell
   in the forward scatter (a ring-free `direct_mask` bit). The two half
   tests rasterize the same motion from opposite ends; requiring only
   their union removes the aliasing flicker of the old gather-only test.
2. **Per-pair subduction polarity.** Between two soft crusts, which side
   subducts is decided once per unordered plate pair per step — the side
   with the older (denser) mean crust age along the shared boundary goes
   under, tie → the higher plate id subducts — instead of per cell from
   shifting source samples, which flip-flopped along a single front and
   let the fronts interpenetrate (pairs measurably consumed each other
   BOTH ways in the same step). Hard continent still overrides per cell.
3. **Connectivity-preserving consumption.** After the gather, while any
   plate's new ownership contains a fragment, the ownership flips that
   caused it are reverted (every changed cell in the fragment plus every
   adjacent cell that changed away from the fragment's plate) — a bite
   may not pinch off a piece of a plate; severed pieces stay attached
   through the reverted neck and are consumed face-first. Reverts only
   un-do the current step's changes, so the loop terminates.

Measured at L6 (both seeds, 2 Gy): `connectivity_reassigned` fell from
~2,900–6,700 cells per 100 My to 0–5, worst window ≤ 10.

- [x] Step 1: branch, work orders committed (with S0's install)
- [x] Step 2: model addendum + decision log
- [x] Step 3: relic-basin closure (+ enclosure-based condition 3)
- [x] Step 4: rift linkage
- [x] Step 5: seam fix (rule above)
- [x] Step 6: continental balance (accretion guard + inventory guard)
- [ ] Step 7: gates armed (calibration in progress)
- [x] Step 8: tests (basin closure, relic sea; linkage + seam pending)
- [ ] Step 9: probe s4 committed
- [ ] Step 10: PR merged
- [ ] Step 11: report

DONE WHEN. PR merged; the four updated gates pass at both seeds; probe JSON committed; workspace green except ignored goldens.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

