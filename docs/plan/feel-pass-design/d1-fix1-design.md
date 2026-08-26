# Stage D design — Fix 1 (Track A): pending edits, strokes, fold

Complete design for the Fix 1 interaction contract. Implementers code against
this document plus the pinned contracts in `../feel-pass-design.md`. Line
references are against main @ 9d5d272. No code was changed in Stage D.

Scope recap (pinned): strokes never trigger simulation; pending edits are an
ordered stroke list with badge, Cmd/Ctrl+Z, Discard; Regenerate folds into
`TectonicsParams`; preset switch discards pending craton strokes and keeps
pending hotspot strokes; seed change keeps everything; pending strokes render
tint + outline through the frozen A↔C interface; the Stroke type is serde,
in worldmaker-io; pure view controls stay live.

---

## 1. The Stroke type (worldmaker-io)

**File: `crates/worldmaker-io/src/stroke.rs`** (new module beside `save.rs`;
`save.rs` itself stays untouched — stubs stay stubs). `lib.rs` gains
`pub mod stroke;` and `pub use stroke::{Stroke, StrokePayload, StrokeTool};`.
worldmaker-io already depends on serde; worldmaker-app already depends on
worldmaker-io (Cargo.toml:12) — **no new dependency edges**.

```rust
use serde::{Deserialize, Serialize};

/// One pending edit. The contract's { tool, payload } pair: the tool is
/// stored implicitly as the payload discriminant (see `tool()`), so an
/// inconsistent tool/payload combination is unrepresentable.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Stroke {
    pub payload: StrokePayload,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum StrokePayload {
    /// Craton brush/eraser stroke. `cells` are grid cell ids at the level the
    /// stroke was painted on, sorted ascending, deduplicated (canonical
    /// form). `sign` is +1 (paint continent) or -1 (force ocean).
    CratonPaint { cells: Vec<u32>, sign: i8 },
    /// Add a hotspot at this unit vector (the clicked cell's center,
    /// `grid.positions[cell]`).
    HotspotAdd { unit: [f32; 3] },
    /// Remove the hotspot nearest this unit vector (matching rule in §3).
    HotspotRemove { unit: [f32; 3] },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeTool {
    Craton,
    Hotspot,
}

impl Stroke {
    /// Canonicalizes: sorts + dedups cells; debug_asserts sign is ±1
    /// (release builds normalize: sign >= 0 → +1, else −1).
    pub fn craton(cells: Vec<u32>, sign: i8) -> Self { /* … */ }
    pub fn hotspot_add(unit: [f32; 3]) -> Self { /* … */ }
    pub fn hotspot_remove(unit: [f32; 3]) -> Self { /* … */ }
    pub fn tool(&self) -> StrokeTool { /* discriminant map */ }
}
```

Decisions, with reasons:

- **`tool` is an accessor, not a stored field.** The pinned "Stroke = { tool,
  payload }" is satisfied semantically; a serialized duplicate field could
  desync from the payload (`tool: Hotspot` + `CratonPaint` payload). The enum
  discriminant IS the tool.
- **`Stroke` stays a wrapper struct** (not `#[serde(transparent)]`): Phase 6
  save files can add fields (grid level, timestamp) additively without a
  format break. Serialized form is externally tagged serde default, e.g.
  `{"payload":{"CratonPaint":{"cells":[1,2],"sign":1}}}` — pinned by a JSON
  shape test (§8).
- **`cells` canonical form: sorted ascending + deduped.** Undo pops whole
  strokes, so intra-stroke paint order carries no information; the canonical
  form makes serde output and fold input deterministic regardless of drag
  path, and matches `craton_overlay`'s "sorted by cell id" doc
  (tectonics/mod.rs:85–87).
- Payload matches how `TectonicsParams` stores each overlay today (pinned):
  craton = cell ids + i8 sign; hotspot = `[f32; 3]` unit positions.

### Per-stroke capture (replaces `craton_stroke_dirty`)

Today's single bool (app.rs:139) and the absorbed `BTreeMap::insert` cannot
reproduce stroke boundaries (a-report §8). Capture design:

- `PendingEdits` (§2) holds `live: Option<LiveCraton>` where
  `struct LiveCraton { sign: i8, cells: BTreeSet<u32> }` — the stroke being
  drawn right now. One accumulator suffices for both canvases: there is one
  pointer, egui tracks a drag on its originating widget, and each canvas's
  `Response` fires its own `drag_stopped()`.
- A stroke **starts** at the first brush application (`dragged() || clicked()`
  with a hit) when `live` is `None` — `Response::drag_started()` stays unused;
  first-application start is equivalent and also covers click-only dabs.
  Every subsequent hit frame merges `cells_within_radius(...)` into
  `live.cells`.
- A stroke **ends** on `drag_stopped() || clicked()`, checked outside the hit
  gate exactly like today (app.rs:551–558) so a release past the globe limb /
  projection outline still ends the stroke (preserves the committed review
  finding). End = `Stroke::craton(live.cells, live.sign)` pushed onto the
  pending list; empty live strokes are dropped. A click is begin+end in the
  same frame (hit block runs first, end check after — same order as today).
- Defensive: if a brush event arrives while `live` has a different sign
  (cannot happen through the UI — tool switches need the pointer), the live
  stroke is committed first, then a new one begins.
- Hotspot strokes are click-only and atomic: classification (§3) pushes a
  one-item `HotspotAdd`/`HotspotRemove` stroke immediately; no live state.
- `end_stroke()` is also called (commit-if-nonempty, else no-op) on: tool
  switch (side_panel), seed Generate, Regenerate, and inside
  `discard_cratons()`/`discard_all()` (which cancel the live stroke instead).
  No paint is ever silently half-recorded.

`craton_stroke_dirty` is **deleted**.

---

## 2. The `pending_edits` module (worldmaker-app)

