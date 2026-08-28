//! Stage 1: kinematic plate tectonics.
//!
//! Rigid plates on the Goldberg grid, moved by Euler-pole rotations in 2 My
//! steps; boundaries classified from relative motion; crust created at ridges,
//! consumed at trenches, thickened in collisions; elevation derived from crust
//! state by isostasy each keyframe. Design of record:
//! docs/plan/tectonics-design.md. Reference: Cortial et al. 2019.
//!
//! Determinism: all randomness through `sub_rng(seed, STAGE_ID, purpose)`
//! where per-step purposes embed the absolute step index (so a re-run from a
//! keyframe replays identical randomness); no libm transcendentals anywhere in
//! this module (see `worldmaker_core::dmath`); all cross-cell reductions are
//! integer counts or fixed-order serial loops.

mod elevation;
mod keyframe;
/// Per-cell GLiM lithology classes (WO-0009 S2). Public: the terrain
/// stage's erodibility table and the app's Lithology layer index it.
pub mod lithology;
/// Committed plate-map metrics (WO-0003 Fix 2). Public forever: the CI gate
/// test and the acceptance harness call it.
pub mod metrics;
/// The t=0 plate generator (WO-0003 Fix 2 winner: retuned hybrid
/// growth+warp; judge record docs/plan/feel-pass-design/plate-judge-record.md).
/// Private since commit M3 — only setup.rs calls it.
mod plate_gen;
mod setup;
mod step;
/// WO-0009 water-inventory CI gates (conservation + highstand sign).
#[cfg(test)]
mod water_gates;

pub use keyframe::{
    ActiveRift, Keyframe, MicroplateOrigin, PairTimer, PlateState, RiftDriverKind, RunDiagnostics,
    SlabSegment, TectonicEvent, TectonicsHistory, Weld, NEVER_SUTURED,
};
pub use setup::CONT_AREA_FACTOR;
pub use step::{SimState, SLAB_DETACH_MY, SLAB_NONE};

use worldmaker_core::hash::{fnv1a_continue, FNV_OFFSET};

use crate::pipeline::{Cancelled, Stage, StageContext, WorldState};

pub const STAGE_ID: &str = "phase1-tectonics";

// ----- field names this stage writes -----
/// Shared with the Phase 0 noise stage: the canonical elevation field.
pub const ELEVATION_M: &str = "elevation_m";
pub const CRUST_THICKNESS_KM: &str = "crust_thickness_km";
pub const CRUST_AGE_MY: &str = "crust_age_my";
pub const OROGENY_AGE_MY: &str = "orogeny_age_my";
pub const RIFT_AGE_MY: &str = "rift_age_my";
pub const HOTSPOT_BUILDUP_KM: &str = "hotspot_buildup_km";
pub const PLATE_ID: &str = "plate_id";
/// 0 = oceanic, 1 = continental.
pub const CRUST_TYPE: &str = "crust_type";
pub const FEATURES: &str = "features";
/// GLiM lithology class per cell (WO-0009 S2), values from
/// [`lithology`]; stored as a u32 field, class values fit in u8.
pub const LITHOLOGY: &str = "lithology";

// ----- feature bits -----
pub const F_RIDGE: u32 = 1 << 0;
pub const F_TRENCH: u32 = 1 << 1;
pub const F_ARC: u32 = 1 << 2;
pub const F_HOTSPOT: u32 = 1 << 3;
pub const F_RIFT: u32 = 1 << 4;
/// Boundary-type display bits, recomputed every step.
pub const F_BND_DIVERGENT: u32 = 1 << 5;
pub const F_BND_CONVERGENT: u32 = 1 << 6;
pub const F_BND_TRANSFORM: u32 = 1 << 7;

/// Time step (My). Fixed by the spec; the step math assumes it is small
/// enough that per-step rotations stay in `det_sin_cos` range.
pub const DT_MY: f32 = 2.0;

