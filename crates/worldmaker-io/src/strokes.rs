//! Pending-edit stroke types — the A↔C frozen interface (WO-0003 Stage D,
//! feel-pass-design.md § D1). Shape FROZEN; behavior and impls are Track A's.
//! Neither track changes these shapes until Track A's rebase.

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StrokeTool {
    CratonPaint,
    CratonErase,
    Hotspot,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum StrokePayload {
    /// Cell ids at the current grid level; sign = +1 paint continent, −1 force ocean.
    CratonPaint { cells: Vec<u32>, sign: i8 },
    /// Unit-vector position.
    HotspotAdd { pos: [f32; 3] },
    /// Unit-vector position the removal targets.
    HotspotRemove { pos: [f32; 3] },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Stroke {
    pub tool: StrokeTool,
    pub payload: StrokePayload,
}
