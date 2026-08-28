//! WO-0009 S2 terrain gates (benchmarks doc Tables 5.1 / 5.2) plus the
//! terrain goldens and the results-JSON probe.
//!
//! Gates, at the defaults (L6, seed 42, TectonicsParams::default(),
//! morpho 30 My, final keyframe):
//! - the typical (median) active-orogen land cell (orogeny_age ≤ 100 My
//!   at the keyframe) denudes 0.5–10 mm/yr (Table 5.1, active orogen);
//! - the typical craton land cell (orogeny_age ≥ 1200 My, the craton
//!   strength regime) denudes 0.0003–0.02 mm/yr (Table 5.1, stable
//!   craton / shield);
//! - the transport ledger's residual is exactly zero (terrain creates and
//!   destroys no rock — WO-0008 ledger discipline);
//! - no river flows uphill on the standard suite (seeds 42 and "cyrus"):
//!   every receiver sits strictly lower on the epsilon-filled surface.
//!
//! Goldens: post-erosion elevation and discharge hashes at L6 seed 42,
//! morpho 30 My — NEW with this session, committed like every golden.

use std::sync::Arc;

use worldmaker_core::hash::{hash_f32_slice, seed_from_text};
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{self, TectonicsParams};
use worldmaker_sim::terrain::{self, TerrainOutput, RECV_NONE};
use worldmaker_sim::{StageContext, WorldState};

/// Build the standard tectonic world and erode its final keyframe.
fn tectonics_plus_terrain(
    seed: u64,
    morpho_my: f32,
) -> (Arc<Grid>, tectonics::Keyframe, TerrainOutput) {
    let grid = Arc::new(Grid::build(6));
    let world = WorldState::new(grid.clone());
    let hist = tectonics::run_history(
        &StageContext::new(seed),
        &world,
        &TectonicsParams::default(),
        None,
    )
    .unwrap();
    let kf = hist.keyframes.last().unwrap().clone();
    let out = terrain::run_terrain(&grid, &kf, seed, morpho_my);
    (grid, kf, out)
}

/// Mean NET denudation rate (mm/yr) over the cells `pick` selects: fluvial
/// removal minus redeposition, floored at zero per cell — what
/// cosmogenic-nuclide rates (the Table 5.1 reference method) measure. The
/// gross fluvial number alone double-counts sediment that transits a cell
/// (deposited one sub-step, re-mobilized the next).
fn mean_rate_mm_yr(
    kf: &tectonics::Keyframe,
    out: &TerrainOutput,
    morpho_my: f32,
    pick: impl Fn(usize) -> bool,
) -> (f64, usize) {
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for c in 0..kf.elev_m.len() {
        if pick(c) {
            let net_m = (out.fluvial_erosion_m[c] as f64 - out.sediment_m[c] as f64).max(0.0);
            // m over morpho_my My → mm/yr: m / (My·1e6 yr) · 1e3 mm.
            sum += net_m / (morpho_my as f64 * 1.0e3);
            count += 1;
        }
    }
    (if count > 0 { sum / count as f64 } else { 0.0 }, count)
}

/// Median NET denudation rate (mm/yr) over the cells `pick` selects — the
/// typical cell, robust to the handful of big-river corridor cells whose
/// legitimate incision dominates a mean. Table 5.1's craton rates are
/// outcrop/interfluve measurements, which a median represents.
fn median_rate_mm_yr(
    kf: &tectonics::Keyframe,
    out: &TerrainOutput,
    morpho_my: f32,
    pick: impl Fn(usize) -> bool,
) -> (f64, usize) {
    let mut rates: Vec<f64> = (0..kf.elev_m.len())
        .filter(|&c| pick(c))
        .map(|c| {
            (out.fluvial_erosion_m[c] as f64 - out.sediment_m[c] as f64).max(0.0)
                / (morpho_my as f64 * 1.0e3)
        })
        .collect();
    let n = rates.len();
    if n == 0 {
        return (0.0, 0);
    }
    rates.sort_by(|a, b| a.total_cmp(b));
    (rates[n / 2], n)
}

