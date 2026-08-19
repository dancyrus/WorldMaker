//! Stage trait and the cache/dirty-propagation skeleton.
//!
//! Phase 0 runs a single placeholder stage, but the contract is the real one:
//! stages run in fixed order, each declares a content hash over everything
//! that can change its output, and a stage re-runs only when that hash
//! changes (or an upstream stage re-ran). Later phases add edit overlays and
//! keyframed history on the same skeleton.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use worldmaker_core::hash::{fnv1a_continue, FNV_OFFSET};
use worldmaker_core::{FieldStore, Grid};

/// Everything a stage may read besides the world itself.
pub struct StageContext {
    pub master_seed: u64,
    /// Progress/cancel channel for long stages; `None` for headless callers
    /// that don't need it.
    pub progress: Option<Arc<Progress>>,
}

impl StageContext {
    pub fn new(master_seed: u64) -> Self {
        StageContext {
            master_seed,
            progress: None,
        }
    }
}

/// Shared progress state between a running stage (worker thread) and the UI.
#[derive(Default)]
pub struct Progress {
    /// Completed fraction, stored as f32 bits.
    fraction: AtomicU32,
    cancel: AtomicBool,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_fraction(&self, f: f32) {
        self.fraction
            .store(f.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn fraction(&self) -> f32 {
        f32::from_bits(self.fraction.load(Ordering::Relaxed))
    }
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Error a stage returns when it stops because cancel was requested. The
/// pipeline records nothing for a failed stage, so a cancelled run never
/// poisons the cache. Detect it with `err.downcast_ref::<Cancelled>()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stage cancelled by user request")
    }
}

impl std::error::Error for Cancelled {}

static NEXT_WORLD_ID: AtomicU64 = AtomicU64::new(1);

/// The world being built: the grid plus all per-cell fields produced so far.
pub struct WorldState {
    pub grid: Arc<Grid>,
    pub fields: FieldStore,
    /// Keyframed tectonic history, filled by the tectonics stage. Boxed
    /// side-output rather than fields because it is time-indexed, not
    /// per-cell-at-present.
    pub history: Option<crate::tectonics::TectonicsHistory>,
    /// Unique per instance: cached stage outputs live in this world's fields,
    /// so the pipeline's cache is only valid against the same instance.
    id: u64,
}

impl WorldState {
    pub fn new(grid: Arc<Grid>) -> Self {
        let cell_count = grid.cell_count();
        WorldState {
            grid,
            fields: FieldStore::new(cell_count),
            history: None,
            id: NEXT_WORLD_ID.fetch_add(1, Ordering::Relaxed),
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
///
/// Cache entries describe fields written into the last `WorldState` this
/// pipeline ran against; handing in a different world invalidates everything
/// (the outputs live in the world, not in the pipeline).
pub struct Pipeline {
    slots: Vec<StageSlot>,
    last_world: Option<u64>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Pipeline {
    pub fn new() -> Self {
        Pipeline {
            slots: Vec::new(),
            last_world: None,
        }
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
        // A different WorldState instance means none of our cached outputs
        // exist in it — everything is dirty regardless of matching keys.
        let mut upstream_dirty = self.last_world != Some(world.id);
        self.last_world = Some(world.id);
        for slot in &mut self.slots {
            let mut key = upstream_key;
            key = fnv1a_continue(key, slot.stage.id().as_bytes());
            key = fnv1a_continue(key, &ctx.master_seed.to_le_bytes());
            key = fnv1a_continue(key, &world.grid.level.to_le_bytes());
            key = fnv1a_continue(key, &slot.stage.params_hash().to_le_bytes());
            let clean = slot.last_key == Some(key) && !upstream_dirty;
            if !clean {
                log::debug!("stage '{}' running (key {key:016x})", slot.stage.id());
                // Mark dirty before running: a failed (or cancelled) run may
                // have half-written the world's fields, so its previous cache
                // key must not survive the error.
                slot.last_key = None;
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

        let ctx = StageContext::new(42);
        pipe.run(&ctx, &mut world).unwrap();
        pipe.run(&ctx, &mut world).unwrap(); // clean: nothing re-runs
        assert_eq!(runs_a.load(Ordering::SeqCst), 1);
        assert_eq!(runs_b.load(Ordering::SeqCst), 1);

        // New seed dirties everything.
        let ctx2 = StageContext::new(43);
        pipe.run(&ctx2, &mut world).unwrap();
        assert_eq!(runs_a.load(Ordering::SeqCst), 2);
        assert_eq!(runs_b.load(Ordering::SeqCst), 2);

        // Invalidating a downstream stage re-runs only it.
        pipe.invalidate_from("b");
        pipe.run(&ctx2, &mut world).unwrap();
        assert_eq!(runs_a.load(Ordering::SeqCst), 2);
        assert_eq!(runs_b.load(Ordering::SeqCst), 3);
    }

    /// A failed (e.g. cancelled) run may have half-written the world, so it
    /// must leave the stage dirty even for the PREVIOUS cache key: complete
    /// with params A, fail with params B, then re-running with A again must
    /// re-execute the stage rather than serve the half-written B output.
    #[test]
    fn failed_run_dirties_the_stage() {
        struct FlakyStage {
            fail: StdArc<AtomicUsize>, // nonzero => fail
            runs: StdArc<AtomicUsize>,
        }
        impl Stage for FlakyStage {
            fn id(&self) -> &'static str {
                "flaky"
            }
            fn params_hash(&self) -> u64 {
                self.fail.load(Ordering::SeqCst) as u64 // params A=0, B=1
            }
            fn run(&self, _ctx: &StageContext, _world: &mut WorldState) -> anyhow::Result<()> {
                self.runs.fetch_add(1, Ordering::SeqCst);
                if self.fail.load(Ordering::SeqCst) != 0 {
                    anyhow::bail!(Cancelled);
                }
                Ok(())
            }
        }
        let grid = Arc::new(Grid::build(2));
        let mut world = WorldState::new(grid);
        let fail = StdArc::new(AtomicUsize::new(0));
        let runs = StdArc::new(AtomicUsize::new(0));
        let mut pipe = Pipeline::new();
        pipe.push(Box::new(FlakyStage {
            fail: fail.clone(),
            runs: runs.clone(),
        }));
        let ctx = StageContext::new(1);
        pipe.run(&ctx, &mut world).unwrap(); // params A, ok
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        fail.store(1, Ordering::SeqCst); // params B, will fail
        assert!(pipe.run(&ctx, &mut world).is_err());
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        fail.store(0, Ordering::SeqCst); // back to params A
        pipe.run(&ctx, &mut world).unwrap();
        assert_eq!(
            runs.load(Ordering::SeqCst),
            3,
            "stage must re-run after a failed run, even for the old key"
        );
    }

    /// The cache must not survive a change of WorldState: outputs live in the
    /// world, so a fresh world with an identical cache key still needs every
    /// stage to run.
    #[test]
    fn fresh_world_invalidates_cache() {
        let grid = Arc::new(Grid::build(2));
        let runs = StdArc::new(AtomicUsize::new(0));
        let mut pipe = Pipeline::new();
        pipe.push(Box::new(CountingStage {
            id: "a",
            params: 1,
            runs: runs.clone(),
        }));
        let ctx = StageContext::new(42);

        let mut world_a = WorldState::new(grid.clone());
        pipe.run(&ctx, &mut world_a).unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        // Same seed, same grid level, but a different world instance.
        let mut world_b = WorldState::new(grid);
        let ran = pipe.run(&ctx, &mut world_b).unwrap();
        assert_eq!(ran, vec!["a"], "stage must re-run against a fresh world");
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }
}
