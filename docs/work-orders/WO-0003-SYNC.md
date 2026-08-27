# WO-0003-SYNC: bring local branches and GitHub main into one line

CONTEXT. On 2026-08-27 the repo has three lines of work that do not agree. GitHub `main` is at `0ccc5eb` (PR #12, coast-crop screenshot fix), two commits past local `main` (`bb0535b`). Local branch `feat/feel-pass-motion` (WO-0003-S2) is in the primary checkout and is not on GitHub. Local branch `feat/feel-pass-pending` (WO-0003-S3) is in the worktree `../WorldMaker-trackA` and is not on GitHub. WO-0003-S4 states "Sessions 1–3 are merged" as its precondition. This work order makes that statement true.

RULES. Single-track. No subagents. Do not edit source files except to resolve merge conflicts. Do not touch `.DS_Store` files. If any step fails, stop and report the failing step and the exact error. If the usage limit nears: commit, push, stop with a one-line note.

STEPS.

1. Run `git status --porcelain` in the primary checkout. Ignore lines that name `.DS_Store`.
2. If other lines remain, stop. Report the file names to Dan. Do not commit them.
3. Run `git -C ../WorldMaker-trackA status --porcelain`. Ignore lines that name `.DS_Store`.
4. If other lines remain, stop. Report the file names to Dan. Do not commit them.
5. Append `.DS_Store` on its own line to `.gitignore` in the primary checkout.
6. Run `git fetch origin --prune`.
7. Run `git checkout main`.
8. Run `git pull --ff-only origin main`. If the pull is not a fast-forward, stop and report.
9. Commit the `.gitignore` change to `main` with message `chore: ignore .DS_Store`.
10. Run `git push origin main`.
11. Run `git checkout feat/feel-pass-motion`.
12. Run `git rebase main`. Resolve conflicts only by keeping the intent of both sides. Record each conflicted file in the report.
13. Run `cargo test --workspace`. If it fails, stop and report the failing test names.
14. Run `git push -u origin feat/feel-pass-motion --force-with-lease`.
15. Open a pull request from `feat/feel-pass-motion` into `main` with title `WO-0003-S2: plate-motion liveliness`. Use `gh pr create`.
16. Wait for CI on that pull request. Use `gh pr checks --watch`. If CI is red, stop and report the failing job.
17. Merge the pull request with `gh pr merge --merge --delete-branch=false`.
18. Run `git checkout main`.
19. Run `git pull --ff-only origin main`.
20. Run `git -C ../WorldMaker-trackA rebase main`. Resolve conflicts only by keeping the intent of both sides. Record each conflicted file in the report.
21. Run `cargo test --workspace` inside `../WorldMaker-trackA`. If it fails, stop and report the failing test names.
22. Run `git -C ../WorldMaker-trackA push -u origin feat/feel-pass-pending --force-with-lease`.
23. Open a pull request from `feat/feel-pass-pending` into `main` with title `WO-0003-S3: pending edits`. Use `gh pr create`.
24. Wait for CI on that pull request. Use `gh pr checks --watch`. If CI is red, stop and report the failing job.
25. Merge the pull request with `gh pr merge --merge --delete-branch=false`.
26. Run `git checkout main` in the primary checkout.
27. Run `git pull --ff-only origin main`.
28. Run `cargo test --workspace` on `main`. If it fails, stop and report the failing test names.
29. Run `git log --oneline -12 main`. Confirm that PR #12, the S2 merge, and the S3 merge all appear.
30. Run `git worktree prune --dry-run`. Do not run it without `--dry-run`. Report its output.
31. Commit this work order and the two checked boxes in `docs/work-orders/WO-0003-feel-pass.md` for S2 and S3, if those boxes exist, with message `docs: WO-0003-SYNC complete; S2 and S3 merged`.
32. Run `git push origin main`.
33. Report to Dan in plain language: the two PR numbers, the commit hash of `main`, each conflicted file and how it was resolved, the test counts, and the output of step 30.

DONE WHEN. `main` on GitHub contains PR #12, the S2 pull request, and the S3 pull request; `cargo test --workspace` passes on `main`; the primary checkout is on `main` with a clean status.
