# v0.5.10 reference-authoring native matrix

Date: 2026-07-16

Scope: approved C1 real-observation-boundary extension only. Production code,
the regression catalog, changelog, issue records, and release state were not
edited by this task.

## Implemented observations

`scripts/e2e-og-parity-references.mjs` retains the routed-page block-reference
proof and now performs the complete section 6.2 reference-authoring matrix with
literal WebDriver keyboard/pointer input:

- bare `/` -> Page reference -> active row-free `[[]]` lifecycle, followed by
  first-character results;
- adaptive exact, prefix, fuzzy-only, and nonexistent page-name ordering;
- byte-exact OG spacing off and the separate Tine continuation-space-on path;
- `/A`, `/priority`, `/kanban`, and `/query` active/order sentinels;
- Linux Mod-L selected, empty, and parser-recognized URL branches, including a
  `defaultPrevented` observation;
- selection-toolbar Link, simple slash Link, and a Settings-recorded Insert link
  shortcut override through the same editor boundary;
- typed policy changed through Advanced Settings and observed live, then after a
  same-XDG restart; existing-first changed through Settings and observed with a
  fuzzy candidate;
- real page-reference acceptance in a duplicate split-pane editor;
- Escape commit, exact graph Markdown on disk, process reload, and rendered
  reference text;
- on Linux, the established `scripts/e2e-capture.mjs` scenario is executed as a
  child gate, not named as proxy coverage. It performs the byte-identical cold
  non-default policy, page/tag Enter, main-Settings change, hidden persistent
  WebView reopen, and same-XDG cold-restart observations.

The JSON artifact at `receipt.json` records popup text/order, textarea values and
carets, persisted Markdown/rendering, policy state, and the quick-capture child
result. Screenshots and native logs remain under the scenario artifact directory.

## Checks run in this task

Passed:

```text
node --check scripts/e2e-og-parity-references.mjs
git diff --check -- scripts/e2e-og-parity-references.mjs
```

The staged release binary is owned by the parent G1 gate, so this task did not
claim a native runtime pass against an unstaged or stale binary.

## Exact candidate command

After building/staging the exact candidate with the matching embedded frontend:

```bash
source scripts/env.sh
TINE_APP=/absolute/path/to/the/staged/candidate npm run e2e:linux:release -- --scenario=og-parity-references
```

This command runs the routed-page matrix under Xvfb/private D-Bus; the matrix in
turn runs the real native quick-capture observation with Openbox and xdotool.
