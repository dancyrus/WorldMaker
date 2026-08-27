//! Pending-edit machinery for the app's brushes (WO-0003 Fix 1, Track A).
//!
//! Strokes accumulate here and NEVER launch a simulation. This module lives
//! in worldmaker-io, a crate with no dependency on the sim crate, so the
//! stroke-handling path has no route to any sim entry point — that crate
//! boundary is Fix 1's structural no-sim wall, enforced by the guard tests
//! below. The app folds the list into its staged overlay parameters only
//! when the user presses Regenerate.
//!
//! Determinism: fold output feeds the sim's hashed parameters, so every
//! angular threshold goes through `dmath::det_sin_cos` and every dot product
//! through `dmath::dot3` (fixed evaluation order). Replays are serial and
//! list-ordered; committed cell sets are id-sorted (BTreeSet); the BFS
//! `visited` HashSet is membership-only (output order comes from the queue,
//! which follows the grid's fixed CCW neighbor order).

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use worldmaker_core::{dmath, Grid};

use crate::strokes::{Stroke, StrokePayload, StrokeTool};

/// Hotspot tool: clicking within this range of an existing hotspot removes
/// it (moved here from the app; single source).
pub const HOTSPOT_REMOVE_KM: f32 = 300.0;
pub const EARTH_RADIUS_KM: f32 = 6371.0;

/// The craton stroke being drawn right now. One accumulator suffices for
/// both canvases: there is one pointer and egui tracks a drag on its
/// originating widget.
#[derive(Debug)]
struct LiveCraton {
    sign: i8,
    cells: BTreeSet<u32>,
}

/// The ordered pending-edit list (oldest first) plus the in-progress craton
/// stroke. Drawing mutates only this; the world regenerates when the app
/// folds the list at Regenerate.
#[derive(Debug, Default)]
pub struct PendingEdits {
    strokes: Vec<Stroke>,
    live: Option<LiveCraton>,
}

/// One frame of canvas input, extracted from `egui::Response` by the caller.
#[derive(Clone, Copy, Debug)]
pub struct StrokeEvent {
    pub hit_cell: Option<u32>,
    pub dragged: bool,
    pub clicked: bool,
    pub drag_stopped: bool,
}

/// The active brush, mapped from the app's tool selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActiveBrush {
    Craton { sign: i8, radius_km: f32 },
    Hotspot,
}

/// Fold result: plain data the app merges into its staged overlay params.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldOutcome {
    /// Final sign per touched cell, id-sorted (later strokes already won).
    /// Caller merges: `for (c, v) { craton_paint.insert(c, v) }`.
    pub craton_updates: Vec<(u32, i8)>,
    /// `Some(set)` = new absolute hotspot overlay; `None` = leave the param
    /// untouched (no hotspot strokes were pending).
    pub hotspot_overlay: Option<Vec<[f32; 3]>>,
}

