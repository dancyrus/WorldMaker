# WO-0010: startup defaults hotfix

CONTEXT. Dan's ruling (2026-08-28): the app must start in Draft preset, Flat view, Eckert IV projection. Current defaults differ (High8 preset per WO-0003; view/projection set elsewhere in `WorldApp::new`). App-side only; sim hashes must not move. This is a hotfix: it may run before, between, or after WO-0009 sessions — branch from current `main`, whatever it is.

RULES. Single-track. Branch `fix/startup-defaults` from `main`. One short session.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch. Commit this work order with message `docs: install WO-0010`.
2. In `WorldApp::new` (`app.rs`), set the startup state: preset `Preset::Draft6`, `view_mode = ViewMode::Flat`, `projection = Projection::EckertIv`.
3. Check every startup path: CLI flags (`--preset`), the screenshot harness, and the perf loop keep their own explicit settings — only the interactive default changes. The committed screenshot scripts must not silently change resolution; confirm their flags are explicit and note it in this file.

   AUDIT NOTE (2026-08-28). Several scripted stages inherit startup state they
   don't set (the `--screenshots` trio and wo4 stage 0 inherit the projection;
   wo7, wo8-S0, and wo9 inherit view mode and projection; the perf loop
   inherits the projection), so the new defaults are gated in `Script::startup_state()`:
   any scripted mode keeps the pre-WO-0010 state (High8 `--preset` fallback,
   Split view, equirectangular); only the interactive launch changes.
   `--preset` still wins everywhere it did before; perf's pinned
   Standard7→High8→Ultra9 loop and graft-7 screenshot parity (forced
   Standard7) are untouched. Resolution: the window opens at a hard-coded
   1600×900 (`main.rs`), each WO shot driver sends its own explicit
   `InnerSize` (1600×900; 1440×900 for hud-1440), and the committed scripts
   are explicit — `detail-sweep.sh` pins `--seed --preset high8 --detail 1
   --detail-octaves --detail-amp-m`; `perf-feelpass.sh` runs `--perf-out`,
   whose preset loop is pinned in code. No script or harness resolution
   changed.

4. Test: a unit test on the default-constructed `WorldApp` state asserting the three values.
5. Run `cargo test --workspace`. Confirm goldens unmoved.
6. Screenshot of first launch to `docs/media/wo-0010/startup.png`.
7. Commit, push, PR `WO-0010: startup defaults`. Merge when CI is green. Delete the branch.
8. Report to Dan in one paragraph.

DONE WHEN. PR merged; the app opens in Draft, Flat, Eckert IV; harness paths unchanged; workspace green.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
