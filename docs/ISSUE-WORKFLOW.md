# Issue workflow

The regression catalogs and GitHub are one operating system, not separate lists.
GitHub milestones and the [Tine Project](https://github.com/users/martinkoutecky/projects/1)
are the public planning surface;
`docs/BACKLOG.md` explains durable product direction and must not duplicate a
second manually maintained board.

## Bugs

Verified high/critical audit findings and maintainer-internal bug reports are
handled immediately. GitHub reports use the clear/unclear triage below. Once a
behavior is accepted as a bug, all three sources share the same catalog,
fail-before test, implementation, deployment, and release lifecycle.

1. Triage a report as clear/reproducible, unclear, or not a bug. An accepted bug
   gets a regression entry **before** production code changes: UI/native behavior
   goes in `tests/ui-regressions/catalog.json`; other bugs go in
   `tests/regressions/non-ui.json`.
2. For an unclear report, ask precisely for missing version/platform/install
   details, exact steps, and a minimal anonymized sample graph or block text. Add
   `needs-info`; do not guess at a fix.
3. For a clear bug, prove it before the fix, implement the smallest safe change,
   and run the cheapest test layer that can actually observe it. Visual, caret,
   focus, filesystem, and WebKit behavior require browser/native evidence.
4. Once the verified fix is pushed to `master`, comment that it is fixed on
   master and expected in the next release (normally 1–2 days), and leave the
   issue open with `fixed-on-master`.
5. After the relevant platform artifact is published, comment “closing, should
   be fixed in vX; please report back here if not,” link the release, and close a
   wholly addressed issue. A new non-maintainer comment automatically reopens a
   closed issue with `needs-triage`.

## Feature requests

Before recommending implementation, scan every other open issue for adjacent or
duplicate needs and research how strong outliners/note apps—and other relevant
software—solve the interaction. Do not implement a reporter's proposed mechanism
blindly when a more standard or capable design is available.

Classify the request against the current plugin API:

- Plugin-capable: recommend one of **make a plugin**, **add to core**, or **defer
  for discussion**. Prefer a plugin for useful but niche/uncertain behavior; use
  core for broadly correct product behavior.
- Not currently plugin-capable: also say whether it *should* become possible by
  a bounded plugin-API extension. Recommend **do now**, **future minor release**,
  or **defer for discussion**.
- Omnibus requests are decomposed into small bugs/features. Straightforward
  pieces can move independently; coupled design decisions remain queued.

Every proposal calls out data-safety, privacy/security, performance, compatibility,
and platform risk. A material performance-budget breach is a stop condition, not
an acceptable side effect of adding a feature.

## Planning and work sessions

Release milestones answer “which release”: v0.6.0 is Plugins and v0.7.0 is
Sync. The GitHub Project answers workflow `Status`, execution `Horizon`
(Now/Next/Later), and `Priority`; labels capture triage and decision state.
Decision sessions queue work without builds; after the maintainer says “go,” the
queued work is implemented, tested, pushed, and deployed autonomously. Product,
risk, or API-boundary decisions return to the decision queue rather than being
silently guessed.
