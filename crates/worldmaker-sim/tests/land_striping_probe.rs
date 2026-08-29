//! Diagnostic probe for the within-plate LAND striping artifact
//! (Cowork investigation, 2026-08-28; committed by WO-0012 S1 step 1).
//! Dan's app shows parallel dashed trains of land cells across plate
//! interiors at 2 Gy. Candidate mechanisms:
//!   M1 arc-band sweep: the 150-250 km arc band is anchored to the trench;
//!      trench migration relative to the overrider's material strands
//!      matured islands behind it, chain after chain.
//!   M2 hotspot trails: buildup at a world-fixed cell while the plate
//!      slides over it — a real mechanism (Emperor chain), one trail per
//!      hotspot per motion regime.
//!   M3 advection duplication: the gather can source two destination
//!      cells from one source cell, copying island crust; repeated commits
//!      smear an island into a dashed train (cont_gained_by_advection).
//!      CONFIRMED as the dominant mechanism; fixed by WO-0012 S1's
//!      conservative parcel transport.
//!
//! Measures every 100 My:
//!   land%      crust_type==1 share of the sphere
//!   comps      connected land components (all plates)
//!   tiny       components of 1-3 cells (islets)
//!   chainC     components of >=4 cells where EVERY cell has <=2 land
//!              neighbours (pure chains — the visible trains)
//!   chain%     land cells with <=2 of 6 land neighbours / land cells
//!   far%       of chain cells, share farther than 600 km (back-arc max)
//!              from any CURRENT trench cell — stranded relics
//!   d(arc/adv/cls|rgap/cons/rift)  counter deltas since last sample
//!
//! Diagnostic run (Dan's app settings, seed cyrus):
//!   cargo test -p worldmaker-sim --release land_striping_probe -- --ignored --nocapture
//! WO-0012 S1 step-4 measurement (3 seeds x 2 plate configs, JSON to
//! docs/results; env WM_STRIPING_VARIANT / WM_STRIPING_OUT override the
//! variant tag and output path so a before-fix build can record too):
//!   cargo test -p worldmaker-sim --release land_striping_measure -- --ignored --nocapture

use std::collections::VecDeque;
use std::sync::Arc;

use serde_json::json;
use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{SimState, TectonicsParams, F_TRENCH};

const DT_MY: f32 = 2.0;
const STEPS_PER_KEYFRAME: u32 = 5;
const SAMPLE_EVERY_STEPS: u32 = 50;
const FAR_KM: f32 = 600.0;

fn quantize(s: &mut SimState) {
    let q_u16 = |v: f32| -> f32 { (v.round().clamp(0.0, 65_535.0) as u16) as f32 };
    let q_suture = |v: f32| -> f32 {
        if v < 0.0 {
            v
        } else {
            (v.round().clamp(0.0, 65_534.0) as u16) as f32
        }
    };
    for i in 0..s.crust_age.len() {
        s.crust_age[i] = q_u16(s.crust_age[i]);
        s.thickness[i] = q_u16(s.thickness[i] * 100.0) * 0.01;
        s.orogeny_age[i] = q_u16(s.orogeny_age[i]);
        s.rift_age[i] = q_u16(s.rift_age[i]);
        s.buildup[i] = q_u16(s.buildup[i] * 100.0) * 0.01;
        s.slab_since_my[i] = q_u16(s.slab_since_my[i]);
        s.suture_at_my[i] = q_suture(s.suture_at_my[i]);
    }
    for v in s.hotspot_cont_my.iter_mut() {
        *v = q_u16(*v);
    }
}

