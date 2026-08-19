//! Stage trait and the cache/dirty-propagation skeleton.
//!
//! Phase 0 runs a single placeholder stage, but the contract is the real one:
//! stages run in fixed order, each declares a content hash over everything
//! that can change its output, and a stage re-runs only when that hash
//! changes (or an upstream stage re-ran). Later phases add edit overlays and
//! keyframed history on the same skeleton.

use std::sync::Arc;

use worldmaker_core::hash::{fnv1a_continue, FNV_OFFSET};
use worldmaker_core::{FieldStore, Grid};

/// Everything a stage may read besides the world itself.
pub struct StageContext {
    pub master_seed: u64,
}

/// The world being built: the grid plus all per-cell fields produced so far.
pub struct WorldState {
    pub grid: Arc<Grid>,
    pub fields: FieldStore,
}

impl WorldState {
    pub fn new(grid: Arc<Grid>) -> Self {
        let cell_count = grid.cell_count();
        WorldState {
            grid,
            fields: FieldStore::new(cell_count),
        }
    }
}

/// One simulation stage. Implementations must be deterministic: fixed
/// iteration order, RNG only via `worldmaker_core::rng::sub_rng` keyed on the
/// stage id, no HashMap iteration, no fast-math.
pub trait Stage {
    /// Stable identifier; also the RNG sub-stream key. Never reuse across stages.
    fn id(&self) -> &'static str;

    /// Hash of this stage's own parameters. Combined with the master seed,
    /// grid level, and upstream hash to form the cache key.
    fn params_hash(&self) -> u64;

    /// Run the stage, reading and writing fields on `world`.
    fn run(&self, ctx: &StageContext, world: &mut WorldState) -> anyhow::Result<()>;
}

struct StageSlot {
    stage: Box<dyn Stage>,
    /// Cache key of the last completed run, if any.
    last_key: Option<u64>,
}

/// Fixed-order pipeline with per-stage cache keys and downstream dirtying.
pub struct Pipeline {
    slots: Vec<StageSlot>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Pipeline {
    pub fn new() -> Self {
        Pipeline { slots: Vec::new() }
    }

    /// Append a stage. Order of registration is execution order.
    pub fn push(&mut self, stage: Box<dyn Stage>) {
        self.slots.push(StageSlot {
            stage,
            last_key: None,
        });
    }

    /// Forget cached results for the named stage and everything after it.
    /// (Phase 6 painting will call this when an overlay changes.)
    pub fn invalidate_from(&mut self, stage_id: &str) {
        if let Some(pos) = self.slots.iter().position(|s| s.stage.id() == stage_id) {
            for slot in &mut self.slots[pos..] {
                slot.last_key = None;
            }
        }
    }

    /// Run all stages in order, skipping any whose cache key is unchanged.
    /// Returns the ids of the stages that actually ran.
    pub fn run(
        &mut self,
        ctx: &StageContext,
        world: &mut WorldState,
    ) -> anyhow::Result<Vec<&'static str>> {
        let mut ran = Vec::new();
        let mut upstream_key: u64 = FNV_OFFSET;
        let mut upstream_dirty = false;
        for slot in &mut self.slots {
            let mut key = upstream_key;
            key = fnv1a_continue(key, slot.stage.id().as_bytes());
            key = fnv1a_continue(key, &ctx.master_seed.to_le_bytes());
            key = fnv1a_continue(key, &world.grid.level.to_le_bytes());
            key = fnv1a_continue(key, &slot.stage.params_hash().to_le_bytes());
            let clean = slot.last_key == Some(key) && !upstream_dirty;
            if !clean {
                log::debug!("stage '{}' running (key {key:016x})", slot.stage.id());
                slot.stage.run(ctx, world)?;
                slot.last_key = Some(key);
                upstream_dirty = true;
                ran.push(slot.stage.id());
            }
            upstream_key = key;
        }
        Ok(ran)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;

    struct CountingStage {
        id: &'static str,
        params: u64,
        runs: StdArc<AtomicUsize>,
    }

    impl Stage for CountingStage {
        fn id(&self) -> &'static str {
            self.id
        }
        fn params_hash(&self) -> u64 {
            self.params
        }
        fn run(&self, _ctx: &StageContext, _world: &mut WorldState) -> anyhow::Result<()> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn cache_skips_clean_stages_and_dirt_propagates() {
        let grid = Arc::new(Grid::build(2));
        let mut world = WorldState::new(grid);
        let runs_a = StdArc::new(AtomicUsize::new(0));
        let runs_b = StdArc::new(AtomicUsize::new(0));
        let mut pipe = Pipeline::new();
        pipe.push(Box::new(CountingStage {
            id: "a",
            params: 1,
            runs: runs_a.clone(),
        }));
        pipe.push(Box::new(CountingStage {
            id: "b",
            params: 1,
            runs: runs_b.clone(),
        }));

        let ctx = StageContext { master_seed: 42 };
        pipe.run(&ctx, &mut world).unwrap();
        pipe.run(&ctx, &mut world).unwrap(); // clean: nothing re-runs
        assert_eq!(runs_a.load(Ordering::SeqCst), 1);
        assert_eq!(runs_b.load(Ordering::SeqCst), 1);

        // New seed dirties everything.
        let ctx2 = StageContext { master_seed: 43 };
        pipe.run(&ctx2, &mut world).unwrap();
        assert_eq!(runs_a.load(Ordering::SeqCst), 2);
        assert_eq!(runs_b.load(Ordering::SeqCst), 2);

        // Invalidating a downstream stage re-runs only it.
        pipe.invalidate_from("b");
        pipe.run(&ctx2, &mut world).unwrap();
        assert_eq!(runs_a.load(Ordering::SeqCst), 2);
        assert_eq!(runs_b.load(Ordering::SeqCst), 3);
    }
}
