//! Headless acceptance harness for WO-0002 (`--tectonics-results <file>`).
//!
//! Runs the tectonic simulation at the acceptance configurations, computes
//! every WO-0002 metric, and writes one machine-labelled results JSON. Chat
//! numbers don't count; this file does.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use worldmaker_core::hash::{hash_f32_slice, hash_u32_slice};
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{
    keyframe_interval_my, metrics, Keyframe, SimState, TectonicsHistory, TectonicsParams,
    TectonicsStage, CRUST_TYPE, ELEVATION_M, F_ARC, F_HOTSPOT, F_TRENCH, PLATE_ID,
};
use worldmaker_sim::{Pipeline, StageContext, WorldState};

const SEED: u64 = 42;
/// Mean cell spacing in km for a grid level (unit sphere × Earth radius).
fn cell_spacing_km(n_cells: usize) -> f64 {
    (4.0 * std::f64::consts::PI / n_cells as f64).sqrt() * 6371.0
}

fn run(level: u32, span_my: f32) -> anyhow::Result<(WorldState, f64)> {
    run_with(
        level,
        SEED,
        TectonicsParams {
            span_my,
            ..TectonicsParams::default()
        },
    )
}

fn run_with(level: u32, seed: u64, params: TectonicsParams) -> anyhow::Result<(WorldState, f64)> {
    let grid = Arc::new(Grid::build(level));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(TectonicsStage::new(params)));
    let t0 = Instant::now();
    pipe.run(&StageContext::new(seed), &mut world)?;
    Ok((world, t0.elapsed().as_secs_f64()))
}

const KF_CONTINENT: u16 = 1 << 15;

/// Age-depth: mean physical ocean depth per 10 My age bin (0–80 My),
/// excluding trench/arc/hotspot cells, vs 2600 + 365·√age. The stored
/// elevation is relative to the solved sea level, so the per-keyframe
/// sea offset is added back before comparing (WO-0002 wording).
fn age_depth(kf: &Keyframe, metrics: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    let excluded = (F_TRENCH | F_ARC | F_HOTSPOT) as u16;
    let mut sums = [0.0f64; 8];
    let mut ages = [0.0f64; 8];
    let mut counts = [0u64; 8];
    for c in 0..kf.elev_m.len() {
        if kf.flags[c] & KF_CONTINENT != 0 || kf.flags[c] & excluded != 0 || kf.buildup_ckm[c] > 10
        {
            continue;
        }
        let age = kf.crust_age_my[c] as f64;
        if age >= 80.0 {
            continue;
        }
        let bin = (age / 10.0) as usize;
        let raw_elev = kf.elev_m[c] as f64 + kf.sea_offset_m as f64;
        sums[bin] += -raw_elev;
        ages[bin] += age;
        counts[bin] += 1;
    }
    let mut max_err_pct = 0.0f64;
    let mut bins = Vec::new();
    for b in 0..8 {
        if counts[b] < 20 {
            bins.push(serde_json::json!(null));
            continue;
        }
        let mean_depth = sums[b] / counts[b] as f64;
        let mean_age = ages[b] / counts[b] as f64;
        let curve = 2600.0 + 365.0 * mean_age.sqrt();
        let err = (mean_depth - curve).abs() / curve * 100.0;
        max_err_pct = max_err_pct.max(err);
        bins.push(serde_json::json!({
            "age_bin_my": format!("{}-{}", b * 10, b * 10 + 10),
            "mean_depth_m": (mean_depth * 10.0).round() / 10.0,
            "cooling_curve_m": (curve * 10.0).round() / 10.0,
            "err_pct": (err * 100.0).round() / 100.0,
            "cell_count": counts[b],
        }));
    }
    metrics.insert("age_depth_bins".into(), serde_json::Value::Array(bins));
    metrics.insert(
        "age_depth_max_err_pct".into(),
        serde_json::json!((max_err_pct * 100.0).round() / 100.0),
    );
    let pass = max_err_pct <= 10.0;
    metrics.insert("age_depth_within_10pct".into(), serde_json::json!(pass));
    pass
}

