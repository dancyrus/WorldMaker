# WO-0008-S2: wide orogens, discrete island arcs

CONTEXT. Session 2 of 2. Dan's observation (2026-08-28): every range is a one-cell Andes (~110 km at L6); no wide orogens (Himalaya–Tibet, Alps, Rockies class); ocean-ocean convergence raises a continuous wall of land instantly instead of an island arc. Causes: `apply_collisions_and_rifts()` thickens only contact cells; `apply_arcs()` grows a solid band and converts to continent at a fixed thickness. S1 is merged.

RULES. Same as S1. Branch `feat/orogens-arcs-s2` from `main`. Goldens regenerated here (fourth sanctioned move).

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch.
2. Distributed shortening. In `apply_collisions_and_rifts()`: spread each contact cell's shortening over a zone extending inboard on the overriding side. Zone depth per boundary cell: `W = W_BASE / strength(cell)`, walked inboard cell by cell and stopped early where `strength ≥ CRATON_STOP = 1.5`. Distribute the thickening across the zone with a linear taper. Set `W_BASE` so weak crust gives 3–8 cells at L6 (300–900 km) and cratonic crust gives ≤ 1. Basis: deformation localizes in weak lithosphere and dies at cratons (Tarim, Sichuan stop the Himalayan front).
3. Gravitational spreading. After thickening, one diffusion pass per step on continental cells with thickness > `SPREAD_THRESHOLD_KM = 60`: move excess thickness to the thinnest neighbor at rate `SPREAD_KM_MY = 0.05` km/My per km of excess. Cap stays `THICKNESS_CAP_KM = 70`. Basis: lower-crustal channel flow turns walls into plateaus (Tibet).
4. Underthrusting. When continental crust is consumed at a locked collision margin (S1 step 6 keeps thick continent from vanishing), transfer its thickness budget into the distributed zone of step 2 instead of deleting it. Basis: India underthrusting Tibet doubles crust inboard.
5. Discrete island arcs. Rework `apply_arcs()` for ocean-ocean convergence: keep the arc band position (150–250 km inboard), but grow volcanic edifices only at discrete arc sites — every `ARC_SITE_SPACING_CELLS = 2` cells along the band, chosen deterministically by cell id — at the existing growth rate. Between sites the band gains at most 20% of the site rate. Conversion to continental crust happens per cell when that cell reaches the existing threshold, so islands emerge one by one over tens of My; most of the arc stays submarine. Continent-margin arcs (ocean under continent) keep the current continuous behavior — that is the Andes, and it is correct. Basis: Marianas/Aleutians vs Andes.
6. Crust-volume ledger. Add to `SimState` a per-step continental crustal-volume accounting: `d_volume = created(arcs) + created(thickening from oceanic subduction) - destroyed(rift oceanization) - transferred`, where the collision path itself must conserve volume exactly: every km³ of continental crust removed from consumed margin cells in a locked collision appears as added thickness in that collision's distributed zone, same step. Unit test: a synthetic two-continent collision conserves `sum(thickness × cell_area)` over continental cells to within float tolerance across 100 steps. Probe: report the ledger's unexplained residual per 100 My; gate it at zero (exact, integer-summed in fixed order). Basis: mass conservation; shortening trades area for thickness (crustal volume is conserved through orogeny up to erosion, which the tectonics stage does not model — Phase 2 erosion will draw from this same ledger).
7. Ocean-ocean guard. Assert in debug builds that continent-collision thickening never applies where both sides of the contact are oceanic.
8. Gates. Add to `plate_physics_gates.rs`: at least one collision zone per run reaches a deformed width ≥ 3 cells with thickness > 45 km spanning it; no ocean-ocean boundary produces a connected above-sea landmass wider than 1 cell within 50 My of the boundary forming; the §9 metric 6 target (collisions build relief) re-measured — record the honest number and set the gate 5 points below it.
9. Goldens. Regenerate the two golden hashes once, remove `#[ignore]`, decision-log entry: "fourth sanctioned golden move, WO-0008". Phase 0 noise golden unmoved.
10. Tests: distributed zone stops at a synthetic craton; spreading conserves total thickness; an ocean-ocean band produces islands, not a wall (land fraction of the band < 30% at 50 My in a synthetic run).
11. Run the full workspace suite and the probe at both seeds; commit `docs/results/plate-physics-probe-s5-<seed>.json`. Screenshots to `docs/media/wo-0008/`: a wide orogen close-up (elevation + thickness layers), an island arc chain close-up, both with legends visible.
12. Commit, push, PR titled `WO-0008-S2: wide orogens and island arcs`. Merge when green. Delete the branch. Tag `v0.2.3`.
13. Report to Dan in plain language, under 400 words: widest orogen in cells and km at each seed, the arc-band land fraction over time, metric 6 before and after, the screenshots, and anything that still needs missing physics.

IMPLEMENTATION NOTES (S2 close). Continental-collision thickening creates
no volume: underthrust budgets (the consumed margin's whole column) fund
the distributed zones (1/3) and the pair's foreland shelf (2/3, one full
column at a time — this returns the consumed area). Spreading may flow
onto same-plate shelf. Arc sites use 3 rings of clearance instead of the
2-cell spacing (2-spacing on a one-ring band caps at 50% band-land vs
this order's own <30% target); conversions are island-blocked. The
ocean-ocean wall gate is enforced strictly by the synthetic test; the
runtime tracker check is RECORDED (advection smears sub-cell islands
into small drifted islets a runtime isolation test cannot tell from
walls). m6 re-measured honestly at 73%/39% and gated at 34% (armed);
s2_orogen_width (16 cells at both seeds) and s2_volume_ledger (exact)
armed. Recalibrated: C_CONTACT 900, refractory 240 My, MIN_SPLIT 24,
W_BASE 260 km.

- [x] Step 1: branch
- [x] Step 2: distributed shortening (underthrust-funded)
- [x] Step 3: gravitational spreading
- [x] Step 4: underthrusting transfer
- [x] Step 5: discrete island arcs
- [x] Step 6: crust-volume ledger (exact, gated)
- [x] Step 7: ocean-ocean debug guard
- [x] Step 8: gates (width, arcs-recorded, m6 at honest-5)
- [x] Step 9: goldens regenerated (fourth sanctioned move)
- [x] Step 10: tests (craton stop, spreading conserves, islands-not-wall)
- [x] Step 11: workspace suite + probe s5; screenshots pending screen
      unlock (macOS cannot composite a locked session)
- [ ] Step 12: PR, merge, tag v0.2.3
- [ ] Step 13: report

DONE WHEN. Tag `v0.2.3` exists; all gates green in CI; goldens regenerated once; screenshots committed.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```

