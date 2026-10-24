# Repository instructions

Keep this file to rules that change how work is performed. Design explanations
and build details belong in the [README](README.md), measurements in
[docs/benchmarks.md](docs/benchmarks.md).

## Workflow

- Strict no backward-compatibility or legacy paths.
- No squash merges
- No change logs
- After test changes, run `bun test` and `bun run typecheck`, once each.
- Run `bun run fixtures` only when a spec in `test/fixtures.ts` changes, and
  commit what it writes to `test/data`.
- Put temporary files and test configuration under `tmp/`. Always run local
  Python through `uv` (GitHub Actions excluded).
- Do not infer GUI or network capabilities from an SSH attachment. A tmux server
  retains the environment and access of the user that started it. Test a
  capability once and read its error; being able to drive the GUI is still not
  permission to do so.

## Benchmarks

- The module is for Chrome, whose engine is V8: measure it under Node
  (`node bench/decode.ts`, `bench/run.sh`), never under Bun, whose
  JavaScriptCore is Safari's, and Safari decodes the Mac's HEVC itself.
  `bun test` runs the tests; the loader uses only Node's APIs, which Bun has
  too, so both run it.
- Measure only on an idle machine, never on this workstation while anything
  else runs. The benchmark host is the ssh Host `hevc-lab` (its address and
  key in `~/.ssh/config`), an idle VM. Stage everything there under
  `~/codes/staging-area`, as ../devtools has it: the repository rsynced to
  `hevc-wasm`, the drive's `bench` folder of captures to
  `hevc-artifacts/bench`, and the runs' worktrees, builds and raw rows to
  `hevc-wasm-bench`. The scripts of a run live in `tmp/bench-lab` here,
  with a README of the host.