/// Hypsometry: 2-means split of the elevation distribution plus Ashman's D
/// bimodality coefficient (D > 2 = clearly bimodal). Deterministic fixed
/// initialization and iteration count.
fn hypsometry(kf: &Keyframe, metrics: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    let elev: Vec<f64> = kf.elev_m.iter().map(|&e| e as f64).collect();
    let (mut c_low, mut c_high) = (-4000.0f64, 400.0f64);
    for _ in 0..40 {
        let (mut s0, mut n0, mut s1, mut n1) = (0.0f64, 0u64, 0.0f64, 0u64);
        for &e in &elev {
            if (e - c_low).abs() <= (e - c_high).abs() {
                s0 += e;
                n0 += 1;
            } else {
                s1 += e;
                n1 += 1;
            }
        }
        if n0 > 0 {
            c_low = s0 / n0 as f64;
        }
        if n1 > 0 {
            c_high = s1 / n1 as f64;
        }
    }
    let (mut v0, mut n0, mut v1, mut n1) = (0.0f64, 0u64, 0.0f64, 0u64);
    for &e in &elev {
        if (e - c_low).abs() <= (e - c_high).abs() {
            v0 += (e - c_low) * (e - c_low);
            n0 += 1;
        } else {
            v1 += (e - c_high) * (e - c_high);
            n1 += 1;
        }
    }
    let sd0 = (v0 / n0.max(1) as f64).sqrt();
    let sd1 = (v1 / n1.max(1) as f64).sqrt();
    let ashman_d = (2.0f64).sqrt() * (c_high - c_low).abs() / (sd0 * sd0 + sd1 * sd1).sqrt();
    metrics.insert(
        "hypsometry_ocean_mode_m".into(),
        serde_json::json!(c_low.round()),
    );
    metrics.insert(
        "hypsometry_land_mode_m".into(),
        serde_json::json!(c_high.round()),
    );
    metrics.insert(
        "hypsometry_ashman_d".into(),
        serde_json::json!((ashman_d * 100.0).round() / 100.0),
    );
    // Bimodal with one mode in the deep ocean and one near sea level.
    let pass = ashman_d > 2.0 && c_low < -2500.0 && c_high.abs() < 1500.0;
    metrics.insert("hypsometry_bimodal".into(), serde_json::json!(pass));
    pass
}

/// Arc placement: ≥95% of arc cells must have a trench within 400 km on
/// their own (overriding) plate side — i.e. the arc's trench, across the
/// boundary. Local BFS per arc cell: the globally nearest trench can belong
/// to a different subduction zone without invalidating the arc.
fn arc_trench(
    kf: &Keyframe,
    grid: &Grid,
    metrics: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    let n = kf.elev_m.len();
    let spacing = cell_spacing_km(n);
    let max_rings = (400.0 / spacing).floor() as u32;
    let (mut arcs, mut good) = (0u64, 0u64);
    let mut dist = vec![u32::MAX; n];
    let mut touched: Vec<u32> = Vec::new();
    for c in 0..n {
        if kf.flags[c] as u32 & F_ARC == 0 {
            continue;
        }
        arcs += 1;
        // BFS out to max_rings looking for a same-plate trench cell.
        let mut queue: VecDeque<u32> = VecDeque::new();
        dist[c] = 0;
        touched.push(c as u32);
        queue.push_back(c as u32);
        let mut found = false;
        'search: while let Some(q) = queue.pop_front() {
            let d = dist[q as usize];
            if kf.flags[q as usize] as u32 & F_TRENCH != 0
                && kf.plate_id[q as usize] == kf.plate_id[c]
            {
                found = true;
                break 'search;
            }
            if d >= max_rings {
                continue;
            }
            for &nb in grid.neighbors_of(q) {
                let nbu = nb as usize;
                if dist[nbu] == u32::MAX {
                    dist[nbu] = d + 1;
                    touched.push(nb);
                    queue.push_back(nb);
                }
            }
        }
        if found {
            good += 1;
        }
        for &t in &touched {
            dist[t as usize] = u32::MAX;
        }
        touched.clear();
    }
    let pct = if arcs > 0 {
        good as f64 / arcs as f64 * 100.0
    } else {
        0.0
    };
    metrics.insert("arc_cell_count".into(), serde_json::json!(arcs));
    metrics.insert(
        "arc_trench_within_400km_pct".into(),
        serde_json::json!((pct * 10.0).round() / 10.0),
    );
    let pass = arcs > 0 && pct >= 95.0;
    metrics.insert("arc_trench_pass".into(), serde_json::json!(pass));
    pass
}

