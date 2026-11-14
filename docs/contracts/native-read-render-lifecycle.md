# Native reads and renderer work ownership

Working-set admission compares returned page generations at the synchronous
installation boundary, including after a scheduled commit wait. Each relevant
invalidation during an outstanding read can justify one sequential reread;
N such invalidations cost at most N+1 reads, with concurrent changes coalesced.
Unrelated changes add no reads. Owner retirement or a cancelled commit returns
stale; an actual owned read failure rejects. A finite burst of invalidations
therefore ends with the current answer, without a fabricated read failure.
`readAdmission.test.ts` and `Page.readAdmission.test.tsx` enforce this boundary.

Live block rows and their editors read the live node while it exists. During the
reactive flush that deletes or merges a block, render facts retain the node
captured at mount until the owner retires; they must not take down the containing
page. Both use the shared block node binding. Writes still require a live slot,
and detached editors cannot write into a replacement node. The retiring-row and
stale-input cases in `Block.deleteRendered.test.tsx` enforce these lifetimes;
`Page.windowing.test.tsx` also checks successive structural edits keep the editor,
later rows and unchanged window shells.

Read-only IPC for assets, opted-in local images, dropped CSV/TSV files,
journal inspection and sync-conflict diffs runs blocking I/O on the blocking
pool. Graph reads capture their admitted graph slot before dispatch. Device
reads retain canonical-path and bound-graph refusals. Byte responses, read
limits and error tokens are unchanged. `nativeReadWorkers.guard.test.ts`
ratchets synchronous command additions; the Rust `off_ui_slow_read` test proves
the shared dispatcher yields while a read waits (L7s4-03).

Quick Capture has no general graph slot. Shared editor imports, scratch edits
and rendered page links must not start block-reference count reads (including
the warm-cache probe) or page-inventory reads there. Page/tag completion uses
the dedicated bounded capture query. `capture.graphReaders.test.tsx` exercises
the capture entry point with native-equivalent refusals for general reads.

CM6 input transactions own completion refresh. The matching platform input
notification does not start another read or invalidate the first reply; a
notification before DOM reconciliation waits for that reconciliation. Standalone
notifications still refresh changed graph data when the buffer is unchanged. Composition
pauses completion until the final flush, and caret movement keeps the existing
selection-only policy. Slash commands filter locally after loading templates;
property keys and values retain their shared per-session reads rather than
reading on every character. `Block.completionInput.test.tsx` exercises the shared
editor driver, and `e2e-completion-perf.mjs` counts native reads per physical key.

Page/tag completion retains every match of its previous query, including
matches outside the displayed limit. Extending both the search-folded query
and any Mandarin expansion query re-ranks that set with the existing page
comparator; shortening, replacement or a non-prefix Unicode fold rebuilds it.
The pool is in memory only. A block editor builds its session on the first
nonblank query and releases it on popup close. Calls without an editor lease,
including Capture's dedicated query, use a bounded anonymous slot retained
until replacement, invalidation or snapshot disposal. Each snapshot keeps at
most eight recent pools in total.
Physical entries share the existing materialized page list. Closing a session
also frees its pools in held older snapshots, through a graph-owned weak registry.
Publications carry a pool only when the changed pages leave its physical
names, aliases, namespace seeds and baseline-adjusted reference names unchanged;
create/delete/rename and changed name inputs invalidate it. Body autosaves
that leave those inputs unchanged retain the pool without enumerating names.
Property-value reads share one pending request per key; its arrival refreshes
the latest owned query without another keystroke, in both block editors.
`query_plan/completion_tests.rs` differentially checks the previous evaluator,
including Unicode fold rewrites, aliases, namespaces and expansion matching.

User macro substitution is charged before constructing the substituted string.
The DOM renderer and Copy/Export rendered-text walker use the same expansion
budget owner: depth 12, 1000 expansions and 256 KiB of expanded string units
per top-level tree, cumulatively across siblings. The existing visible
`macro expansion limit` cue marks refusal. The budget's existing string-length
accounting is preserved; it is not an encoded file-size limit. Templates remain
unexpanded until admitted, including export's resolver (L7s8-02, macro half).

Native print and static HTML export share a separate embed/query expansion
budget: depth 4, 1,000 expansions and 16 MiB of cumulatively rendered HTML.
Each outer block body starts one budget; sibling macros and their descendants
share it. Admission precedes subtree rendering; a refused expansion retains
the visible `macro expansion limit` cue. Independent block bodies start fresh.
The live app's embeds use their existing link-depth guard.

A PDF viewer owns one pending pdf.js loading task. Close, owner cancellation
and load failure destroy it once; cancellation-induced rejection is silent.
On success, ownership passes to the document proxy and its existing cleanup.
The render queue and zoom behavior are unchanged (L7s6-01).

Regression proof: `macroExpansionBudget.test.tsx`, `ExportModal.test.tsx`,
`PdfViewer.annotationLoad.test.tsx` and the native read tests above. No graph
write path or persisted format changes.
