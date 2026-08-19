# WorldMaker: Phase 0 kickoff prompt v3

Supersedes v1 and v2, neither of which was run. Changed in v3: performance benchmarks target the PC only; the M1 Air requirement is excluded for now. Dan: open Claude Code on your PC in a local session, in any new empty folder. Copy everything below the line and paste it in. It runs for roughly one to two hours on its own. It ends with a plain-English report and a double-click launcher for the app. Nothing else is needed from you until then.

---

You are building Phase 0 of WorldMaker: a fantasy world maker and map painter for Dan, grounded in real geology and climatology. A world will build in five simulated global stages (planet setup, plate tectonics, terrain and rivers, climate, biomes), Dan will paint at any stage with downstream stages recomputing around his edits, and a later regional-refinement stage will render a chosen continent in fine detail, because a detailed region backed by a believable planet is the end goal. Phase 0 is the walking skeleton: repo, docs, CI, and a seeded planet visible in both a 3D globe and a flat projected map. Later phases add the science.

About Dan: he does not code and does not use git. Do not ask him to read code, run commands, or make merge decisions. Report to him in plain language. You own this repository end to end, including GitHub.

## Ground rules

1. main must always build. Do feature work on branches. Merge only when CI is green. Use conventional commits.
2. Keep CLAUDE.md under 150 lines. Route detail into docs/. Add lessons learned to it as you find them.
3. Commit every test and benchmark number as machine-labelled JSON under docs/results/. Numbers reported only in chat do not count.
4. Use subagents for research and verification work that can run in parallel.
5. The app makes no network calls and has no telemetry. Commit Cargo.lock.
6. Approved dependencies: eframe/egui, wgpu, rayon, rand, rand_pcg, serde, serde_json, image, log plus a file logger, anyhow, thiserror. Record any addition beyond this list in docs/plan/decision-log.md with one line of why.
7. Do not start Phase 1 features. Phase 0 ends where this prompt ends.

## Step 1: preflight

Check for git, an authenticated gh CLI, and a current stable Rust toolchain. Install or repair quietly. Ask Dan only if you need something only he can give (for example a GitHub login approval in the browser).

## Step 2: repository and workspace

Create the GitHub repo dancyrus/WorldMaker, public, MIT license. If the name is taken, tell Dan and ask for another. Set up a Cargo workspace with four crates:

- worldmaker-core: grid, seeded RNG, field storage, math. Depends on nothing internal.
- worldmaker-sim: a Stage trait and empty stage scaffolding with a cache/dirty-propagation skeleton. Depends on core.
- worldmaker-io: results-JSON writer now; save/export stubs. Depends on core.
- worldmaker-app: eframe/egui + wgpu shell. Depends on the rest.

## Step 3: repository documents

Create: README.md (what this is, how to run it); CLAUDE.md; START-HERE.md (how any new Claude Code session picks up: read CLAUDE.md, read the work-order queue in docs/work-orders/, take the next open order); docs/plan/science-outline.md containing the science outline embedded at the bottom of this prompt, verbatim; docs/plan/roadmap.md containing the phase table embedded at the bottom, with Phase 0 marked in progress; docs/plan/decision-log.md seeded from the decisions section below; docs/work-orders/WO-0001-phase0.md holding this phase's acceptance checklist, which you keep updated; docs/results/README.md describing the results file schema (machine name, date, app version, metrics object).

Decisions to seed the log with (all dated 2026-08-19): staged-sim-with-painting core loop [Dan]; grounded approximations validated against Earth [Dan]; Rust + wgpu + egui [Dan]; globe and flat canvases from day one with selectable projections [Dan]; timeline at the bottom as the era picker [Dan]; direct plus intent brushes, each stroke hard or soft [Dan]; regions in high detail are the real goal, regional refinement is Phase 5 [Dan]; named branches with side-by-side compare [Dan]; three switchable map styles: atlas, satellite, parchment [Dan]; auto-generated editable names [Dan]; magic toolkit on the backlog [Dan]; performance benchmarked to the PC only, M1 Air requirement excluded for now [Dan]; repo name, public, MIT [default]; geodesic grid presets L6/L7/L8/L9 [default]; Earth-test regression gate from Phase 3 [default].

## Step 4: the walking skeleton

In worldmaker-core:

- Icosphere grid generator for subdivision levels 6 through 9. Vertex count must equal 10·4^L + 2. Build the dual (Goldberg) cell structure: exactly 12 pentagons, the rest hexagons. Store cell centers as unit vectors plus latitude/longitude, and neighbors in a CSR table. Structure-of-arrays f32 field storage keyed by cell id.
- Seeded RNG: one u64 master seed, PCG sub-streams keyed by (seed, stage id, purpose). Fixed iteration order everywhere. No HashMap iteration in simulation paths. No fast-math.