fn hotspots(kf: &Keyframe, metrics: &mut serde_json::Map<String, serde_json::Value>) {
    let (mut flagged, mut emergent) = (0u64, 0u64);
    for c in 0..kf.elev_m.len() {
        if kf.flags[c] as u32 & F_HOTSPOT != 0 {
            flagged += 1;
            if kf.elev_m[c] > 0 && kf.flags[c] & KF_CONTINENT == 0 {
                emergent += 1;
            }
        }
    }
    metrics.insert("hotspot_flagged_cells".into(), serde_json::json!(flagged));
    metrics.insert(
        "hotspot_emergent_island_cells".into(),
        serde_json::json!(emergent),
    );
}

fn stability(
    history: &TectonicsHistory,
    metrics: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    let n = history.keyframes[0].elev_m.len() as f64;
    let (mut min_plates, mut max_plates) = (usize::MAX, 0usize);
    let (mut min_land, mut max_land) = (1.0f64, 0.0f64);
    for kf in &history.keyframes {
        let alive = kf.plates.iter().filter(|p| p.alive).count();
        min_plates = min_plates.min(alive);
        max_plates = max_plates.max(alive);
        let land = kf.elev_m.iter().filter(|&&e| e >= 0).count() as f64 / n;
        min_land = min_land.min(land);
        max_land = max_land.max(land);
    }
    let first = &history.keyframes[0];
    let last = history.keyframes.last().unwrap();
    let cont =
        |kf: &Keyframe| kf.flags.iter().filter(|&&f| f & KF_CONTINENT != 0).count() as f64 / n;
    metrics.insert(
        "stability_plate_count_min".into(),
        serde_json::json!(min_plates),
    );
    metrics.insert(
        "stability_plate_count_max".into(),
        serde_json::json!(max_plates),
    );
    metrics.insert(
        "stability_land_fraction_min".into(),
        serde_json::json!((min_land * 10_000.0).round() / 10_000.0),
    );
    metrics.insert(
        "stability_land_fraction_max".into(),
        serde_json::json!((max_land * 10_000.0).round() / 10_000.0),
    );
    metrics.insert(
        "stability_cont_fraction_start".into(),
        serde_json::json!((cont(first) * 1000.0).round() / 1000.0),
    );
    metrics.insert(
        "stability_cont_fraction_end".into(),
        serde_json::json!((cont(last) * 1000.0).round() / 1000.0),
    );
    metrics.insert(
        "stability_sea_offset_start_m".into(),
        serde_json::json!(first.sea_offset_m.round()),
    );
    metrics.insert(
        "stability_sea_offset_end_m".into(),
        serde_json::json!(last.sea_offset_m.round()),
    );
    metrics.insert(
        "stability_suture_count".into(),
        serde_json::json!(history.diagnostics.suture_count),
    );
    metrics.insert(
        "stability_breakup_count".into(),
        serde_json::json!(history.diagnostics.breakup_count),
    );
    // Sea level is solved once at t = 0 and then drifts (decision log, Dan),
    // so the land-fraction gate applies to the anchor keyframe — exact by
    // construction — while the over-run range is recorded as data (the drift
    // IS the feature). The quantity the solver cannot mask stays gated:
    // continental-crust inventory drift over the whole run (review finding).
    let land_first = first.elev_m.iter().filter(|&&e| e >= 0).count() as f64 / n;
    metrics.insert(
        "stability_land_fraction_anchor".into(),
        serde_json::json!((land_first * 10_000.0).round() / 10_000.0),
    );
    let cont_start = cont(first);
    let cont_end = cont(last);
    let cont_drift = (cont_end - cont_start).abs() / cont_start.max(1e-9);
    metrics.insert(
        "stability_cont_drift_rel".into(),
        serde_json::json!((cont_drift * 1000.0).round() / 1000.0),
    );
    let target = 0.29;
    let pass = (6..=24).contains(&min_plates)
        && (6..=24).contains(&max_plates)
        && (land_first - target).abs() / target <= 0.05
        && cont_drift <= 0.05;
    metrics.insert("stability_pass".into(), serde_json::json!(pass));
    pass
}

