//! The world-building recipe, factored out of `start_job`'s worker thread so
//! the UI's Regenerate path and headless tests share ONE path
//! (feel-pass-design.md § D4, d3a §11).

use std::sync::Arc;

use worldmaker_core::Grid;
use worldmaker_sim::{
    Pipeline, Progress, Stage, StageContext, TectonicsParams, TectonicsStage, WorldState,
};

/// The one world-building recipe. `start_job` calls this on its worker
/// thread; the guard test calls it headlessly. Render-only state (Detail,
/// sea level, layer, projection, debug flags) is structurally absent from
/// the signature — that absence IS the guard.
pub fn build_world(
    grid: Arc<Grid>,
    master_seed: u64,
    params: TectonicsParams,
    progress: Option<Arc<Progress>>,
) -> anyhow::Result<(WorldState, u64 /* params_hash */)> {
    let stage = TectonicsStage::new(params);
    let params_hash = stage.params_hash();
    let mut world = WorldState::new(grid);
    let mut pipeline = Pipeline::new();
    pipeline.push(Box::new(stage));
    let mut ctx = StageContext::new(master_seed);
    ctx.progress = progress;
    pipeline.run(&ctx, &mut world)?;
    Ok((world, params_hash))
}

#[cfg(test)]
mod tests {
    use worldmaker_sim::TectonicsParams;

    /// The sim-params surface is pinned two ways (feel-pass-design.md § D4,
    /// judgement graft 4 + d3a §11.3): an exhaustive 7-field struct literal
    /// (no `..Default`) so any field added to `TectonicsParams` breaks this
    /// build, and a name check so no render-shaped parameter (detail, render,
    /// octaves, noise, amplitude) can appear on the sim surface unnoticed.
    #[test]
    fn sim_exposes_no_render_detail_parameter() {
        let params = TectonicsParams {
            plate_count: 12,
            land_fraction: 0.29,
            tectonic_vigor: 1.0,
            span_my: 500.0,
            hotspot_count: 6,
            craton_overlay: Vec::new(),
            hotspot_overlay: None,
        };
        let dbg = format!("{params:?}");
        for field in [
            "plate_count",
            "land_fraction",
            "tectonic_vigor",
            "span_my",
            "hotspot_count",
            "craton_overlay",
            "hotspot_overlay",
        ] {
            assert!(
                dbg.contains(field),
                "field {field} missing from TectonicsParams Debug output: {dbg}"
            );
        }
        let lower = dbg.to_lowercase();
        for needle in ["detail", "render", "octav", "noise", "amp"] {
            assert!(
                !lower.contains(needle),
                "render-shaped name {needle:?} found on the sim params surface: {dbg}"
            );
        }
    }
}
