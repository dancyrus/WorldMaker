# Stage U reader (d) — goldens, determinism tests, harness.rs, results schema, CI

Read on branch `feat/feel-pass-docs` (main @ 9d5d272 + docs commit 8381450). No code changes made.

## 1. Golden constants — what exactly is pinned

All in `crates/worldmaker-sim/tests/determinism_tests.rs`. Hash functions are
`worldmaker_core::hash::hash_f32_slice` / `hash_u32_slice` (FNV-1a 64 over
little-endian bit patterns; `crates/worldmaker-core/src/hash.rs:34-56`,
`FNV_OFFSET`/`FNV_PRIME` at lines 8-9).

| Constant | Line | Value | Config | Field hashed |
|---|---|---|---|---|
| `GOLDEN_HASH_L6_SEED42` | 29 | `0xa86a_7471_79a3_5a46` | `NoiseElevationStage::default()`, `Grid::build(6)`, seed 42 | `hash_f32_slice` of `noise_stage::ELEVATION_FIELD` (`"elevation_m"`) |
| `GOLDEN_TECTONIC_ELEVATION_L6_SEED42` | 59 | `0xf751_0e72_14ed_5b62` | `TectonicsStage::new(TectonicsParams::default())`, `Grid::build(6)`, seed 42 | `hash_f32_slice` of `tectonics::ELEVATION_M` |
| `GOLDEN_TECTONIC_PLATES_L6_SEED42` | 60 | `0x70df_6db8_ec5f_653d` | same run | `hash_u32_slice` of `tectonics::PLATE_ID` |

- `TectonicsParams::default()` (`tectonics/mod.rs:92-104`): plate_count 12,
  land_fraction 0.29, tectonic_vigor 1.0, **span_my 500**, hotspot_count 6, no
  overlays. The hashed fields are the **final keyframe (t = 500 My)** — 
  `TectonicsStage::run` writes the last keyframe into `world.fields`
  (`tectonics/mod.rs:159-172`).
- Enforcing tests: `fixed_seed_reproduces_committed_hash` (line 42) and
  `tectonics_reproduces_committed_goldens` (line 63).
- The crust-type hash (`0xd5c797a8cc26afb5`) is recorded in results JSON only —
  **not** a committed golden constant.
- No goldens exist anywhere else (grepped workspace + docs).

### Regeneration procedure used in past phases (manual, no script)

History from `git log -S` and decision-log.md:

1. `b810e19` — noise golden first committed. Regenerated once on 2026-08-19
   when the noise stage switched to `sub_rng` seed derivation (decision-log
   line 26).
2. `f0d678b` — tectonic goldens first committed (elevation was
   `0x3e0d_ffc9_ef43_510e`).
3. `a1ad584` (sea-level drift) — elevation golden regenerated to
   `0xf751_0e72_14ed_5b62`; **plates constant untouched** and that was cited as
   proof the change was display-datum-only (decision-log line 53).

Procedure each time: make the behavior change → run the run that produces the
hash (the acceptance harness's determinism section prints/records it; its
config L6/500 My/seed 42 is **identical** to the golden config, so
`determinism_elevation_hash_l6_500my_seed42` in
`tectonics-phase1-*.json` literally equals the committed constant) → paste the
new value → update the `/// History:` doc-comment on the constant → add a
decision-log row → commit constant + results JSON on the same branch. For
WO-0003: B regenerates both tectonic constants exactly once, and the new
`tectonics-feelpass-*.json` determinism hashes should equal the new constants —
a free cross-check.

## 2. Test inventory (all green on this Mac, 2026-08-25)

`cargo test --workspace --release`: **52 tests pass, 2 ignored, 3.2 s wall**
with a warm build cache (cold compile adds minutes; CI cold ~4-5 min total).

| Target | Tests | Notes |
|---|---|---|
| worldmaker-app unittests (binary) | 7 | layers.rs 4, render.rs 3. App is a **binary-only crate** — no `tests/` dir possible; new app tests must be `#[cfg(test)]` modules in `src/`. |
| worldmaker-core lib | 13 | dmath 3, fields 4, hash 2, proj 3, rng 1 |
| core `tests/grid_tests.rs` | 9 + 1 `#[ignore]` | ignored: `generates_l9_without_panic` ("heavy: 2.6M-cell build; run with --ignored") — precedent for Fix 2's dev-only `#[ignore]` panel test |
| core `tests/mapping_tests.rs` | 3 | projection round-trips; Eckert IV tests will live here/proj.rs |
| worldmaker-io lib | 3 | results 2, save 1 |
| worldmaker-sim lib | 5 | pipeline 3, keyframe 2 |
| sim `tests/determinism_tests.rs` | 5 | incl. the three goldens |
| sim `tests/tectonics_tests.rs` | 7 + 1 `#[ignore]` | ignored: `debug_keyframe_stats` (env `WM_DEBUG_SPAN`/`WM_DEBUG_LEVEL`) |

