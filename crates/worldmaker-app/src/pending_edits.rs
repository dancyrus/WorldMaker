//! Pending-edit overlay — module OWNED BY TRACK A (WO-0003 Stage D,
//! feel-pass-design.md § D1). Track C created it with exactly these items and
//! a no-op `apply_overlay` body; Track A fills the behavior at rebase. The
//! item shapes and doc semantics below are FROZEN until then.
//!
//! Fix 1's structural no-sim guard: no route to `Pipeline`, `start_job`, or
//! `TectonicsParams` — none of those types appear in this module's API or
//! imports.

#![allow(dead_code)]

use std::collections::BTreeMap;

use worldmaker_io::pending::{replay_hotspots, HotspotOrigin};
use worldmaker_io::StrokePayload;

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

/// Tint codes of the frozen word layout above.
const TINT_CRATON_PAINT: u32 = 1;
const TINT_CRATON_ERASE: u32 = 2;
const TINT_HOTSPOT_ADD: u32 = 4;
const TINT_HOTSPOT_REMOVE: u32 = 5;

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
    // The shared tagged replay (worldmaker_io::pending, d1-F9) keeps display
    // coherent with click classification and fold: a remove that matched a
    // pending add erases that add's code-4 marker (the add is absent from
    // `working`); a remove that matched a base spot lands in `removed_base`
    // and renders code 5 on that spot's marker cells.
    let replay = replay_hotspots(input.generated_hotspots, input.pending);
    let mut surviving_adds: BTreeMap<usize, [f32; 3]> = BTreeMap::new();
    for t in &replay.working {
        if let HotspotOrigin::PendingAdd(si) = t.origin {
            surviving_adds.insert(si, t.unit);
        }
    }
    let mut base_removals: BTreeMap<usize, [f32; 3]> = BTreeMap::new();
    for r in &replay.removed_base {
        base_removals.insert(r.remove_stroke, r.unit);
    }

    // Replay in list order so the newest stroke wins per cell.
    for (si, stroke) in input.pending.iter().enumerate() {
        match &stroke.payload {
            StrokePayload::CratonPaint { cells, sign } => {
                let code = if *sign >= 0 {
                    TINT_CRATON_PAINT
                } else {
                    TINT_CRATON_ERASE
                };
                for &c in cells {
                    if let Some(w) = out.get_mut(c as usize) {
                        *w = code;
                    }
                }
            }
            StrokePayload::HotspotAdd { .. } => {
                if let Some(unit) = surviving_adds.get(&si) {
                    write_marker(input.grid, *unit, TINT_HOTSPOT_ADD, out);
                }
            }
            StrokePayload::HotspotRemove { .. } => {
                if let Some(unit) = base_removals.get(&si) {
                    write_marker(input.grid, *unit, TINT_HOTSPOT_REMOVE, out);
                }
            }
        }
    }
}