**File: `crates/worldmaker-app/src/pending_edits.rs`** (new; `mod
pending_edits;` in main.rs — a one-line main.rs edit A is allowed since module
registration is not script surface; coordinate textually at rebase). The
module imports **only** `std`, `worldmaker_core` (Grid, dmath), and
`worldmaker_io::stroke`. It never imports `worldmaker_sim`, egui, or anything
from `app.rs`/`render.rs` — this is the substrate of the structural guard
(§5).

```rust
use worldmaker_core::grid::Grid;
use worldmaker_io::stroke::{Stroke, StrokePayload, StrokeTool};

/// Hotspot tool: clicking within this range of an existing hotspot removes
/// it. Moved here from app.rs (single source; app.rs deletes its copy).
pub const HOTSPOT_REMOVE_KM: f32 = 300.0;
pub const EARTH_RADIUS_KM: f32 = 6371.0;

pub struct PendingEdits {
    strokes: Vec<Stroke>,        // ordered, oldest first
    live: Option<LiveCraton>,    // in-progress craton stroke
}

/// One frame of canvas input, extracted from egui::Response by the caller.
pub struct StrokeEvent {
    pub hit_cell: Option<u32>,
    pub dragged: bool,
    pub clicked: bool,
    pub drag_stopped: bool,
}

pub enum ActiveBrush {
    Craton { sign: i8, radius_km: f32 },
    Hotspot,
}

impl PendingEdits {
    pub fn new() -> Self;
    pub fn stroke_count(&self) -> usize;      // badge value = strokes.len()
    pub fn is_empty(&self) -> bool;           // strokes empty AND live none
    pub fn has_craton(&self) -> bool;         // any pending or live craton
    pub fn has_hotspot(&self) -> bool;
    pub fn end_stroke(&mut self);             // commit live → strokes
    /// Cancels the live stroke if any, else pops the newest stroke.
    /// Returns true if anything was removed.
    pub fn undo(&mut self) -> bool;
    pub fn discard_all(&mut self);            // live + all strokes
    /// Preset switch: drop live + CratonPaint strokes, keep hotspot strokes.
    pub fn discard_cratons(&mut self);
    /// Regenerate: ends the live stroke, drains and returns the list.
    pub fn take_all(&mut self) -> Vec<Stroke>;
    pub fn strokes(&self) -> &[Stroke];
    /// Display view for the frozen A↔C interface (§7).
    pub fn overlay_view(&self, hotspot_base: Option<&[[f32; 3]]>)
        -> PendingOverlayView;
}

/// THE stroke input path. Both canvases route every tool frame through this
/// one function. Returns true when the visible overlay changed (caller sets
/// needs_bake). It can only mutate `pending` — it has no access, by module
/// dependency, to any job or sim machinery.
pub fn handle_stroke_event(
    pending: &mut PendingEdits,
    grid: &Grid,
    brush: &ActiveBrush,
    ev: &StrokeEvent,
    hotspot_base: Option<&[[f32; 3]]>,
) -> bool;

/// Fold pending strokes for Regenerate. Pure data in, pure data out.
pub fn fold(strokes: &[Stroke], hotspot_base: Option<&[[f32; 3]]>)
    -> FoldOutcome;

pub struct FoldOutcome {
    /// Final sign per touched cell, id-sorted (later strokes already won).
    /// Caller merges: for (c, v) { craton_paint.insert(c, v) }.
    pub craton_updates: Vec<(u32, i8)>,
    /// Some(set) = new absolute hotspot_overlay; None = leave param as is.
    pub hotspot_overlay: Option<Vec<[f32; 3]>>,
}

/// Base ⊕ pending hotspot strokes, replayed in order (§3). Shared by click
/// classification, fold, and overlay_view — one implementation.
pub fn effective_hotspots(
    base: Option<&[[f32; 3]]>,
    strokes: &[Stroke],
) -> Vec<[f32; 3]>;

/// Nearest spot within HOTSPOT_REMOVE_KM; ties → lowest index (§3).
pub fn match_hotspot(set: &[[f32; 3]], unit: [f32; 3]) -> Option<usize>;

/// Moved verbatim from WorldApp::cells_within_radius (app.rs:453–472),
/// as a free function over &Grid.
fn cells_within_radius(grid: &Grid, center: u32, radius_km: f32) -> Vec<u32>;
```

`handle_stroke_event` body, pinned:

- `Craton { sign, radius_km }`: if `hit_cell` is Some and
  `(dragged || clicked)` → merge `cells_within_radius` into `live`
  (creating it), `changed` = any cell was new. Then, hover-independent:
  if `drag_stopped || clicked` → `end_stroke()`. Returns `changed` (commit
  alone changes only the badge, not the overlay pixels).
- `Hotspot`: requires `clicked` and a hit, else false. If `hotspot_base` is
  `None` → **false** (recording refused; §3 base rule). Else compute
  `effective = effective_hotspots(base, strokes-so-far)`; if
  `match_hotspot(&effective, grid.positions[hit])` is Some → push
  `Stroke::hotspot_remove(positions[hit])`, else push
  `Stroke::hotspot_add(positions[hit])`; return true.

Deterministic trig: both `cells_within_radius` (≤ 2000 km → ≤ 0.314 rad) and
`match_hotspot` (300 km → 0.047 rad) compute their cosine thresholds via
`worldmaker_core::dmath::det_sin_cos` (valid |x| ≤ 0.75 rad — both in range)
instead of std `f32::cos`. Fold output enters `TectonicsParams` and
`params_hash`, so the stroke path is kept bit-stable across platforms even
though it is UI code. Everything in this module is serial and id/list-ordered;
no rayon, no HashMap iteration in any output-producing path (the BFS `visited`
HashSet is membership-only; output order comes from the BTreeSet).

### Badge, undo, discard (app-side wiring, A-owned)