/// One 2 Gy L6 run; prints the sample table and returns it as JSON.
fn run(
    label: &str,
    seed_text: &str,
    plate_count: u32,
    land_fraction: f32,
    hotspot_count: u32,
) -> serde_json::Value {
    let grid = Arc::new(Grid::build(6));
    let seed = seed_from_text(seed_text);
    let params = TectonicsParams {
        plate_count,
        land_fraction,
        tectonic_vigor: 1.0,
        span_my: 2000.0,
        hotspot_count,
        ..TectonicsParams::default()
    };
    let total_steps = (params.span_my / DT_MY) as u32;
    let n = grid.cell_count() as usize;
    // L6 cell spacing: sphere area / cell count, sqrt -> km.
    let cell_km = (510.07e6_f64 / n as f64).sqrt() as f32;
    let far_hops = (FAR_KM / cell_km).ceil() as u16;

    let mut sim = SimState::setup(seed, &grid, &params);
    quantize(&mut sim);

    println!(
        "=== {label}: seed {seed_text}, plates {plate_count}, land {land_fraction}, \
         hotspots {hotspot_count}, L6 2 Gy ==="
    );
    println!(
        "t_My  land%  comps  tiny  chainC  chain%  far%   d_arc  d_adv  d_cls | \
         d_rgap d_cons d_rift  d_mrg"
    );

    let mut prev = [0u64; 13];
    let mut samples: Vec<serde_json::Value> = Vec::new();

    for step_idx in 0..total_steps {
        // On sampled steps, snapshot the land mask to classify this step's
        // ocean->land gains: coastline redraw (adjacent to previous land)
        // vs isolated debris birth (no previous-land neighbour).
        let sampled = (step_idx + 1) % SAMPLE_EVERY_STEPS == 0;
        let prev_land: Option<Vec<bool>> = if sampled {
            Some(sim.crust_type.iter().map(|&t| t == 1).collect())
        } else {
            None
        };
        sim.step(seed, step_idx);
        let mut step_coast = 0u32;
        let mut step_debris = 0u32;
        if let Some(pl) = &prev_land {
            for c in 0..n {
                if sim.crust_type[c] == 1 && !pl[c] {
                    let adj = grid
                        .neighbors_of(c as u32)
                        .iter()
                        .any(|&nb| pl[nb as usize]);
                    if adj {
                        step_coast += 1;
                    } else {
                        step_debris += 1;
                    }
                }
            }
            println!("      step-gains: coastline {step_coast}, isolated-debris {step_debris}");
        }
        if (step_idx + 1) % STEPS_PER_KEYFRAME == 0 {
            quantize(&mut sim);
        }
        if (step_idx + 1) % SAMPLE_EVERY_STEPS != 0 {
            continue;
        }

        let land: Vec<bool> = sim.crust_type.iter().map(|&t| t == 1).collect();
        let land_cells = land.iter().filter(|&&l| l).count();

        // Land neighbour counts.
        let land_nbrs: Vec<u8> = (0..n)
            .map(|c| {
                grid.neighbors_of(c as u32)
                    .iter()
                    .filter(|&&nb| land[nb as usize])
                    .count() as u8
            })
            .collect();
        let chain_cells: Vec<usize> = (0..n).filter(|&c| land[c] && land_nbrs[c] <= 2).collect();

        // Components.
        let mut comp = vec![u32::MAX; n];
        let mut sizes: Vec<u32> = Vec::new();
        let mut all_chain: Vec<bool> = Vec::new();
        let mut q: VecDeque<u32> = VecDeque::new();
        for c0 in 0..n {
            if !land[c0] || comp[c0] != u32::MAX {
                continue;
            }
            let id = sizes.len() as u32;
            sizes.push(0);
            all_chain.push(true);
            comp[c0] = id;
            q.push_back(c0 as u32);
            while let Some(c) = q.pop_front() {
                sizes[id as usize] += 1;
                if land_nbrs[c as usize] > 2 {
                    all_chain[id as usize] = false;
                }
                for &nb in grid.neighbors_of(c) {
                    let nbu = nb as usize;
                    if land[nbu] && comp[nbu] == u32::MAX {
                        comp[nbu] = id;
                        q.push_back(nb);
                    }
                }
            }
        }
        let comps = sizes.len();
        let tiny = sizes.iter().filter(|&&s| s <= 3).count();
        let chain_comps = sizes
            .iter()
            .zip(all_chain.iter())
            .filter(|(&s, &ac)| s >= 4 && ac)
            .count();

        // Distance from current trench cells (hops), whole sphere.
        let mut dist = vec![u16::MAX; n];
        let mut q2: VecDeque<u32> = VecDeque::new();
        for c in 0..n {
            if sim.features[c] & F_TRENCH != 0 {
                dist[c] = 0;
                q2.push_back(c as u32);
            }
        }
        while let Some(c) = q2.pop_front() {
            let d = dist[c as usize];
            if d >= far_hops {
                continue;
            }
            for &nb in grid.neighbors_of(c) {
                let nbu = nb as usize;
                if dist[nbu] == u16::MAX {
                    dist[nbu] = d + 1;
                    q2.push_back(nb);
                }
            }
        }
        let far = chain_cells.iter().filter(|&&c| dist[c] == u16::MAX).count();

        let cur = [
            sim.cont_gained_by_arc,
            sim.cont_gained_by_advection,
            sim.cont_gained_by_closure,
            sim.cont_lost_to_ridge_gap,
            sim.cont_lost_to_consumption,
            sim.cont_lost_to_rift,
            sim.parcels_merged,
            sim.suture_count,
            sim.breakup_count,
            sim.underthrust_removed_q as u64,
            sim.underthrust_deposited_q as u64,
            sim.underthrust_spilled_q as u64,
            sim.underthrust_incorporated_q as u64,
        ];
        let d: Vec<u64> = cur.iter().zip(prev.iter()).map(|(a, b)| a - b).collect();
        prev = cur;

        // Slow-plate / weld context (m8 diagnostics): which plates sit
        // below the liveliness floor, and the live welds.
        let slow: Vec<String> = sim
            .plates
            .iter()
            .filter(|p| p.alive && p.speed_deg_my < 0.05)
            .map(|p| format!("{}@{:.3}", p.id, p.speed_deg_my))
            .collect();
        let welds: Vec<String> = sim
            .welds
            .iter()
            .map(|w| format!("{}<-{}", w.winner, w.loser))
            .collect();
        if !slow.is_empty() || !welds.is_empty() {
            println!(
                "      slow: [{}] welds: [{}] cc-timers: {}",
                slow.join(" "),
                welds.join(" "),
                sim.collisions.len()
            );
        }

        let t_my = ((step_idx + 1) as f32 * DT_MY) as u32;
        let land_pct = 100.0 * land_cells as f32 / n as f32;
        let chain_pct = 100.0 * chain_cells.len() as f32 / land_cells.max(1) as f32;
        let far_pct = 100.0 * far as f32 / chain_cells.len().max(1) as f32;
        println!(
            "{:5}  {:4.1}  {:5}  {:4}  {:6}  {:5.1}  {:5.1}  {:6} {:6} {:6} | {:6} {:6} {:6} {:6} | su {} bk {} | uq rm {} dep {} sp {} inc {}",
            t_my, land_pct, comps, tiny, chain_comps, chain_pct, far_pct,
            d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7], d[8], d[9], d[10], d[11], d[12],
        );
        samples.push(json!({
            "t_my": t_my,
            "land_pct": land_pct,
            "components_count": comps,
            "tiny_components_count": tiny,
            "chain_components_count": chain_comps,
            "chain_pct": chain_pct,
            "far_chain_pct": far_pct,
            "d_arc_count": d[0],
            "d_advection_count": d[1],
            "d_closure_count": d[2],
            "d_ridge_gap_count": d[3],
            "d_consumption_count": d[4],
            "d_rift_count": d[5],
            "d_merge_count": d[6],
            "d_suture_count": d[7],
            "d_breakup_count": d[8],
            "d_underthrust_removed_q": d[9],
            "d_underthrust_deposited_q": d[10],
            "d_underthrust_spilled_q": d[11],
            "d_underthrust_incorporated_q": d[12],
            "step_gain_coastline_count": step_coast,
            "step_gain_isolated_debris_count": step_debris,
        }));
    }
    println!(
        "end: rifts started {} failed {} linked {} breakups {} sutures {} ledger {}",
        sim.rift_start_count,
        sim.rift_failed_count,
        sim.rift_link_count,
        sim.breakup_count,
        sim.suture_count,
        sim.rifts.len()
    );
    for r in &sim.rifts {
        println!(
            "  rift plate {} kind {:?} done {}/{} started {} My, {} cells",
            r.plate,
            r.kind,
            r.done_a,
            r.done_b,
            r.started_my,
            r.cells.len()
        );
    }
    json!({
        "config": {
            "level": 6,
            "seed": seed_text,
            "span_my": 2000.0,
            "plate_count": plate_count,
            "land_fraction": land_fraction,
            "hotspot_count": hotspot_count,
        },
        "samples": samples,
    })
}

