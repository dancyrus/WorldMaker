//! Tectonic setup at t = 0: plate seeding, Voronoi ownership, cratons,
//! initial ocean ages, plate motions, hotspots.

use std::collections::VecDeque;
use std::sync::Arc;

use rand::RngCore;
use rayon::prelude::*;

use worldmaker_core::dmath::{dot3, gaussian_f32, random_unit_vec, uniform_f32, uniform_range};
use worldmaker_core::rng::sub_rng;
use worldmaker_core::Grid;

use super::keyframe::{PlateState, IDENTITY3, NEVER_SUTURED};
use super::plate_gen;
use super::step::{SimState, OCEAN_THICKNESS_KM, SPEED_MAX};
use super::{TectonicsParams, STAGE_ID};

/// Continental crust fraction relative to the land-fraction target: the
/// margin allows for shelves below the solved sea level (decision log).
/// Public since WO-0008 S0 so the single-crust setup test can rebuild the
/// budget it checks against.
pub const CONT_AREA_FACTOR: f32 = 1.35;
/// Craton core size as a fraction of its continental plate's area
/// (WO-0008 S0).
const CRATON_FRAC_MIN: f32 = 0.3;
const CRATON_FRAC_MAX: f32 = 0.6;
/// Non-craton continental crust age range (My): the younger platform crust
/// surrounding each craton core (WO-0008 S0).
const PLATFORM_AGE_MIN_MY: f32 = 200.0;
const PLATFORM_AGE_MAX_MY: f32 = 800.0;
/// Craton peak thickness range (km); edges taper to the 35 km base.
const CRATON_BASE_KM: f32 = 35.0;
const CRATON_PEAK_MIN_KM: f32 = 40.0;
const CRATON_PEAK_MAX_KM: f32 = 45.0;
/// Craton age range (My).
const CRATON_AGE_MIN_MY: f32 = 1500.0;
const CRATON_AGE_MAX_MY: f32 = 3500.0;
/// Painted craton cells (overlay) get this thickness / age.
const PAINTED_THICKNESS_KM: f32 = 40.0;
const PAINTED_AGE_MY: f32 = 2000.0;
/// Initial ocean age ramp: 30..80 My across the sphere.
const OCEAN_AGE_BASE_MY: f32 = 30.0;
const OCEAN_AGE_SPAN_MY: f32 = 50.0;
/// Initial plate speed distribution (deg/My), scaled by vigor.
const SPEED_MEAN: f32 = 0.5;
const SPEED_SIGMA: f32 = 0.15;
/// Minimum angular separation between generated hotspots: cos(15°).
const HOTSPOT_MIN_SEP_COS: f32 = 0.965_925_8;

