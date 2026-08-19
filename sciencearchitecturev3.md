# WorldMaker: science architecture v3

2026-08-19, revised after the design interview (both canvases, per-stroke hard/soft edits, branching, regional refinement as the real goal), then again to benchmark performance to the PC only. Supersedes v2. Audience: Claude Code. This is the modeling contract: what each stage simulates, the math it uses, what it skips, where painting hooks in, and how it is validated. Formulas and constants here are starting points; tune against the validation targets and record final values in the repo.

## Pipeline contract

Each stage is a pure function: (planet params, upstream fields, edit overlays) → output fields. Rules that hold everywhere:

- One u64 master seed; every stage draws from its own PCG sub-stream keyed by (seed, stage id), so editing one stage never reshuffles another.
- Edits are sparse overlays (masks, forcings, pinned values), never destructive writes, so any stage can re-run without losing intent. Brushes come in two kinds, direct (raise/lower/smooth, MS Paint feel) and intent (declare geology, the sim realizes it), both available at every stage.
- Every stroke carries a mode. Soft: a suggestion merged into the stage's inputs for the sim to reconcile. Hard: a post-stage override that locks its cells; hard cells that fail the stage's consistency check wear an "unphysical" badge rather than silently lying.
- Stage outputs are cached by content hash; a change dirties only downstream stages.
- A world is a tree of named branches (before/after an edit, two eras, two parameter sets). Branches share stage caches through the content-hash keying, so forking is cheap and any two branches render side by side.
- Re-runs happen at draft resolution live (about a second), full resolution asynchronously.

Stage outputs, by stage: (0) planet params, insolation constant, rotation regime; (1) crust type, crust age, crust thickness, base elevation, orogeny age, volcanic flags, plate id, event log; (2) eroded elevation, flow directions, discharge, rivers, lakes, sediment; (3) monthly temperature and precipitation, winds, currents, sea ice, snow; (4) Köppen class, biome, NPP, permafrost, final rivers; (R) per-region fine-grid terrain, hydrology, climate, and biomes.

## Grid

Subdivided icosahedron with dual Goldberg cells: 12 pentagons, the rest hexagons, no pole singularities. Vertex count at level L is 10·4^L + 2.

| Preset | Level | Cells | Cell pitch (Earth-size) | Intended use |
|---|---|---|---|---|
| Draft | 6 | 40,962 | ~112 km | live editing re-runs |
| Standard | 7 | 163,842 | ~56 km | default generation |
| High | 8 | 655,362 | ~28 km | high-detail generation |
| Ultra | 9 | 2,621,442 | ~14 km | exports |

All performance budgets benchmark to the PC (i7-12700KF, RTX 3080). If Phase 1 benchmarks show headroom, High becomes the default generation preset; record the numbers and decide then.

Fields are structure-of-arrays f32 indexed by cell id, with a CSR neighbor table. Render and export may upsample with slope-conditioned noise; simulation stays at the preset level.

## Stage 0: planet setup

"Planet formation" is parameterized, not simulated: no accretion model. The player sets what formation would have determined, and the app derives the consequences self-consistently.

Parameters: seed; radius 2,000–15,000 km (Earth 6,371); bulk density preset → surface gravity; day length 6 h–120 d (Earth 24 h); axial tilt 0–90° (Earth 23.4°); star class F/G/K/M plus orbital distance → insolation constant (Earth 1,361 W/m²), with a habitable-zone hint in the UI; water budget → target ocean fraction (Earth 0.71); greenhouse strength (scales the EBM emission term); plate count 8–24; tectonic vigor as mean plate speed 2–10 cm/yr (Earth ~5); simulated span 200 My–2 Gy.

Derived: gravity g = (4/3)·π·G·ρ·R; Coriolis parameter f = 2Ω·sin(lat); circulation regime from rotation period, roughly one cell per hemisphere for slow rotators (period over ~8 d), the Earth-like three-cell pattern near 10 h–5 d, more and narrower bands when faster. Label this scaling as an approximation in the UI.

