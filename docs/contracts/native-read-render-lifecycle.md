# Native reads and renderer work ownership

Read-only IPC for assets, opted-in local images, dropped CSV/TSV files,
journal inspection and sync-conflict diffs runs blocking I/O on the blocking
pool. Graph reads capture their admitted graph slot before dispatch. Device
reads retain canonical-path and bound-graph refusals. Byte responses, read
limits and error tokens are unchanged. `nativeReadWorkers.guard.test.ts`
ratchets synchronous command additions; the Rust `off_ui_slow_read` test proves
the shared dispatcher yields while a read waits (L7s4-03).

User macro substitution is charged before constructing the substituted string.
The DOM renderer and Copy/Export rendered-text walker use the same expansion
budget owner: depth 12, 1000 expansions and 256 KiB of expanded string units
per top-level tree, cumulatively across siblings. The existing visible
`macro expansion limit` cue marks refusal. The budget's existing string-length
accounting is preserved; it is not an encoded file-size limit. Templates remain
unexpanded until admitted, including export's resolver (L7s8-02, macro half).

A PDF viewer owns one pending pdf.js loading task. Close, owner cancellation
and load failure destroy it once; cancellation-induced rejection is silent.
On success, ownership passes to the document proxy and its existing cleanup.
The render queue and zoom behavior are unchanged (L7s6-01).

Regression proof: `macroExpansionBudget.test.tsx`, `ExportModal.test.tsx`,
`PdfViewer.annotationLoad.test.tsx` and the native read tests above. No graph
write path or persisted format changes.