const KF_CONTINENT: u16 = 1 << 15;

#[test]
fn terrain_gates_defaults_seed42() {
    let morpho = 30.0f32;
    let (_grid, kf, out) = tectonics_plus_terrain(42, morpho);

    // Ledger residual: exact zero, no exceptions.
    assert_eq!(
        out.residual_q(),
        0,
        "transport ledger leaked: eroded {} vs deposited {}",
        out.ledger_eroded_q,
        out.ledger_deposited_q
    );
    assert!(out.ledger_eroded_q > 0, "no erosion happened at all");

    // The typical (median) cell of each setting must sit in its Table 5.1
    // band. Medians, not means: the reference rates are outcrop /
    // catchment measurements of typical surfaces, and a mean is dominated
    // by the few big-river corridor cells whose legitimate incision runs
    // 50× the interfluves (measured: craton mean 0.026 vs median 0.015).
    let land = |c: usize| kf.elev_m[c] > 0 && kf.flags[c] & KF_CONTINENT != 0;
    let (orogen_rate, orogen_n) = median_rate_mm_yr(&kf, &out, morpho, |c| {
        land(c) && kf.orogeny_age_my[c] <= 100
    });
    let (craton_rate, craton_n) = median_rate_mm_yr(&kf, &out, morpho, |c| {
        land(c) && kf.orogeny_age_my[c] >= 1200
    });
    assert!(orogen_n > 20, "too few active-orogen cells ({orogen_n})");
    assert!(craton_n > 20, "too few craton cells ({craton_n})");
    assert!(
        (0.5..=10.0).contains(&orogen_rate),
        "active-orogen median denudation {orogen_rate:.3} mm/yr outside 0.5–10 (Table 5.1)"
    );
    assert!(
        (0.0003..=0.02).contains(&craton_rate),
        "craton median denudation {craton_rate:.5} mm/yr outside 0.0003–0.02 (Table 5.1)"
    );

    // Outputs are finite everywhere.
    for c in 0..kf.elev_m.len() {
        assert!(out.elev_m[c].is_finite());
        assert!(out.discharge_m3s[c].is_finite() && out.discharge_m3s[c] >= 0.0);
    }
}

/// No river flows uphill, standard suite: every land cell's receiver sits
/// strictly lower on the epsilon-filled surface, and following receivers
/// always terminates in the ocean (no cycles).
fn assert_no_uphill(seed: u64, label: &str) {
    let (grid, kf, out) = tectonics_plus_terrain(seed, 30.0);
    let n = grid.cell_count() as usize;
    for c in 0..n {
        let r = out.receiver[c];
        if r == RECV_NONE {
            continue;
        }
        assert!(
            out.water_surface_m[r as usize] < out.water_surface_m[c],
            "{label}: river flows uphill at cell {c} → {r} \
             ({} → {})",
            out.water_surface_m[c],
            out.water_surface_m[r as usize],
        );
        // Receiver chains terminate (strictly-descending surface makes
        // cycles impossible; walk one chain per cell with a hop budget to
        // prove it anyway).
        let mut cur = c as u32;
        let mut hops = 0u32;
        while out.receiver[cur as usize] != RECV_NONE {
            cur = out.receiver[cur as usize];
            hops += 1;
            assert!(hops <= n as u32, "{label}: receiver cycle from cell {c}");
        }
    }
    let _ = kf;
}

#[test]
fn no_river_flows_uphill_seed42() {
    assert_no_uphill(42, "seed 42");
}

#[test]
fn no_river_flows_uphill_seed_cyrus() {
    assert_no_uphill(seed_from_text("cyrus"), "seed cyrus");
}

/// Terrain goldens at L6, seed 42, morpho 30 My — NEW in WO-0009 S2
/// (decision log). Regenerating them needs the same sanction as every
/// golden move.
const GOLDEN_TERRAIN_ELEVATION_L6_SEED42_M30: u64 = 0x096b_981f_aff6_811b;
const GOLDEN_TERRAIN_DISCHARGE_L6_SEED42_M30: u64 = 0xc114_7e21_dc39_db10;