`WorldApp` gains `pending: PendingEdits` and `hotspot_baseline:
Option<Vec<[f32; 3]>>` (§3); loses `craton_stroke_dirty`, `apply_tool`,
`cells_within_radius`, `HOTSPOT_REMOVE_KM`, `EARTH_RADIUS_KM`.

- **Badge = `pending.stroke_count()`** — committed strokes only; the live
  stroke joins the count on stroke end (pinned: mid-drag the count is the
  pre-stroke value). Rendered in the side panel next to the "Paint" heading,
  visible whenever count > 0 **regardless of tool** (Navigate included):
  `ui.label(format!("{n} pending edit(s)"))`.
- **Undo — Cmd+Z on macOS, Ctrl+Z elsewhere**: in `WorldApp::ui`, before the
  panels:

  ```rust
  const UNDO_STROKE: egui::KeyboardShortcut =
      egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
  if !ctx.wants_keyboard_input()
      && ctx.input_mut(|i| i.consume_shortcut(&UNDO_STROKE))
      && self.pending.undo()
  {
      self.needs_bake = true;
  }
  ```

  `Modifiers::COMMAND` maps to ⌘ on the Mac and Ctrl on Windows/Linux
  (a-report §9); the `!ctx.wants_keyboard_input()` gate leaves the seed
  TextEdit's own text-undo alone. Undo cancels the live stroke first if one
  exists, else pops the newest pending stroke. **No redo** in this WO. A small
  "Undo" button beside the badge does the same for mouse-only use (Dan).
- **Discard button** ("Discard edits", enabled when `!pending.is_empty()`):
  `pending.discard_all()`, `needs_bake = true`. Discards **pending only** —
  applied overlays (`craton_paint`, `hotspot_overlay`) are untouched (§6).
- **Regenerate — the fold point.** The side-panel "Generate history" button is
  relabeled **"Regenerate"** (same `add_enabled(self.job.is_none(), …)` gate,
  app.rs:782–787). Handler:

  ```rust
  self.pending.end_stroke();
  let strokes = self.pending.take_all();
  let base = self.hotspot_fold_base();          // §3
  let outcome = pending_edits::fold(&strokes, base.as_deref());
  for (c, v) in outcome.craton_updates {
      self.craton_paint.insert(c, v);
  }
  if let Some(hs) = outcome.hotspot_overlay {
      self.hotspot_overlay = Some(hs);
  }
  self.start_job();                              // unchanged: off-thread,
                                                 // existing progress + cancel
  ```

  `current_params()` (app.rs:262–272) then folds `craton_paint` (BTreeMap ⇒
  id-sorted) and `hotspot_overlay` into `TectonicsParams` exactly as today —
  no `TectonicsParams` change, no serde on it, params_hash untouched in shape.
  With no pending strokes, Regenerate degenerates to today's Generate history.

---

## 3. Hotspot fold semantics — PINNED

Strokes are deltas; `hotspot_overlay: Option<Vec<[f32; 3]>>` is an absolute
replacement set (mod.rs:88–89); `start_job` drops `history` immediately
(app.rs:316), so `history.hotspots` can be absent. All rules below are pinned.

**Baseline capture (new).** `WorldApp.hotspot_baseline: Option<Vec<[f32; 3]>>`
is set unconditionally in `poll_job` on every successful install:
`self.hotspot_baseline = Some(history.hotspots.clone())` (the set the finished
run actually used — generated or overlay). It survives `start_job`'s history
drop, preset switches (unit vectors are level-independent) and seed changes.
It is `None` only before the first run ever completes.

**Fold base rule.** `hotspot_fold_base()` =
`self.hotspot_overlay.clone().or_else(|| self.hotspot_baseline.clone())`.
The applied overlay wins when present; else the last completed run's set; else
`None`.

