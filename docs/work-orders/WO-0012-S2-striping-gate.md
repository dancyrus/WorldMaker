# WO-0012-S2: striping gate at Dan's numbers

CONTEXT. Land-striping fix, session 2 of 2. S1 is merged and Dan has chosen the gate numbers from S1's measured runs (his ruling; the numbers arrive in the paste line or the chat alongside it). This session arms the anti-regression gate. No goldens move. Islet planation is EXCLUDED by Dan's ruling: the transport fix stops debris creation; if dead volcanic islets ever need removal, wave erosion enters later as a real process, not as a special-case rule here.

RULES. Single-track. Branch `feat/striping-gate` from `main`. Checkpoint commits every 30–45 minutes. Determinism rules apply. No subagents except a final verification pass capped at three lookups. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Run `git pull --ff-only origin main`. Create the branch. Confirm Dan's chosen numbers are in hand; if not, stop with `NOT DONE — awaiting Dan's gate numbers`.
2. Gate file `crates/worldmaker-sim/tests/land_striping_gates.rs`, built on the probe's metrics (the probe stays as the diagnostic, unchanged). Two 2 Gy L6 runs, seeds cyrus and 42, at 24 plates/land 0.40 (the canonical gate params). ARMED clauses at Dan's chosen values:
   2.1 Chain fraction (land cells with <=2 land neighbours) <= Dan's ceiling at every sample after 100 My.
   2.2 `cont_gained_by_advection` delta per 100 My <= Dan's churn bound (post-fix this measures ~0; the bound catches any re-raster regression).
   2.3 No stranded chain component: no all-chain land component of >= 4 cells farther than 600 km from any current trench, hotspot, or active arc band.
   2.4 Crust-volume ledger residual exactly zero (re-assert here so a transport regression trips this file too).
3. Record the gate run to `docs/results/land-striping-gates-<machine>.json`. Decision-log row: gates armed, Dan's numbers, and which S1-measured run they came from.
4. Run `cargo test --workspace`: green. Commit, push, PR `WO-0012-S2: land striping gate`. Merge when green. Delete the branch.
5. Report to Dan, under 200 words: gate results at both seeds and the WO-0009-S3 paste line (Phase 2 resumes).

DONE WHEN. PR merged; armed clauses green at both seeds at Dan's numbers; results JSON and decision-log row committed; report delivered.

FINAL LINE. After the report, print this block exactly, as the last output of the session. Print it only when every DONE WHEN condition holds. If the session stops early for any reason, print `NOT DONE` and the reason instead.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
