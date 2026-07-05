# Plan 1 — Rendered-copy fidelity (math, off-screen refs, provider macros)

**Status:** grounded, ready to execute · **Est:** ~1–2 days (staged) · **Backlog:** P2

## Goal

Make **Copy / export as… → Rendered** (the default mode of the export modal) produce
output that matches what's on screen, closing four known gaps: (a) math loses its
delimiters, (b) an off-screen `((block ref))` copies as a bare uuid, (c) provider
macros (`{{query}}`/`{{embed}}`/`{{video}}`) copy as literal `{{…}}`, (d) a multi-line
block-ref target copies only its first line. These are independent; ship them as
separate commits.

## Current state (grounded)

Pipeline: `ContextMenu.tsx:453` "Copy / export as…" → `openExportModal` (`ui.ts:345`)
→ `ExportModal.tsx`. Text is a `createMemo` (`ExportModal.tsx:119`) calling
`exportOutline` (`src/editor/exportText.ts:93`) → rendered mode →
`renderedBlockText` (`src/render/renderedText.ts:245`), which parses each block once
via the single lsdoc `parseBlock` and walks the AST (`blockLines` :199, `inlineText`
:104). Copy = `backend().writeText(text())` (`ExportModal.tsx:128`).

`RenderedTextOptions` (`renderedText.ts:44`) already carries **synchronous** optional
resolvers `resolveBlockRef` / `resolveMacro`, depth-guarded (`MAX_…_DEPTH = 12`).

- **(a) Math → TeX source, delimiters stripped.** `inlineText` `case "latex": out += s.body`
  (`renderedText.ts:151`) — `body` is raw TeX without `$`/`\(` (`ast.ts:495`), and
  `mode` (Inline vs Displayed) is ignored. Block `$$` → `blockLines "displayed_math":
  b.text.split("\n")` (`:225`), also delimiter-free; `latex_env` → `b.content` (`:228`).
  Result: `x^2`, not `$x^2$` — not re-parseable as math.
- **(b) Block refs → sync best-effort, bare-uuid fallback.** `resolvedBlockRefText`
  (`renderedText.ts:89`) calls `o.resolveBlockRef?.(uuid)`; **null → returns the bare
  uuid** (`:92`). The modal injects `resolveExportBlockRef` (`ExportModal.tsx:78`) =
  `resolvedBlockRefSync(uuid)` (`resolveBatch.ts:63`), which reads only `resolvedCache`
  — populated **only** by the async `flush()` (`resolveBatch.ts:39`) for uuids that were
  rendered on-screen this graph epoch. Off-screen ref → sync miss → bare uuid.
- **(c) Provider macros → literal.** `inlineText "macro": resolvedMacroText` (`:146`)
  falls back to `macroLiteral` → `{{name args}}` (`:75`). `resolveExportMacro`
  (`ExportModal.tsx:89`) returns null for every built-in (`BUILT_IN_MACRO_NAMES` :57);
  only user macros from `graphMeta().macros` expand. So `{{query}}`/`{{embed}}`/`{{video}}`
  copy verbatim.
- **(d) Multi-line target → first line only.** Two truncations: `resolveExportBlockRef`
  uses `visibleBody(block.raw)[0]` (`ExportModal.tsx:86`) AND `resolvedBlockRefText`
  does `.split("\n")[0]` (`renderedText.ts:93`). Children/subtree never followed.
- **Async availability:** copy is synchronous only because the resolver *signatures*
  are sync and the modal never awaits — the string is fully built in the `text()` memo
  before `writeText`. Real async paths exist and are unused here: `resolveBlockBatched`
  (`resolveBatch.ts:48`) and `backend().runQuery`/`runAdvancedQuery`/`getPage`
  (as used in `Macro.tsx`).

## Approach — pre-warm async, keep the serializer sync

