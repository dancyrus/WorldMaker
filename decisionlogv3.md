# WorldMaker: decision log v3

Newest first. Decisions marked [Dan] are his; [default] means Claude set a sensible default that Dan can change with one sentence.

## 2026-08-19, scope revision

- [Dan] Exclude the M1 MacBook Air requirement for now. All performance budgets and benchmarks target the PC (i7-12700KF, RTX 3080).
- [default] Consequences applied: macOS CI becomes a non-blocking informational build (never blocks a merge, one config line to restore); the mac launcher script drops from Phase 0; per-machine preset guidance collapses to PC presets; cross-machine determinism testing deferred. The Rust + wgpu stack keeps macOS viable if the Air returns.

## 2026-08-19, design interview

- [Dan] Player role: all three at once. Crafting a specific map, running experiments (tilt, day length, sun brightness, sea level), and watching the god's-eye sim. The UI serves all three rather than centering one.
- [Dan] End use: the making is the point, and the app should be capable of producing a grounded world for someone's fantasy map. Real mountain formation and erosion; climate that responds to tilt, sun brightness, rotation speed.
- [Dan] Canvas: globe and flat map from day one, editable in both, with selectable map projections.
- [Dan] History: a timeline at the bottom to scroll through the geologic history and choose the perfect moment. The era picker is core; rewriting history is a tool, not the game.
- [Dan] Brushes: direct (MS Paint-style) plus intent stamps the sim realizes physically.
- [Dan] Paint vs physics: per-stroke choice. Hard mode locks the paint (badged where unphysical); soft mode lets the sim reconcile.
- [Dan] Detail: "Regions are the real goal." One continent in high detail, planet as backdrop. Regional refinement becomes numbered Phase 5.
- [Dan] Variations: named branches with side-by-side compare.
- [Dan] Look: all three styles wanted as switchable views: classic atlas, satellite realism, inked parchment fantasy.
- [Dan] Names: auto-generated in a consistent invented-language flavor, everything editable.
- [Dan] Magic: full magic toolkit someday, after the physical game works. Backlog.
- [default] Regional windows 500–5,000 km, planar refinement grid up to 8192² on the PC, climate downscaled (not re-simulated) with conservation checks.

## 2026-08-19, kickoff

- [Dan] Core loop: staged simulation (planet → plates → terrain → climate → biomes) with painting at any stage; downstream stages recompute around edits.
- [Dan] Science depth: grounded approximations. Real mechanisms, simplified math, validated against real Earth data. Not stylized noise, not research-grade GCM.
- [Dan] Stack: Rust + wgpu + egui, matching the CFD and KSP repos.
- [Dan] Priority: "Forget the KSP game." WorldMaker takes the active project slot. The CFD repo stays paused and resumable.
- [default] Repo: dancyrus/WorldMaker, public, MIT license. Working title WorldMaker.
- [default] Grid: geodesic (subdivided icosahedron), presets L6 draft / L7 standard / L8 high / L9 ultra.
- [default] Validation: the Earth test (simulated climate on real ETOPO elevation, scored against the Beck 2018 Köppen map) is a regression gate from Phase 3 onward.
- [default] Workflow: single Claude Code track until Phase 2 closes; work-order queue in the repo; test and benchmark numbers committed as machine-labelled JSON, never chat-only.
- [default] Local sessions on the PC for build-and-run phases; cloud sessions allowed for research and documentation orders.

## Open decisions (not blocking Phase 0)

- App name, if WorldMaker isn't it.
- When, or whether, the M1 Air comes back into scope.
- "Someone could make a grounded world here" hints at other people using it eventually. If that firms up, packaging and a friendlier first-run land on the backlog.
- Export formats beyond PNG, heightmap, and GeoJSON (VTT formats sit on the backlog until asked).
