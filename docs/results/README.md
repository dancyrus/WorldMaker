# Results files

Every test and benchmark number in this project is committed here as JSON. A number
reported only in chat does not count.

## File naming

`<topic>-<phase>-<machine>.json` — e.g. `perf-phase0-DANPC.json`,
`determinism-phase0-DANPC.json`. `<machine>` is the Windows computer name (or
hostname) of the machine that produced the numbers. All performance budgets in this
project are benchmarked against Dan's PC (i7-12700KF, RTX 3080) only.

## Schema

```json
{
  "machine": "DANPC",
  "date": "2026-08-19",
  "app_version": "0.1.0",
  "metrics": {
    "any_metric_name": 123.4,
    "nested_groups_are_fine": { "grid_build_ms_L7": 812.0 }
  }
}
```

- `machine` — computer name, matches the filename.
- `date` — ISO date the numbers were produced.
- `app_version` — the workspace crate version that produced them.
- `metrics` — one JSON object; keys are machine-readable snake_case names with the
  unit in the name (`_ms`, `_fps`, `_count`, `_hash`).

Files are written by `worldmaker-io::results::ResultsFile` — use it rather than
hand-writing JSON, so the schema stays consistent.
