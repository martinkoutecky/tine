# 0039. The canonical graph root is a hard filesystem boundary

- **Status:** Accepted
- **Date:** 2026-07-10

## Context

Logseq configuration controls the page and journal directories and the journal
filename pattern. Treating those strings as trusted paths lets an absolute path,
`..`, or an escaping symlink route ordinary saves, deletes, snapshots, and restores
outside the graph selected by the user. Multi-window ownership also cannot be safe
when one open graph is nested inside another graph's recursively scanned tree.

## Decision

- Runtime graph opening uses a canonical root and validates configured managed
  directories as contained relative paths.
- Every path-addressed save validates its concrete target against that root; a
  formatted journal stem is data, not an implicitly trusted path.
- The window registry rejects equal, ancestor, and descendant graph roots owned by
  different windows.
- Backup identity derives from a cryptographic digest of the canonical root and
  every restorable snapshot records that root in a verified manifest.
- Invalid layouts fail closed with a clear error. Tine never substitutes another
  directory and writes there silently.
- One compatibility exception is an `assets` symlink/junction whose canonical
  target is outside the graph. Tine shows that exact resolved directory and
  requires explicit device-local approval before opening the graph. The grant is
  keyed by canonical graph root and canonical asset target; retargeting fails
  closed. Asset reads/writes receive only that separate capability, so approving
  it cannot widen page, journal, configuration, publish, backup-identity, or
  other graph-managed paths.

## Consequences

Safe nested relative directories remain supported. Apart from the explicitly
approved external-assets capability above, graphs that intentionally point managed
directories outside the selected root must move those files inside the graph
before Tine will write them.
