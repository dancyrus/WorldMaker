# WorldMaker: roadmap v3

2026-08-19, revised after the design interview, then again to benchmark performance to the PC only (M1 Air requirement excluded for now; macOS CI becomes informational). Supersedes v2. Two changes drive the rework: both canvases (globe and flat, with projections) move into Phase 0, and regional refinement becomes a numbered phase because a detailed continent is the real goal, with the planet as its backdrop. Session counts are estimates; each phase closes with CI green, results JSON committed, a screenshot in docs/media/, and a plain-English report. Painting tools ship inside the phase that creates their stage.

## Phase 0: walking skeleton (1 session)

Repo, CI, docs, and a spinnable planet in both canvases. Icosphere grid with seeded placeholder elevation; globe view and flat view with a projection dropdown (equirectangular and Robinson) and a Globe / Flat / Split switcher; sea-level slider; seed box; layer dropdown; presets; FPS readout; a disabled timeline strip along the bottom reserving its place for Phase 1.

Dan sees: a planet he can spin, flatten, reproject, reseed, and flood.
Accepts when: grid unit tests pass (vertex counts, 12 pentagons, neighbor symmetry), determinism hash passes, the same cell sits under the cursor in globe and flat views (mapping round-trip test), blocking CI green on ubuntu (the macos build job is informational only), the launcher works, perf numbers committed.

## Phase 1: tectonics and the era picker (2–3 sessions)

Plates with Euler-pole motion and slab-pull bias, boundary classification, subduction with trenches and volcanic arcs, continental collision and orogeny, rifting, hotspot chains, crust age and thickness, isostatic elevation, ocean depth from crust age. The timeline at the bottom comes alive: scroll through the planet's history on cached keyframes and choose the moment that becomes "today". Craton brush, plate-pole drag, hotspot placement, re-run history from any keyframe.

Dan sees: continents drift, collide, and raise mountains as he scrubs; he picks the perfect moment.
Accepts when: depth-vs-age within 10% of the cooling curve, bimodal hypsometry, arcs on the correct side of trenches, determinism holds, 1 Gy at standard preset ≤60 s on the PC, results committed.

## Phase 2: terrain and rivers (2–3 sessions)

Stream-power erosion on the flow tree, hillslope diffusion, priority-flood lakes, discharge-accumulated rivers, sediment shelves and deltas, first-pass latitude precipitation. First direct brushes (raise/lower/smooth) and first intent brushes (mountain range, island chain), each with the hard/soft stroke mode from day one. River assist, lake stamp, live sea-level coastline.

Dan sees: believable ranges with dendritic rivers, and his first painting session where soft strokes get reconciled and hard strokes get obeyed.
Accepts when: the no-river-flows-uphill invariant passes on every generated world, drainage statistics plausible, erosion of a standard world ≤60 s on the PC, hard/soft behavior covered by tests, results committed.

## Phase 3: climate (2–4 sessions)

Energy-balance monthly temperatures with ocean thermal inertia and lapse rate, wind belts plus monsoon circulation, rule-built ocean gyres, iterative moisture transport and precipitation, sea ice and snow. Planet knobs (tilt, day length, sun brightness, greenhouse) visibly drive the outcome. The Earth test harness lands here: real ETOPO elevation in, simulated Köppen out, scored against the Beck 2018 map, committed and regression-gated from now on. Rainfall and temperature bias brushes.

Dan sees: rain shadows behind his ranges, monsoons on east coasts, deserts near 25°, and a scored report card against the real Earth.
Accepts when: Earth test beats the latitude baseline by ≥20 points, the qualitative checklist holds (deserts, Mediterranean bands, ITCZ belt, maritime/continental contrast), monthly solve ≤60 s at standard on the PC.

## Phase 4: biomes (1–2 sessions)

Full Köppen-Geiger ruleset, biome and Whittaker views, NPP, permafrost, growing season, climate-correct river discharge. Biome override brush (hard mode wears the unphysical badge). Planet fact sheet.

Dan sees: a world whose colors mean something.
Accepts when: station classification unit tests pass with sources recorded, Earth test score holds or improves, biome layer renders at 60 fps on the PC.

## Phase 5: regional refinement (3–4 sessions)

The payoff phase. Draw a window (500–5,000 km), and WorldMaker refines it on a fine planar grid: re-run erosion and hydrology at up to ~0.25–1 km per pixel with uplift conditioned on the global orogeny fields, rivers entering with their global discharge, climate downscaled with fine orographic detail, biomes reclassified. Same brushes, same hard/soft modes, inside the window. Multiple regions per world; stale regions re-refine on one click after global changes.

Dan sees: his continent, in the detail he actually wanted, consistent with the planet behind it.
Accepts when: elevation and river flux match the global solution at the window edge within tolerance, downscaled precipitation conserves window totals, no-uphill-rivers at fine scale, a 4096² region refines ≤2 min on the PC, results committed.

## Phase 6: painter and branches (2–3 sessions)

Unify the editing system: consistent brush UI across stages and scales, locks and protect-masks, an undo/redo tree, and named branches with side-by-side compare (two eras, before/after an edit, two parameter sets). Draft-quality downstream re-run in ~2 s at draft preset with async full-res refine. Until this phase, "save a copy" is the stopgap for comparisons.

Dan sees: he can protect a coastline he loves, re-roll everything else, and put two versions of the world next to each other.
Accepts when: stroke-to-draft-update ≤2 s at draft preset on the PC, no edit silently discarded, undo depth ≥100, branch switch ≤1 s, results committed.

## Phase 7: style, names, export (2–3 sessions)

The three looks: classic atlas, satellite realism, inked parchment fantasy, switchable per view. Auto-generated names for ranges, rivers, seas, and regions from a per-world invented-language seed, all editable, with sensible label placement. Exports: styled maps and region crops to 8k (16k PC), 16-bit heightmaps, GeoJSON rivers and coasts, fact sheet; re-importing a fact sheet regenerates the identical world.

Dan sees: a wall-worthy named map in any of three styles, from planet to region crop.
Accepts when: all three styles render globe and flat at 60 fps on the PC at standard preset, names regenerate deterministically per seed, export round-trip test passes, a 16k export completes on the PC.

## Phase 8: performance (1–2 sessions, pulled earlier if any budget misses)

Hot kernels (erosion inner loop, moisture iteration, EBM relaxation, regional refinement) to wgpu compute. High preset becomes the PC default; ultra and 8192² regions become comfortable.

## Phase 9: backlog (unscheduled, by Dan's ask)

The magic toolkit (unnatural climates and impossible landforms as first-class brushes, badged), glacial landforms, tidally locked worlds, supercontinent narrative generator, VTT and Azgaar export formats, art-direction mockups if wanted, in-app world gallery.

## Working agreement

One phase at a time, single Claude Code track until Phase 2 closes, then optionally a sim track and a UI track. Every phase ends by updating the roadmap status in the repo and queueing the next phase's work orders. Dan's cost per phase: paste one kickoff prompt, look at the result, answer whatever open questions the report raises.