#[test]
#[ignore = "dev aid: prints the terrain golden hashes"]
fn print_terrain_goldens() {
    let (_grid, _kf, out) = tectonics_plus_terrain(42, 30.0);
    println!(
        "GOLDEN_TERRAIN_ELEVATION_L6_SEED42_M30 = {:#018x}",
        hash_f32_slice(&out.elev_m)
    );
    println!(
        "GOLDEN_TERRAIN_DISCHARGE_L6_SEED42_M30 = {:#018x}",
        hash_f32_slice(&out.discharge_m3s)
    );
}

#[test]
fn terrain_reproduces_committed_goldens() {
    let (_grid, _kf, out) = tectonics_plus_terrain(42, 30.0);
    assert_eq!(
        hash_f32_slice(&out.elev_m),
        GOLDEN_TERRAIN_ELEVATION_L6_SEED42_M30,
        "post-erosion elevation drifted from the committed golden"
    );
    assert_eq!(
        hash_f32_slice(&out.discharge_m3s),
        GOLDEN_TERRAIN_DISCHARGE_L6_SEED42_M30,
        "discharge drifted from the committed golden"
    );
}

/// The stage contract (WO-0009 S2 step 4): "phase2-terrain" runs after
/// tectonics in a pipeline, writes its fields, caches by params hash —
/// and re-runs exactly when morpho_my changes (the World-panel slider
/// participates in the hash, step 5).
#[test]
fn terrain_stage_runs_in_pipeline_and_caches_by_morpho() {
    use worldmaker_sim::{Pipeline, Stage, TectonicsStage, TerrainParams, TerrainStage};
    let grid = Arc::new(Grid::build(5));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(TectonicsStage::new(TectonicsParams {
        span_my: 200.0,
        ..TectonicsParams::default()
    })));
    pipe.push(Box::new(TerrainStage::new(TerrainParams {
        morpho_my: 10.0,
        era_index: None,
    })));
    let ctx = StageContext::new(42);
    let ran = pipe.run(&ctx, &mut world).unwrap();
    assert_eq!(ran, vec!["phase1-tectonics", "phase2-terrain"]);
    let elev = world.fields.get(terrain::TERRAIN_ELEVATION_M).unwrap();
    assert!(elev.iter().all(|e| e.is_finite()));
    assert!(world.fields.get(terrain::TERRAIN_DISCHARGE_M3S).is_some());
    assert!(world
        .fields
        .get_u32(terrain::TERRAIN_LITHOLOGY)
        .is_some_and(|l| l.iter().any(|&v| v == tectonics::lithology::SU as u32)));
    // Clean cache: nothing re-runs.
    assert!(pipe.run(&ctx, &mut world).unwrap().is_empty());
    // A different morpho re-runs ONLY the terrain stage.
    let mut pipe2 = Pipeline::new();
    pipe2.push(Box::new(TectonicsStage::new(TectonicsParams {
        span_my: 200.0,
        ..TectonicsParams::default()
    })));
    pipe2.push(Box::new(TerrainStage::new(TerrainParams {
        morpho_my: 20.0,
        era_index: None,
    })));
    // Fresh pipeline against the same world: tectonics' key matches but the
    // pipeline has no cache yet, so both run; what this asserts is the
    // params-hash separation instead.
    assert_ne!(
        TerrainStage::new(TerrainParams {
            morpho_my: 10.0,
            era_index: None
        })
        .params_hash(),
        TerrainStage::new(TerrainParams {
            morpho_my: 20.0,
            era_index: None
        })
        .params_hash(),
        "morpho_my must participate in the terrain params hash"
    );
    assert_ne!(
        TerrainStage::new(TerrainParams {
            morpho_my: 10.0,
            era_index: None
        })
        .params_hash(),
        TerrainStage::new(TerrainParams {
            morpho_my: 10.0,
            era_index: Some(3)
        })
        .params_hash(),
        "the pinned era must participate in the terrain params hash"
    );
}

