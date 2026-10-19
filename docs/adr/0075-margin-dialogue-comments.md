# 0075. Margin dialogue: a comment is an ordinary child block with `quote::`

- **Status:** Accepted (Martin, 2026-10-18, D-20261018-MC2) with the amendment below: context is always written.
- **Date:** 2026-10-17

## Context

Vision 2026-10 §3.7 wants a conversation about a passage of a block: a reader
(a person, or an assistant that writes into the graph) selects words, comments on
them, and replies follow. The graph is plain Logseq Markdown/Org, so whatever is
written must stay readable and editable in OG and in an external editor, must
survive a block being edited elsewhere, and must go through the one audited save
path. Alternatives that were on the table: a side file or an app-data index of
comments (a new persisted artifact, sync and divergence problems), character
offsets stored with the comment (stale after any edit of the passage), and a new
block type or syntax (not Logseq-compatible).

Decisions on the surface: D-20261017-MD1 (integrate into og for 0.9) and the
2026-10-10 entry "Margin dialogue: comment threads without bullets" in
`specs/notes/development-process/martin-decision-journal.md`.

## Decision

- A comment is an **ordinary child block** of the commented block, appended after
  its existing children, whose text carries the property `quote:: <the selected
  words>`. Replies are its children. When the quoted words occur more than once
  in the parent, `quote-prefix::` and `quote-suffix::` (at most 32 characters of
  surrounding text each) say which occurrence. They are **always written** when
  the quote occurs in the parent, trimmed at a word boundary, so a later edit
  that adds a second occurrence of the quoted words cannot re-anchor the comment
  (amendment, D-20261018-MC2; no released graph held margin comments, so there
  is no migration). Readers keep accepting comments without them. An empty `quote::` comments on the
  whole block. A block written by an assistant may carry `author:: <name>`;
  human comments carry none unless the user has set **Settings > Editor > Your
  name** (D-20261017-LD1): then the comments and replies the user writes carry
  `author:: <name>`, and an author equal to that name (ignoring case) renders as
  "you". The name is a device preference (`comment_author_name` in
  tine-settings.json), never in the graph; unset writes nothing. These four keys live in `src/comments.ts`.
- **Nothing else is persisted.** The anchor is re-derived at every render from the
  parent's current text (`anchorQuote`); there are no offsets, no index and no
  side file. A quote that no longer occurs is shown struck through with "quoted
  text changed"; the comment is never dropped.
- A comment is **created through the document door** (`insertEmptyChildBlock` and
  `setRaw` inside one `withUndoUnit`), so it is one undo step and uses the audited
  save path; there is no margin-specific write path. The reader of the properties
  is the parser (facets), never a regex.
- Surfaces: Ctrl/Cmd+R in the block editor (consumed, so the webview never
  reloads), the palette command **Comment on selection**, and the block menu's
  **Comment on selection** for words selected in a block's rendered text.
- **Presentation** (`render only`): a comment draws as a tinted card with the
  quote above it, and the quoted words are marked in the parent. On a wide main
  pane (scroller at least 960 px, not split) a page that holds comments draws the
  threads in a right-hand column beside their passages; elsewhere they stay inline
  under the commented block. Comments and their replies draw **no bullet**: depth
  is a thin rule and a small indent, and the author chip is the handle (drag,
  block menu).
- The question "does this page hold a comment" follows the page's **structure**,
  not the text of other blocks, so a keystroke in an unrelated block costs the
  margin nothing (GH #248 shape; `keystrokeScaling.guard.test.tsx`).

## Consequences

- Unit cost: one child bullet per comment or reply, about 58 bytes for an
  18-character reply quoting 24 characters, in the parent's page file only. One
  file and one whole-file rewrite per edit through the existing save path; no
  new file, index, sync record or transport.
- A page opened in OG shows comments as plain nested bullets with a `quote::`
  property: the format degrades gracefully, and a comment written by hand works.
- The text of a quote is a copy: editing the parent leaves the old words in the
  comment (shown as stale), by design.
- A block edited by hand into (or out of) a comment while the page stays open is
  noticed by the margin at the next structural change or reload, not immediately.
- Hiding the `author` property row applies to every block that has a block id; a
  page-level `author::` still shows.
