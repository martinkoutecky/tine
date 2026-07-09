# 0036. In-app Guide pages are read-only bundled templates

- **Status:** Accepted
- **Date:** 2026-07-09

## Context

Tine now has enough advanced surface area, especially Sheets, that static docs and
the first-run demo graph are not enough. Users need how-to material available while
their own graph is open, with live Tine rendering and links to adjacent features.

The hard constraint is data safety. A guide page should behave like a page for
rendering, navigation, and tabs, but it must not silently become a real file in the
user's graph. The only write the Guide is allowed to perform is the explicit
"Copy the guide into your graph" action, and that action must never overwrite an
existing note.

## Decision

We will ship the Guide as bundled markdown templates compiled into `tine-core`.
The backend exposes them as `PageDto`s flagged `read_only` and `guide`; the frontend
loads them into the store under the virtual `Tine-guide/` namespace and treats
intra-guide links as links to that namespace.

Guide pages are excluded from graph persistence, search/page-list/backlink surfaces,
and normal edit/rename/context-menu actions. The save boundary refuses to persist a
`guide` page, and the core graph write path has the same guard as a second line of
defense.

The only mutation path is `copy_guide_into_graph`, which copies the complete
bundled guide set under the real `tine-guide/` namespace. It rewrites links whose
targets are other bundled guide pages to point at the copied namespace, copies
referenced bundled assets into `assets/` without clobbering existing files, skips
any existing `tine-guide/*` pages, and then navigates to the copied counterpart of
the guide page the user was viewing. The one-time Guide announcement is stored in
graph config metadata, not browser localStorage, so it survives WebKit's storage
behavior and stays graph-scoped.

## Consequences

The Guide can reuse the normal page renderer, tabs, and link interactions without
inventing a separate documentation viewer. It also gives users an editable,
interlinked sandbox namespace that remains plain Logseq markdown after copy.

The cost is that page-like code now has to respect a second non-persistent page
class in addition to organic read-only pages. Any new persistence, search, or page
listing path must either derive from the normal graph index or explicitly check the
`guide` flag before writing or surfacing virtual pages.