In worldmaker-app:

- Two synced canvases of the same planet with a Globe / Flat / Split view switcher. Globe: orthographic 3D, drag to rotate, scroll to zoom. Flat: the whole map in a projection chosen from a dropdown, equirectangular and Robinson for now, with pan and zoom and an optional graticule. Elevation for now is seeded fractal noise through worldmaker-sim's placeholder stage (replaced by real tectonics in Phase 1).
- Cursor mapping: hovering either canvas reports the cell id and lat/lon, and the same ground position must resolve to the same cell in both views. Build screen-to-cell and cell-to-screen mapping as a tested core capability now, because every brush in every later phase depends on it.
- A disabled timeline strip along the bottom of the window, reserving the layout for Phase 1's era picker, labeled to say so.
- Controls: seed text field (hash any text to a seed, never crash on input) with a Generate button; sea-level slider that recolors ocean/land live in both canvases; layer dropdown with Elevation active and Plates and Climate greyed out; preset dropdown draft L6 / standard L7 / high L8; FPS readout.
- If wgpu cannot get a GPU device, show a plain-language dialog naming the machine and the likely fix, then exit cleanly. Log to a rotating file beside the executable, not the console.

Performance requirements: every performance budget in this project benchmarks to this PC (i7-12700KF, RTX 3080); spend no effort on other machines. No per-cell heap allocation inside loops; rayon where a pass is parallel; L7 grid build under 2 seconds; 60 fps in both canvases at L7 (decimating the render mesh is allowed if you note it). Measure, do not guess.

## Step 5: tests and CI

Unit tests: vertex count per level; pentagon count is 12; neighbor table symmetric; fixed seed reproduces a fixed hash of the elevation field; generate at L6 through L8 without panic; projection round-trip (cell → flat-map position → cell returns the same cell for both projections, and globe and flat agree on the cell under a given lat/lon). A perf harness writes docs/results/perf-phase0-{machine}.json (grid build ms per level, fps sample) and docs/results/determinism-phase0-{machine}.json. GitHub Actions: fmt check, clippy with warnings denied, tests on ubuntu-latest, blocking. Add a macos-14 build job marked continue-on-error, informational only: macOS is out of scope for now and must never block a merge, but the door stays one config line away. Green blocking CI before the phase closes.

## Step 6: launchers, tag, report

Create WorldMaker.bat (Windows) that builds in release mode and runs the app. Tag v0.1.0. Save screenshots of the globe view, the flat view, and the split view to docs/media/phase0/. Then report to Dan in plain English: what exists, how to open the app, the three numbers that matter (grid build time, fps, tests passing), and any open questions you queued for Phase 1.

## Embedded science outline (write to docs/plan/science-outline.md verbatim)

