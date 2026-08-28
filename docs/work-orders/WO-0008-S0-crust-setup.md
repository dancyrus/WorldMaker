# WO-0008-S0: whole-plate crust setup

CONTEXT. Dan's ruling (2026-08-28): at t = 0 every plate is entirely continental crust or entirely oceanic crust. Mixed plates arise only later, through rifting and arc growth. Runs BEFORE WO-0008-S1. Sim hashes WILL move; this session marks the two golden tests `#[ignore]` ("WO-0008 in progress; regenerated in S2").

RULES. Single-track. Branch `feat/crust-setup-s0` from `main`. Checkpoint commits every 30–45 minutes. Keep determinism. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch. Commit this work order (and its S1/S2 siblings if not yet committed) with message `docs: install WO-0008 S0`.
2. In `setup.rs`, replace the craton-budget block (the `OCEAN_PLATE_CHANCE` draw, `weights`, and the per-plate BFS craton growth): select a subset of plates whose combined cell count is closest to `total_cont` (greedy: shuffle plate order with the existing `crng`, then add plates while the running sum improves the distance to `total_cont`; guarantee at least one continental and one oceanic plate). Every cell of a selected plate becomes continental crust; every cell of the others stays oceanic.
3. Cratons stay, as cores inside continental plates: each continental plate gets one craton nucleus at its most interior cell, grown by the existing BFS to 30–60% of the plate area (`crng` draw), with the existing `CRATON_PEAK_MIN_KM..MAX` thickness taper and `CRATON_AGE_MIN_MY..MAX` age. Non-craton continental cells get `CRATON_BASE_KM` thickness and a younger age (200–800 My draw). Delete `OCEAN_PLATE_CHANCE`.
4. The achieved land fraction is quantized by plate sizes. Record the achieved fraction in `SimState` and show it in the UI next to the Land fraction slider as "target 29% → start 31%" (app side, one label).
5. Mark the two golden tests `#[ignore]` with the reason above.
6. Tests: at seeds 42 and `cyrus`, every plate at t=0 is single-crust (all cells continental or all oceanic); at least one of each kind exists; achieved fraction within one largest-plate area of the target.
7. Run `cargo test --workspace` (green except the ignored goldens). Screenshot at seed `cyrus`, L6, t=0, Elevation and Plates layers, to `docs/media/wo-0008/setup-t0.png`.
8. Commit, push, PR titled `WO-0008-S0: whole-plate crust setup`. Merge when green. Delete the branch.
9. Report to Dan in plain language, under 200 words: achieved vs target land fraction at both seeds, and the screenshot.

DONE WHEN. PR merged; the single-crust test passes; goldens ignored with the WO-0008 reason; screenshot committed.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