## 3. harness.rs and the script surface

### CLI (`main.rs:29-47`, `parse_args`)

Exactly four flags: `--screenshots <dir>`, `--perf-out <file>`,
`--determinism-out <file>`, `--tectonics-results <file>`. **Unknown args are
warn-and-ignore** (`main.rs:43`) — today `--seed`/`--preset`/`--detail` would be
silently swallowed. Headless harnesses run before any window; the process exits
without opening one if only headless flags were given (`main.rs:188-193`).

### Tectonics harness (`harness.rs`, B's territory)

`run_tectonics_harness(out)` (lines 320-424), hardcoded `SEED: u64 = 42`
(line 19). Runs, in order: L7/500 My (age_depth ≤10% max bin error, hypsometry
2-means + Ashman D > 2 with ocean mode < −2500 / land mode |m| < 1500,
arc_trench same-plate BFS ≤400 km ≥95%, hotspot counts) → L7/1 Gy (≤60 s gate)
→ L7/2 Gy (≤1 GB keyframe budget) → L6/2 Gy `stability()` → determinism
double-run L6/500 My (elevation + plate + crust_type hashes as hex strings).
Writes one `ResultsFile`. `all_acceptance_pass` aggregates every gate.

### Screenshot script (`app.rs::drive_shot` 1014-1066, `setup_shot_stage` 944-995)

Waits until the startup history job finishes; then 6 stages, each captured at
frame 30 via `egui::ViewportCommand::Screenshot`, saved as `{dir}/{name}.png`:

| # | file | view | layer | keyframe | extra |
|---|---|---|---|---|---|
| 0 | globe.png | Globe | Elevation | last | |
| 1 | flat.png | Flat | Elevation | last | |
| 2 | split.png | Split | Elevation | last | |
| 3 | plates.png | Split | Plates | `kf_count/2` | |
| 4 | mountains.png | Globe | Elevation | last | zoom 1.6 centered on max-elevation cell |
| 5 | timeline.png | Flat | Elevation | `(kf_count*3)/5` | |

Defaults in effect for scripted runs (`app.rs:178-258`): seed text **"cyrus"**
→ master_seed `0xc4be0bf8f497a575` (FNV-1a; `seed_from_text`), preset
**Standard7 (L7)**, span 500 My, plate_count 12, hotspot_count 6, projection
Equirectangular, graticule on, sea_level_m 0. **The BEFORE set is already
captured and committed** — commit 8381450 added
`docs/media/feel-pass/before/{globe,flat,split,plates,mountains,timeline}.png`
from main @ 9d5d272 with these defaults; the AFTER set must reuse the same
default seed/preset/eras.

### Perf script (`app.rs::drive_perf` 1068-1119, `write_perf_results` 1121-1149)

Waits for history; per view Globe→Flat→Split: 40 warm-up + 240 sampled frames;
vsync off (`main.rs:226-233` sets `AutoNoVsync` when `--perf-out`). Grid builds
L6..L9 are timed in `main()` before the window (`main.rs:198-210`). Metric keys:
`grid_build_ms_L{6..9}`, `globe_fps`, `flat_fps`, `split_fps`,
`fps_grid_level` (= current preset level, today always 7), `fps_vsync_off`,
`layer: "elevation"`. When both flags are given, perf runs first and chains
into screenshots (`app.rs:238-256`, `1108-1117`).

### worldmaker-io writer

`ResultsFile { machine, date, app_version, metrics }`
(`worldmaker-io/src/results.rs:11-42`). `ResultsFile::new(&today_utc_iso(),
metrics)` fills `machine` via `machine_name()` and `app_version` from
`CARGO_PKG_VERSION` (0.1.0). `write()` pretty-prints + trailing newline,
creates parent dirs. `machine_name()` (lines 47-60): `COMPUTERNAME` →
`HOSTNAME` → `hostname -s` → `"unknown-machine"`; yields
`Daniels-MacBook-Air` here.

## 4. Results schema for the two new files

`docs/results/README.md`: filename `<topic>-<phase>-<machine>.json`; top-level
`machine` (must match filename), `date` (ISO), `app_version`, `metrics` (one
object, snake_case keys, unit suffix `_ms/_fps/_count/_hash`); must be written
via `ResultsFile`. So `tectonics-feelpass-Daniels-MacBook-Air.json` and
`perf-feelpass-Daniels-MacBook-Air.json` conform with phase token `feelpass`.
README's "benchmarked against Dan's PC only" is temporarily overridden by the
CLAUDE.md machine note (Air numbers are records, not gates). Phase-1 files stay
untouched per the pinned contract. Note Fix 2's incumbent metrics must land in
the tectonics-feelpass file **before** the replacement generator does.