/// Today's hotspot marker shape (a-report §7): the nearest cell plus its
/// neighbor ring. Alpha bits stay zero ⇒ renderer default.
fn write_marker(grid: &worldmaker_core::Grid, unit: [f32; 3], code: u32, out: &mut [u32]) {
    let c = grid.nearest_cell(unit, None);
    if let Some(w) = out.get_mut(c as usize) {
        *w = code;
    }
    for &nb in grid.neighbors_of(c) {
        if let Some(w) = out.get_mut(nb as usize) {
            *w = code;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldmaker_core::Grid;
    use worldmaker_io::{Stroke, StrokeTool};

    fn grid() -> Grid {
        Grid::build(3)
    }

    fn zeroed(grid: &Grid) -> Vec<u32> {
        vec![0; grid.cell_count() as usize]
    }

    fn craton(cells: &[u32], sign: i8) -> Stroke {
        Stroke {
            tool: if sign > 0 {
                StrokeTool::CratonPaint
            } else {
                StrokeTool::CratonErase
            },
            payload: StrokePayload::CratonPaint {
                cells: cells.to_vec(),
                sign,
            },
        }
    }

    fn add(pos: [f32; 3]) -> Stroke {
        Stroke {
            tool: StrokeTool::Hotspot,
            payload: StrokePayload::HotspotAdd { pos },
        }
    }

    fn remove(pos: [f32; 3]) -> Stroke {
        Stroke {
            tool: StrokeTool::Hotspot,
            payload: StrokePayload::HotspotRemove { pos },
        }
    }

    fn marker_cells(g: &Grid, unit: [f32; 3]) -> Vec<u32> {
        let c = g.nearest_cell(unit, None);
        let mut cells = vec![c];
        cells.extend_from_slice(g.neighbors_of(c));
        cells
    }

    #[test]
    fn craton_codes_and_newest_stroke_wins() {
        let g = grid();
        let mut out = zeroed(&g);
        let strokes = [craton(&[5, 7], 1), craton(&[7, 9], -1)];
        apply_overlay(
            &OverlayInput {
                grid: &g,
                pending: &strokes,
                generated_hotspots: None,
            },
            &mut out,
        );
        assert_eq!(out[5], 1);
        assert_eq!(out[7], 2, "the newer stroke wins the shared cell");
        assert_eq!(out[9], 2);
        assert_eq!(out[6], 0);
    }

    #[test]
    fn stale_cell_ids_are_skipped_silently() {
        let g = grid();
        let mut out = zeroed(&g);
        let stale = g.cell_count() + 1000;
        let strokes = [craton(&[3, stale], 1)];
        apply_overlay(
            &OverlayInput {
                grid: &g,
                pending: &strokes,
                generated_hotspots: None,
            },
            &mut out,
        );
        assert_eq!(out[3], 1);
    }

    #[test]
    fn hotspot_add_renders_marker_even_without_base() {
        let g = grid();
        let mut out = zeroed(&g);
        let unit = g.positions[100];
        let strokes = [add(unit)];
        // Base None (mid-run): adds must still render.
        apply_overlay(
            &OverlayInput {
                grid: &g,
                pending: &strokes,
                generated_hotspots: None,
            },
            &mut out,
        );
        for c in marker_cells(&g, unit) {
            assert_eq!(out[c as usize], 4);
        }
    }

    #[test]
    fn remove_of_base_spot_renders_code_5_on_its_marker() {
        let g = grid();
        let mut out = zeroed(&g);
        let base_unit = g.positions[0];
        let base = [base_unit];
        let strokes = [remove(base_unit)];
        apply_overlay(
            &OverlayInput {
                grid: &g,
                pending: &strokes,
                generated_hotspots: Some(&base),
            },
            &mut out,
        );
        for c in marker_cells(&g, base_unit) {
            assert_eq!(out[c as usize], 5);
        }
    }

    #[test]
    fn remove_of_pending_add_erases_its_marker() {
        let g = grid();
        let mut out = zeroed(&g);
        let unit = g.positions[100];
        let strokes = [add(unit), remove(unit)];
        apply_overlay(
            &OverlayInput {
                grid: &g,
                pending: &strokes,
                generated_hotspots: None,
            },
            &mut out,
        );
        assert!(
            out.iter().all(|&w| w == 0),
            "an erased pending add renders nothing"
        );
    }

    #[test]
    fn newer_hotspot_marker_wins_over_older_craton_tint() {
        let g = grid();
        let mut out = zeroed(&g);
        let unit = g.positions[50];
        let center = g.nearest_cell(unit, None);
        let strokes = [craton(&[center], 1), add(unit)];
        apply_overlay(
            &OverlayInput {
                grid: &g,
                pending: &strokes,
                generated_hotspots: None,
            },
            &mut out,
        );
        assert_eq!(out[center as usize], 4, "the newer stroke wins the cell");
    }

    /// D2 guard (b) for this module: the code (comments stripped — the
    /// frozen doc text above legitimately names what it excludes) never
    /// names sim machinery.
    #[test]
    fn pending_edits_names_no_sim_machinery() {
        let src: String = include_str!("pending_edits.rs")
            .lines()
            .map(|l| l.split("//").next().unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n");
        // Needles assembled at runtime so this test's own source never
        // matches.
        for parts in [
            ["worldmaker", "_sim"],
            ["Pipe", "line"],
            ["start", "_job"],
            ["Tectonics", "Params"],
            ["Sim", "Job"],
            ["thread::", "spawn"],
            ["mp", "sc"],
            // The app's own sim entry points: this module lives in a crate
            // that CAN link the sim, so the wall here is lexical only.
            ["build", "_world"],
            ["world", "gen"],
        ] {
            let needle = parts.concat();
            assert!(
                !src.contains(&needle),
                "pending_edits.rs must not name {needle}"
            );
        }
    }
}
