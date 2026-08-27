# WorldMaker roadmap

| Phase | Name | Scope | Status |
|---|---|---|---|
| 0 | Walking skeleton | Repo, CI, docs, seeded noise planet in globe and flat canvases with projection dropdown, view switcher, timeline placeholder, presets, determinism and mapping tests. | **Complete — v0.1.0 (2026-08-19)** |
| 1 | Tectonics and era picker | Plates, boundaries, orogeny, rifts, hotspots, isostatic elevation, bottom-timeline history scrubbing to choose the present, craton and hotspot painting (plate drag queued to Phase 2). | **Complete — v0.2.0 (2026-08-19)** |
| 1½ | Feel pass (WO-0003) | Pending edits with explicit Regenerate, hybrid plate generator (no soccer ball), smooth per-fragment rendering + render detail + Eckert IV + L8 default, plates never freeze for good. | **Complete — v0.2.1 (2026-08-27)** |
| 1¾ | Plate physics (WO-0005/0006) | Audit of the ad-hoc mechanics, then the accepted force-balance model: slab ledger, strength field, three-condition suture, driver-based rifting, microplates, §9 metric gates, slab Overlay layer. | **Complete — v0.2.2 (2026-08-28)** |
| 2 | Terrain and rivers | Plate-drag brush (queued from Phase 1), erosion, flow routing, lakes, sediment, first direct and intent brushes with hard/soft stroke modes, live sea level. | Not started |
| 3 | Climate | EBM temperatures, winds, gyres, moisture transport, planet knobs driving outcomes, Earth test harness and gate, bias brushes. | Not started |
| 4 | Biomes | Köppen, Whittaker view, NPP, fact sheet, override brush with unphysical badge. | Not started |
| 5 | Regional refinement | Windowed fine-detail continent with downscaled terrain, rivers, climate, biomes, same brushes inside the window. | Not started |
| 6 | Painter and branches | Unified brush UI, locks, undo tree, named branches with side-by-side compare, 2 s draft re-runs. | Not started |
| 7 | Style, names, export | Atlas/satellite/parchment styles, auto-named features, labels, PNG/heightmap/GeoJSON exports, reproducibility stamp. | Not started |
| 8 | Performance | Hot kernels to wgpu compute. | Not started |
| 9 | Backlog | Magic toolkit, glaciers, tidally locked worlds, VTT formats. | Not started |
