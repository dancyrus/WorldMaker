//! worldmaker-io: results-JSON writing now; save/load and export stubs for
//! later phases.

pub mod results;
pub mod save;
pub mod strokes;

pub use results::ResultsFile;
pub use strokes::{Stroke, StrokePayload, StrokeTool};