## 5. CI (`.github/workflows/ci.yml`, 60 lines)

Triggers: push to main + all PRs. Jobs: `fmt` (ubuntu, `cargo fmt --check`);
`clippy` (ubuntu, `--workspace --all-targets --locked -- -D warnings`, apt
X/wayland packages); `test` (ubuntu, `cargo test --workspace --release
--locked`); `macos-build` (macos-14, build only, `continue-on-error: true` —
informational, never gates). Merge gate = the three ubuntu jobs (ground rule
1). Observed durations (`gh run list`): 51 s–1m03 warm cache, 3m08–4m46 cold;
one 6-hour anomaly on 2026-08-19 was cancelled. `#[ignore]` tests never run in
CI, but **clippy compiles all targets**, so the dev-only panel test still
costs CI compile time. `image` is currently a dependency of worldmaker-app
only — Fix 2's PNG test needs it added as a **dev-dependency of
worldmaker-sim** (already on the approved list). worldmaker-io already has
serde for the Stroke type.

## 6. Plumbing for the Detail 0-vs-max guard (Track C)

- `params_hash`: `Stage::params_hash(&self) -> u64` (`pipeline.rs:109`), pub
  via `worldmaker_sim::Stage`. `TectonicsStage` impl at
  `tectonics/mod.rs:136-157` hashes all 7 param fields. Pipeline cache key =
  FNV chain of stage id + master_seed + grid.level + params_hash
  (`pipeline.rs:176-181`).
- Committed-field hashes: `hash_f32_slice(ELEVATION_M)`,
  `hash_u32_slice(PLATE_ID)`, `hash_u32_slice(CRUST_TYPE)` — all names pub in
  `worldmaker_sim::tectonics`.
- "Same path Regenerate uses": `WorldApp::start_job` (`app.rs:312-342`) =
  `WorldState::new(grid)` + `Pipeline::new()` + `push(TectonicsStage::new(
  self.current_params()))` + `StageContext::new(self.master_seed)` → `run`.
  **Complication:** `current_params` (`app.rs:262-272`) is a `&self` method on
  `WorldApp`, which cannot be constructed headlessly (needs an eframe/wgpu
  `CreationContext`). The app's UI defaults currently coincide exactly with
  `TectonicsParams::default()`, but nothing enforces that. For the guard test
  to be honest, C should factor the world-building recipe into a free function
  shared by `start_job` and the test (a `#[cfg(test)]` module inside the
  binary, like layers.rs/render.rs tests). Otherwise "same path" is
  by-convention only.