/// WO-0003 Fix 4 liveliness gates (7.1 ownership-overlap, 7.2 speed-floor)
/// over a run's history. Implementation and constants are canonical in
/// `tectonics::metrics`; the CI gate tests (liveliness_tests.rs) enforce
/// the same functions. Violation lines go into the JSON verbatim so a
/// failure is diagnosable from the results file alone.
fn liveliness_rows(
    history: &TectonicsHistory,
    prefix: &str,
    m: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    let rep = metrics::liveliness(history);
    m.insert(
        format!("liveliness_{prefix}overlap_violations"),
        serde_json::json!(rep.overlap_violations),
    );
    m.insert(
        format!("liveliness_{prefix}speed_violations"),
        serde_json::json!(rep.speed_violations),
    );
    let pass = rep.pass();
    m.insert(
        format!("liveliness_{prefix}pass"),
        serde_json::json!(pass),
    );
    pass
}

/// The 5 fixed WO-0003 Fix 2 competition seeds (pinned in Stage D).
/// 0xc4be0bf8f497a575 = seed_from_text("cyrus"), the app-default seed of the
/// committed BEFORE screenshots.
const PLATEGEN_SEEDS: [(u64, &str); 5] = [
    (42, "42"),
    (0xc4be_0bf8_f497_a575, "cyrus"),
    (7, "7"),
    (1002, "1002"),
    (271828, "271828"),
];

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// WO-0003 Fix 2: t=0 plate-map metrics (d2-fix2-design §8). Setup-only —
/// `SimState::setup` + `tectonics::metrics`, no pipeline run — on the 5
/// fixed competition seeds at L6 and L7. Deliberately does NOT call
/// `stability()` or read `params.land_fraction`; the 0.29 stability target
/// stays hardcoded there by design.
///
/// Since commit M3 (winner wired) this also evaluates the FINAL feel gates
/// (`metrics::GATE_CV` / `metrics::GATE_SINUOSITY`) on the pinned gate
/// triple — L7 seed 42, L6 seed 7, L6 seed cyrus — and the returned pass
/// feeds `all_acceptance_pass`. The off-triple pairs are records, not gates.
fn plategen_metrics(m: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    let params = TectonicsParams::default();
    m.insert("plategen_generator".into(), serde_json::json!("hybrid"));
    m.insert(
        "plategen_seed_cyrus".into(),
        serde_json::json!("0xc4be0bf8f497a575"),
    );
    m.insert(
        "plategen_gate_cv".into(),
        serde_json::json!(metrics::GATE_CV),
    );
    m.insert(
        "plategen_gate_sinuosity".into(),
        serde_json::json!(metrics::GATE_SINUOSITY),
    );
    let mut gates_pass = true;
    for level in [6u32, 7] {
        let grid = Arc::new(Grid::build(level));
        for (seed, label) in PLATEGEN_SEEDS {
            let s = SimState::setup(seed, &grid, &params);
            let cv = metrics::plate_area_cv(&s.plate_id, params.plate_count);
            let rep = metrics::boundary_sinuosity(&grid, &s.plate_id);
            m.insert(
                format!("plategen_area_cv_l{level}_seed{label}"),
                serde_json::json!(round4(cv)),
            );
            m.insert(
                format!("plategen_sinuosity_l{level}_seed{label}"),
                serde_json::json!(round4(rep.weighted_mean)),
            );
            m.insert(
                format!("plategen_open_segments_l{level}_seed{label}"),
                serde_json::json!(rep.open_segment_count),
            );
            m.insert(
                format!("plategen_loops_l{level}_seed{label}"),
                serde_json::json!(rep.loop_count),
            );
            let on_gate_triple = (level == 7 && seed == 42)
                || (level == 6 && (seed == 7 || seed == 0xc4be_0bf8_f497_a575));
            if on_gate_triple {
                gates_pass &=
                    cv >= metrics::GATE_CV && rep.weighted_mean >= metrics::GATE_SINUOSITY;
            }
        }
    }
    m.insert("plategen_gates_pass".into(), serde_json::json!(gates_pass));
    gates_pass
}

