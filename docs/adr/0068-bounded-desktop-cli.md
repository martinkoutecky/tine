# 0068. The desktop binary exposes a bounded, scriptable CLI

- **Status:** Proposed (og-D port of master's ADR 0068, which Martin accepted
  2026-09-20; the og shape below differs on `--output`, `--replace` and `doctor`).
- **Date:** 2026-09-29
- **Unit cost:** none — no persisted record, index or transport record; a headless
  command reads the graph and, for `export`, creates one new folder.

## Context

The og desktop executable accepted a graph path, `--capture` and `--debug`, plus a
hand-rolled `export`/`doctor` parser in `cli.rs`. Three places then re-read argv
independently — `cli::dispatch`, `graph::resolve_root` and the single-instance
handler — and they disagreed: a second `tine open GRAPH` was forwarded to the
running instance as a page called `open` under the caller's directory, so the
documented command opened the wrong folder. Nothing could print a man page, and a
Windows GUI-subsystem binary printed nothing at all from `--help`.

## Decision

One typed clap schema (`Cli`/`Command` in `src-tauri/src/cli.rs`) owns parsing,
`--help`, `--version`, the generated man pages, the cold GUI launch and the
forwarded second-instance launch. `cli::launch_request(argv, cwd)` returns
`Focus | Open(path) | Capture`; `resolve_root`, the setup-time capture check and the
single-instance handler all consume it, so there is no second argv parser.

- `tine open GRAPH` and `tine capture` name the existing GUI actions; `tine GRAPH`,
  `tine --capture` and `--debug` keep working.
- `tine export static|live GRAPH --output PARENT [--name NAME] [--all-pages]` and
  `tine doctor GRAPH` run headlessly. **og keeps its own export contract:**
  `--output` is an existing absolute folder outside the graph and both formats
  create one new child there (og's existing export contract: publication never
  writes inside the graph and there is no `--replace`). Master's graph-relative
  `--output` and `--replace` are not ported.
- On Windows the parent console is attached (`AttachConsole`) only for terminal
  commands; GUI launches never attach.
- Eight man pages are generated from the schema into `docs/tine*.1`; a test fails
  when they drift from it or when a page is missing from the `.deb`/`.rpm` file
  lists in `tauri.conf.json`. `TINE_UPDATE_MAN_PAGE=1` regenerates them.

## Consequences

Scripts get stable exit codes (0 ok, 1 command failure, 2 usage error) without a
webview. The desktop binary gains `clap` (runtime) and `clap_mangen` (dev-only), a
small build-time cost that replaces a parser that had already drifted from its
callers. Adding a command means changing the schema and its behaviour together.
Query-specific export, a server mode and graph-mutating commands stay out of the
CLI until they have contracts of their own.
