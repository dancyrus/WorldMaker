# WorldMaker: vision and scope v3

2026-08-19, revised twice: after the design interview, then to exclude the M1 Air requirement. Supersedes v2. Companion docs: science-architecture-v3, stack-and-conventions-v2, roadmap-v3, decision-log-v3.

## The problem

Fantasy map tools let you draw geography that no planet would produce. Rivers fork going downhill, deserts sit where rain would fall, and mountain chains run wherever the pen wandered. The world looks fine until a curious reader asks why anything is where it is, and then it comes apart. WorldMaker fixes this by earning its maps: it grows a planet from plates upward, and it hands you a brush at every stage without breaking the physics underneath.

## The product in one paragraph

WorldMaker is a personal desktop app benchmarked to Dan's PC (i7-12700KF, RTX 3080); the M1 Air requirement is excluded for now, and the Rust + wgpu stack keeps macOS one step away if it returns. A world builds in five global stages: planet setup, plate tectonics, terrain and rivers, climate, biomes. Each stage simulates the real mechanism (subduction, isostasy, erosion, energy balance, moisture transport, Köppen classification) with the math simplified enough to run in seconds to minutes, and planet-level knobs (axial tilt, day length, sun brightness, greenhouse) genuinely drive the outcome. A timeline along the bottom holds the planet's geologic history; you scroll through it and choose the perfect moment to call "today." Then comes the part the whole app points at: you pick a region, and WorldMaker refines that continent to fine detail with downscaled terrain, rivers, and climate. The planet makes the region believable; the region is the map you wanted.

## The three ways it gets used

Dan named all three, so the UI serves all three rather than centering one. As a god's-eye sim: generate, watch, poke, see consequences in the layers. As a craft tool: realize a specific world believably, with brushes and locks. As an experiment bench: crank tilt, rotation, sun brightness, or sea level, branch the world, and compare outcomes side by side. The end use is the making itself, with one bar to clear: someone who wanted a grounded world for their fantasy map should be able to make one here.

## The canvas

Globe and flat map from day one, editable in either, always in sync, with selectable projections (equirectangular and Robinson first, more later). Painting works in any view; the brush maps through the projection to the same cells.

## Painting

Two kinds of brush, both available at every stage. Direct brushes work like MS Paint: raise, lower, smooth, exactly where you drag. Intent brushes place geology: a mountain range along this line, an island chain here, and the simulation realizes them physically. Every stroke carries a mode switch: soft means the stroke is a suggestion the sim reconciles; hard means the paint is law, the cells lock, and anything the physics can't explain wears a small badge so the world stays honest. Edits are overlays that survive re-runs; nothing you paint is ever silently discarded.

## Worlds branch

A world can fork into named branches: before and after an edit, two eras of the same planet, two greenhouse settings. Any two branches view side by side. Undo/redo rides on the same machinery.

## Presentation

Three switchable looks, all wanted and all planned: classic atlas (hypsometric tints and hillshade), satellite realism, and inked parchment fantasy. Features get auto-generated names in a consistent invented-language flavor per world, every one editable. Exports: styled maps and region crops at print resolution, heightmaps, vector rivers and coastlines, and a fact sheet that reproduces the world from its seed.

## What v1 is not

No civilizations or cities. No magic yet: the physical game comes first, and a magic toolkit (unnatural climates, impossible landforms as first-class brushes) sits on the backlog for later. Not a general circulation model. No multiplayer, no accounts, no telemetry. Built for Dan's PC, by Claude Code, end to end.

## Success criteria

1. Someone who knows geography looks at a generated world and finds nothing to mock.
2. The Earth test beats a latitude-only baseline by at least 20 percentage points, committed to the repo.
3. A full draft-quality planet generates in well under a minute on the PC.
4. A paint stroke shows its draft-quality downstream consequences in about two seconds.
5. A chosen region refines to roughly 1 km detail that holds up at zoom, with rivers and climate consistent with the planet around it.
6. Dan's time stays near two hours a week: paste a prompt, look at a world, decide.
