//! Diagnostic probe for the within-plate LAND striping artifact
//! (Cowork investigation, 2026-08-28, cloud clone only — NOT committed).
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
//! Run: cargo test -p worldmaker-sim --release land_striping_probe -- --ignored --nocapture

use std::collections::VecDeque;
use std::sync::Arc;

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

fn run(label: &str, plate_count: u32, land_fraction: f32, hotspot_count: u32) {
    let grid = Arc::new(Grid::build(6));
    let seed = seed_from_text("cyrus");
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

    println!("=== {label}: plates {plate_count}, land {land_fraction}, hotspots {hotspot_count}, L6 cyrus 2 Gy ===");
    println!("t_My  land%  comps  tiny  chainC  chain%  far%   d_arc  d_adv  d_cls | d_rgap d_cons d_rift");

    let mut prev = [0u64; 6];

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
        if let Some(pl) = &prev_land {
            let mut coast = 0u32;
            let mut debris = 0u32;
            for c in 0..n {
                if sim.crust_type[c] == 1 && !pl[c] {
                    let adj = grid
                        .neighbors_of(c as u32)
                        .iter()
                        .any(|&nb| pl[nb as usize]);
                    if adj {
                        coast += 1;
                    } else {
                        debris += 1;
                    }
                }
            }
            println!("      step-gains: coastline {coast}, isolated-debris {debris}");
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
        ];
        let d: Vec<u64> = cur.iter().zip(prev.iter()).map(|(a, b)| a - b).collect();
        prev = cur;

        println!(
            "{:5}  {:4.1}  {:5}  {:4}  {:6}  {:5.1}  {:5.1}  {:6} {:6} {:6} | {:6} {:6} {:6}",
            ((step_idx + 1) as f32 * DT_MY) as u32,
            100.0 * land_cells as f32 / n as f32,
            comps,
            tiny,
            chain_comps,
            100.0 * chain_cells.len() as f32 / land_cells.max(1) as f32,
            100.0 * far as f32 / chain_cells.len().max(1) as f32,
            d[0],
            d[1],
            d[2],
            d[3],
            d[4],
            d[5],
        );
    }
}

#[test]
#[ignore = "dev probe: land striping series"]
fn land_striping_probe() {
    // (a) Dan's in-app settings from the report.
    run("dan-app", 12, 0.29, 6);
    // (b) hotspots off: isolates arc + advection mechanisms.
    run("no-hotspots", 12, 0.29, 0);
}
