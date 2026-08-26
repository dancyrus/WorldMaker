//! Plate-boundary smoothed polylines (WO-0003 Fix 3, d3a §8).
//!
//! Leg 2 ships only the types so `WorldBundle`'s field list is final; leg 3
//! fills in the chain extraction (mixed-triangle nodes, boundary-crossing
//! links, type-split chains, Chaikin ×2 on the sphere) and the ribbon
//! pipelines that draw them on both canvases.

#![allow(dead_code)] // read by leg 3's extraction + ribbon pipelines

/// One smoothed boundary polyline; `btype` is the boundary code (1 trench /
/// convergent, 2 ridge / divergent, 3 transform), `pts` unit vectors.
pub struct BoundaryChain {
    pub btype: u8,
    pub pts: Vec<[f32; 3]>,
}

/// All boundary chains for the viewed keyframe; empty when the layer draws
/// none.
pub struct BoundarySet {
    pub chains: Vec<BoundaryChain>,
}

impl BoundarySet {
    pub fn empty() -> BoundarySet {
        BoundarySet { chains: Vec::new() }
    }
}