/// Where an entry of the effective hotspot set came from (d1-F9 tagged
/// replay): index into the base set, or the pending stroke that added it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotspotOrigin {
    Base(usize),
    PendingAdd(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct TaggedHotspot {
    pub origin: HotspotOrigin,
    pub unit: [f32; 3],
}

/// A base spot deleted by a pending remove stroke.
#[derive(Clone, Copy, Debug)]
pub struct RemovedBase {
    /// Index (in the stroke list) of the remove stroke that deleted it.
    pub remove_stroke: usize,
    /// Index of the spot in the base set.
    pub base_index: usize,
    pub unit: [f32; 3],
}

/// Outcome of replaying the hotspot strokes over the base set, with origin
/// tags kept so a display layer can distinguish surviving pending adds from
/// deleted base spots. Classification, fold and display all share this one
/// replay (coherence rule, d1 §3/F9).
#[derive(Clone, Debug, Default)]
pub struct HotspotReplay {
    /// The effective set after all strokes, in base order then add order.
    pub working: Vec<TaggedHotspot>,
    /// Base spots deleted by pending removes, in remove-stroke order.
    pub removed_base: Vec<RemovedBase>,
}

impl PendingEdits {
    pub fn new() -> Self {
        Self::default()
    }

    /// Badge value: committed strokes only (the live stroke joins the count
    /// on stroke end).
    pub fn stroke_count(&self) -> usize {
        self.strokes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty() && self.live.is_none()
    }

    /// Any pending or live craton stroke.
    pub fn has_craton(&self) -> bool {
        self.live.is_some()
            || self
                .strokes
                .iter()
                .any(|s| matches!(s.payload, StrokePayload::CratonPaint { .. }))
    }

    pub fn has_hotspot(&self) -> bool {
        self.strokes.iter().any(|s| {
            matches!(
                s.payload,
                StrokePayload::HotspotAdd { .. } | StrokePayload::HotspotRemove { .. }
            )
        })
    }

    /// Commit the live stroke to the list (no-op when there is none or it is
    /// empty). Canonical craton form: cells id-sorted + deduplicated (the
    /// BTreeSet guarantees both), sign normalized to ±1.
    pub fn end_stroke(&mut self) {
        if let Some(live) = self.live.take() {
            if !live.cells.is_empty() {
                let sign: i8 = if live.sign >= 0 { 1 } else { -1 };
                let tool = if sign > 0 {
                    StrokeTool::CratonPaint
                } else {
                    StrokeTool::CratonErase
                };
                self.strokes.push(Stroke {
                    tool,
                    payload: StrokePayload::CratonPaint {
                        cells: live.cells.into_iter().collect(),
                        sign,
                    },
                });
            }
        }
    }

    /// Cancels the live stroke if any, else pops the newest stroke.
    /// Returns true if anything was removed. No redo.
    pub fn undo(&mut self) -> bool {
        if self.live.take().is_some() {
            return true;
        }
        self.strokes.pop().is_some()
    }

    /// Discard everything pending: the live stroke and the whole list.
    pub fn discard_all(&mut self) {
        self.live = None;
        self.strokes.clear();
    }

    /// Preset switch: drop the live stroke and every craton stroke (cell ids
    /// are per grid level), keep hotspot strokes (unit vectors survive).
    pub fn discard_cratons(&mut self) {
        self.live = None;
        self.strokes.retain(|s| {
            matches!(
                s.payload,
                StrokePayload::HotspotAdd { .. } | StrokePayload::HotspotRemove { .. }
            )
        });
    }

    /// "Reset to generated": drop pending hotspot strokes, keep craton
    /// strokes (committed and live).
    pub fn discard_hotspots(&mut self) {
        self.strokes
            .retain(|s| matches!(s.payload, StrokePayload::CratonPaint { .. }));
    }

    /// Regenerate: ends the live stroke, drains and returns the list.
    pub fn take_all(&mut self) -> Vec<Stroke> {
        self.end_stroke();
        std::mem::take(&mut self.strokes)
    }

    pub fn strokes(&self) -> &[Stroke] {
        &self.strokes
    }

    /// The committed list plus the live stroke synthesized as a stroke —
    /// what a display overlay should render mid-drag.
    pub fn display_strokes(&self) -> Vec<Stroke> {
        let mut out = self.strokes.clone();
        if let Some(live) = &self.live {
            if !live.cells.is_empty() {
                let sign: i8 = if live.sign >= 0 { 1 } else { -1 };
                out.push(Stroke {
                    tool: if sign > 0 {
                        StrokeTool::CratonPaint
                    } else {
                        StrokeTool::CratonErase
                    },
                    payload: StrokePayload::CratonPaint {
                        cells: live.cells.iter().copied().collect(),
                        sign,
                    },
                });
            }
        }
        out
    }
}

/// THE stroke input path. Both canvases route every tool frame through this
/// one function. Returns true when the visible overlay changed (the caller
/// re-bakes). By crate boundary it can only mutate `pending` — there is
/// nothing job-shaped to reach from here.
pub fn handle_stroke_event(
    pending: &mut PendingEdits,
    grid: &Grid,
    brush: &ActiveBrush,
    ev: &StrokeEvent,
    hotspot_base: Option<&[[f32; 3]]>,
) -> bool {
    match *brush {
        ActiveBrush::Craton { sign, radius_km } => {
            let mut changed = false;
            if let Some(hit) = ev.hit_cell {
                if ev.dragged || ev.clicked {
                    // Defensive: a sign flip mid-stroke (cannot happen
                    // through the UI) commits the live stroke first.
                    if pending.live.as_ref().is_some_and(|l| l.sign != sign) {
                        pending.end_stroke();
                    }
                    let live = pending.live.get_or_insert_with(|| LiveCraton {
                        sign,
                        cells: BTreeSet::new(),
                    });
                    for c in cells_within_radius(grid, hit, radius_km) {
                        changed |= live.cells.insert(c);
                    }
                }
            }
            // Hover-independent, checked outside the hit gate: a release past
            // the globe limb / projection outline still ends the stroke.
            if ev.drag_stopped || ev.clicked {
                pending.end_stroke();
            }
            changed
        }
        ActiveBrush::Hotspot => {
            // Click-only and atomic; recording is refused while no base set
            // exists (before the first run ever completes) so a click can
            // never blindly replace the generated set.
            let (Some(hit), true) = (ev.hit_cell, ev.clicked) else {
                return false;
            };
            let Some(base) = hotspot_base else {
                return false;
            };
            let effective = effective_hotspots(Some(base), &pending.strokes);
            let unit = grid.positions[hit as usize];
            let payload = if match_hotspot(&effective, unit).is_some() {
                StrokePayload::HotspotRemove { pos: unit }
            } else {
                StrokePayload::HotspotAdd { pos: unit }
            };
            pending.strokes.push(Stroke {
                tool: StrokeTool::Hotspot,
                payload,
            });
            true
        }
    }
}

/// Fold pending strokes for Regenerate. Pure data in, pure data out: craton
/// strokes reduce to a final per-cell sign (later strokes win); hotspot
/// strokes replay over the base set in list order. With no hotspot strokes
/// the hotspot param is left untouched (`None` here). A remove whose target
/// shifted away between click and fold (the base changed under it) deletes
/// nothing; a shifted base can likewise leave an add as a near-duplicate of
/// a base spot — both deterministic given (base, strokes), documented d1
/// §3/F6.
pub fn fold(strokes: &[Stroke], hotspot_base: Option<&[[f32; 3]]>) -> FoldOutcome {
    let mut craton: BTreeMap<u32, i8> = BTreeMap::new();
    let mut any_hotspot = false;
    for s in strokes {
        match &s.payload {
            StrokePayload::CratonPaint { cells, sign } => {
                for &c in cells {
                    craton.insert(c, *sign);
                }
            }
            StrokePayload::HotspotAdd { .. } | StrokePayload::HotspotRemove { .. } => {
                any_hotspot = true;
            }
        }
    }
    let hotspot_overlay = if any_hotspot {
        Some(effective_hotspots(hotspot_base, strokes))
    } else {
        None
    };
    FoldOutcome {
        craton_updates: craton.into_iter().collect(),
        hotspot_overlay,
    }
}

/// Base ⊕ pending hotspot strokes, replayed in list order with origin tags
/// (d1-F9). Adds append; a remove deletes its nearest match within
/// [`HOTSPOT_REMOVE_KM`] (ties → lowest index) — which may be an earlier
/// pending add — and an unmatched remove is a no-op. An absent base folds
/// from the empty set.
pub fn replay_hotspots(base: Option<&[[f32; 3]]>, strokes: &[Stroke]) -> HotspotReplay {
    let mut replay = HotspotReplay {
        working: base
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(|(i, &unit)| TaggedHotspot {
                origin: HotspotOrigin::Base(i),
                unit,
            })
            .collect(),
        removed_base: Vec::new(),
    };
    for (si, s) in strokes.iter().enumerate() {
        match &s.payload {
            StrokePayload::HotspotAdd { pos } => replay.working.push(TaggedHotspot {
                origin: HotspotOrigin::PendingAdd(si),
                unit: *pos,
            }),
            StrokePayload::HotspotRemove { pos } => {
                let units: Vec<[f32; 3]> = replay.working.iter().map(|t| t.unit).collect();
                if let Some(i) = match_hotspot(&units, *pos) {
                    let gone = replay.working.remove(i);
                    if let HotspotOrigin::Base(base_index) = gone.origin {
                        replay.removed_base.push(RemovedBase {
                            remove_stroke: si,
                            base_index,
                            unit: gone.unit,
                        });
                    }
                }
            }
            StrokePayload::CratonPaint { .. } => {}
        }
    }
    replay
}

/// The effective hotspot set (tags dropped). Shared by click classification,
/// fold, and display — one implementation, so what the user saw when
/// clicking is what the fold produces.
pub fn effective_hotspots(base: Option<&[[f32; 3]]>, strokes: &[Stroke]) -> Vec<[f32; 3]> {
    replay_hotspots(base, strokes)
        .working
        .into_iter()
        .map(|t| t.unit)
        .collect()
}

/// Nearest spot within [`HOTSPOT_REMOVE_KM`]; ties → lowest index. Scan
/// ascending, keep the strictly greatest dot, so the order is total and
/// deterministic.
pub fn match_hotspot(set: &[[f32; 3]], unit: [f32; 3]) -> Option<usize> {
    let (_, cos_thresh) = dmath::det_sin_cos(HOTSPOT_REMOVE_KM / EARTH_RADIUS_KM);
    let mut best: Option<(usize, f32)> = None;
    for (i, spot) in set.iter().enumerate() {
        let d = dmath::dot3(*spot, unit);
        if d >= cos_thresh && best.is_none_or(|(_, bd)| d > bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// All cells within the brush radius of a hit cell (neighbor flood over the
/// grid CSR). Radius ≤ 2000 km ⇒ angle ≤ 0.314 rad, inside det_sin_cos's
/// valid range.
fn cells_within_radius(grid: &Grid, center: u32, radius_km: f32) -> Vec<u32> {
    let (_, cos_thresh) = dmath::det_sin_cos(radius_km / EARTH_RADIUS_KM);
    let cpos = grid.positions[center as usize];
    let mut visited = HashSet::from([center]);
    let mut out = vec![center];
    let mut queue = VecDeque::from([center]);
    while let Some(c) = queue.pop_front() {
        for &nb in grid.neighbors_of(c) {
            if dmath::dot3(grid.positions[nb as usize], cpos) >= cos_thresh && visited.insert(nb) {
                out.push(nb);
                queue.push_back(nb);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid::build(3)
    }

    fn ev_drag(cell: u32) -> StrokeEvent {
        StrokeEvent {
            hit_cell: Some(cell),
            dragged: true,
            clicked: false,
            drag_stopped: false,
        }
    }

    fn ev_release() -> StrokeEvent {
        StrokeEvent {
            hit_cell: None,
            dragged: false,
            clicked: false,
            drag_stopped: true,
        }
    }

    fn ev_click(cell: u32) -> StrokeEvent {
        StrokeEvent {
            hit_cell: Some(cell),
            dragged: false,
            clicked: true,
            drag_stopped: false,
        }
    }

    const PAINT: ActiveBrush = ActiveBrush::Craton {
        sign: 1,
        radius_km: 300.0,
    };
    const ERASE: ActiveBrush = ActiveBrush::Craton {
        sign: -1,
        radius_km: 300.0,
    };

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

    /// A cell whose center is far (> 60°) from every given unit vector.
    fn far_cell(g: &Grid, from: &[[f32; 3]]) -> u32 {
        (0..g.cell_count())
            .find(|&c| {
                from.iter()
                    .all(|&u| dmath::dot3(g.positions[c as usize], u) < 0.5)
            })
            .expect("no far cell on the grid")
    }

    // ----- §5(d): the real-input-path test -----

    #[test]
    fn strokes_drive_pending_state_only() {
        let g = grid();
        let mut p = PendingEdits::new();
        // Drag: three hit frames, then a release off the map.
        for cell in [0u32, 1, 2] {
            assert!(handle_stroke_event(
                &mut p,
                &g,
                &PAINT,
                &ev_drag(cell),
                None
            ));
        }
        // Mid-drag the badge holds the pre-stroke value.
        assert_eq!(p.stroke_count(), 0);
        assert!(!p.is_empty());
        assert!(!handle_stroke_event(
            &mut p,
            &g,
            &PAINT,
            &ev_release(),
            None
        ));
        assert_eq!(p.stroke_count(), 1); // one stroke, not three
        match &p.strokes()[0].payload {
            StrokePayload::CratonPaint { cells, sign } => {
                assert_eq!(*sign, 1);
                let mut canon = cells.clone();
                canon.sort_unstable();
                canon.dedup();
                assert_eq!(*cells, canon, "committed cells are sorted + deduped");
                for c in [0u32, 1, 2] {
                    assert!(cells.contains(&c));
                }
            }
            other => panic!("expected a craton payload, got {other:?}"),
        }
        assert!(matches!(p.strokes()[0].tool, StrokeTool::CratonPaint));

        // Hotspot: refused with no base, recorded against an explicit base
        // (held as an Option, passed via as_deref — F7).
        let c0 = 0u32;
        let base: Option<Vec<[f32; 3]>> = Some(vec![g.positions[c0 as usize]]);
        let far = far_cell(&g, base.as_deref().unwrap());
        assert!(!handle_stroke_event(
            &mut p,
            &g,
            &ActiveBrush::Hotspot,
            &ev_click(far),
            None
        ));
        assert_eq!(p.stroke_count(), 1);
        assert!(handle_stroke_event(
            &mut p,
            &g,
            &ActiveBrush::Hotspot,
            &ev_click(far),
            base.as_deref()
        ));
        assert_eq!(p.stroke_count(), 2);
        assert!(p.has_hotspot());

        // Undo pops the newest; fold produces data only.
        assert!(p.undo());
        assert_eq!(p.stroke_count(), 1);
        let out = fold(p.strokes(), base.as_deref());
        assert!(out.hotspot_overlay.is_none());
        assert!(!out.craton_updates.is_empty());
    }

    #[test]
    fn craton_stroke_boundaries_survive_overlap() {
        let g = grid();
        let mut p = PendingEdits::new();
        // Stroke 1: paint at cell 5. Stroke 2: erase at cells 5 and 6 —
        // overlapping the first.
        assert!(handle_stroke_event(&mut p, &g, &PAINT, &ev_drag(5), None));
        handle_stroke_event(&mut p, &g, &PAINT, &ev_release(), None);
        assert!(handle_stroke_event(&mut p, &g, &ERASE, &ev_drag(5), None));
        assert!(handle_stroke_event(&mut p, &g, &ERASE, &ev_drag(6), None));
        handle_stroke_event(&mut p, &g, &ERASE, &ev_release(), None);
        assert_eq!(p.stroke_count(), 2);

        // Undo removes only the newer stroke; the older one is intact —
        // the per-stroke property a flat coverage map could never provide.
        assert!(p.undo());
        assert_eq!(p.stroke_count(), 1);
        let out = fold(p.strokes(), None);
        assert!(out.craton_updates.iter().any(|&(c, v)| c == 5 && v == 1));
        assert!(out.craton_updates.iter().all(|&(_, v)| v == 1));
    }

    #[test]
    fn undo_cancels_live_stroke_first() {
        let g = grid();
        let mut p = PendingEdits::new();
        // Committed stroke, then a live one.
        handle_stroke_event(&mut p, &g, &PAINT, &ev_drag(0), None);
        handle_stroke_event(&mut p, &g, &PAINT, &ev_release(), None);
        handle_stroke_event(&mut p, &g, &PAINT, &ev_drag(1), None);
        assert_eq!(p.stroke_count(), 1);
        assert!(p.undo()); // cancels the live stroke…
        assert_eq!(p.stroke_count(), 1); // …not the committed one
        assert!(p.undo()); // now pops the committed stroke
        assert!(p.is_empty());
        assert!(!p.undo());
    }

    #[test]
    fn discard_cratons_keeps_hotspot_strokes() {
        let g = grid();
        let mut p = PendingEdits::new();
        let base = vec![g.positions[0]];
        let far = far_cell(&g, &base);
        handle_stroke_event(&mut p, &g, &PAINT, &ev_click(2), None); // craton (click = begin+end)
        handle_stroke_event(
            &mut p,
            &g,
            &ActiveBrush::Hotspot,
            &ev_click(far),
            Some(&base),
        );
        handle_stroke_event(&mut p, &g, &PAINT, &ev_drag(3), None); // live craton
        assert!(p.has_craton() && p.has_hotspot());
        p.discard_cratons();
        assert!(!p.has_craton());
        assert!(p.has_hotspot());
        assert_eq!(p.stroke_count(), 1);
    }

    #[test]
    fn discard_hotspots_keeps_craton_strokes() {
        let g = grid();
        let mut p = PendingEdits::new();
        let base = vec![g.positions[0]];
        let far = far_cell(&g, &base);
        handle_stroke_event(&mut p, &g, &PAINT, &ev_click(2), None);
        handle_stroke_event(
            &mut p,
            &g,
            &ActiveBrush::Hotspot,
            &ev_click(far),
            Some(&base),
        );
        p.discard_hotspots();
        assert!(p.has_craton());
        assert!(!p.has_hotspot());
        assert_eq!(p.stroke_count(), 1);
    }

    #[test]
    fn discard_all_clears_everything() {
        let g = grid();
        let mut p = PendingEdits::new();
        handle_stroke_event(&mut p, &g, &PAINT, &ev_click(2), None);
        handle_stroke_event(&mut p, &g, &PAINT, &ev_drag(3), None); // live
        assert!(!p.is_empty());
        p.discard_all();
        assert!(p.is_empty());
        assert_eq!(p.stroke_count(), 0);
    }

    #[test]
    fn fold_craton_later_stroke_wins() {
        let strokes = [craton(&[0, 1], 1), craton(&[1, 2], -1)];
        let out = fold(&strokes, None);
        assert_eq!(out.craton_updates, vec![(0, 1), (1, -1), (2, -1)]);
        assert!(out.hotspot_overlay.is_none());
    }

    #[test]
    fn fold_hotspot_semantics() {
        let a = [1.0, 0.0, 0.0];
        let b = dmath::normalize3([1.0, 0.03, 0.0]); // ~0.03 rad (~191 km) from a
        let c = [0.0, 0.0, 1.0];
        let base = vec![a, b];

        // Add appends (order = stroke order).
        let out = fold(&[add(c)], Some(&base));
        assert_eq!(out.hotspot_overlay, Some(vec![a, b, c]));

        // Remove picks the NEAREST in range, not the first: a click between
        // a and b but closer to b deletes b.
        let click = dmath::normalize3([1.0, 0.02, 0.0]);
        let out = fold(&[remove(click)], Some(&base));
        assert_eq!(out.hotspot_overlay, Some(vec![a]));

        // Equal dots tie to the lowest index.
        let t0 = dmath::normalize3([1.0, 0.02, 0.0]);
        let t1 = dmath::normalize3([1.0, -0.02, 0.0]);
        let tie_base = vec![t0, t1];
        let out = fold(&[remove([1.0, 0.0, 0.0])], Some(&tie_base));
        assert_eq!(out.hotspot_overlay, Some(vec![t1]));

        // Remove deletes an earlier pending add when it is the nearest match.
        let out = fold(&[add(c), remove(c)], Some(&base));
        assert_eq!(out.hotspot_overlay, Some(vec![a, b]));

        // Unmatched remove is a no-op (e.g. the base shifted under a
        // recorded stroke).
        let out = fold(&[remove([0.0, 0.0, -1.0])], Some(&base));
        assert_eq!(out.hotspot_overlay, Some(vec![a, b]));

        // No hotspot strokes ⇒ the param is left untouched.
        assert!(fold(&[], Some(&base)).hotspot_overlay.is_none());

        // Absent base folds from the empty set: adds append, removes no-op.
        assert_eq!(fold(&[add(c)], None).hotspot_overlay, Some(vec![c]));
        assert_eq!(fold(&[remove(a)], None).hotspot_overlay, Some(vec![]));

        // F6: a shifted base can leave an add as a near-duplicate of a base
        // spot — accepted verbatim, deterministic.
        let near_a = dmath::normalize3([1.0, 0.01, 0.0]);
        let out = fold(&[add(near_a)], Some(&base));
        assert_eq!(out.hotspot_overlay, Some(vec![a, b, near_a]));
    }

    #[test]
    fn hotspot_click_classification_matches_fold() {
        let g = grid();
        let c0 = 0u32;
        let base = vec![g.positions[c0 as usize]];
        let far = far_cell(&g, &base);

        // Click the base spot (⇒ remove), then a far cell (⇒ add).
        let mut p = PendingEdits::new();
        handle_stroke_event(
            &mut p,
            &g,
            &ActiveBrush::Hotspot,
            &ev_click(c0),
            Some(&base),
        );
        handle_stroke_event(
            &mut p,
            &g,
            &ActiveBrush::Hotspot,
            &ev_click(far),
            Some(&base),
        );
        assert!(matches!(
            p.strokes()[0].payload,
            StrokePayload::HotspotRemove { .. }
        ));
        assert!(matches!(
            p.strokes()[1].payload,
            StrokePayload::HotspotAdd { .. }
        ));
        // Both strokes stay in the list; the badge counts 2.
        assert_eq!(p.stroke_count(), 2);

        // The fold replays to exactly the set the user saw while clicking.
        let out = fold(p.strokes(), Some(base.as_slice()));
        assert_eq!(
            out.hotspot_overlay,
            Some(vec![g.positions[far as usize]]),
            "classification and fold share one replay"
        );

        // F9: remove-of-base appears in removed_base with its base index.
        let replay = replay_hotspots(Some(&base), p.strokes());
        assert_eq!(replay.removed_base.len(), 1);
        assert_eq!(replay.removed_base[0].base_index, 0);
        assert_eq!(replay.removed_base[0].remove_stroke, 0);
        assert!(matches!(
            replay.working.as_slice(),
            [TaggedHotspot {
                origin: HotspotOrigin::PendingAdd(1),
                ..
            }]
        ));

        // F9: remove-of-pending-add — the add vanishes from the working set
        // and removed_base stays empty. Clicking a pending marker records a
        // remove, and the marker disappears from the preview.
        let mut p2 = PendingEdits::new();
        handle_stroke_event(
            &mut p2,
            &g,
            &ActiveBrush::Hotspot,
            &ev_click(far),
            Some(&base),
        );
        handle_stroke_event(
            &mut p2,
            &g,
            &ActiveBrush::Hotspot,
            &ev_click(far),
            Some(&base),
        );
        assert!(matches!(
            p2.strokes()[1].payload,
            StrokePayload::HotspotRemove { .. }
        ));
        let replay2 = replay_hotspots(Some(&base), p2.strokes());
        assert!(replay2.removed_base.is_empty());
        assert!(replay2
            .working
            .iter()
            .all(|t| !matches!(t.origin, HotspotOrigin::PendingAdd(_))));
        let out2 = fold(p2.strokes(), Some(base.as_slice()));
        assert_eq!(out2.hotspot_overlay, Some(base.clone()));
    }

    /// F2 (pinned): after "Reset to generated" (applied overlay = None, no
    /// job), the last-run set still anchors classification and fold — a
    /// hotspot stroke re-anchors to the still-displayed set, resurrecting
    /// every un-clicked spot at the next Regenerate. Reset holds only if no
    /// hotspot stroke follows before Regenerate.
    #[test]
    fn hotspot_stroke_after_reset_reanchors_to_visible_set() {
        let g = grid();
        let baseline = vec![g.positions[0], g.positions[200]];
        let far = far_cell(&g, &baseline);

        let mut p = PendingEdits::new();
        // The app passes overlay.or(baseline); after Reset the overlay is
        // None, so the base is the baseline.
        assert!(handle_stroke_event(
            &mut p,
            &g,
            &ActiveBrush::Hotspot,
            &ev_click(far),
            Some(&baseline)
        ));
        let out = fold(p.strokes(), Some(baseline.as_slice()));
        let set = out.hotspot_overlay.expect("a hotspot stroke was pending");
        assert_eq!(
            set,
            vec![baseline[0], baseline[1], g.positions[far as usize]],
            "the un-clicked baseline spots come back with the stroke"
        );
    }

    #[test]
    fn display_strokes_include_the_live_stroke() {
        let g = grid();
        let mut p = PendingEdits::new();
        handle_stroke_event(&mut p, &g, &PAINT, &ev_click(2), None);
        handle_stroke_event(&mut p, &g, &ERASE, &ev_drag(3), None); // live
        let shown = p.display_strokes();
        assert_eq!(shown.len(), 2);
        assert!(matches!(
            &shown[1].payload,
            StrokePayload::CratonPaint { sign: -1, .. }
        ));
        assert_eq!(p.stroke_count(), 1, "display never commits the live stroke");
    }

    // ----- serde: the frozen Stroke type round-trips exactly -----

    #[test]
    fn stroke_serde_round_trip() {
        let samples = [
            craton(&[1, 2, 40], 1),
            craton(&[7], -1),
            add([0.0, 1.0, 0.0]),
            remove([0.36, -0.48, 0.8]),
        ];
        for s in &samples {
            let json = serde_json::to_string(s).unwrap();
            let back: Stroke = serde_json::from_str(&json).unwrap();
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
    }

    /// Exact serialized strings pinned — the future save-format guard.
    #[test]
    fn stroke_json_shape_is_stable() {
        let cases = [
            (
                craton(&[1, 2], 1),
                r#"{"tool":"CratonPaint","payload":{"CratonPaint":{"cells":[1,2],"sign":1}}}"#,
            ),
            (
                craton(&[3], -1),
                r#"{"tool":"CratonErase","payload":{"CratonPaint":{"cells":[3],"sign":-1}}}"#,
            ),
            (
                add([0.0, 1.0, 0.0]),
                r#"{"tool":"Hotspot","payload":{"HotspotAdd":{"pos":[0.0,1.0,0.0]}}}"#,
            ),
            (
                remove([0.0, 0.0, -1.0]),
                r#"{"tool":"Hotspot","payload":{"HotspotRemove":{"pos":[0.0,0.0,-1.0]}}}"#,
            ),
        ];
        for (stroke, expected) in &cases {
            assert_eq!(&serde_json::to_string(stroke).unwrap(), expected);
        }
    }

    // ----- structural guards (D2 a + b) -----

    /// Cut every line at its first `//`: the guard is about code, and the
    /// needles may legitimately appear in prose (frozen doc text elsewhere
    /// names what it excludes).
    fn strip_line_comments(src: &str) -> String {
        src.lines()
            .map(|l| l.split("//").next().unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn pending_names_no_sim_machinery() {
        let src = strip_line_comments(include_str!("pending.rs"));
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
        ] {
            let needle = parts.concat();
            assert!(!src.contains(&needle), "pending.rs must not name {needle}");
        }
    }

    /// The crate-boundary no-sim wall: this crate's manifest never names the
    /// sim crate, so nothing in it can reach a sim entry point.
    #[test]
    fn io_manifest_never_names_the_sim_crate() {
        let manifest = include_str!("../Cargo.toml");
        let needle = ["worldmaker", "-sim"].concat();
        assert!(
            !manifest.contains(&needle),
            "worldmaker-io must not depend on {needle}"
        );
    }
}