Initial crust: a cooled surface seeded with N cratons (thick, ancient continental nuclei) sized so continental crust hits the target land fraction after tectonics. This matches how continents actually nucleated, without pretending to simulate the Hadean.

## Stage 1: tectonics

Kinematic rigid plates driven by physically informed rules, not mantle convection. Time step 2 My; keyframes every 10 My for the timeline scrubber. See Cortial et al. 2019 (reference list) for workable plate split/merge machinery.

- Plates: Voronoi seeds on the sphere; each plate has an Euler pole and angular speed. Motion drifts slowly (random walk) but is biased by slab pull: plates with more subducting boundary accelerate toward their trenches. Slab pull dominance is the real force balance and produces supercontinent-cycle behavior cheaply.
- Boundary classification each step from relative velocity: divergent, convergent, transform.
- Divergent: mid-ocean ridge, new oceanic crust at age 0. Through a continent: crust thins, subsides, then splits into a new ocean.
- Convergent, ocean under anything: subduction. Trench at the boundary (deepen to 8–11 km), volcanic arc on the overriding plate 150–250 km inboard. Ocean-ocean makes island arcs.
- Convergent, continent-continent: no subduction; crust thickens (orogeny) up to ~70 km, building Himalaya-class ranges. Colliding continents suture into one plate.
- Transform: strike-slip, minor relief, fracture zones for texture.
- Hotspots: a few fixed plumes; moving plates leave age-progressive volcanic chains.
- Crust state per cell: type (continental/oceanic), age, thickness.

Elevation from isostasy and cooling, not noise:

- Continental (Airy): elevation ≈ 0.15 × (thickness − 35 km), which puts 40 km crust near +750 m and 70 km crust near +5.3 km. Orogens erode-decay toward ~38 km thickness with a ~200 My time constant.
- Oceanic (half-space cooling, GDH1-flavored): depth ≈ 2,600 m + 365·√(age in My) m, flattening near 5,600 m for old basins.

Painting hooks: craton brush (place or erase continental nuclei), plate pole drag (steer a plate), hotspot placement, force-a-rift and force-a-suture nudges, and re-run history from any keyframe.

Validation: ocean depth-vs-age within 10% of the cooling curve; land/sea hypsometry bimodal like Earth's; arcs sit on the correct side of trenches; identical seed reproduces identical output hash.

## Stage 2: terrain and water

Erosion and hydrology turn isostatic elevation into landscape. Precipitation is needed before climate exists, so pass 1 uses a latitude-band precipitation proxy, then stage 3 runs, then one re-erosion pass uses real rainfall. Two iterations suffice; then freeze.

- Uplift: active orogens and arcs keep rising (0.1–10 mm/yr scale) while erosion works.
- Fluvial erosion: stream power law, dh/dt = U − K·A^0.5·S, solved with the Braun & Willett 2013 implicit O(n) method on the flow tree. Calibrate K so equilibrium relief looks terrestrial; record the value.
- Hillslopes: diffusion for soil creep plus a ~33° talus limit.
- Depressions: priority-flood (Barnes et al. 2014) → lakes and endorheic basins, not artifacts.
- Flow routing: steepest-descent on the dual mesh; discharge accumulates precipitation minus evaporation; rivers render above a discharge threshold, width ~ √discharge, Strahler order kept for styling.
- Sediment: capacity-limited routing (capacity ~ Q·S); deposits build shelves, deltas, and basin fills.
- Glaciation (later polish): carve where annual temperature stays well below freezing.

Painting hooks: raise/lower brush, ridge and valley pens, river assist (carve a least-cost channel to make a painted river physically valid), lake stamp, sea level slider with live coastline.

Validation: hard invariant that no river flows uphill (test walks every river); drainage density and longest-river continuity in plausible ranges; hypsometry re-checked after erosion.

## Stage 3: climate

Monthly resolution. Solve the annual cycle to equilibrium (a few simulated years). This is an energy-balance model plus rule-based circulation and iterative moisture transport, which is the honest middle ground between noise and a GCM.