#[test]
#[ignore = "dev probe: 24-plate consolidation check"]
fn consolidation_probe() {
    run("shape-config", "42", 24, 0.40, 6);
}

#[test]
#[ignore = "dev probe: land striping series"]
fn land_striping_probe() {
    // (a) Dan's in-app settings from the report.
    run("dan-app", "cyrus", 12, 0.29, 6);
    // (b) hotspots off: isolates arc + advection mechanisms.
    run("no-hotspots", "cyrus", 12, 0.29, 0);
}

/// WO-0012 S1 step 4: the measurement Dan's S2 gate ruling reads. Three
/// seeds at both plate configs, machine-labelled JSON to docs/results.
#[test]
#[ignore = "measurement run: WO-0012 S1 step 4"]
fn land_striping_measure() {
    let variant =
        std::env::var("WM_STRIPING_VARIANT").unwrap_or_else(|_| "after_fix".to_owned());
    let mut runs: Vec<serde_json::Value> = Vec::new();
    for seed_text in ["cyrus", "42", "7"] {
        for &(plates, land) in &[(12u32, 0.29f32), (24, 0.40)] {
            let label = format!("{variant} {seed_text} p{plates}");
            runs.push(run(&label, seed_text, plates, land, 6));
        }
    }
    let metrics = json!({ "variant": variant, "runs": runs });
    let path = std::env::var("WM_STRIPING_OUT").unwrap_or_else(|_| {
        format!(
            "{}/../../docs/results/land-striping-wo0012-{}.json",
            env!("CARGO_MANIFEST_DIR"),
            worldmaker_io::results::machine_name()
        )
    });
    let file = worldmaker_io::ResultsFile::new(&worldmaker_io::results::today_utc_iso(), metrics);
    file.write(std::path::Path::new(&path)).unwrap();
    eprintln!("wrote {path}");
}
