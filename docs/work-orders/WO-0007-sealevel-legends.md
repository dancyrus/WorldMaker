# WO-0007: sea-level hotfix and map legends

CONTEXT. Dan's report (2026-08-27, v0.2.2): in the Elevation layer, moving the sea-level slider down does not expose additional land. Also: every layer needs a legend. The slider writes `sea_level_m` into `ShadeParams` via `pack_shade_params()` (`render.rs`), and `shaders.wgsl` computes `e_base = dot(w, s) - sp.sea_level_m`, so the path looks wired; the defect is unproven. Diagnose before fixing.

RULES. Single-track. Branch `fix/sealevel-legends` from `main`. No sim changes; sim hashes must not move. Checkpoint commits every 30–45 minutes. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Remove the stale lock file `.git/packed-refs.lock`. Run `git fetch --prune origin` and `git pull --ff-only origin main`. Create the branch. Commit this work order with message `docs: install WO-0007`.
2. Diagnose. Capture two screenshots at seed `cyrus`, L6, defaults: sea level 0 m and sea level −4000 m. Compare land pixel counts.
   2.1 If the land area does not change at all: the uniform is stale. Find where `shade_params()` output reaches the GPU each frame and fix the upload. Record the cause in the work order.
   2.2 If the land area changes only slightly: the rendering is correct and the world's hypsometry is the cause. Compute the hypsometric curve from the current keyframe (fraction of cells within each 500 m elevation band). Record the band populations in the work order. Do not change the sim. Report the finding to Dan as evidence for the continental-area item in the S3 open questions.
3. Whatever step 2 finds, make the slider honest: extend its range to −6000..+6000 m and show the resulting land fraction next to it as a percentage, computed on the CPU from the active keyframe elevations minus `sea_level_m` (sampled every 8th cell is enough; update on slider release).
4. Legends. Add a legend panel anchored to the bottom-left of the canvas, one implementation, per-layer content, collapsible, hidden in Split view only if it overlaps the seam. Contents:
   4.1 Elevation: the hypsometric ramp from `hypsometric()` / the LUT rows 0–1 as a vertical color bar with labeled ticks at −6000, −4000, −2000, 0, 1000, 3000, 5500 m, and a marker at the current sea level.
   4.2 Plates: a swatch row per alive plate in the viewed keyframe: color, plate id, area as % of sphere, speed in cm/yr. Cap at the 12 largest plates plus "+N more".
   4.3 CrustAge: the viridis ramp bar, ticks 0 to the ramp maximum in My.
   4.4 Thickness: ramp bar, ticks in km.
   4.5 Plate velocity and Velocity field: an arrow-length scale bar labeled in cm/yr, plus the note "white = velocity".
   4.6 Overlay: two swatches (bright = attached slab, faded = detaching) with the age scale in My from `SLAB_DETACH_MY`.
5. The legend reads from the viewed keyframe (`viewing_kf`), not the latest.
6. Tests. A unit test that the land-fraction readout at sea level −6000 m reports ≥ the value at 0 m on a fixed synthetic elevation set; a test that every `Layer::ALL` variant returns legend content (no panics, no empty legend).
7. Run `cargo test --workspace`. Confirm goldens unmoved.
8. Screenshots to `docs/media/wo-0007/`: elevation legend at sea level 0 and −4000, plates legend, overlay legend.
9. Commit, push, PR titled `WO-0007: sea-level fix and legends`. Merge when CI is green. Delete the branch.
10. Report to Dan in plain language: which case step 2 found, the band populations if 2.2, and the four screenshots.

STEP-2 VERDICT (2026-08-28, case 2.2). The rendering is correct: the sea-level
uniform is live (packed in `shade_params()` every frame, uploaded in both
canvases' `prepare`), and the two diagnosis screenshots differ — land pixels
grow ~9.6% → ~13.3% of the map going from 0 to −4000 m, with mid-ocean ridges
emerging and continents shifting up the ramp. The world's hypsometry is the
cause: the hypsometric curve of the final keyframe (seed cyrus, L6, 500 My;
full table in docs/results/wo-0007-hypsometry-cyrus-l6.json,
Daniels-MacBook-Air) puts 41.1% of all cells in the single −6000..−5500 m
band (the abyssal plain), while the whole span −4000..−500 m holds only ~6.3%
of cells. Land fraction vs sea level: 0 m → 26.6%, −2000 m → 30.2%,
−4000 m → 36.1%, −6000 m → 97.1%. The old slider floor of −4000 m sits above
essentially all of the ocean floor, so dragging it down exposed almost
nothing — evidence for the continental-area item in the WO-0006 S3 open
questions. No sim change made.

DONE WHEN. PR merged; the step-2 verdict is recorded in this file; legends render for all seven layers; screenshots committed; workspace tests green.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