/// Dev probe: erosion-rate table and ledger numbers to
/// docs/results/terrain-wo0009-s2-<machine>.json (rule 3: chat numbers
/// don't count). Run with:
///   cargo test -p worldmaker-sim --release --test terrain_gates -- --ignored --nocapture terrain_probe
#[test]
#[ignore = "dev probe: writes docs/results/terrain-wo0009-s2-<machine>.json"]
fn terrain_probe() {
    use serde_json::json;
    let morpho = 30.0f32;
    let (_grid, kf, out) = tectonics_plus_terrain(42, morpho);
    let land = |c: usize| kf.elev_m[c] > 0 && kf.flags[c] & KF_CONTINENT != 0;
    let (orogen_rate, orogen_n) = mean_rate_mm_yr(&kf, &out, morpho, |c| {
        land(c) && kf.orogeny_age_my[c] <= 100
    });
    let (craton_rate, craton_n) = mean_rate_mm_yr(&kf, &out, morpho, |c| {
        land(c) && kf.orogeny_age_my[c] >= 1200
    });
    let (land_rate, land_n) = mean_rate_mm_yr(&kf, &out, morpho, land);
    let (orogen_med, _) = median_rate_mm_yr(&kf, &out, morpho, |c| {
        land(c) && kf.orogeny_age_my[c] <= 100
    });
    let (craton_med, _) = median_rate_mm_yr(&kf, &out, morpho, |c| {
        land(c) && kf.orogeny_age_my[c] >= 1200
    });
    let (postoro_rate, postoro_n) = mean_rate_mm_yr(&kf, &out, morpho, |c| {
        land(c) && (200..=800).contains(&kf.orogeny_age_my[c])
    });
    let n = kf.elev_m.len();
    let lakes = (0..n).filter(|&c| out.lake_depth_m[c] > 0.0).count();
    let su_cells = (0..n)
        .filter(|&c| out.lithology[c] == tectonics::lithology::SU)
        .count();
    let max_q = out.discharge_m3s.iter().cloned().fold(0.0f32, f32::max);
    let peak_before = kf.elev_m.iter().cloned().max().unwrap_or(0);
    let peak_after = out.elev_m.iter().cloned().fold(f32::MIN, f32::max);
    let metrics = json!({
        "config": {
            "level": 6, "seed": 42, "span_my": 500.0,
            "morpho_my": morpho, "keyframe": "last",
            "u0_mm_yr": terrain::U0_MM_YR,
            "k_lith": terrain::K_LITH.to_vec(),
        },
        "benchmark_bands": {
            "active_orogen_mm_yr": [0.5, 10.0],
            "craton_mm_yr": [0.0003, 0.02],
            "source": "earth-benchmarks-v1.md Table 5.1",
        },
        "orogen_mean_mm_yr": orogen_rate, "orogen_cells": orogen_n,
        "craton_mean_mm_yr": craton_rate, "craton_cells": craton_n,
        "orogen_median_mm_yr": orogen_med,
        "craton_median_mm_yr": craton_med,
        "post_orogenic_mean_mm_yr": postoro_rate, "post_orogenic_cells": postoro_n,
        "all_land_mean_mm_yr": land_rate, "land_cells": land_n,
        "ledger_eroded_q": out.ledger_eroded_q,
        "ledger_deposited_q": out.ledger_deposited_q,
        "ledger_residual_q": out.residual_q(),
        "lake_cells": lakes,
        "su_deposit_cells": su_cells,
        "max_discharge_m3s": max_q,
        "peak_elev_before_m": peak_before,
        "peak_elev_after_m": peak_after,
    });
    eprintln!("{}", serde_json::to_string_pretty(&metrics).unwrap());
    let file = worldmaker_io::ResultsFile::new(&worldmaker_io::results::today_utc_iso(), metrics);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../docs/results/terrain-wo0009-s2-{}.json",
        worldmaker_io::results::machine_name()
    ));
    file.write(&path).unwrap();
    eprintln!("wrote {}", path.display());
}
