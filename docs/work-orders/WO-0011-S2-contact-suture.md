# WO-0011-S2: contact-proportional suture (anti-dumbbell)

CONTEXT. Plate-shape fix, session 2 of 3, same branch as S1 (`feat/plate-shape`). The weld GATE (contact extent, kinematic lock, ocean closed) is sound Wilson-cycle physics and stays untouched. The merge ACTION is the defect: `update_pair_timers_and_sutures()` relabels the loser's entire cell set in one step, so a partial contact welds a dumbbell. Real welding is progressive — the India–Eurasia suture advanced along the front over tens of My. This WO replaces the wholesale relabel with a front-limited merge, then regenerates the tectonic goldens once: the sixth sanctioned move, covering S1 and S2 together.

RULES. Single-track. Continue on `feat/plate-shape` with S1 complete. Checkpoint commits every 30–45 minutes. Determinism rules apply. No subagents except a final verification pass capped at three lookups. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Confirm the branch holds S1 (regularization pass in, goldens `#[ignore]`d).
2. Weld state. On maturity (the existing gate and `SUTURE_AFTER_MY` clock, unchanged): push the `Suture` event, bump `suture_count`, stamp `suture_at_my` on the contact cells, and set `youngest_suture_my` — all as today. Do NOT relabel the loser. Instead append to a new `welds: Vec<Weld>` with `{ winner, loser, front_km: 0.0 }`. A weld is permanent: it never un-matures, and the pair leaves the pair-timer bookkeeping.
3. Slaved motion. While a weld is live, the loser is mechanically part of the winner: `motion_update()` copies the winner's pole and `speed_deg_my` onto the loser instead of running its own balance. The pair already passed the lock gate (< `SUTURE_LOCK_CMYR` relative), so this is a small correction — and it stops the advancing front from shearing against an independently rotating loser.
4. Front advance. Each step: `front_km += WELD_FRONT_KM_MY * DT_MY`. Transfer to the winner every loser cell whose graph distance from the stamped contact cells, walked over loser cells only, is at most `front_km` (distance = hops x `cell_spacing_km`; serial, id-ordered BFS). Stamp `suture_at_my` at transfer time on transferred cells that still touch remaining loser cells — the scar advances with the front. `plate_cells`, `cont_cells_per_plate` and the crust-volume ledger update per transferred cell, never in one lump.
   `WELD_FRONT_KM_MY = 50.0`, const with doc comment. Physical range 20–100: the India–Eurasia deformation front propagated on the order of 2000 km into Asia in ~50 My. The rate is in km, not cells, so the merge speed is grid-level independent.
5. Retirement. When the loser's cell count reaches zero, the existing fully-consumed path retires it; slab-ledger and live-rift inheritance transfer exactly as the wholesale path did, but at retirement (rifts whose cells transferred earlier follow their cells).
6. Interactions. A live weld's loser still advects, can still be rifted, and still runs third-party pair timers — a shrinking plate can be claimed by a second weld. If a loser is itself a winner of another weld, both fronts advance independently. `MAX_ALIVE_PLATES` logic unchanged.
7. Dumbbell probe test (committed): a two-plate L7 world with an off-centre partial contact, forced to maturity. Assert: the winner grows as one mass — at no step does it hold a neck narrower than 170 km joining two masses each above 2% of the sphere (Dan's ruled bound), and the winner's boundary/area never spikes in the firing step (bound taken from a clean run, commented). Deterministic.
8. Goldens. Regenerate ALL tectonic goldens exactly once (`print_tectonic_goldens` on Daniels-MacBook-Air), remove the S1 `#[ignore]` markers, verify the Phase 0 noise golden UNMOVED in the same suite run, and write the decision-log row: sixth sanctioned move, S1+S2 together, whole-world change by design.
9. Probe run (L6, Dan's params, seed cyrus, 2 Gy): record the table. Expect largest share well below the old 67% and alive above 7; the exact band is S3's measurement, not a gate here.
10. Run `cargo test --workspace`: green. Commit, push, PR `WO-0011 S1+S2: plate-shape integrity — anti-fray + contact-proportional suture`. Merge when green. Delete the branch.
11. Report to Dan, under 300 words: probe table, what a weld now looks like over time, and the S3 paste line.

DONE WHEN. PR merged; goldens moved exactly once with the decision-log row; dumbbell test green and committed; workspace green.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
