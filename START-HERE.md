# START HERE — for any new Claude Code session

You are picking up work on WorldMaker. Do this, in order:

1. **Read [CLAUDE.md](CLAUDE.md)** — ground rules, constraints, and lessons learned.
2. **Read the work-order queue in [docs/work-orders/](docs/work-orders/)** — work
   orders are numbered `WO-XXXX-*.md`. Find the lowest-numbered order that is not
   marked complete.
3. **Take the next open order.** Keep its acceptance checklist updated as you go.
   Close it (mark complete, with the date) only when every box is checked and CI is
   green on main.

Context you should also know:

- Dan (the user) does not code and does not use git. Report to him in plain
  language. Never ask him to read code, run commands, or make merge decisions.
  You own this repository end to end, including GitHub.
- The plan of record is [docs/plan/roadmap.md](docs/plan/roadmap.md); the science
  spec is [docs/plan/science-outline.md](docs/plan/science-outline.md); every
  notable choice goes in [docs/plan/decision-log.md](docs/plan/decision-log.md).
- Benchmark and test numbers only count when committed as machine-labelled JSON
  under [docs/results/](docs/results/) — see its README for the schema.
