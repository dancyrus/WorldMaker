//! Pending-edit overlay — module OWNED BY TRACK A (WO-0003 Stage D,
//! feel-pass-design.md § D1). Track C created it with exactly these items and
//! a no-op `apply_overlay` body; Track A fills the behavior at rebase. The
//! item shapes and doc semantics below are FROZEN until then.
//!
//! Fix 1's structural no-sim guard: no route to `Pipeline`, `start_job`, or
//! `TectonicsParams` — none of those types appear in this module's API or
//! imports.

#![allow(dead_code)]

/// Overlay word layout (frozen):
///   bits 0..=3   tint code: 0 none, 1 craton +1, 2 craton −1,
///                3 hotspot existing marker, 4 pending hotspot add,
///                5 pending hotspot remove, 6..=15 reserved
///   bit  4       force-outline (outline this cell's region edge even where
///                the neighbor has the same tint code)
///   bits 5..=7   reserved, MUST be zero (renderer may assume so)
///   bits 8..=15  tint alpha 0..=255 (0 ⇒ renderer default 160)
///   bits 16..=31 reserved (zero)
/// Tint colors come from palette LUT row 5; tint code 5 has no texel — the
/// shader multiplies the row-5 code-6 HOTSPOT_MARK color by 0.55.
pub const OVERLAY_TINT_MASK: u32 = 0xF;
pub const OVERLAY_FORCE_OUTLINE: u32 = 1 << 4;
pub const OVERLAY_ALPHA_SHIFT: u32 = 8;
pub const OVERLAY_ALPHA_MASK: u32 = 0xFF << OVERLAY_ALPHA_SHIFT;

pub struct OverlayInput<'a> {
    pub grid: &'a worldmaker_core::Grid,
    /// Pending stroke list, oldest first — passed EXPLICITLY; the function
    /// must not read tool state, history, or WorldApp.
    pub pending: &'a [worldmaker_io::Stroke],
    /// Base hotspot set for rendering pending hotspot deltas; None mid-run
    /// (history dropped at job start) — adds/removes must still render.
    pub generated_hotspots: Option<&'a [[f32; 3]]>,
}

/// Fill `out` (len == grid.cell_count(), pre-zeroed by the caller) with
/// per-cell overlay words, newest stroke winning per cell. Pure function of
/// its arguments; cell ids >= out.len() are skipped silently (stale ids
/// across a level switch never panic); hotspot positions resolve to marker
/// cells via grid.nearest_cell + neighbor ring (today's marker shape).
/// No route to Pipeline, start_job, or TectonicsParams — none of those types
/// appear in this module's API or imports (Fix 1's structural no-sim guard).
pub fn apply_overlay(input: &OverlayInput<'_>, out: &mut [u32]) {
    // NO-OP until Track A's rebase: renders nothing, leaves `out` untouched.
    let _ = (input, out);
}
