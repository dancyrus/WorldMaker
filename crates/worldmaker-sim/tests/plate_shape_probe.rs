//! Diagnostic probe for the "gerrymander" plate-shape artifact
//! (Cowork investigation, 2026-08-28). NOT a gate. Reproduces at Dan's
//! exact UI settings (24 plates, land 0.40, vigor 1.0, 2 Gy, seed "cyrus")
//! but at L6 for speed — the advection shredding is grid-resolution
//! independent, so L6 shows the same mechanism the L7 video shows.
//!
//! It measures, every 100 My:
//!   * compactness  = mean over alive plates of boundary_cells / area
//!                    (a compact cap is low; a shredded comb is high)
//!   * finger_frac  = fraction of all owned cells that are "fingers":
//!                    <=2 of 6 same-plate neighbours (thin strips + necks)
//!   * interior_flip_frac = of all cells that changed owner this step,
//!                    the fraction that were INTERIOR of their old plate
//!                    last step (all neighbours same plate). A rigid plate
//!                    translating on the grid flips ONLY edge cells; an
//!                    interior flip is the resample shredding the plate.
//!   * largest_share = biggest plate's share of the sphere (welding)
//!
//! Run:
//!   cargo test -p worldmaker-sim --release plate_shape_probe -- --ignored --nocapture

use std::sync::Arc;

use worldmaker_core::hash::seed_from_text;
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{SimState, TectonicsParams};

const DT_MY: f32 = 2.0;
const STEPS_PER_KEYFRAME: u32 = 5; // 10 My at L6
const SAMPLE_EVERY_STEPS: u32 = 50; // 100 My

/// Mirror of SimState::quantize_state, as run_history applies it.
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

fn same_plate_neighbours(grid: &Grid, plate_id: &[u32], c: usize) -> u32 {
    let p = plate_id[c];
    grid.neighbors_of(c as u32)
        .iter()
        .filter(|&&nb| plate_id[nb as usize] == p)
        .count() as u32
}

#[test]
#[ignore = "dev probe: prints the shape-degradation series"]
fn plate_shape_probe() {
    let grid = Arc::new(Grid::build(6));
    let seed = seed_from_text("cyrus");
    let params = TectonicsParams {
        plate_count: 24,
        land_fraction: 0.40,
        tectonic_vigor: 1.0,
        span_my: 2000.0,
        ..TectonicsParams::default()
    };
    let total_steps = (params.span_my / DT_MY) as u32;
    let n = grid.cell_count() as usize;

    let mut sim = SimState::setup(seed, &grid, &params);
    quantize(&mut sim);

    println!("t_My  alive  largest%  compact  finger%  intFlip%");

    let sample = |sim: &SimState, prev: &[u32]| {
        let alive: Vec<usize> = (0..sim.plates.len())
            .filter(|&p| sim.plates[p].alive)
            .collect();
        // compactness + fingers
        let mut boundary = vec![0u32; sim.plates.len()];
        let mut area = vec![0u32; sim.plates.len()];
        let mut fingers = 0u64;
        for c in 0..n {
            let p = sim.plate_id[c] as usize;
            area[p] += 1;
            let same = same_plate_neighbours(&sim.grid, &sim.plate_id, c);
            let deg = sim.grid.neighbors_of(c as u32).len() as u32;
            if same < deg {
                boundary[p] += 1;
            }
            if same <= 2 {
                fingers += 1;
            }
        }
        let mut ratios = Vec::new();
        for &p in &alive {
            if area[p] > 0 {
                ratios.push(boundary[p] as f64 / area[p] as f64);
            }
        }
        let compact = ratios.iter().sum::<f64>() / ratios.len().max(1) as f64;
        let largest = *area.iter().max().unwrap_or(&0);
        let finger_frac = fingers as f64 / n as f64;

        // interior flip fraction (this-step ownership change vs prev)
        let mut flips = 0u64;
        let mut interior_flips = 0u64;
        for c in 0..n {
            if sim.plate_id[c] != prev[c] {
                flips += 1;
                // was c interior of its OLD plate last step?
                let op = prev[c];
                let all_same = sim
                    .grid
                    .neighbors_of(c as u32)
                    .iter()
                    .all(|&nb| prev[nb as usize] == op);
                if all_same {
                    interior_flips += 1;
                }
            }
        }
        let int_flip_frac = interior_flips as f64 / flips.max(1) as f64;
        (
            alive.len(),
            largest as f64 / n as f64,
            compact,
            finger_frac,
            int_flip_frac,
        )
    };

    for step_idx in 0..total_steps {
        let prev_plate = sim.plate_id.clone();
        sim.step(seed, step_idx);

        if (step_idx + 1) % STEPS_PER_KEYFRAME == 0 {
            quantize(&mut sim);
        }
        if (step_idx + 1) % SAMPLE_EVERY_STEPS == 0 {
            let (alive, largest, compact, finger, intflip) = sample(&sim, &prev_plate);
            println!(
                "{:>5} {:>5} {:>7.1}% {:>7.3} {:>6.1}% {:>7.1}%",
                sim.t_my,
                alive,
                largest * 100.0,
                compact,
                finger * 100.0,
                intflip * 100.0
            );
        }
    }
}