**Empty/absent base.** Recording is guarded: `handle_stroke_event` refuses
hotspot clicks (returns false, side panel shows "hotspots available after the
first world finishes generating") while the base is `None` — preserving the
intent of today's bail (app.rs:497–507: never replace the generated set
blind), but with a far smaller window: after the first completion, baseline
always exists, so **hotspot strokes work mid-run** (the pinned requirement
today's code fails). Invariant: pending hotspot strokes ⇒ a base existed when
recorded. Defensively, if `fold` is ever called with base `None` it folds onto
the empty set: adds append, removes are no-ops — deterministic and harmless.

**Fold procedure.** `fold` replays hotspot strokes **in list order** over a
working `Vec` initialized from the base (base order preserved):

- `HotspotAdd { unit }` → push `unit` (append; order = stroke order).
- `HotspotRemove { unit }` → `match_hotspot(&working, unit)`; `Some(i)` →
  `working.remove(i)`; `None` → **no-op** (pinned: a remove that no longer
  matches — e.g. the baseline changed between click and fold because an
  in-flight run with a different hotspot_count/seed completed — deletes
  nothing; deterministic given (base, strokes)).

Output: if there were **no** hotspot strokes, `hotspot_overlay: None` — the
param is left untouched (in particular a `None` param stays `None`, so the sim
keeps generating from seed). Otherwise `Some(working)` — an absolute
replacement set, even if it happens to equal the generated set.

**Remove-matching rule — changed from first-match to nearest, pinned.**
Today's rule is FIRST spot within 300 km in Vec order (app.rs:509–512), which
depends on insertion history and can remove a farther spot than the one
clicked. Pinned new rule, implemented once in `match_hotspot`:

> Scan indices ascending; candidate = dot(spot, unit) ≥ cos(300 km / 6371 km)
> (threshold via `dmath::det_sin_cos`); keep the candidate with the strictly
> greatest dot (nearest); equal dots keep the earlier index. Result: nearest
> within 300 km, ties → lowest index.

Why change: nearest matches user intent when spots are within 300 km of each
other; the lowest-index tiebreak keeps the order total and deterministic. The
old behavior was only load-bearing through `start_job`, which this design
severs, so there is no compatibility to preserve; the change is logged in
decision-log.md at merge.

**Remove matching a pending add.** Because the working Vec includes earlier
pending adds, a `HotspotRemove` naturally deletes a previously added pending
spot if it is the nearest match. Both strokes **stay in the list** (the list
is an append-only log until undo/discard/fold; the badge counts 2). Click
classification uses the same `effective_hotspots` replay, so clicking a
pending marker records a remove, and the marker disappears from the preview.

**Classification/fold coherence.** Click-time classification, fold, and
`overlay_view` all call `effective_hotspots`/`match_hotspot` with the same
base rule — one implementation, so what the user saw when clicking is what the
fold produces (up to the documented baseline-shift no-op case).

---

## 4. Severing the four stroke→sim routes

| # | Route today | Becomes |
|---|---|---|
| 1 | canvas_common stroke end → `start_job()` (app.rs:551–558) | `pending.end_stroke()` via `handle_stroke_event`'s craton arm. No sim. The dirty-flag branch is deleted with the field. |
| 2 | apply_tool hotspot arm → `hotspot_overlay = Some(spots)` + `start_job()` (app.rs:493–522) | `handle_stroke_event`'s hotspot arm records a `HotspotAdd`/`HotspotRemove` stroke. Never touches `hotspot_overlay`, never runs sim. `apply_tool` itself is deleted; `canvas_common` maps `Tool` → `Option<ActiveBrush>` and calls `handle_stroke_event`, setting `needs_bake` on true. |
| 3 | "Clear craton paint" → `craton_paint.clear()` + `start_job()` (app.rs:823–827) | **Pinned meaning:** discards pending craton strokes (incl. live) AND empties the applied `craton_paint` map; sets `needs_bake`; **no job**. Takes world effect at the next Regenerate. Enabled when `!craton_paint.is_empty() \|\| pending.has_craton()`. |
| 4 | "Reset to generated" → `hotspot_overlay = None` + `start_job()` (app.rs:832–835) | **Pinned meaning:** discards pending hotspot strokes AND sets applied `hotspot_overlay = None`; sets `needs_bake`; **no job**. Next Regenerate (with no new strokes) folds nothing and the sim regenerates hotspots from seed. Enabled when `hotspot_overlay.is_some() \|\| pending.has_hotspot()`. |

Why the buttons may touch applied state without being strokes: "strokes are
the only mutations" governs the **brush path** — every other parameter control
(plate count, land fraction, span…) already edits staged params that take
effect only at the next run. The two buttons become exactly such controls:
staged-param edits plus a pending-discard, deferred to the fold point. This
keeps a way to un-apply overlays (undo is pre-commit only, §8) without adding
any sim trigger. The buttons are **not undoable** (they are not strokes; same
irrevocability as Discard). Pure view controls (sea level, layers, projection,
timeline, Detail) remain untouched by all of this.

Non-stroke sim routes that legitimately remain (the only ones): `rebuild_grid`
(preset switch / startup), seed Generate/Enter, and Regenerate. Enforcement in
§5.

---

## 5. Structural guard: "no route to Pipeline::run", enforced

Rust has no call-graph reflection, so the guard is layered — each layer is
real and testable:

**(a) Module-dependency wall.** All stroke mutation logic lives in
`pending_edits`, which imports only `std`, `worldmaker_core`,
`worldmaker_io::stroke`. It cannot call `Pipeline::run` (would require `use
worldmaker_sim`), and it cannot call `WorldApp::start_job` even in-crate:
`start_job` is a private inherent method of `WorldApp` inside `mod app`, not
visible to a sibling module. `handle_stroke_event`'s signature admits only
`&mut PendingEdits`, `&Grid`, brush/event data — there is nothing job-shaped
to reach.

**(b) Lexical guard test** — `#[cfg(test)]` in pending_edits.rs (the app is a
binary-only crate; in-src test modules are the established pattern):

```rust
#[test]
fn pending_edits_names_no_sim_machinery() {
    let src = include_str!("pending_edits.rs");
    // Needles assembled at runtime so this test's own source never matches.
    for parts in [
        ["worldmaker", "_sim"], ["Pipe", "line"], ["start", "_job"],
        ["Tectonics", "Params"], ["Sim", "Job"], ["thread::", "spawn"],
        ["mp", "sc"],
    ] {
        let needle = parts.concat();
        assert!(!src.contains(&needle), "pending_edits must not name {needle}");
    }
}
```

**(c) Sanctioned call-site count** — same test module:

```rust
#[test]
fn app_has_exactly_the_sanctioned_sim_triggers() {
    let src = include_str!("app.rs");
    let needle = ["self.start", "_job()"].concat();
    let n = src.matches(&needle).count();
    // rebuild_grid (preset/startup), seed Generate, Regenerate. Adding a
    // call site is a design change: update this count deliberately, with a
    // decision-log row — never to make a red test green.
    assert_eq!(n, 3);
}
```

Deliberately brittle in the safe direction: any new `start_job()` route fails
CI loudly. (Today the count is 7; after Fix 1 it is 3.) A rebases last
(B → C → A), so A owns keeping the count correct at rebase; the partition
gives job triggering to A, so C/B adding sim triggers would itself be a
partition violation this test catches.

**(d) Runtime input-path test.** Because `canvas_common` shrinks to
Response-field extraction (`StrokeEvent`) + brush mapping + one call, the real
stroke logic IS `handle_stroke_event`, and it is testable headlessly (no
eframe/wgpu needed — this is how the design resolves the d-report's
"WorldApp cannot be constructed headlessly" warning for Track A):

```rust
#[test]
fn strokes_drive_pending_state_only() {
    let grid = Grid::build(3);              // tiny, fast
    let mut p = PendingEdits::new();
    // drag: three hit frames, then release off-map
    for cell in [0u32, 1, 2] {
        assert!(handle_stroke_event(&mut p, &grid,
            &ActiveBrush::Craton { sign: 1, radius_km: 300.0 },
            &StrokeEvent { hit_cell: Some(cell), dragged: true,
                           clicked: false, drag_stopped: false }, None));
    }
    handle_stroke_event(&mut p, &grid, /*…*/,
        &StrokeEvent { hit_cell: None, dragged: false, clicked: false,
                       drag_stopped: true }, None);
    assert_eq!(p.stroke_count(), 1);        // one stroke, not three
    // hotspot add + remove-of-pending against an explicit base…
    // undo pops the newest; fold produces data only:
    let out = fold(p.strokes(), Some(&base));
    // FoldOutcome is plain vectors — by (a) there is nothing else it could be.
}
```

Together: (d) proves the real input path routes through the guarded module;
(a)+(b) prove that module has no route to sim; (c) pins the full set of sim
triggers in app.rs. That is the strongest honest enforcement available without
call-graph reflection, and it is what "enforced by a test" means for this WO.

---

## 6. Applied vs pending; preset-switch and seed-change sequencing

**The model, pinned.**

- **Applied state** = `craton_paint: BTreeMap<u32, i8>` +
  `hotspot_overlay: Option<Vec<[f32; 3]>>` — the staged params of the
  next/last run. Mutated only by fold (Regenerate) and the two §4 buttons.
  Never counted by the badge, never popped by undo, never touched by Discard.
- **Pending state** = the `PendingEdits` stroke list + live stroke. Badge
  counts committed strokes; Discard discards exactly this; undo pops exactly
  this.
- **They coexist**: pending strokes render over a world that already includes
  applied overlays; fold merges pending INTO applied (craton: per-cell insert,
  later strokes win — same absorption as painting today, but now deferred and
  undoable until folded; hotspot: replay onto the base per §3).

**Preset switch.** The discard-cratons/keep-hotspots step must run before
`rebuild_grid`'s unconditional `craton_paint.clear()` (app.rs:298) and its
`start_job()`. Pinned placement: **first statement of `rebuild_grid`**:

```rust
fn rebuild_grid(&mut self, level: u32) {
    // Pending craton strokes are per-level cell ids: discard them (and any
    // live stroke). Pending hotspot strokes are unit vectors: keep them.
    self.pending.discard_cratons();
    …
}
```

`rebuild_grid` is not on A's forbidden list (that list is rebake, WorldBundle,
render.rs, shaders.wgsl, layers.rs, main.rs, harness.rs); C also edits
rebuild_grid for WorldBundle plumbing, and A rebases onto C, so this lands as
a one-line insertion at the top of whatever C's version looks like. Putting it
inside the function (not at the combo call site, which C owns) makes the
ordering correct by construction for every grid rebuild, including
`WorldApp::new` (pending is empty there — no-op). `hotspot_baseline` survives
(positions are level-independent) and the applied `hotspot_overlay` is kept,
exactly as today.

