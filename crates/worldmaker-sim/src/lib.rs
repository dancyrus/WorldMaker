//! worldmaker-sim: the staged simulation pipeline.
//!
//! Each stage is a pure function of (params, upstream fields, edit overlays),
//! cached by content hash, with dirty propagation downstream. Phase 0 ships the
//! scaffolding plus one placeholder stage (fractal-noise elevation) that Phase 1
//! replaces with real tectonics.

pub mod noise_stage;
pub mod pipeline;

pub use noise_stage::NoiseElevationStage;
pub use pipeline::{Pipeline, Stage, StageContext, WorldState};
