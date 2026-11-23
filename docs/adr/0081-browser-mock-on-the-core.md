# 0081. The browser mock answers no graph question: the browser core does

- **Status:** Accepted. Supersedes [0016](0016-mock-backend-intentionally-lossy.md).
- **Date:** 2026-11-23

## Prior decisions

Sources: the decision journal, `tine-agents/specs/notes/browser-core/SLICE-5.md` (Prior decisions, Rules 1-10) and
the map of the old mock, `tine-agents/subagent-tasks/notes/browser-core-5-map.md`.

| Record | Says | Here |
| --- | --- | --- |
| D-20261120-ONE | one answerer per question, codebase-wide, without growing the code | follow: the mock's 24 graph answers go; the core answers them |
| D-20261122-WEX, D-20261122-CRU | the browser core lands in slices, then the page-name fold; the mock counts as an answerer everywhere and is deleted before the fold | follow: this is slice 5 |
| ADR 0016 | the mock is a lossy dev double; fidelity beyond the harness is a non-goal; real behaviour comes from the real backend | replace: the mock keeps no answers of its own, so it has no fidelity to lose. Its rule that write paths beyond one page's text belong to E2E stays (Decision 4) |
| BCORE-PACK (b) | the app module holds the parser only; the core is its own chunk | follow: the mock loads that chunk |
| ADR 0006, 2026-09-20 "reject dual-bundle split" | one frontend bundle; the core's bytes are inlined and initialized explicitly | follow: the mock is a lazy chunk of the same bundle, not a second build |
| Slice 5a (16cee4693) | `BrowserGraph::replace_files`, `submit_page_json`, file times and the Rule 2 store commands | follow: the mock edits only through these |

## Context

`src/mock.ts` grew to about 1,700 lines that answered graph questions in TypeScript: name resolution, inventory,
backlinks, search, aliases, queries, templates and more. Each one was a second answerer beside the Rust store, and
every change to the store's answer tempted a matching change in the mock. ADR 0016 accepted the drift as harmless to
users. It was not harmless to the work: tests and screenshots passed against answers the app never gives, and the
page-name fold would have had to keep the TypeScript copies in step.

Since slice 4, the browser core (`lsdoc-wasm`'s `BrowserGraph`) answers every read a published export needs, from
the same crates as the desktop store. Slice 5a gave it an editable mode: `replace_files` and `submit_page_json`
through the store's own page serializer. Logseq's browser demo, and the web builds of Obsidian and VS Code, take the
same shape: one engine, with storage behind an interface.

## Decision

1. **The mock answers no graph question.** One forward table, `src/graphForwards.ts`, maps each `Backend` read to
   its core command and result. `publishedBackend.ts` and `src/mock.ts` both spread it, so there is no second
   adapter.
2. **The graph is files.** The mock opens `src/mock/graph/` (and the files its URL flags add under `src/mock/flags/`
   or generate) as an editable `BrowserGraph`. Each file is given a fixed time, so recency order is deterministic.
   Journals are named by `:journal/file-name-format` and namespaces by `:file/name-format`, as on disk.
3. **An edit replaces files.** A page submit goes through `submit_page_json`, the desktop page host's serializer.
   A delete goes through `replace_files`. A later read, search or Open answers the saved text. A window reload
   reopens the fixture files, as a browser reload of the preview does. The mock's graph is editable, so its page
   reads are writable (`WholeGraph::page_read_only`: only an Org page that does not round-trip reads read-only),
   while a published export's are read-only. The host's `externalWrite` (not a `Backend` member) stands in for the
   desktop's file watcher: scripts write a file through `replace_files` and the window gets it as page mail.
   The core's page reads carry no file revision, so the mock's page host names one per file (its session and the
   file's version, which every write moves) and uses it in Open, submit mail, the publication wait and the exact
   delete. A delete by name refuses when another file claims the same name. A request made before a window reload
   sends no mail and moves no version after it.
4. **Write paths beyond one page's text are refused.** Rename, merge, conflict resolution, journal filename
   migration and its proposals, guide copy, print and the rename probe (`renameWouldChange`, the rename preview) all
   refuse, with one localized message. A block move to another page is refused before either page is written. Any
   `Backend` member the mock does not define refuses the same way. End-to-end tests on the real backend cover these
   paths.
5. **Host state stays host state.** Plugins, assets, highlights, session, workspaces, app settings, diagnostics,
   git, quick share, the conflict-demo inventory (the `?conflicts` duplicate day) and the guide flag are not graph
   questions. They stay in TypeScript. A journal file's text is the core's answer (`read_journal_file_json`).
6. **Config setters stay no-ops** until the core exposes `config.edn` edits. The `tine-graph-features` setters
   operate on a `Store`; exposing them as a `BrowserGraph` command is Rust work with a wasm-size cost, left to a
   later slice.
   - **Scope exception.** This departs from slice 5's Rule 6
     (`tine-agents/specs/notes/browser-core/SLICE-5.md`), which has the setters edit `logseq/config.edn` through
     the core and `replace_files`, so a Settings change shows in `loadGraph`. Slice 5 ships without it. The
     preview keeps its earlier behaviour: a setter changes nothing, and `loadGraph` answers the fixture config. The
     exception closes when the core exposes the slice 4b config edits as a `BrowserGraph` command.
7. **Loading.** `backend.ts` reaches the mock only through `src/browserMock.ts`, by one dynamic import, outside
   Tauri and outside a published export, and only in builds that carry it: the dev server, the tests and
   `npm run build:mock`. The core loads on the mock's first graph question.
   - The `browserCoreAssets` Vite rule admits exactly two importers of the core chunk: the publication worker and
     `src/mockGraph.ts`.
   - A build without the mock (`npm run build`, the quick-share viewer) fails if its main bundle contains a
     `src/mock*` module or the core.

## Consequences

- A test or screenshot sees the store's real answers. A test that relied on a mock answer the core gives
  differently is updated to the core's answer. A test that drives a refused write path fakes that host member in
  the test.
- Tests that reach the core load it once per test file, about 0.1 s. Vitest serves the core's wasm bytes from disk
  (`scripts/lib/vitest-wasm-bytes.mjs`) rather than through its module server. Without that, the 6 MB module made
  the unit suite about 40% slower.
- Screenshot and smoke scripts that run against the mock build it with `npm run build:mock`. Scripts that used to
  patch mock source at load time use fixture files or URL flags instead.
- The core gives each block a positional id (its file and sibling path). Scripts name a fixture block by its
  text and ask the core for its id (`scripts/lib/mock-block-ids.mjs`); `id::` uuids in the fixture files are what
  block references and embeds name.
- Settings changes in the browser preview do not reach `loadGraph` until Decision 6's follow-up.
