//! worldmaker-core: geodesic grid, seeded RNG, field storage, and projection math.
//!
//! This crate depends on nothing else in the workspace. Everything here is pure
//! CPU math with a fixed iteration order, so results are bit-identical across
//! runs and platforms (transcendental functions are kept out of anything that
//! feeds a determinism hash).

pub mod fields;
pub mod grid;
pub mod hash;
pub mod proj;
pub mod rng;

pub use fields::FieldStore;
pub use grid::Grid;
pub use proj::Projection;