pub(super) fn setup(master_seed: u64, grid: &Arc<Grid>, params: &TectonicsParams) -> SimState {
    let mut s = SimState::new_empty(grid);
    let n = grid.cell_count() as usize;

    // --- 1–2. t=0 plate map (WO-0003 Fix 2): the retuned hybrid growth+warp
    // generator, wired at commit M3 after winning the four-way competition
    // 3–0 twice (judge record + re-judging addendum). The generator sees only
    // PlateGenParams (plate_count) — overlays are structurally out of reach.
    let p_count = params.plate_count as usize;
    s.plate_id =
        plate_gen::generate_plates(master_seed, grid, &plate_gen::PlateGenParams::from(params));

    // --- 3. plate motions ---
    // Initial poles and speeds are random draws (WO-0006: the only RNG left
    // in motion); from step 1 on, the force balance owns both.
    for pid in 0..p_count {
        let mut prng = sub_rng(master_seed, STAGE_ID, &format!("plate-init-{pid}"));
        let speed = ((SPEED_MEAN + SPEED_SIGMA * gaussian_f32(&mut prng)).abs()
            * params.tectonic_vigor)
            .min(SPEED_MAX);
        s.plates.push(PlateState {
            id: pid as u32,
            alive: true,
            pole: random_unit_vec(&mut prng),
            speed_deg_my: speed,
            youngest_suture_my: NEVER_SUTURED,
            youngest_rift_my: NEVER_SUTURED,
            youngest_breakup_my: NEVER_SUTURED,
            quiet_my: 0.0,
            pending_rot: IDENTITY3,
            pending_deg: 0.0,
            slab: Vec::new(),
            boundary_cells: 0,
            subducting_cells: 0,
            colliding_cells: 0,
            colliding_strength: 0.0,
            ridge_cells: 0,
            transform_cells: 0,
            drive_torque: [0.0; 3],
        });
    }

    // --- 4. cratons ---
    // Depth-from-plate-boundary BFS (multi-source), for nucleus placement.
    let mut depth = vec![u32::MAX; n];
    let mut queue: VecDeque<u32> = VecDeque::new();
    for (c, d) in depth.iter_mut().enumerate() {
        let is_boundary = grid
            .neighbors_of(c as u32)
            .iter()
            .any(|&nb| s.plate_id[nb as usize] != s.plate_id[c]);
        if is_boundary {
            *d = 0;
            queue.push_back(c as u32);
        }
    }
    while let Some(c) = queue.pop_front() {
        let dc = depth[c as usize];
        for &nb in grid.neighbors_of(c) {
            if depth[nb as usize] == u32::MAX {
                depth[nb as usize] = dc + 1;
                queue.push_back(nb);
            }
        }
    }

    let mut plate_cells = vec![0u32; p_count];
    for &p in &s.plate_id {
        plate_cells[p as usize] += 1;
    }

    // Whole-plate crust assignment (WO-0008 S0, Dan's ruling 2026-08-28):
    // at t = 0 every plate is entirely continental or entirely oceanic —
    // mixed plates only arise later, through rifting and arc growth. Greedy
    // subset pick: shuffle the plate order, then take each plate that moves
    // the running cell sum closer to the continental budget.
    let total_cont =
        ((params.land_fraction * CONT_AREA_FACTOR).min(0.85) * n as f32).round() as u32;
    let mut crng = sub_rng(master_seed, STAGE_ID, "cratons");
    let mut order: Vec<usize> = (0..p_count).collect();
    for i in (1..p_count).rev() {
        let j = ((uniform_f32(&mut crng) * (i + 1) as f32) as usize).min(i);
        order.swap(i, j);
    }
    let mut continental = vec![false; p_count];
    let mut cont_sum = 0u32;
    for &pid in &order {
        let c = plate_cells[pid];
        if (cont_sum + c).abs_diff(total_cont) < cont_sum.abs_diff(total_cont) {
            continental[pid] = true;
            cont_sum += c;
        }
    }
    // Guarantee at least one plate of each kind (ties resolve to the
    // earliest plate in the shuffled order — deterministic).
    if cont_sum == 0 {
        let &best = order
            .iter()
            .min_by_key(|&&pid| plate_cells[pid].abs_diff(total_cont))
            .expect("at least one plate");
        continental[best] = true;
        cont_sum = plate_cells[best];
    }
    if continental.iter().all(|&c| c) {
        let &drop = order
            .iter()
            .min_by_key(|&&pid| (cont_sum - plate_cells[pid]).abs_diff(total_cont))
            .expect("at least one plate");
        continental[drop] = false;
        cont_sum -= plate_cells[drop];
    }
    // The achieved land fraction is quantized by plate sizes: record what
    // the pick actually landed on (the budget carries the shelf margin, so
    // divide it back out to compare against `params.land_fraction`).
    s.achieved_land_frac = cont_sum as f32 / (CONT_AREA_FACTOR * n as f32);
    for c in 0..n {
        if continental[s.plate_id[c] as usize] {
            s.crust_type[c] = 1;
            s.thickness[c] = CRATON_BASE_KM;
        }
    }

    // Cratons stay, as cores inside continental plates: one nucleus per
    // plate at its most interior cell, grown to 30–60% of the plate area;
    // the rest of the plate is younger platform crust at base thickness.
    for pid in 0..p_count {
        if !continental[pid] {
            continue;
        }
        let frac = uniform_range(&mut crng, CRATON_FRAC_MIN, CRATON_FRAC_MAX);
        let target = ((plate_cells[pid] as f32 * frac).round() as u32).clamp(1, plate_cells[pid]);
        // Nucleus: this plate's most interior cell (max boundary depth, tie
        // to the lower id).
        let mut nucleus = 0usize;
        let mut best_depth = 0u32;
        let mut found = false;
        for (c, &d) in depth.iter().enumerate() {
            if s.plate_id[c] == pid as u32 && (!found || d > best_depth) {
                best_depth = d;
                nucleus = c;
                found = true;
            }
        }
        // Grow by BFS inside the plate to the target size, tracking ring
        // depth for the thickness taper.
        let peak = uniform_range(&mut crng, CRATON_PEAK_MIN_KM, CRATON_PEAK_MAX_KM);
        let age = uniform_range(&mut crng, CRATON_AGE_MIN_MY, CRATON_AGE_MAX_MY);
        let platform_age = uniform_range(&mut crng, PLATFORM_AGE_MIN_MY, PLATFORM_AGE_MAX_MY);
        let mut cd = vec![u32::MAX; n];
        cd[nucleus] = 0;
        let mut q: VecDeque<u32> = VecDeque::new();
        q.push_back(nucleus as u32);
        let mut collected: Vec<(u32, u32)> = Vec::with_capacity(target as usize);
        collected.push((nucleus as u32, 0));
        while let Some(c) = q.pop_front() {
            if collected.len() >= target as usize {
                break;
            }
            let dc = cd[c as usize];
            for &nb in grid.neighbors_of(c) {
                let nbu = nb as usize;
                if cd[nbu] == u32::MAX && s.plate_id[nbu] == pid as u32 {
                    cd[nbu] = dc + 1;
                    q.push_back(nb);
                    collected.push((nb, dc + 1));
                    if collected.len() >= target as usize {
                        break;
                    }
                }
            }
        }
        let max_d = collected.iter().map(|&(_, d)| d).max().unwrap_or(0) as f32;
        for &(c, d) in &collected {
            let cu = c as usize;
            let taper = 1.0 - d as f32 / (max_d + 1.0);
            s.thickness[cu] = CRATON_BASE_KM + (peak - CRATON_BASE_KM) * taper;
            s.crust_age[cu] = age;
            // Primordial: exempt from orogenic relaxation from the start.
            s.orogeny_age[cu] = age;
        }
        // Platform crust: every continental cell of this plate the craton
        // BFS did not reach (cd untouched) keeps base thickness and gets
        // the younger age.
        for (c, &craton_depth) in cd.iter().enumerate() {
            if s.plate_id[c] == pid as u32 && craton_depth == u32::MAX {
                s.crust_age[c] = platform_age;
                s.orogeny_age[c] = platform_age;
            }
        }
    }

    // --- 5. initial ocean ages: smooth deterministic ramp ---
    let mut orng = sub_rng(master_seed, STAGE_ID, "ocean-age-ramp");
    let ramp_axis = random_unit_vec(&mut orng);
    let ctype = &s.crust_type;
    s.crust_age.par_iter_mut().enumerate().for_each(|(c, a)| {
        if ctype[c] == 0 {
            let t = 0.5 + 0.5 * dot3(grid.positions[c], ramp_axis);
            *a = OCEAN_AGE_BASE_MY + OCEAN_AGE_SPAN_MY * t;
        }
    });

    // --- 6. craton paint overlay (sorted by cell id; applied last) ---
    for &(cell, v) in &params.craton_overlay {
        let c = cell as usize;
        if c >= n {
            continue;
        }
        if v > 0 {
            s.crust_type[c] = 1;
            s.thickness[c] = PAINTED_THICKNESS_KM;
            s.crust_age[c] = PAINTED_AGE_MY;
            s.orogeny_age[c] = PAINTED_AGE_MY; // primordial, like cratons
        } else if v < 0 {
            s.crust_type[c] = 0;
            s.thickness[c] = OCEAN_THICKNESS_KM;
            let t = 0.5 + 0.5 * dot3(grid.positions[c], ramp_axis);
            s.crust_age[c] = OCEAN_AGE_BASE_MY + OCEAN_AGE_SPAN_MY * t;
            s.orogeny_age[c] = 0.0;
        }
    }

    // --- 7. hotspots ---
    s.hotspots = match &params.hotspot_overlay {
        Some(list) => list.clone(),
        None => {
            let mut hrng = sub_rng(master_seed, STAGE_ID, "hotspots");
            let mut pts: Vec<[f32; 3]> = Vec::new();
            let mut attempts = 0;
            while pts.len() < params.hotspot_count as usize && attempts < 400 {
                attempts += 1;
                let v = random_unit_vec(&mut hrng);
                if pts.iter().all(|p| dot3(*p, v) < HOTSPOT_MIN_SEP_COS) {
                    pts.push(v);
                }
            }
            pts
        }
    };
    s.hotspot_hints = vec![0; s.hotspots.len()];
    s.hotspot_cont_my = vec![0.0; s.hotspots.len()];

    // --- 8. detail-noise seed ---
    s.noise_seed = sub_rng(master_seed, STAGE_ID, "detail-noise").next_u64();

    s.init_stats();
    s
}