**Seed change (Generate/Enter, app.rs:685–691).** Keeps everything pending,
pinned: **no fold, no discard** — the branch gains only a defensive
`self.pending.end_stroke()` before `start_job()`. Applied overlays ride along
in `current_params()` as today; pending strokes remain pending and keep
rendering over the new world when it lands. (The new world may shift beneath
pending craton cells — cell ids are stable per level, the paint stays where it
was drawn; accepted, mandated by the pinned keep-all rule.)

**Job completion (`poll_job`).** Adds one line (baseline capture, §3);
otherwise unchanged, including the keep-`viewing_kf`=0-while-craton-tool
behavior and `needs_bake = true`, which now also re-composites pending
overlays over the fresh world through the frozen interface.

---

## 7. Consuming the frozen A↔C overlay interface

C publishes the exact frozen signature in `feel-pass-design.md` during Stage D
(D3); A implements `pending_edits::apply_overlay(...)` against it and provides
the input view. **A's REQUIRED inputs — D3 must satisfy all of these:**

1. **Pending set passed explicitly.** The call takes a
   `&PendingOverlayView` (below) produced by
   `app.pending.overlay_view(base)`; it is **never** inferred from
   `self.tool`. Pending overlays render whenever the view is non-empty, under
   every tool including Navigate.
2. **History-independent / works mid-run.** The composite must work while
   `self.history` is `None` (a run in flight): "current world behind" =
   the last published base. rebake's current early-return-and-drop
   (app.rs:399–401) must not silently discard a pending-overlay update — C's
   rebake successor either retains base colors/values to re-composite, or
   re-applies the overlay artifact GPU-side. Either satisfies A.
3. **Not defined over `Vec<u32>` colors.** Per b-report risk 1, the frozen
   artifact must survive Fix 3's value-buffer rewrite: a per-cell overlay
   buffer (tint + outline/flag bits) or equivalent that C's shaders composite.
   A passes cell sets and positions, never pre-blended colors.
4. **Tint + outline expressible.** The interface carries enough structure for
   C to draw distinct pending visuals: per-cell final sign for craton
   (outline = boundary of the pending set), and add/removed-base unit vectors
   for hotspots (markers / crossed-out markers).
5. **Grid consistency guaranteed by A.** Every craton cell id in the view is
   valid for `bundle.grid`: pending craton strokes are discarded inside
   `rebuild_grid` before any rebake can observe the new grid (§6).
6. **Deterministic and cheap.** View construction is serial, id-sorted, O(pending);
   called at most once per rebake.

A-side view type (io-free, defined in pending_edits.rs):

```rust
pub struct PendingOverlayView {
    /// Union of pending craton strokes incl. live, id-sorted, later strokes
    /// won: (cell, sign).
    pub craton: Vec<(u32, i8)>,
    /// Effective pending hotspot additions (unit vectors), stroke order.
    pub hotspot_adds: Vec<[f32; 3]>,
    /// Base spots effectively removed by pending strokes (unit vectors),
    /// base order — C renders these crossed-out/greyed.
    pub hotspot_removed_base: Vec<[f32; 3]>,
}
```

