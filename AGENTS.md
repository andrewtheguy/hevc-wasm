# Repository instructions

Keep this file to rules that change how work is performed. Design explanations,
measurements, and build details belong in the [README](README.md).

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
