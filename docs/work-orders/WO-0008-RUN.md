# WO-0008-RUN: execute S1 then S2 unattended

CONTEXT. Dan runs this overnight. It chains two work orders in one session.

STATUS. S1 done (PR #25 merged 2026-08-28; all four S1 gates + armed set green at both seeds). S2 done (PR #26 merged 2026-08-28; tag v0.2.3; goldens regenerated — fourth sanctioned move; screenshots committed). BOTH ORDERS COMPLETE.

STEPS.

1. Read `docs/work-orders/WO-0008-S1-closure.md`. If its DONE WHEN conditions already hold on `main` (check the merged PRs and gates), skip to step 3.
2. Execute WO-0008-S1 to completion, including its merge. Do not print its FINAL LINE banner; record "S1 done" in this file instead.
3. Read `docs/work-orders/WO-0008-S2-orogens-arcs.md`. Execute it to completion, including its merge and the tag `v0.2.3`.
4. Write both reports for Dan, S1's then S2's, in one message.

RESUME. If a run of this work order stopped early, the same paste resumes it: the step-1 check and the per-order checkboxes identify what remains. Never redo a merged PR.

USAGE LIMIT. If the limit nears at any point: commit, push, update the checkboxes in whichever order is in progress, and stop with `NOT DONE — resume with the same paste`.

FINAL LINE. Print the banner below only when BOTH orders' DONE WHEN conditions hold and tag `v0.2.3` exists. Otherwise print `NOT DONE` and the reason.

```
██████╗  ██████╗ ███╗   ██╗███████╗
██╔══██╗██╔═══██╗████╗  ██║██╔════╝
██║  ██║██║   ██║██╔██╗ ██║█████╗
██║  ██║██║   ██║██║╚██╗██║██╔══╝
██████╔╝╚██████╔╝██║ ╚████║███████╗
╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚══════╝
```