built by replaying `effective_hotspots` bookkeeping against the same base rule
as §3 (`hotspot_overlay.or(baseline)`, passed in by the app). Indicative
frozen call C is asked to ratify — final say is C's per the contract, any
artifact type meeting 1–6 is acceptable to A:

```rust
pending_edits::apply_overlay(
    out: &mut <C's overlay artifact>,
    grid: &Grid,
    view: &PendingOverlayView,
)
```

(`&Grid` because hotspot markers need `nearest_cell` + neighbor rings, as
today's rebake does at app.rs:417–421.)

Display split, pinned: the **applied** craton tint and base hotspot markers
remain a tool-gated courtesy preview exactly as today (C's choice to keep or
restyle); **pending** strokes are the tint + outline overlay and are
tool-independent. The two must be visually distinguishable (outline is the
distinguishing mark).

---

## 8. Undo scope, badge — and the test list

**Undo scope, pinned:** pre-commit only. Cmd/Ctrl+Z pops pending strokes
(live first); nothing un-applies a folded overlay or un-presses a §4 button.
No redo. Badge = pending stroke count only.

### Tests (all new, named)

worldmaker-io `src/stroke.rs` `#[cfg(test)]`:
- `stroke_serde_round_trip` — every variant, JSON → Stroke → JSON.
- `stroke_json_shape_is_stable` — exact serialized strings pinned (future
  save-format guard).
- `craton_constructor_canonicalizes` — sorts, dedups, normalizes sign.

worldmaker-app `src/pending_edits.rs` `#[cfg(test)]` (binary-crate in-src
module, per layers.rs/render.rs precedent):
- `strokes_drive_pending_state_only` — §5(d), the real-input-path test.
- `craton_stroke_boundaries_survive_overlap` — two overlapping drags → two
  strokes; undo removes only the newer; fold of the remainder is correct
  (this is the capture property `craton_paint` alone could never provide).
- `undo_cancels_live_stroke_first`.
- `discard_cratons_keeps_hotspot_strokes` (preset rule) and
  `discard_all_clears_everything`.
- `fold_craton_later_stroke_wins`.
- `fold_hotspot_semantics` — add appends; remove picks nearest with
  lowest-index tie-break; remove deletes a pending add; remove with no match
  is a no-op; empty stroke list leaves `hotspot_overlay` = None; absent base
  folds from empty.
- `hotspot_click_classification_matches_fold` — a click sequence classified
  via `effective_hotspots` replays to the same final set as `fold`.
- `pending_edits_names_no_sim_machinery` — §5(b).
- `app_has_exactly_the_sanctioned_sim_triggers` — §5(c).

No golden, no results-JSON, no harness.rs, no sim-crate change anywhere in
Track A; sim tests re-run green after A's merge per the pinned golden rule.

### Files touched (complete)

| File | Change |
|---|---|
| crates/worldmaker-io/src/stroke.rs | new: Stroke/StrokePayload/StrokeTool + tests |
| crates/worldmaker-io/src/lib.rs | register + re-export stroke |
| crates/worldmaker-app/src/pending_edits.rs | new: everything in §2/§3/§5/§7 + tests |
| crates/worldmaker-app/src/main.rs | one line: `mod pending_edits;` (coordinate with C at rebase) |
| crates/worldmaker-app/src/app.rs | fields ±; canvas_common → StrokeEvent funnel; delete apply_tool/cells_within_radius/craton_stroke_dirty/consts; Cmd+Z; side-panel badge/Undo/Discard/Regenerate/buttons; seed branch end_stroke; rebuild_grid first line; poll_job baseline; Regenerate fold |
| docs/plan/decision-log.md | at merge: interaction contract (pinned text), nearest-match rule change, buttons-as-staged-param-edits |

### Resolution of every flagged risk (a-report §10)

1. Four routes severed — §4 table; enforced §5(c).
2. Delta-vs-absolute hotspot fold, absent base — §3, fully pinned.
3. Tool-/history-gated overlay display — §7 requirements 1–2.
4. Tint + outline data — §7 requirement 3–4 (cell sets + positions, not colors).
5. Stroke-boundary capture — §1 live accumulator; test
   `craton_stroke_boundaries_survive_overlap`.
6. No serde on TectonicsParams needed — fold outputs plain data; Stroke lives
   in worldmaker-io with existing serde dep; no new dependency edges.
7. Preset-switch ordering before the unconditional clear — §6 (first line of
   rebuild_grid).
8. A/C seam — §7; A never edits rebake/WorldBundle/render.rs/shaders.wgsl/
   layers.rs/harness.rs.
9. Era-picker-disabled-while-running behavior kept; Regenerate uses existing
   progress + cancel unchanged.

### Residual risks

- The frozen artifact type is C's to publish; if D3 freezes something not
  meeting §7's six requirements, A's rendering breaks — mitigation: D3 must
  cite §7 explicitly; merge order B → C → A gives A the last rebase.
- Lexical guard tests are string-level, not semantic; treated as tripwires
  plus the module-dependency wall, not as proof.
- §5(c)'s count test needs a deliberate update if a legitimate sim trigger is
  ever added (by design — failure is the feature).
- Baseline shift between hotspot click and fold makes a remove a no-op (rare,
  documented, deterministic).

---

## Adversarial review (fix1)

Reviewed against main @ b4896ac (code bit-identical to 9d5d272 for every
cited file — only docs landed since; all line refs re-verified in source).
Claims checked and confirmed: `self.start_job()` occurs exactly 7 times today
(app.rs:306, 520, 557, 690, 786, 826, 834) and exactly 3 survive the §4
severing; `start_job` is a private inherent method (:312, no `pub`), so the
sibling-module privacy claim in §5(a) is true; the seed branch is :688–691;
`craton_paint.clear()` is rebuild_grid:298 with `rebuild_grid` called only
from `new` (:258) and the preset combo (:732); rebake runs once at end of
`ui`, after any same-frame rebuild_grid, so §7 req 5's ordering argument
holds; `TectonicsParams` docs match §1's payload claims (mod.rs:85–89);
setup.rs:253 clones `hotspot_overlay` verbatim into `s.hotspots` and
mod.rs:236 stores it in the history, so baseline-after-an-overlay-run equals
the overlay (used by F2 below); dmath::det_sin_cos range 0.75 rad covers both
thresholds (2000 km → 0.314 rad, 300 km → 0.047 rad); worldmaker-io depends
on worldmaker-core + serde and has **zero** references to worldmaker-sim;
`{"payload":{...}}` is the correct externally-tagged serde shape; serde_json
f32 round-trips are exact (ryu shortest). No golden can move: Track A touches
no sim crate, and the det_sin_cos threshold change only alters user-driven
overlay params, never the default-params golden configs.

### Findings

**F1 — MAJOR (pinned-contract violation): A editing main.rs.** §2 self-grants
an exception to the pinned partition ("A does NOT edit … main.rs"; "C owns
main.rs"). Modules are declared in main.rs:5–8, so `mod pending_edits;` is a
real main.rs edit; a pinned rule cannot be waived by the track it binds.
Moving pending_edits under `mod app` instead (`#[path]` child) is NOT a fix:
a child module can see `WorldApp::start_job`'s privates, gutting guard §5(a).
**Fix (adopted):** put the whole pending_edits module in **worldmaker-io**
(e.g. `crates/worldmaker-io/src/pending.rs`) beside stroke.rs. The partition
already assigns "pending-set type in worldmaker-io" to A; io depends on core
(Grid, dmath) + serde, needs no new edges, and io has no worldmaker-sim
dependency — guard (a) hardens from a module wall to a **crate-boundary
wall** provable by a test asserting io's Cargo.toml never names
worldmaker-sim. Guard (b) scans pending.rs in io; guard (c)
(`include_str!("app.rs")`) moves to an A-owned `#[cfg(test)]` module in
app.rs (in-src test precedent: layers.rs:271). `PendingOverlayView` moves to
io (plain data); `apply_overlay` against C's frozen artifact stays app-side
in A-owned code. main.rs is then untouched by A. Fallback if io is rejected
at implementation: C adds the one `mod pending_edits;` line on C's branch
(C merges before A) with a decision-log row — never A editing main.rs.
**AMENDED.**

**F2 — MAJOR (forgotten state): "Reset to generated" is silently voided by a
subsequent hotspot stroke.** After Reset (route 4: overlay=None, pending
hotspot strokes discarded, **no job**), `history`/`hotspot_baseline` still
hold the pre-reset set and the canvas still displays it (rebake:416 falls
back to `history.hotspots`). A hotspot click then re-anchors the fold base to
that set (`overlay.or(baseline)`), so Regenerate folds `Some(old set ±
click)` — resurrecting every un-clicked spot the user just "reset away".
Route 4's "(with no new strokes)" hedge hints at this but the design never
states the consequence. The behavior is display-coherent (the user edits the
set they can see) and the re-enabled Reset button (`pending.has_hotspot()`)
is the escape hatch, so the mechanism stands. **Fix:** pin it explicitly in
§3/§4 — "a hotspot stroke recorded after Reset re-anchors to the last-run
set; Reset holds only if no hotspot stroke follows before Regenerate" — add
test `hotspot_stroke_after_reset_reanchors_to_visible_set` (base = baseline
with overlay None → fold yields Some(base ± stroke)), and include this rule
in the decision-log row for the buttons. **AMENDED.**

**F3 — minor: baseline capture must not be literally unconditional.**
`world.history.take()` is an Option (poll_job:353–360 already handles None).
Pin the §3 line as `if let Some(h) = &self.history { self.hotspot_baseline =
Some(h.hotspots.clone()); }` so a hypothetical history-less success can't
panic or clear the baseline. **AMENDED.**

**F4 — minor: Regenerate leaves a stale pending tint mid-run.** The §2
handler drains pending (badge → 0) but never sets `needs_bake`, so the
drained strokes keep rendering as "pending" until poll_job's completion
rebake. Add `self.needs_bake = true;` after the fold; whether the composite
can refresh while `history` is None is exactly §7 req 2 (C's side), and at
worst it clears at completion. **AMENDED.**

**F5 — minor: "moved verbatim" is false, and the change must be logged.**
cells_within_radius today uses std `f32::cos` (app.rs:454, :509); the design
simultaneously mandates det_sin_cos. Not verbatim: a brush-edge cell whose
dot sits within ~2e-6 of the threshold can flip inclusion versus today.
No golden exposure (goldens carry no overlays), but it lands in
`craton_overlay`/params_hash, so log it in the same decision-log row as the
nearest-match rule change. Also pin that all dot products in
cells_within_radius/match_hotspot use `dmath::dot3` (fixed evaluation order),
not ad-hoc arithmetic. **AMENDED.**

**F6 — minor: baseline shift affects adds too, not only removes.** §3
documents the remove no-op case only. A `HotspotAdd` recorded before an
in-flight run completes can fold to within 300 km of a shifted-baseline spot;
setup.rs:253 accepts overlay sets verbatim (the 15° min-sep applies only to
generated sets), so near-duplicate spots are possible. Deterministic and
harmless; document beside the remove case and cover in
`fold_hotspot_semantics`. **AMENDED.**

**F7 — minor: §5(d) snippet has a type error.** `fold(p.strokes(),
Some(&base))` passes `Option<&Vec<[f32;3]>>` where `Option<&[[f32;3]]>` is
required (no auto-deref through Option) — write `base.as_deref()` or
`Some(base.as_slice())`. Pseudocode, but implementers copy snippets.
**AMENDED.**

**F8 — minor (accepted tripwire limits): guard (c).** The
`"self.start_job()"` needle also counts occurrences in comments and misses
non-`self` receivers (`app.start_job()`) or a renamed wrapper. Acceptable as
a declared tripwire, with two pins added: `start_job` stays a **private**
inherent method (so no cross-module receiver can exist — verified true
today), and any comment quoting the literal call text is forbidden in app.rs.
The count-3 target itself is verified correct. **VERIFIED** (as tripwire,
with pins).

### Verdict by design element

| Element | Verdict |
|---|---|
| §1 Stroke type, canonical form, JSON shape, per-stroke capture | VERIFIED (F5 wording corrected) |
| §2 module + API, handle_stroke_event, badge/undo/discard/Regenerate wiring | AMENDED — module home moves to worldmaker-io (F1); `needs_bake` after fold (F4) |
| §3 hotspot fold semantics, baseline, nearest-match rule | AMENDED — F2 pinned semantics, F3 if-let capture, F6 add-shift doc; fold/replay/no-op rules otherwise VERIFIED |
| §4 four routes severed, button redefinitions | AMENDED — F2 note on Reset-then-stroke; route table itself VERIFIED against :551–558/:493–522/:823–827/:832–835 |
| §5 structural guard (a)–(d) | VERIFIED — with F1 strengthening (crate wall + Cargo.toml test), F7 snippet fix, F8 pins; 7→3 count verified exact |
| §6 applied/pending model, preset/seed sequencing | VERIFIED — discard-before-:298 ordering and rebake-after-rebuild ordering confirmed in source; note: rebuild_grid interior is C-assigned plumbing, so A's first-line insertion is a coordinated one-line edit at rebase (log it with F1's row) |
| §7 six requirements + PendingOverlayView | VERIFIED (view type relocates to io per F1; requirements unchanged, D3 must cite them) |
| §8 undo scope, badge, test list | AMENDED — add `hotspot_stroke_after_reset_reanchors_to_visible_set` (F2) and the io Cargo.toml no-sim-dependency test (F1); files-touched table: drop the main.rs row, add worldmaker-io/src/pending.rs and the app.rs test module |

No BLOCKERs: nothing found that breaks determinism, moves a golden outside
the sanctioned regeneration, or contradicts the frozen-interface plan.
Determinism re-checked end to end: serial, id/list-ordered, BTree-ordered
outputs, membership-only HashSet, dmath-only trig, no rayon, no NaN sources,
exact f32 JSON round-trip; hotspot ties broken by strictly-greatest-dot then
lowest index (total order). Poles/antimeridian are non-issues (all positions
are 3D unit vectors); L9 stroke memory is trivial (~10 MB per full-planet
stroke, user-bounded).

### Addendum — independent second-pass verification

A second adversarial pass re-verified every factual claim above directly
against source at b4896ac (`git diff --stat 9d5d272..HEAD -- crates/` is
empty — code truly bit-identical). All confirmed: the 7 `self.start_job()`
sites (app.rs:306/520/557/690/786/826/834); `start_job` private (:312);
module decls in main.rs:5–8 (F1's premise); rebake's sole call site is
app.rs:1216; rebake's hotspot display fallback `hotspot_overlay.as_deref()
.unwrap_or(&history.hotspots)` (:416, F2's premise); `poll_job` takes
`world.history.take()` into an Option (:353, F3's premise);
`cells_within_radius`/hotspot arm use std `f32::cos` + ad-hoc dot products
today (:454–458, :509–512, F5's premise); `dmath::det_sin_cos` pinned to
|x| ≤ 0.75 rad with `dot3`/`normalize3` present (dmath.rs:14–47);
`TectonicsParams` has exactly 7 fields with docs matching §1 (mod.rs:73–90);
setup.rs:253 clones `hotspot_overlay` verbatim and mod.rs:236 stores
`sim.hotspots` in the history; worldmaker-io's Cargo.toml deps are exactly
worldmaker-core + serde + serde_json + anyhow — **no worldmaker-sim**, and
the core dep already exists, so F1's "no new dependency edges" claim is
source-verified, not assumed. `Grid::build(level)`, `positions`,
`neighbors_of`, `nearest_cell(target, hint)` all exist as the §5(d) test
snippet assumes.

**F9 — minor (spec gap the first pass missed): `PendingOverlayView`
provenance is unimplementable from `effective_hotspots` alone.** §7 requires
`hotspot_removed_base` to contain only *base* spots removed by pending
strokes, and a `HotspotRemove` that matches a *pending add* must instead
drop that add from `hotspot_adds` (§3: "the marker disappears from the
preview") — but `effective_hotspots` returns a plain `Vec<[f32; 3]>` with no
origin information, so `overlay_view` cannot classify a removal's target
from it. **Fix (pinned):** implement one shared replay over tagged entries —
`(origin: Base(i) | PendingAdd(stroke_idx), unit)` — used by
`effective_hotspots` (drops tags), `overlay_view` (uses tags to route
removals: Base → `hotspot_removed_base` in base order, PendingAdd → omit
from `hotspot_adds`), and click classification/fold (tags ignored). One
implementation keeps §3's classification/fold coherence intact; determinism
unchanged (tags are data, replay stays list-ordered). Add assertions to
`hotspot_click_classification_matches_fold` covering: remove-of-pending-add
⇒ `hotspot_adds` shrinks and `hotspot_removed_base` stays empty;
remove-of-base-spot ⇒ appears in `hotspot_removed_base`. **AMENDED** (§7
view construction; requirements 1–6 and the view type's fields unchanged).

Second-pass verdict: F1–F8 stand as written; one new minor amendment (F9);
still no BLOCKERs.