WorldMaker simulation outline. Grid: subdivided icosahedron with dual Goldberg cells; presets L6 40,962 / L7 163,842 / L8 655,362 / L9 2,621,442 cells; SoA f32 fields; CSR neighbors. Pipeline: five global stages plus regional refinement, each a pure function of (params, upstream fields, edit overlays), cached by content hash, with dirty propagation downstream; edits are sparse overlays that survive re-runs, made by direct brushes (raise/lower/smooth) and intent brushes (declared geology the sim realizes), each stroke either soft (a suggestion the sim reconciles) or hard (locked paint, badged where the physics can't explain it); worlds fork into named branches that share stage caches and render side by side; every stage draws from a PCG sub-stream of one master seed. UI: globe and flat canvases in sync with selectable projections (equirectangular, Robinson first), and a bottom timeline that scrolls the geologic history to pick the present moment. Stage 0 planet setup: radius, gravity, day length (sets Coriolis and wind-belt count), axial tilt, star class and orbit (sets insolation), water budget, greenhouse knob, plate count, tectonic vigor; no accretion sim, initial crust is seeded cratons. Stage 1 tectonics (kinematic, 2 My steps, keyframes every 10 My): rigid plates with Euler poles, slab-pull-biased motion; divergent boundaries make ridges and new ocean crust; oceanic convergence subducts with trench and volcanic arc 150–250 km inboard; continent-continent collision thickens crust into orogens (cap ~70 km, erode toward ~38 km with ~200 My time constant); transforms, rifts that split continents, fixed hotspots leaving age-progressive chains; continental elevation from Airy isostasy, elevation ≈ 0.15 × (thickness − 35 km); ocean depth ≈ 2600 + 365·sqrt(age My) meters flattening near 5600 m; reference: Cortial et al. 2019, Procedural Tectonic Planets, https://perso.liris.cnrs.fr/eric.galin/Articles/2019-planets.pdf. Stage 2 terrain: uplift from active belts; stream-power erosion dh/dt = U − K·A^0.5·S solved with the Braun & Willett 2013 implicit O(n) method; hillslope diffusion with ~33° talus limit; priority-flood depression filling (Barnes et al. 2014) for lakes; steepest-descent flow routing on the dual mesh, discharge-accumulated rivers, capacity-limited sediment (shelves, deltas); latitude-proxy precipitation on the first pass, one re-erosion pass after climate runs; invariant: no river flows uphill. Stage 3 climate (monthly, iterate the annual cycle to equilibrium): Budyko–Sellers energy balance, absorbed S(1−albedo) vs emitted A + B·T with A ≈ 203 (greenhouse-scaled), B ≈ 2.1, diffusion D ≈ 0.55–0.6 tuned on the Earth test; ocean heat capacity ~30× land; lapse −6.5 K/km; albedo land 0.28 ocean 0.08 ice 0.60 with ice-albedo iteration; winds from a belt template by rotation regime plus monsoon pressure anomalies from land-sea contrast, Coriolis-rotated with ~20° friction angle; rule-built gyres with ×3 western intensification advect SST; sea ice below −1.8 °C; moisture iterated to convergence: Clausius–Clapeyron evaporation, semi-Lagrangian advection, precipitation from orographic lift, convergence, and saturation excess, land recycling 0.3–0.5. Stage 4 biomes: full Köppen–Geiger ruleset per Peel et al. 2007 (https://hess.copernicus.org/articles/11/1633/2007/); Miami-model NPP; permafrost and growing-season proxies; discharge re-accumulated from monthly P minus Thornthwaite PET. Stage R regional refinement (the end goal: one continent in high detail, planet as backdrop): user-drawn window 500–5,000 km projected onto a planar raster up to 8192²; terrain re-eroded at fine scale with uplift conditioned on global orogeny fields; boundary rivers carry global discharge; climate downscaled (lapse rate on fine terrain, fine orographic precipitation renormalized to conserve window totals), biomes reclassified; elevation and river flux pinned to the global solution at window edges. Validation: classifier unit tests against ~30 real stations; the Earth test ingests ETOPO 2022 (https://www.ncei.noaa.gov/products/etopo-global-relief-model), runs stages 3–4 on real elevation, and scores against Beck et al. 2018 Köppen maps (https://www.gloh2o.org/koppen/) on exact-class and five-group agreement; must beat a latitude-only baseline by ≥20 points and never regress. Presentation: three switchable styles (classic atlas, satellite realistic, inked parchment fantasy) and auto-generated editable feature names from a per-world language seed. Not modeled in v1: mantle convection, GCM dynamics, deep ocean overturning, carbon feedback, orbital cycles, ecology, tidal locking (stretch); magic toolkit deliberately deferred to the backlog.

## Embedded phase table (write to docs/plan/roadmap.md)

Phase 0 walking skeleton: repo, CI, docs, seeded noise planet in globe and flat canvases with projection dropdown, view switcher, timeline placeholder, presets, determinism and mapping tests. Phase 1 tectonics and era picker: plates, boundaries, orogeny, rifts, hotspots, isostatic elevation, bottom-timeline history scrubbing to choose the present, craton and plate-drag painting. Phase 2 terrain and rivers: erosion, flow routing, lakes, sediment, first direct and intent brushes with hard/soft stroke modes, live sea level. Phase 3 climate: EBM temperatures, winds, gyres, moisture transport, planet knobs driving outcomes, Earth test harness and gate, bias brushes. Phase 4 biomes: Köppen, Whittaker view, NPP, fact sheet, override brush with unphysical badge. Phase 5 regional refinement: windowed fine-detail continent with downscaled terrain, rivers, climate, biomes, same brushes inside the window. Phase 6 painter and branches: unified brush UI, locks, undo tree, named branches with side-by-side compare, 2 s draft re-runs. Phase 7 style, names, export: atlas/satellite/parchment styles, auto-named features, labels, PNG/heightmap/GeoJSON exports, reproducibility stamp. Phase 8 performance: hot kernels to wgpu compute. Phase 9 backlog: magic toolkit, glaciers, tidally locked worlds, VTT formats.
