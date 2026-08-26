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
    use std::sync::Arc;

    use worldmaker_core::hash::{hash_f32_slice, hash_u32_slice};
    use worldmaker_core::Grid;
    use worldmaker_sim::tectonics::{CRUST_TYPE, ELEVATION_M, PLATE_ID};
    use worldmaker_sim::TectonicsParams;

    /// Exhaustive 7-field struct literal (judgement graft 4): no
    /// `..Default`, so any field added to `TectonicsParams` breaks this
    /// build and forces a review of the sim surface.
    fn exhaustive_default_params() -> TectonicsParams {
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

    /// The render-only guard (feel-pass-design.md § D4, d3a §11): Detail and
    /// sea level terminate in `pack_shade_params` (the real uniform-packing
    /// helper both canvases use) while the world is a pure function of
    /// (grid, seed, params) through the SAME `build_world` path Regenerate
    /// uses. Two extreme shade packings differ; two world builds do not —
    /// neither in params_hash nor in any committed field hash.
    #[test]
    fn detail_and_sea_level_cannot_touch_the_world() {
        let grid = Arc::new(Grid::build(5));
        let cells = grid.cell_count();

        let u_lo = crate::render::pack_shade_params(42, 0, 5, -4000.0, 0.0, cells);
        let u_hi = crate::render::pack_shade_params(42, 0, 5, 4000.0, 220.0, cells);
        assert_ne!(
            bytemuck::bytes_of(&u_lo),
            bytemuck::bytes_of(&u_hi),
            "the render path DID change between the two settings"
        );

        let (w1, h1) =
            super::build_world(grid.clone(), 42, exhaustive_default_params(), None).unwrap();
        let (w2, h2) = super::build_world(grid, 42, exhaustive_default_params(), None).unwrap();
        assert_eq!(h1, h2, "params_hash moved between identical builds");
        assert_eq!(
            hash_f32_slice(w1.fields.get(ELEVATION_M).unwrap()),
            hash_f32_slice(w2.fields.get(ELEVATION_M).unwrap()),
            "committed elevation field moved"
        );
        assert_eq!(
            hash_u32_slice(w1.fields.get_u32(PLATE_ID).unwrap()),
            hash_u32_slice(w2.fields.get_u32(PLATE_ID).unwrap()),
            "committed plate-id field moved"
        );
        assert_eq!(
            hash_u32_slice(w1.fields.get_u32(CRUST_TYPE).unwrap()),
            hash_u32_slice(w2.fields.get_u32(CRUST_TYPE).unwrap()),
            "committed crust-type field moved"
        );
    }

    /// The sim-params surface is pinned two ways (feel-pass-design.md § D4,
    /// judgement graft 4 + d3a §11.3): the exhaustive struct literal above,
    /// and a name check so no render-shaped parameter (detail, render,
    /// octaves, noise, amplitude) can appear on the sim surface unnoticed.
    #[test]
    fn sim_exposes_no_render_detail_parameter() {
        let params = exhaustive_default_params();
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