- Insolation: standard top-of-atmosphere value from latitude, tilt, season, and the stage 0 constant.
- Temperature: Budyko–Sellers energy balance per cell: absorbed = S·(1−albedo); emitted = A + B·T with A ≈ 203 W/m² (scaled by the greenhouse knob) and B ≈ 2.1 W/m²/°C; horizontal transport as diffusion, D ≈ 0.55–0.6 W/m²/°C tuned on the Earth test (North et al. 1981 for the framework). Ocean cells get ~30× land heat capacity, giving maritime moderation and a 1–2 month seasonal lag. Surface temperature applies a −6.5 K/km lapse rate over terrain.
- Albedo: land 0.28, ocean 0.08, snow/ice 0.60; iterate for the ice-albedo feedback.
- Winds: zonal belt template from the stage 0 circulation regime (trades, westerlies, polar easterlies for the three-cell case), plus a monsoon term: seasonal surface pressure anomaly proportional to land-sea temperature contrast; flow follows the pressure gradient rotated by Coriolis with ~20° surface friction cross-isobar angle. Seasonal ITCZ migration follows the thermal equator.
- Ocean currents (rule-built v1): subtropical gyres centered near 30° (anticyclonic in the north), subpolar gyres opposite, western boundary intensification ×3, equatorial currents and countercurrent. Currents advect sea-surface temperature anomalies, which is what warms high-latitude west coasts. Sea ice where SST < −1.8 °C.
- Moisture: iterate to convergence per month: evaporate over water (Clausius–Clapeyron capacity, q_sat ∝ 6.11·exp(17.67·T/(T+243.5))) and partially over wet land (recycling factor 0.3–0.5); advect humidity along the wind field (semi-Lagrangian); precipitate from orographic lift (wind · elevation gradient), convergence (ITCZ, lows), and saturation excess when air cools.

Painting hooks: rainfall and temperature bias brushes stored as forcings that survive re-runs; pin a current; everything else is indirect through terrain and sea level, which is how it should feel.

Validation, the Earth test: ingest ETOPO 2022 (60 arc-second netCDF, downsampled to the grid), run stages 3–4 only on real Earth geography, classify, and score against the Beck et al. 2018 observed Köppen map aggregated to the same grid. Metrics: exact-class agreement, five-group agreement (A/B/C/D/E), and per-class F1 for the major classes, all committed to docs/results/. Gate: beat a latitude-only baseline by at least 20 percentage points, and never regress in later phases. Qualitative checklist that must hold: subtropical deserts near 20–30°, Mediterranean climates on 30–45° west coasts, monsoon rains on subtropical east coasts, maritime west / continental interior contrast, rain shadows leeward of ranges, an ITCZ rainforest belt, tundra gradients toward the poles.

## Stage 4: biomes

- Köppen–Geiger classification with the full published ruleset (Peel et al. 2007 thresholds), including the B-climate aridity formula and the a/b/c/d seasonal letters. About 30 classes.
- Views: Köppen colors, a Whittaker temperature-precipitation biome view, and a fantasy-friendly naming layer on top of the same data.
- Proxies: net primary productivity from the Miami model (min of its temperature and precipitation curves); permafrost from mean annual temperature thresholds; growing season as months above 5 °C.
- Rivers finalized: re-accumulate discharge from monthly precipitation minus Thornthwaite-style evapotranspiration.

Painting hook: biome override brush; where an override contradicts the climate, the cell carries the unphysical badge rather than silently lying.

Validation: classifier unit tests against ~30 real stations with known classes (Singapore Af, Cairo BWh, Rome Csa, London Cfb, Winnipeg Dfb, Phoenix BWh, Mumbai Am, Reykjavik Cfc, and so on); each test records its data source alongside the expected class in the committed test file.

## Stage R: regional refinement

The global pipeline exists to make this stage trustworthy: Dan mostly wants one continent in high detail, with the planet as the backdrop that keeps it believable. A region is a user-drawn window, roughly 500 to 5,000 km across, refined to fine resolution while staying consistent with the planet around it.