/// Keyframe cadence (My): 10 My per the spec at L6/L7 — the levels the WO's
/// 1 GB / 2 Gy budget is defined for (527 MB measured) — 20 My at L8
/// (~1.06 GB at 2 Gy; recorded, not budgeted), and 100 My at L9: at
/// 16 B/cell a 2.62 M-cell keyframe is ~42 MB, so 20 My would cost 4.2 GB
/// over 2 Gy; 100 My keeps a maximum-span Ultra history at ~0.88 GB, the
/// same ballpark as L8. Histories carry their own interval, so mixed
/// cadences never confuse the era picker. Decision log 2026-08 (WO-0003).
pub fn keyframe_interval_my(grid_level: u32) -> f32 {
    if grid_level >= 9 {
        100.0
    } else if grid_level >= 8 {
        20.0
    } else {
        10.0
    }
}

/// User-facing tectonics parameters. All are hashed into `params_hash`, so
/// changing any of them re-runs the stage.
#[derive(Clone, Debug)]
pub struct TectonicsParams {
    /// Number of plates at setup (8–24).
    pub plate_count: u32,
    /// Target land fraction the sea-level solve enforces (0.05–0.7).
    pub land_fraction: f32,
    /// Scales plate speeds (0.25–2.0).
    pub tectonic_vigor: f32,
    /// Simulated span in My (200–2,000).
    pub span_my: f32,
    /// Fixed mantle hotspots (0–12).
    pub hotspot_count: u32,
    /// Craton paint overlay: sorted by cell id; +1 paints a continental
    /// nucleus, −1 forces ocean at setup.
    pub craton_overlay: Vec<(u32, i8)>,
    /// When set, replaces the generated hotspot set entirely.
    pub hotspot_overlay: Option<Vec<[f32; 3]>>,
}

impl Default for TectonicsParams {
    fn default() -> Self {
        TectonicsParams {
            plate_count: 12,
            land_fraction: 0.29,
            tectonic_vigor: 1.0,
            span_my: 500.0,
            hotspot_count: 6,
            craton_overlay: Vec::new(),
            hotspot_overlay: None,
        }
    }
}

impl TectonicsParams {
    pub fn clamped(mut self) -> Self {
        self.plate_count = self.plate_count.clamp(8, 24);
        self.land_fraction = self.land_fraction.clamp(0.05, 0.7);
        self.tectonic_vigor = self.tectonic_vigor.clamp(0.25, 2.0);
        self.span_my = self.span_my.clamp(200.0, 2000.0);
        self.hotspot_count = self.hotspot_count.min(12);
        self
    }
}

/// The tectonics stage. Owns nothing but its parameters; all state lives in
/// the run.
pub struct TectonicsStage {
    pub params: TectonicsParams,
}

impl TectonicsStage {
    pub fn new(params: TectonicsParams) -> Self {
        TectonicsStage {
            params: params.clamped(),
        }
    }
}

impl Stage for TectonicsStage {
    fn id(&self) -> &'static str {
        STAGE_ID
    }

    fn params_hash(&self) -> u64 {
        let p = &self.params;
        let mut h = FNV_OFFSET;
        h = fnv1a_continue(h, &p.plate_count.to_le_bytes());
        h = fnv1a_continue(h, &p.land_fraction.to_le_bytes());
        h = fnv1a_continue(h, &p.tectonic_vigor.to_le_bytes());
        h = fnv1a_continue(h, &p.span_my.to_le_bytes());
        h = fnv1a_continue(h, &p.hotspot_count.to_le_bytes());
        for (cell, v) in &p.craton_overlay {
            h = fnv1a_continue(h, &cell.to_le_bytes());
            h = fnv1a_continue(h, &v.to_le_bytes());
        }
        if let Some(hs) = &p.hotspot_overlay {
            h = fnv1a_continue(h, &[1u8]);
            for v in hs {
                h = fnv1a_continue(h, &v[0].to_le_bytes());
                h = fnv1a_continue(h, &v[1].to_le_bytes());
                h = fnv1a_continue(h, &v[2].to_le_bytes());
            }
        }
        h
    }

    fn run(&self, ctx: &StageContext, world: &mut WorldState) -> anyhow::Result<()> {
        let history = run_history(ctx, world, &self.params, None)?;

        // Publish the final keyframe into the world fields. The app pins a
        // different "present" by decoding that keyframe itself — a buffer
        // update, never a re-simulation.
        history
            .keyframes
            .last()
            .expect("history has at least the t=0 keyframe")
            .write_fields(&mut world.fields);
        world.history = Some(history);
        Ok(())
    }
}