The cleanest, lowest-risk shape (Martin's own note in the backlog): **don't make the
serializer async**; instead **pre-warm** the async resolvers when the modal opens (or
just before copy), populating `resolvedCache` so the existing sync `resolveExportBlockRef`
hits, and pass resolved query/embed results in via new sync resolver branches.

Staged:

### (a) Math delimiters — trivial, do first
Emit delimiters so the copy is re-parseable and unambiguous:
- inline `latex` Inline → `$` + body + `$`; Displayed inline → `$$…$$`.
- `displayed_math` block → wrap the lines in `$$` … `$$`; `latex_env` → keep
  `\begin{env}…\end{env}` (already self-delimiting).
Update `renderedText.test.tsx` (there's an existing math expectation to adjust).
Optional stretch: a `mathAsUnicode` option for *trivial* inline math (`E=mc^2`→`E=mc²`,
`H_2O`→`H₂O`) via a superscript/subscript char map, default off — but keep TeX for
anything non-trivial (fractions/integrals are genuinely lossy). Decide when building;
delimiters alone already fix the reported gap.

### (b)+(d) Block refs — pre-warm + follow more
1. On modal open, walk `nodes` for every `((uuid))` / `[l](((uuid)))` and call
   `resolveBlockBatched(uuid)` (one coalesced IPC), `await` all, so `resolvedCache` is
   populated before `text()` is read. Re-run the `text()` memo after (a signal the
   pre-warm flips on completion).
2. For (d): give `resolveExportBlockRef` an option to return the **full visible body**
   (all lines) instead of `[0]`, and drop the `.split("\n")[0]` truncation in
   `resolvedBlockRefText` when that option is set. Keep first-line as the default for
   inline-position refs; full body when the ref is the block's whole content. (Matches
   on-screen `BlockRefView`, which shows the first line inline but the full block when
   zoomed — pick the inline behavior for parity, and expose a "resolve refs fully"
   export toggle for the power case.)

### (c) Provider macros — resolve at copy time
Pre-warm like (b), then add sync resolver branches fed by pre-fetched results:
- `{{embed ((uuid))}}` / `{{embed [[Page]]}}` → inline the resolved block/page text
  (reuse the same pre-warm + `getPage`/`load_page_doc`). This is the highest-value one.
- `{{query …}}` → run `backend().runQuery`/`runAdvancedQuery` during pre-warm; serialize
  the result blocks as a rendered list (the export already knows how to render blocks).
- `{{video url}}` → the bare URL (sensible text form); other widgets (`{{tweet}}`,
  `{{cloze}}`) → a sensible literal or the inner content.
Mirror `Macro.tsx`'s resolution so copy == screen. Gate behind the existing modal so
it only runs for rendered mode.

## Steps (commit boundaries)

1. **(a)** math delimiters + test. (½ hr)
2. **Pre-warm harness**: on modal open, collect all uuids + query/embed macro targets
   from `nodes`, resolve via the async batched/query paths, store in a local map, flip a
   "warmed" signal that re-triggers `text()`. (core plumbing)
3. **(b)+(d)** block-ref resolution through the warmed cache + full-body option + export
   toggle.
4. **(c)** embed, then query, then video/other macro text forms.
5. Docs: FEATURES (rendered-copy now resolves refs/embeds/queries + keeps math
   delimiters), CHANGELOG; remove the four open sub-bullets from the backlog row.

## Risks / decisions

- **Pre-warm latency:** resolving all refs + running queries can take a beat on a big
  subtree. Do it on modal *open* (modal already builds `nodes` once at open,
  `ExportModal.tsx:118`) with a small "resolving…" state, so copy stays instant. Cap
  query cost; if a query is huge, serialize a bounded list and note truncation (never
  silently).
- **Math Unicode is lossy** — keep it opt-in and trivial-only; TeX-with-delimiters is
  the safe default.
- **Determinism:** query results in a copy are a point-in-time snapshot — fine for
  paste, but note it's not live (obvious, no action).
- Don't make `renderedBlockText` itself async (keeps the on-screen `renderedText`
  callers — breadcrumbs, previews — untouched); the async lives only in the modal's
  pre-warm.

## Acceptance

- Inline `$x^2$` and `$$…$$` copy **with** delimiters, re-parseable.
- A ref to a block **not currently on screen** copies the block's text, not the uuid
  (pre-warm hit) — test by copying a subtree whose refs point off-screen.
- `{{embed ((uuid))}}` copies the target's text; `{{query (task TODO)}}` copies a
  rendered result list; `{{video url}}` copies the url.
- A ref to a multi-line block copies more than the first line when the "resolve refs
  fully" toggle is on.
- Existing `renderedText.test.tsx` updated; on-screen rendered-text callers unaffected.