- "sim exposes no render-detail parameter": `TectonicsParams` has exactly 7
  pub fields (`tectonics/mod.rs:74-90`); nothing render-related. Also note
  `SimState.noise_seed` (`step.rs:191`, "Deterministic seed for the elevation
  detail noise") feeds the **sim-side** ±300 m fBm — part of goldens, not to
  be confused with C's renderer detail seed.
- For Fix 1's structural guard: the stroke→sim route exists **today** at
  `app.rs:551-558` (craton stroke end → `self.start_job()`, comment "Stroke
  finished: re-run history from t=0, same seed"). That call is what Track A
  removes; an app unit test can then assert the new pending-edits module has
  no `Pipeline`/`start_job` reachability (by construction/API, since Rust has
  no call-graph reflection — design the module so it simply cannot name them).

## 7. Existing tests that constrain Fix 2

- `TectonicsParams::clamped` (`tectonics/mod.rs:107-114`): plate_count clamped
  **8..=24** at setup (the 6..=24 band is for alive plates later; plates may
  die but not below 6 without failing gates).
- `short_run_produces_a_sane_world` (`tectonics_tests.rs:73-141`, L5 seed 42,
  200 My): final land fraction within ±0.05 (abs) of 0.29; anchor land within
  ±0.005; exactly 21 keyframes (200 My @ 10 My + t=0); `hotspots.len() == 6`;
  alive plates 6..=24 at the final keyframe; ridges > 20 and trenches > 5
  cells; >100 young and >100 old ocean cells. **The new generator must pass
  all of this at L5**, not just L6/L7.
- `craton_overlay_changes_world_deterministically`
  (`tectonics_tests.rs:371-396`): asserts keyframe-0 `plate_id` is **identical
  with and without a craton overlay** — plate setup must stay independent of
  the craton overlay. A candidate that seeds plates from continents breaks
  this test (and Fix 1's semantics).
- `same_seed_reproduces_identical_hashes` (L5) and
  `resume_from_keyframe_is_bit_exact` (L5): bit-exact rerun + resume must
  survive the new setup.
- Harness `stability()` (`harness.rs:235-318`, L6 seed 42, 2 Gy): min/max
  alive plates ∈ 6..=24 across every keyframe; anchor land fraction within
  ±5% of a **hardcoded 0.29 target** (`harness.rs:311` — not
  `params.land_fraction`; fine while the harness uses defaults, a trap if B
  parameterizes it); continental-inventory drift ≤5%.
- Setup-only gate test: `SimState` is pub with pub fields (`step.rs:169-217`,
  re-exported `tectonics::SimState`); `SimState::setup(master_seed, &grid,
  &params)` (`step.rs:260`) is callable from a worldmaker-sim integration test
  and exposes `plate_id`, `plates`, `crust_type` directly — the L7+2×L6 CV/
  sinuosity gate needs no pipeline run. But note: after `setup` the elevation
  field is not derived (`elevation::derive_and_solve` runs in `run_history`),
  so the gate should read plate geometry only.
- Keyframe cadence: `keyframe_interval_my(grid_level)` (`tectonics/mod.rs:63-69`)
  returns 20.0 for level ≥ 8, 10.0 below — L9 currently inherits the L8
  cadence (≈4.2 GB at 2 Gy); B's cadence decision lands here and in its doc
  comment.

## 8. Phase-1 baseline numbers (for later comparison)

`tectonics-phase1` (identical metrics both machines except wall times):
age_depth_max_err_pct 2.4; arc 100.0% (2878 cells); Ashman D 8.52; modes
−5501 / +301 m; hotspot 558 flagged / 14 emergent; stability plates 6–12,
land anchor 0.2905, land range 0.2905–0.342, cont 0.391→0.387 (drift 0.012),
sea offset 165.0 m, sutures 23, breakups 17; keyframe bytes 500 My/1 Gy/2 Gy
L7 = 133 753 212 / 264 897 492 / 527 187 816; hashes elev
`0xf7510e7214ed5b62`, plates `0x70df6db8ec5f653d`, crust
`0xd5c797a8cc26afb5`. Wall times PC vs Air (s): 500 My L7 2.06/1.80, 1 Gy L7
4.03/3.44, 2 Gy L7 7.57/6.20, 2 Gy L6 2.20/1.69.
`perf-phase1` PC: globe/flat/split fps 1656.9/1434.9/1446.6; Air:
214.9/55.6/91.1; grid_build_ms L6–L9 PC 6.2/17.8/72.8/348.6, Air
11.2/28.2/87.8/384.4. `perf-phase0` PC fps 2130.7/2224.7/2041.8 (pre-
tectonics; includes `render_mesh_decimated: false`, a key the phase-1 writer
dropped). `determinism-phase0` (both machines identical):
`elevation_hash_L6_seed42 0xa86a747179a35a46`,
`elevation_hash_L7_seed42 0x3096ffe0115a671d`.

## Contradictions / risks flagged

1. **Fix 2 vs plate/continent independence**: `tectonics_tests.rs:391-395`
   pins plate layout independent of craton overlay — constrains candidate
   generators that couple plates to continents.
2. **Setup clamp 8..=24 vs contract band 6–24**: the 6 floor is a runtime
   survival band, not a setup parameter; keep the distinction when writing
   metrics gates.
3. **L5 keyframe-count and hotspot-count assertions** (`== 21`, `== 6`) will
   break if B touches sub-L8 cadence or hotspot defaults.
4. **Harness stability target hardcoded 0.29** (`harness.rs:311`), not
   `params.land_fraction`.
5. **Detail-guard "same path" is not currently testable structurally** —
   `current_params`/`start_job` are only reachable through a constructed
   `WorldApp`; needs a shared free function (C's WorldBundle plumbing remit).
6. **Golden regen is manual**; both tectonic constants and the harness's
   determinism hex metrics change together — regenerate once on B's branch,
   verify feelpass JSON hashes equal the new constants, decision-log entry.
   Phase-0 noise golden and determinism-phase0 files must not move.
7. **New flags**: unknown CLI args are warn-and-ignore today, so `--seed 7`
   passed to an old binary silently produces seed-"cyrus" output — scripts
   should verify flag support after C lands.
8. **worldmaker-sim needs `image` as dev-dependency** for the panel test;
   clippy `--all-targets` will compile it in CI.
9. **BEFORE screenshots already committed** (8381450) with default seed
   "cyrus" (`0xc4be0bf8f497a575`), Standard L7, 500 My — AFTER set must match.
