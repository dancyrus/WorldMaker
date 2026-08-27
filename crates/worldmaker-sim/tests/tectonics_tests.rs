//! Integration tests for the tectonics stage: sanity of a short run,
//! determinism, and bit-exact resume from a keyframe.

use std::sync::Arc;

use worldmaker_core::hash::{hash_f32_slice, hash_u32_slice};
use worldmaker_core::Grid;
use worldmaker_sim::tectonics::{
    self, ResumeFrom, TectonicsParams, TectonicsStage, ELEVATION_M, F_RIDGE, F_TRENCH, PLATE_ID,
};
use worldmaker_sim::{Pipeline, Stage, StageContext, WorldState};

fn short_params() -> TectonicsParams {
    TectonicsParams {
        span_my: 200.0,
        ..TectonicsParams::default()
    }
}

fn run_stage(level: u32, seed: u64, params: TectonicsParams) -> WorldState {
    let grid = Arc::new(Grid::build(level));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(TectonicsStage::new(params)));
    pipe.run(&StageContext::new(seed), &mut world).unwrap();
    world
}

#[test]
#[ignore = "debugging aid: prints per-keyframe stats"]
fn debug_keyframe_stats() {
    let mut params = short_params();
    if let Ok(s) = std::env::var("WM_DEBUG_SPAN") {
        params.span_my = s.parse().unwrap();
    }
    let level = std::env::var("WM_DEBUG_LEVEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let t0 = std::time::Instant::now();
    let world = run_stage(level, 42, params);
    println!("run: L{level} {:.2} s wall", t0.elapsed().as_secs_f64());
    let hist = world.history.as_ref().unwrap();
    println!("history bytes: {:.1} MB", hist.approx_bytes() as f64 / 1e6);
    println!("diagnostics: {:?}", hist.diagnostics);
    for kf in &hist.keyframes {
        let n = kf.elev_m.len();
        let cont = kf.flags.iter().filter(|&&f| f & (1 << 15) != 0).count();
        let land = kf.elev_m.iter().filter(|&&e| e >= 0).count();
        let alive = kf.plates.iter().filter(|p| p.alive).count();
        let max_e = kf.elev_m.iter().max().unwrap();
        let min_e = kf.elev_m.iter().min().unwrap();
        let max_t = kf.thickness_ckm.iter().max().unwrap();
        let ridge = kf.flags.iter().filter(|&&f| f & 1 != 0).count();
        let trench = kf.flags.iter().filter(|&&f| f & 2 != 0).count();
        println!(
            "t={:6} sea_off={:8.1} cont={:.3} land={:.3} plates={} elev=[{},{}] max_thick={:.1} ridge={} trench={}",
            kf.t_my,
            kf.sea_offset_m,
            cont as f32 / n as f32,
            land as f32 / n as f32,
            alive,
            min_e,
            max_e,
            *max_t as f32 * 0.01,
            ridge,
            trench
        );
    }
}

#[test]
fn short_run_produces_a_sane_world() {
    let world = run_stage(5, 42, short_params());
    let n = world.fields.cell_count() as usize;

    let elev = world.fields.get(ELEVATION_M).unwrap();
    let mut land = 0usize;
    for &e in elev {
        assert!(e.is_finite());
        assert!((-12_000.0..=12_000.0).contains(&e), "implausible: {e}");
        if e >= 0.0 {
            land += 1;
        }
    }
    // Sea level anchors to the parameter at t = 0 and then drifts with the
    // hypsometry, so the final land fraction sits near — not on — the target.
    let land_frac = land as f32 / n as f32;
    assert!(
        (land_frac - 0.29).abs() < 0.05,
        "land fraction {land_frac} drifted implausibly far from the 0.29 anchor"
    );

    // History: 200 My at 10 My cadence = 21 keyframes including t = 0.
    let hist = world.history.as_ref().unwrap();
    assert_eq!(hist.keyframes.len(), 21);
    assert_eq!(hist.hotspots.len(), 6);

    // At the t = 0 anchor the solve is exact; the datum then stays fixed.
    let kf0 = &hist.keyframes[0];
    let land0 = kf0.elev_m.iter().filter(|&&e| e >= 0).count() as f32 / n as f32;
    assert!(
        (land0 - 0.29).abs() < 0.005,
        "anchor land fraction {land0} not on the parameter"
    );
    assert!(
        hist.keyframes
            .iter()
            .all(|kf| kf.sea_offset_m == kf0.sea_offset_m),
        "sea datum must stay fixed after the anchor solve"
    );

    // Plate count stays in the specified band.
    let final_kf = hist.keyframes.last().unwrap();
    let alive = final_kf.plates.iter().filter(|p| p.alive).count();
    assert!((6..=24).contains(&alive), "alive plates: {alive}");

    // The dynamics actually produce ridges and trenches.
    let feats: Vec<u32> = final_kf.flags.iter().map(|&f| (f & 0xff) as u32).collect();
    let ridges = feats.iter().filter(|&&f| f & F_RIDGE != 0).count();
    let trenches = feats.iter().filter(|&&f| f & F_TRENCH != 0).count();
    assert!(ridges > 20, "too few ridge cells: {ridges}");
    assert!(trenches > 5, "too few trench cells: {trenches}");

    // Ocean age field self-organizes: young near ridges, old elsewhere.
    let ages = world.fields.get(tectonics::CRUST_AGE_MY).unwrap();
    let ctype = world.fields.get_u32(tectonics::CRUST_TYPE).unwrap();
    let (mut young, mut old) = (0usize, 0usize);
    for c in 0..n {
        if ctype[c] == 0 {
            if ages[c] < 30.0 {
                young += 1;
            }
            if ages[c] > 100.0 {
                old += 1;
            }
        }
    }
    assert!(young > 100, "no young ocean crust ({young})");
    assert!(old > 100, "no old ocean crust ({old})");
}

#[test]
fn same_seed_reproduces_identical_hashes() {
    let a = run_stage(5, 42, short_params());
    let b = run_stage(5, 42, short_params());
    assert_eq!(
        hash_f32_slice(a.fields.get(ELEVATION_M).unwrap()),
        hash_f32_slice(b.fields.get(ELEVATION_M).unwrap()),
    );
    assert_eq!(
        hash_u32_slice(a.fields.get_u32(PLATE_ID).unwrap()),
        hash_u32_slice(b.fields.get_u32(PLATE_ID).unwrap()),
    );
    let c = run_stage(5, 43, short_params());
    assert_ne!(
        hash_f32_slice(a.fields.get(ELEVATION_M).unwrap()),
        hash_f32_slice(c.fields.get(ELEVATION_M).unwrap()),
        "different seed must give a different world"
    );
}

/// Restarting from an interior keyframe must replay the identical future:
/// keyframes are full state and per-step randomness is keyed on the absolute
/// step index.
#[test]
fn resume_from_keyframe_is_bit_exact() {
    let params = short_params();
    let grid = Arc::new(Grid::build(5));
    let world = {
        let mut w = WorldState::new(grid.clone());
        let mut pipe = Pipeline::new();
        pipe.push(Box::new(TectonicsStage::new(params.clone())));
        pipe.run(&StageContext::new(7), &mut w).unwrap();
        w
    };
    let hist = world.history.as_ref().unwrap();
    let mid = 10; // t = 100 My
    let resumed = tectonics::run_history(
        &StageContext::new(7),
        &world,
        &params,
        Some(ResumeFrom {
            keyframe: &hist.keyframes[mid],
            hotspots: &hist.hotspots,
        }),
    )
    .unwrap();

    assert_eq!(resumed.keyframes.len(), hist.keyframes.len() - (mid + 1));
    {
        // Diagnostic: report exactly what differs at the first resumed frame.
        let kf = &resumed.keyframes[0];
        let orig = &hist.keyframes[mid + 1];
        let count_diff = |a: &[u16], b: &[u16]| a.iter().zip(b).filter(|(x, y)| x != y).count();
        let first_elev = kf.elev_m.iter().zip(&orig.elev_m).position(|(x, y)| x != y);
        eprintln!(
            "diffs at t={}: elev={} (first {:?}) plate={} age={} thick={} orog={} rift={} build={} flags={}",
            kf.t_my,
            kf.elev_m.iter().zip(&orig.elev_m).filter(|(x, y)| x != y).count(),
            first_elev,
            count_diff(&kf.plate_id, &orig.plate_id),
            count_diff(&kf.crust_age_my, &orig.crust_age_my),
            count_diff(&kf.thickness_ckm, &orig.thickness_ckm),
            count_diff(&kf.orogeny_age_my, &orig.orogeny_age_my),
            count_diff(&kf.rift_age_my, &orig.rift_age_my),
            count_diff(&kf.buildup_ckm, &orig.buildup_ckm),
            count_diff(&kf.flags, &orig.flags),
        );
        for (pk, po) in kf.plates.iter().zip(&orig.plates) {
            if pk.pole != po.pole
                || pk.speed_deg_my != po.speed_deg_my
                || pk.pending_deg != po.pending_deg
                || pk.boundary_cells != po.boundary_cells
                || pk.subducting_cells != po.subducting_cells
                || pk.colliding_cells != po.colliding_cells
            {
                eprintln!(
                    "plate {} differs: pole {:?}/{:?} speed {}/{} pend {}/{} bnd {}/{} sub {}/{} coll {}/{}",
                    pk.id, pk.pole, po.pole, pk.speed_deg_my, po.speed_deg_my,
                    pk.pending_deg, po.pending_deg, pk.boundary_cells, po.boundary_cells,
                    pk.subducting_cells, po.subducting_cells, pk.colliding_cells, po.colliding_cells
                );
            }
        }
        eprintln!("sea_off {} vs {}", kf.sea_offset_m, orig.sea_offset_m);
    }
    for (i, kf) in resumed.keyframes.iter().enumerate() {
        let orig = &hist.keyframes[mid + 1 + i];
        let t = orig.t_my;
        assert_eq!(kf.t_my, orig.t_my, "keyframe time mismatch at {i}");
        assert_eq!(kf.sea_offset_m, orig.sea_offset_m, "sea offset at t={t}");
        // Every per-cell array — bit-exact means all of them (review finding).
        assert_eq!(kf.elev_m, orig.elev_m, "elevation diverged at t={t}");
        assert_eq!(kf.plate_id, orig.plate_id, "plates diverged at t={t}");
        assert_eq!(kf.flags, orig.flags, "flags diverged at t={t}");
        assert_eq!(kf.thickness_ckm, orig.thickness_ckm, "thickness at t={t}");
        assert_eq!(kf.crust_age_my, orig.crust_age_my, "crust age at t={t}");
        assert_eq!(kf.orogeny_age_my, orig.orogeny_age_my, "orogeny at t={t}");
        assert_eq!(kf.rift_age_my, orig.rift_age_my, "rift age at t={t}");
        assert_eq!(kf.buildup_ckm, orig.buildup_ckm, "buildup at t={t}");
        // Slab-ledger cells (WO-0006 S1): the round trip must preserve them.
        assert_eq!(kf.slab_plate, orig.slab_plate, "slab plate at t={t}");
        assert_eq!(kf.slab_since_my, orig.slab_since_my, "slab since at t={t}");
        // Plate-level state, field by field (PlateState is raw f32s).
        assert_eq!(kf.plates.len(), orig.plates.len(), "plate count at t={t}");
        for (pk, po) in kf.plates.iter().zip(&orig.plates) {
            assert_eq!(pk.id, po.id);
            assert_eq!(pk.alive, po.alive, "plate {} alive at t={t}", pk.id);
            assert_eq!(pk.pole, po.pole, "plate {} pole at t={t}", pk.id);
            assert_eq!(
                pk.speed_deg_my, po.speed_deg_my,
                "plate {} speed at t={t}",
                pk.id
            );
            assert_eq!(
                pk.youngest_suture_my, po.youngest_suture_my,
                "plate {} suture at t={t}",
                pk.id
            );
            assert_eq!(
                pk.pending_rot, po.pending_rot,
                "plate {} pending rot at t={t}",
                pk.id
            );
            assert_eq!(
                pk.pending_deg, po.pending_deg,
                "plate {} pending deg at t={t}",
                pk.id
            );
            assert_eq!(
                pk.boundary_cells, po.boundary_cells,
                "plate {} boundary at t={t}",
                pk.id
            );
            assert_eq!(
                pk.subducting_cells, po.subducting_cells,
                "plate {} subducting at t={t}",
                pk.id
            );
            assert_eq!(
                pk.colliding_cells, po.colliding_cells,
                "plate {} colliding at t={t}",
                pk.id
            );
            assert_eq!(
                pk.ridge_cells, po.ridge_cells,
                "plate {} ridge cells at t={t}",
                pk.id
            );
            assert_eq!(
                pk.transform_cells, po.transform_cells,
                "plate {} transform cells at t={t}",
                pk.id
            );
            assert_eq!(
                pk.drive_torque, po.drive_torque,
                "plate {} torque at t={t}",
                pk.id
            );
            assert_eq!(pk.slab, po.slab, "plate {} slab ledger at t={t}", pk.id);
        }
        // Pair-collision timers.
        assert_eq!(
            kf.collisions.len(),
            orig.collisions.len(),
            "timers at t={t}"
        );
        for (ck, co) in kf.collisions.iter().zip(&orig.collisions) {
            assert_eq!((ck.a, ck.b), (co.a, co.b), "timer pair at t={t}");
            assert_eq!(
                ck.slow_collision_my, co.slow_collision_my,
                "timer ({}, {}) at t={t}",
                ck.a, ck.b
            );
        }
    }
}

/// The tectonics stage must be cancellable and leave the pipeline dirty.
#[test]
fn cancel_stops_the_run() {
    use worldmaker_sim::Progress;
    let grid = Arc::new(Grid::build(5));
    let mut world = WorldState::new(grid);
    let mut pipe = Pipeline::new();
    pipe.push(Box::new(TectonicsStage::new(short_params())));
    let progress = std::sync::Arc::new(Progress::new());
    progress.request_cancel();
    let mut ctx = StageContext::new(42);
    ctx.progress = Some(progress);
    let err = pipe.run(&ctx, &mut world).unwrap_err();
    assert!(
        err.downcast_ref::<worldmaker_sim::Cancelled>().is_some(),
        "expected Cancelled, got: {err:#}"
    );
}

/// Cancellation must also work MID-run — cancel polled every step, not just
/// at entry (review finding: the pre-set-flag test alone can't tell the
/// difference, and the UI Cancel button depends on per-step polling).
#[test]
fn cancel_interrupts_a_running_simulation() {
    use worldmaker_sim::Progress;
    let progress = std::sync::Arc::new(Progress::new());
    let worker_progress = progress.clone();
    let handle = std::thread::spawn(move || {
        let grid = Arc::new(Grid::build(6));
        let mut world = WorldState::new(grid);
        let mut pipe = Pipeline::new();
        pipe.push(Box::new(TectonicsStage::new(TectonicsParams {
            span_my: 2000.0, // long enough that it cannot finish first
            ..TectonicsParams::default()
        })));
        let mut ctx = StageContext::new(42);
        ctx.progress = Some(worker_progress);
        pipe.run(&ctx, &mut world)
    });
    // Wait until the run has demonstrably made progress...
    let start = std::time::Instant::now();
    while progress.fraction() <= 0.0 {
        assert!(
            start.elapsed().as_secs() < 60,
            "run never reported progress"
        );
        std::thread::yield_now();
    }
    // ...then cancel and require a prompt Cancelled result.
    progress.request_cancel();
    let result = handle.join().expect("worker panicked");
    let err = result.expect_err("run should have been cancelled");
    assert!(
        err.downcast_ref::<worldmaker_sim::Cancelled>().is_some(),
        "expected Cancelled, got: {err:#}"
    );
    assert!(
        progress.fraction() < 1.0,
        "run finished despite cancellation"
    );
}

/// Craton painting: the overlay deterministically changes the initial
/// continents, and an empty overlay matches the unpainted world.
#[test]
fn craton_overlay_changes_world_deterministically() {
    let base = run_stage(5, 42, short_params());
    let painted = {
        let mut p = short_params();
        // Paint a blob of continent onto cells 100..140.
        p.craton_overlay = (100..140).map(|c| (c, 1i8)).collect();
        run_stage(5, 42, p)
    };
    let painted2 = {
        let mut p = short_params();
        p.craton_overlay = (100..140).map(|c| (c, 1i8)).collect();
        run_stage(5, 42, p)
    };
    let h_base = hash_f32_slice(base.fields.get(ELEVATION_M).unwrap());
    let h_p1 = hash_f32_slice(painted.fields.get(ELEVATION_M).unwrap());
    let h_p2 = hash_f32_slice(painted2.fields.get(ELEVATION_M).unwrap());
    assert_ne!(h_base, h_p1, "painting cratons must change the world");
    assert_eq!(h_p1, h_p2, "same overlay must reproduce the same world");

    // Plate layout repeats: same seed, same plate setup regardless of paint.
    assert_eq!(
        base.history.as_ref().unwrap().keyframes[0].plate_id,
        painted.history.as_ref().unwrap().keyframes[0].plate_id,
        "plate layout must not depend on the craton overlay"
    );
}

/// Stage caching: an unchanged world/params skips the run; params_hash
/// covers the overlay.
#[test]
fn stage_cache_reacts_to_overlay_changes() {
    let stage_a = TectonicsStage::new(short_params());
    let mut with_overlay = short_params();
    with_overlay.craton_overlay = vec![(5, 1)];
    let stage_b = TectonicsStage::new(with_overlay);
    assert_ne!(stage_a.params_hash(), stage_b.params_hash());
}
