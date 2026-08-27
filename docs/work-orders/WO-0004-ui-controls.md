# WO-0004: timeline controls, HUD fit, zoom reset, plate velocity layers

CONTEXT. Dan's review of v0.2.0 + feel pass (2026-08-27). Four UI items, no sim changes. All work is in `crates/worldmaker-app`. Sim hashes must not move.

RULES. Single-track. Branch `feat/ui-controls` in worktree `../WorldMaker-ui`, created from `main`. Checkpoint commits every 30–45 minutes. No subagents except one final verification pass capped at three short lookups. If the usage limit nears: commit, push, update checkboxes, stop with a one-line note.

STEPS.

0. On `main`: run `git pull --ff-only origin main`. Commit `docs/work-orders/WO-0003-SYNC.md`, `WO-0003-S4.md`, `WO-0004-ui-controls.md`, and `WO-0005-plate-physics-diag.md` with message `docs: install WO-0004 and WO-0005; close WO-0003-SYNC`. Run `git push origin main`. Delete `docs/work-orders/WO-0004-ui-controls.md.bak` if it exists; never commit it. Run `rmdir ../WorldMaker-trackA` (an empty leftover folder; if it is not empty, leave it and report).
1. Create the worktree: `git worktree add ../WorldMaker-ui -b feat/ui-controls main`. Do all work there.
2. Playback speed. Replace the constant `PLAY_MY_PER_SECOND` in `app.rs` with a field `play_my_per_s: f32` on `WorldApp`, default 100.0. Add a `ComboBox` beside the play button with choices 25, 50, 100, 200, 400 My/s, labelled 0.25×, 0.5×, 1×, 2×, 4×. Playback reads the field.
3. Step button. Add a button `⏭` right of the play button. On click: `viewing_kf += 1`, clamped to `kf_count - 1`, and set `playing = false`. Add a button `⏮` left of the play button that does the same with `-= 1`, clamped to 0.
4. HUD fit. In `top_bar()`, move `Detail:` slider and the FPS label to the second row, after `Legacy bands`. Verify at a 1440 px wide window that no control is clipped. Take a screenshot to `docs/media/ui-controls/hud-1440.png`.
5. Zoom reset. Add three buttons at the end of the first row of `top_bar()`: `Reset globe`, `Reset map`, `Reset both`. `Reset globe` sets `globe.zoom = 1.0`, `globe.yaw = 0.0`, `globe.pitch = 0.0`. `Reset map` sets `flat_zoom = 1.0`, `flat_pan = [0.0, 0.0]`, `flat_center_target = None`. `Reset both` calls both.
6. Plate velocity layer. Add `Layer::PlateVelocity` to the `Layer` enum in `layers.rs` and to `Layer::ALL`, name "Plate velocity". It draws the `Plates` layer underneath. On top, for each alive plate, draw one white arrow at the plate's area centroid on the sphere. Arrow direction is the surface velocity `v = ω × r` from `PlateState::pole` and `PlateState::speed_deg_my` at the centroid. Arrow length is proportional to `speed_deg_my`, with the longest arrow on screen equal to 8% of the canvas width. Render arrows through the boundary-ribbon path in `boundaries.rs` / `render.rs` (`build_globe_ribbons`, `build_flat_ribbons`) as two-segment polylines: shaft plus a V head. Arrows must appear on both canvases and follow the projection.
7. Velocity field layer. Add `Layer::VelocityField`, name "Velocity field". Same base as step 6. Instead of one arrow per plate, draw one white arrow per sample cell on a fixed sample set: every cell at icosphere level 4 (2,562 samples), mapped to its containing cell at the active level. Direction and length as in step 6, length capped at 2.5% of canvas width.
8. Both new layers must respect `viewing_kf`: arrows come from the keyframe being viewed, not the latest.
9. Tests. Add a unit test that `v = ω × r` at a pole gives zero speed and at the rotation equator gives `speed_deg_my` in the tangent plane. Add a test that `Layer::ALL.len()` equals the number of enum variants.
10. Run `cargo test --workspace`. Confirm the goldens are unmoved.
11. Screenshots to `docs/media/ui-controls/`: `plate-velocity.png`, `velocity-field.png`, both at seed `cyrus`, Draft L6, t = 600 My, Split view, Eckert IV.
12. Commit, push, open a pull request titled `WO-0004: UI controls and velocity layers`. Wait for CI. Merge when green. Delete the branch and run `git worktree remove ../WorldMaker-ui`.
13. Report to Dan in plain language: where each new control is, and the two screenshots.

DONE WHEN. PR merged; the three screenshots exist; `cargo test --workspace` green on `main`; worktree removed.


FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
