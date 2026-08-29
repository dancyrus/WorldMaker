# WO-0012-S1: conservative land transport (anti-striping)

CONTEXT. Land-striping fix, session 1 of 2. Runs AFTER WO-0009-S2 merges and BEFORE WO-0009-S3 (Dan's sequencing ruling, 2026-08-28). Diagnosis: Project doc `land-striping-diagnosis-v1` (Cowork). The short version: plate OWNERSHIP has a shape law (WO-0011) but crust CONTENT does not — every rotation commit re-rasterizes each coastline with one-cell error, shedding and duplicating full-thickness continental cells at 2,000–8,000 cells per 100 My (real arc creation: ~30). Shed debris is immortal in the plate interior and accumulates into motion-aligned dashed trains: chain-shaped land grows 0.5% → 12–21% over 2 Gy. The same churn dominates `cont_gained_by_advection` / `cont_lost_to_consumption`, and is the likely bulk of the open continental-inventory leak (0.14 vs the 0.05 gate). Probe: `crates/worldmaker-sim/tests/land_striping_probe.rs` (committed in step 1). This WO changes tectonic trajectories: ALL tectonic goldens AND the WO-0009-S2 terrain goldens regenerate once at the end of this session — the eighth sanctioned move, announced now (WO-0009-S2's was the seventh).

RULES. Single-track. Branch `feat/land-transport` from `main`. Checkpoint commits every 30–45 minutes. Determinism rules apply: serial cell-id order, fixed CCW ring order, no float reductions, no wall-clock RNG. No subagents except a final verification pass capped at three lookups. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch. Commit this work order, its S2 sibling, and `crates/worldmaker-sim/tests/land_striping_probe.rs` with message `docs: install WO-0012 S1-S2 + land striping probe`.
2. Conservative land transport in `advect()`. The principle: continental crust is material — a coastline is carried by the plate, never re-rasterized against it.
   2.1 Forward parcel pass, before the gather, serial in cell-id order over PREVIOUS land cells (`crust_type == 1`) of committing plates: `dst = nearest_cell(fwd * pos)`. If dst is unclaimed by a parcel this step, claim it. If dst is already claimed, probe dst's neighbor ring in fixed CCW order and claim the first unclaimed cell. If the whole ring is claimed (rare), the parcel MERGES into dst: no cell is created, and its crust volume books to the ledger explicitly. Non-committing plates' land cells claim their own cell.
   2.2 The gather consults the parcel map for CONTENT: a cell claimed by a parcel takes that parcel's full crust state (type, age, thickness, orogeny, rift, buildup, suture scar, slab fields as today). A cell claimed by no parcel can never copy land from a source cell — it resolves as ocean per the existing rules (ridge fill at divergent gaps, ocean crust copy otherwise). OWNERSHIP resolution is untouched: the WO-0008 seam rule and the WO-0011 regularization stay exactly as they are.
   2.3 Land count per plate now changes ONLY through real processes: subduction consumption at margins, ridge gaps, rift oceanization, arc conversion, relic-basin closure, and the WO-0008 terrane-accretion transfer (which keeps working — a parcel consumed at a hard-continent margin transfers, never deletes).
   2.4 The WO-0008 same-plate continental inventory guard in `advect()` retires — conservation is now structural. Keep `cont_gained_by_advection` as a diagnostic; it should read ~0 after this change, and S2 gates it.
   2.5 A WO-0011 ownership revert must not duplicate land: content follows the parcel outcome regardless of which plate wins the cell.
3. Crust-volume ledger: parcels carry their volume; the residual gate stays exactly zero; the 2.1 merge case books its volume explicitly in the same step.
4. Measurement for Dan's gate ruling (this report is how he chooses the test numbers — present measurements, not recommendations baked as decisions): run the striping probe at L6, 2 Gy, seeds cyrus, 42, and 7, at BOTH 12 plates/land 0.29 and 24 plates/land 0.40, before-fix (from the diagnosis tables) and after-fix. Record to `docs/results/land-striping-wo0012-<machine>.json`. Capture L7 app screenshots at Dan's recording settings (seed cyrus, 53 / 600 / 2000 My) to `docs/media/wo-0012/`.
5. Regenerate ALL tectonic goldens and the terrain goldens once (eighth sanctioned move), remove no gate, write the decision-log row, verify the Phase 0 noise golden UNMOVED in the same suite run.
6. Run `cargo test --workspace`: green. Commit, push, PR `WO-0012-S1: conservative land transport`. Merge when green. Delete the branch.
7. Report to Dan, under 300 words, in plain language: the before/after chain tables at all three seeds, what the screenshots show, the measured post-fix values (chain%, churn counters) laid out as CHOICES for the S2 gate numbers, and the S2 paste line. Dan picks the numbers; S2 arms them.

DONE WHEN. PR merged; parcel transport in with the guard retired; ledger residual zero; goldens moved exactly once with the decision-log row; measurements and screenshots committed; report with gate-number choices delivered.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
