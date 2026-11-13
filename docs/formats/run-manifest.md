# Run manifest, version 0

> Brought from game-printer with wave 9 of the port. As in game-printer, every `gp` command writes one. Different here: `--json` output doesn't carry a `run_manifest` key (other tools read gameprinter's JSON as it is), the manifest path is in `gp runs list`; a run that can't be written is skipped silently. gameprinter has no `budgets.toml` yet, so `gp metrics check` needs `--budgets`.

Every `gp` command writes one JSON manifest describing the run, so any result can be traced back to exactly what produced it. Defined by `RunManifest` in `crates/gp-metrics/src/run.rs`.

Manifests go to `$GP_HOME/runs/<YYYY-MM-DD>/<run_id>.json`, and each run and its metrics are also recorded in `$GP_HOME/metrics.sqlite`. With `--json`, a command's output includes a `run_manifest` field with the manifest's path.

| Field | Meaning |
|---|---|
| `schema` | Run manifest version (0) |
| `run_id` | UUID v7, so IDs sort by time |
| `command`, `args` | The subcommand (such as `store put`) and the full argument list |
| `run_group` | `$GP_RUN_GROUP` when set. `just` sets one per invocation, so every run in one `just check` shares it |
| `started_at`, `finished_at`, `duration_ms` | RFC 3339 times and wall duration |
| `status` | `{"kind": "ok"}` or `{"kind": "failed", "message"}` |
| `git` | `commit`, `dirty` (tracked files changed) and `branch`, when run inside a work tree |
| `host` | `os` and `arch` |
| `tool` | `name` and `version` of `gp` |
| `tools` | Other programs the run depended on, as `[{name, version}]`, when any. `gp metrics time --tool wasmtime=48.0.7` adds one; the wasm checks use it |
| `inputs`, `outputs` | `[{name, hash, bytes}]`, with hashes prefixed by algorithm: `blake3:` for store objects, `sha256:` for ROM identities |
| `metrics` | `[{name, value, unit}]` |
| `spans` | `[{name, target, parent, duration_us}]` for every `tracing` span that closed during the run |

Every run also records `gp.<command>.duration` in milliseconds, with spaces in the command name replaced by dots (for example `gp.store.put.duration`).

## Metric names

Dotted, lowercase, most general part first: `check.test.native.duration`, `store.verify.corrupt`, `roms.found`. Units are explicit (`ms`, `bytes`, `count`, `code`, `frames/s`, `ns`). `gp bench` names its metrics `bench.<source>.<baseline>.<what>` and the criterion results `bench.criterion.<group>.<name>.ns` ([benchmarks.md](../benchmarks.md)). A metric's name and unit never change meaning; when the meaning changes, use a new name.

## Budgets

`budgets.toml` at the repo root sets `max` or `min` for any metric. `gp metrics check` exits 1 if any budgeted metric is over, which fails `just check`. Two more files in the same format hold the budgets of checks outside `just check`: `budgets-cycle.toml` for `just check-open-platforms` and `budgets-roms.toml` for the ROM tier (`just check-roms`), each passed with `--budgets`.

With a run group (`--group`, or `$GP_RUN_GROUP`, which `just` always sets), only values recorded in that group count, and a budgeted metric the group never recorded fails as missing. A step that silently stops running can't pass on an old value. Without a group, the latest value ever recorded is checked, and a metric that was never recorded is reported but doesn't fail.

A budget can also set `os` (a list such as `["macos"]`) to apply only on those platforms, and `native_only = true` to be skipped when `GP_EMULATED=1`, for timing budgets that mean nothing under CPU emulation. A skipped budget is listed as `skip` with its reason, never dropped silently, and the summary line counts budgets checked, over or missing, and skipped.

## Report

`gp metrics report` writes `$GP_HOME/reports/metrics.html`: every metric's latest value, change from the previous value, a trend line over the last 30 values, its budget, and the commit it came from.