/// A resume point for forward re-runs (plate drag, later branching): a
/// full-state keyframe plus the hotspot set of the history it came from.
pub struct ResumeFrom<'a> {
    pub keyframe: &'a Keyframe,
    pub hotspots: &'a [[f32; 3]],
}

/// Run the tectonic history: from setup (t = 0) when `resume` is `None`, or
/// forward from a full-state keyframe. Returns the keyframes this run
/// produced (on resume, the caller supplies the prefix).
pub fn run_history(
    ctx: &StageContext,
    world: &WorldState,
    params: &TectonicsParams,
    resume: Option<ResumeFrom<'_>>,
) -> anyhow::Result<TectonicsHistory> {
    let resumed = resume.is_some();
    let mut sim = match resume {
        None => SimState::setup(ctx.master_seed, &world.grid, params),
        Some(r) => SimState::from_keyframe(&world.grid, ctx.master_seed, r.hotspots, r.keyframe),
    };
    let kf_my = keyframe_interval_my(world.grid.level);
    let steps_per_keyframe = (kf_my / DT_MY) as u32;
    // Round the span to whole keyframes: steps past the last snapshot would
    // be simulated but never captured (review finding).
    let keyframe_count = ((params.span_my / kf_my).round() as u32).max(1);
    let total_steps = keyframe_count * steps_per_keyframe;
    let start_step = (sim.t_my / DT_MY) as u32;

    // At every keyframe the working state is round-tripped through the
    // keyframe quantization BEFORE the snapshot, so the snapshot IS the
    // state the run continues from — resume is bit-exact by construction.
    let mut keyframes = Vec::new();
    if !resumed {
        sim.quantize_state();
        elevation::derive_and_solve(&mut sim, params);
        keyframes.push(sim.encode_keyframe());
    }

    for step_idx in start_step..total_steps {
        if let Some(p) = &ctx.progress {
            if p.cancel_requested() {
                return Err(Cancelled.into());
            }
            p.set_fraction(step_idx as f32 / total_steps as f32);
        }
        sim.step(ctx.master_seed, step_idx);
        if (step_idx + 1) % steps_per_keyframe == 0 {
            sim.quantize_state();
            elevation::derive_and_solve(&mut sim, params);
            keyframes.push(sim.encode_keyframe());
        }
    }
    if let Some(p) = &ctx.progress {
        p.set_fraction(1.0);
    }

    Ok(TectonicsHistory {
        dt_my: DT_MY,
        keyframe_interval_my: kf_my,
        keyframes,
        hotspots: sim.hotspots.clone(),
        land_frac_target: params.land_fraction,
        achieved_land_frac: sim.achieved_land_frac,
        diagnostics: keyframe::RunDiagnostics {
            cont_lost_to_ridge_gap: sim.cont_lost_to_ridge_gap,
            cont_lost_to_consumption: sim.cont_lost_to_consumption,
            cont_lost_to_rift: sim.cont_lost_to_rift,
            cont_gained_by_advection: sim.cont_gained_by_advection,
            cont_gained_by_arc: sim.cont_gained_by_arc,
            cont_gained_by_closure: sim.cont_gained_by_closure,
            suture_count: sim.suture_count,
            breakup_count: sim.breakup_count,
            rift_start_count: sim.rift_start_count,
            rift_failed_count: sim.rift_failed_count,
            microplate_count: sim.microplate_count,
        },
    })
}