/// Optional XL rows (WO-0003: "1 Gy wall time, measured keyframe bytes" at
/// L8/L9), behind env `WM_HARNESS_XL=1` so the default harness stays fast.
/// Records, not gates (Air interim; CLAUDE.md machine note) — no
/// `all_acceptance_pass` contribution.
fn xl_rows(m: &mut serde_json::Map<String, serde_json::Value>) -> anyhow::Result<()> {
    for level in [8u32, 9] {
        log::info!("harness: XL 1 Gy L{level}");
        let (world, secs) = run(level, 1000.0)?;
        m.insert(
            format!("run_1gy_l{level}_s"),
            serde_json::json!((secs * 100.0).round() / 100.0),
        );
        m.insert(
            format!("keyframe_bytes_1gy_l{level}"),
            serde_json::json!(world.history.as_ref().unwrap().approx_bytes()),
        );
        drop(world);
    }
    // Echo of the L9 cadence decision (d2 §7).
    m.insert(
        "keyframe_interval_my_l9".into(),
        serde_json::json!(keyframe_interval_my(9)),
    );
    Ok(())
}

pub fn run_tectonics_harness(out: &std::path::Path) -> anyhow::Result<()> {
    let mut m = serde_json::Map::new();
    let mut all_pass = true;

    // --- WO-0003 Fix 2: t=0 plate-map metrics + feel gates (setup-only) ---
    log::info!("harness: plategen metrics (5 seeds x L6/L7)");
    all_pass &= plategen_metrics(&mut m);

    // --- default 500 My at L7: timing + physics metrics on the final frame ---
    log::info!("harness: 500 My L7");
    let (world, secs) = run(7, 500.0)?;
    m.insert(
        "run_500my_l7_s".into(),
        serde_json::json!((secs * 100.0).round() / 100.0),
    );
    let history = world.history.as_ref().unwrap();
    m.insert(
        "keyframe_bytes_500my_l7".into(),
        serde_json::json!(history.approx_bytes()),
    );
    let last = history.keyframes.last().unwrap();
    all_pass &= age_depth(last, &mut m);
    all_pass &= hypsometry(last, &mut m);
    all_pass &= arc_trench(last, &world.grid, &mut m);
    hotspots(last, &mut m);
    drop(world);

    // --- 1 Gy at L7: the ≤ 60 s wall-clock budget ---
    log::info!("harness: 1 Gy L7");
    let (world, secs) = run(7, 1000.0)?;
    m.insert(
        "run_1gy_l7_s".into(),
        serde_json::json!((secs * 100.0).round() / 100.0),
    );
    m.insert(
        "run_1gy_l7_within_60s".into(),
        serde_json::json!(secs <= 60.0),
    );
    all_pass &= secs <= 60.0;
    m.insert(
        "keyframe_bytes_1gy_l7".into(),
        serde_json::json!(world.history.as_ref().unwrap().approx_bytes()),
    );
    drop(world);

    // --- 2 Gy at L7: the 1 GB keyframe budget ---
    log::info!("harness: 2 Gy L7");
    let (world, secs) = run(7, 2000.0)?;
    let bytes = world.history.as_ref().unwrap().approx_bytes();
    m.insert(
        "run_2gy_l7_s".into(),
        serde_json::json!((secs * 100.0).round() / 100.0),
    );
    m.insert("keyframe_bytes_2gy_l7".into(), serde_json::json!(bytes));
    m.insert(
        "keyframe_bytes_2gy_l7_within_1gb".into(),
        serde_json::json!(bytes <= 1_000_000_000),
    );
    all_pass &= bytes <= 1_000_000_000;
    drop(world);

    // --- 2 Gy at L6: long-run stability + liveliness (WO-0003 Fix 4) ---
    log::info!("harness: 2 Gy L6 stability");
    let (world, secs) = run(6, 2000.0)?;
    m.insert(
        "run_2gy_l6_s".into(),
        serde_json::json!((secs * 100.0).round() / 100.0),
    );
    all_pass &= stability(world.history.as_ref().unwrap(), &mut m);
    all_pass &= liveliness_rows(world.history.as_ref().unwrap(), "", &mut m);
    drop(world);

    // --- liveliness echo at Dan's recorded freeze settings (WO-0003 Fix 4:
    // seed box "dan", 8 plates, land 0.40, vigor 1.73, 2 Gy, L6) ---
    log::info!("harness: liveliness echo (seed dan, 2 Gy L6)");
    let (world, secs) = run_with(
        6,
        worldmaker_core::hash::seed_from_text("dan"),
        TectonicsParams {
            plate_count: 8,
            land_fraction: 0.40,
            tectonic_vigor: 1.73,
            span_my: 2000.0,
            ..TectonicsParams::default()
        },
    )?;
    m.insert(
        "run_echo_dan_2gy_l6_s".into(),
        serde_json::json!((secs * 100.0).round() / 100.0),
    );
    all_pass &= liveliness_rows(world.history.as_ref().unwrap(), "echo_dan_", &mut m);
    drop(world);

    // --- determinism: same seed, same hashes, twice ---
    log::info!("harness: determinism double-run (500 My L6)");
    let (a, _) = run(6, 500.0)?;
    let (b, _) = run(6, 500.0)?;
    let ha = hash_f32_slice(a.fields.get(ELEVATION_M).unwrap());
    let hb = hash_f32_slice(b.fields.get(ELEVATION_M).unwrap());
    let pa = hash_u32_slice(a.fields.get_u32(PLATE_ID).unwrap());
    let pb = hash_u32_slice(b.fields.get_u32(PLATE_ID).unwrap());
    let ca = hash_u32_slice(a.fields.get_u32(CRUST_TYPE).unwrap());
    m.insert(
        "determinism_elevation_hash_l6_500my_seed42".into(),
        serde_json::json!(format!("{ha:#018x}")),
    );
    m.insert(
        "determinism_plate_hash_l6_500my_seed42".into(),
        serde_json::json!(format!("{pa:#018x}")),
    );
    m.insert(
        "determinism_crust_type_hash_l6_500my_seed42".into(),
        serde_json::json!(format!("{ca:#018x}")),
    );
    let det = ha == hb && pa == pb;
    m.insert("determinism_repeat_matches".into(), serde_json::json!(det));
    all_pass &= det;

    // --- optional XL rows: 1 Gy at L8 and L9 (env WM_HARNESS_XL=1) ---
    if std::env::var("WM_HARNESS_XL")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        xl_rows(&mut m)?;
    }

    m.insert("all_acceptance_pass".into(), serde_json::json!(all_pass));
    let file = worldmaker_io::ResultsFile::new(
        &worldmaker_io::results::today_utc_iso(),
        serde_json::Value::Object(m),
    );
    file.write(out)?;
    log::info!("tectonics results written to {}", out.display());
    println!(
        "tectonics harness: all_acceptance_pass = {all_pass} -> {}",
        out.display()
    );
    Ok(())
}