- Mesh: project the window through an oblique stereographic projection onto a planar raster, 2048² up to 8192² on the PC, giving roughly 0.25–2 km per pixel depending on window size.
- Terrain: resample global elevation, then re-run stream-power erosion and hillslope diffusion at fine resolution with uplift conditioned on the global orogeny age and type fields, plus slope-conditioned detail noise. Elevation pins to the global solution at the window edge.
- Hydrology: full fine-scale flow routing. Rivers crossing the boundary carry the global discharge as boundary flux, so a great river enters the window as exactly the river the planet says it is. Lakes re-solve locally.
- Climate: downscaled, not re-simulated. Monthly global temperature, precipitation, and wind interpolate into the window; temperature re-applies the lapse rate on fine terrain; precipitation gains a fine-scale orographic term (wind against fine slopes) renormalized so window totals conserve the global answer. Köppen and biomes reclassify at fine resolution.
- Coastlines: fractal refinement conditioned on coast type (fjords where glaciated, drowned-valley rias, barrier islands on gentle shelves) as a stretch item inside the phase.
- Editing: the same direct and intent brushes with the same hard/soft modes work inside the window. Multiple regions per world. When a global stage re-runs, affected regions are marked stale with one-click re-refinement.

Validation: elevation and river flux match the global fields at the window edge within tolerance; downscaled precipitation conserves window totals; the no-uphill-river invariant holds at fine scale.

## Stage 5: presentation and export

Two synced canvases from day one: the orthographic globe and a flat map with a projection dropdown (equirectangular and Robinson first; the list grows). Editing works in either canvas; a brush maps through the projection's inverse to the same cells. Layer views for every field, including monthly animation. Three switchable looks, all planned: classic atlas (hypsometric tints, hillshade), satellite realism, and inked parchment fantasy. Features (ranges, rivers, seas, regions) get auto-generated names from a per-world invented-language seed, every one editable. Exports: styled maps and region crops as PNG up to 8k equirectangular (16k on the PC), 16-bit heightmap PNG, rivers and coastlines as GeoJSON, and a planet fact sheet embedding seed and parameters so any world is reproducible.

## Deliberately not modeled in v1

Mantle convection (kinematic plates instead), primitive-equation atmosphere (EBM plus transport instead), deep ocean overturning (surface gyres only), carbon-silicate weathering feedback (greenhouse is a knob), orbital Milankovitch cycles, ecological dynamics (classification instead), soil genesis, and tidally locked worlds. Each has an upgrade path that the grid and stage contract do not foreclose; tidal locking is the most requested likely stretch and only needs substellar-point coordinates in the EBM.

## References

Verified links, checked 2026-08-19:

- Cortial et al., Procedural Tectonic Planets, Computer Graphics Forum (Eurographics 2019): https://perso.liris.cnrs.fr/eric.galin/Articles/2019-planets.pdf (plate split/merge machinery)
- Beck et al. 2018, Present and future Köppen-Geiger climate classification maps at 1-km resolution, Scientific Data 5:180214: https://www.nature.com/articles/sdata2018214 — data at https://www.gloh2o.org/koppen/
- ETOPO 2022 Global Relief Model, NOAA NCEI: https://www.ncei.noaa.gov/products/etopo-global-relief-model (60 arc-second netCDF for the Earth test)
- Peel et al. 2007, Updated world map of the Köppen-Geiger climate classification, Hydrology and Earth System Sciences 11:1633: https://hess.copernicus.org/articles/11/1633/2007/

Cited from literature (no link needed, do not guess URLs): Braun & Willett 2013, Geomorphology 180-181, the O(n) stream power solver. Barnes et al. 2014, Computers & Geosciences 62, priority-flood. Stein & Stein 1992, Nature 359, ocean depth and heat flow vs age. North, Cahalan & Coakley 1981, Reviews of Geophysics 19, energy balance climate models. Lieth 1975, the Miami NPP model.

Prior art to study, not to copy code from: Azgaar's Fantasy-Map-Generator (github.com/Azgaar/Fantasy-Map-Generator), Red Blob Games mapgen4 (redblobgames.com/maps/mapgen4/), mewo2's terrain notes (mewo2.com/notes/terrain/), Nick McDonald's erosion and hydrology write-ups (nickmcd.me).
