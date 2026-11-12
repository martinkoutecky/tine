# 0075. Margin dialogue: a comment is an ordinary child block with `quote::`

Amendment D-20261109-MC9, D-20261111-CSL, DESIGN §12 (0.10): three
conveniences sit on the same blocks and add nothing to the graph. (1) Two
commands, **Next comment** and **Previous comment**, step through a page's open
threads in the order the margin draws them, from the one ordering the cards
already use; the current thread is a view signal, never saved, and the commands
ship with no default key. (2) Reviewing all open comments is the existing query
engine given a `query` right-sidebar item that runs
`(and (property tine.quote) (not (property tine.resolved true)))` (plus an author
filter) without a block; the query text is derived and only the item's scope and
author filter are kept, in the sidebar item lists that already exist. (3) A
comment is unread by the existing "changed since you last looked" marks (ADR
0079), so there is no second unread store. The classes comments draw with,
`data-author` and two colour tokens are a public styling surface
(`docs/contracts/theme-tokens.md` §7). Not done: a bespoke comments panel (a
second list and a second result renderer) and an unread flag written to the file
(churn, and wrong after any external edit). Unit cost of all three: zero bytes
per edit in the graph, zero files and zero transport. Page-level discussion
threads (design §13) are not built.

Amendment D-20261108-MC5/MC6: a root with `tine.date` marks a thread whose
dated blocks are messages and whose undated descendants are ordinary editable
body outlines inside those messages. The existing block renderer supplies the
tree, including in hover previews. Undated threads retain their flat display
and are never migrated: old nesting cannot distinguish a reply from its body.
Writers date only message roots. Folding uses Logseq's `collapsed:: true`,
shared on disk and undone through the existing property path; all thread roots
on one passage fold in a single undo. This reuses the existing date and fold
properties rather than adding a marker, container, viewer-only fold or writer.

Amendment D-20261107-MC3 §§4,9 (0.10): resolution is `tine.resolved:: true`
on the root only, removed by Reopen or a new reply; it is never a task marker.
Resolved cards are hidden by default, revealed greyed by a per-page count, and
excluded from passage highlights and open counts. New comments and replies
receive a creation-only UTC minute `tine.date` displayed in the local language.
Dates never sort messages or stamp ordinary blocks. Both properties use the
existing property writer, guarded whole-file save and undo. No side file or
transport is introduced. See `docs/contracts/margin-comments.md`.

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
  words>`. Replies are direct children of the thread root. Context starts at
  32 characters on either side, trimmed at word boundaries, and grows until
  the selected occurrence is unique within the parent block. Both context
  properties are **always written** when the quote occurs in the parent:
  an explicitly empty prefix/suffix means start/end of the block's prose, as
  identified by the existing kernel text projection (property rows and Org
  drawer delimiters do not shift that boundary). Context keeps a
  later edit that adds a second occurrence from silently re-anchoring the comment
  (amendment, D-20261018-MC2; no released graph held margin comments, so there
  is no migration). Readers keep accepting comments without them. An empty `quote::` comments on the
  whole block. A block written by an assistant may carry `author:: <name>`;
  human comments carry none unless the user has set **Settings > Editor > Your
  name** (D-20261017-LD1): then the comments and replies the user writes carry
  `author:: <name>`, and an author equal to that name (ignoring case) renders as
  "you". The name is a device preference (`comment_author_name` in
  tine-settings.json), never in the graph; unset writes nothing. In a graph under git
  (`gitAutocommitStatus().repository`, probed per opened graph), the first comment or
  reply of a person with no name asks for one once (D-20261101-CN, tine#841): the name is
  a Tine setting on this device, used in all graphs; skipping is remembered in
  `comment_name_prompt_declined` (same file) and is not asked again; the comment is
  written either way, after the dialog. Nothing is asked before the name, the flag and
  the git answer have loaded. These four keys live in `src/comments.ts`.
- **Nothing else is persisted.** The anchor is re-derived at every render from the
  parent's current text (`anchorQuote`); there are no offsets, no index and no
  side file. A quote that no longer occurs is shown struck through with "quoted
  text changed"; the comment is never dropped. Equally supported occurrences
  show "quoted text ambiguous" and contribute no highlight. Explicit empty
  context scores a boundary only to break a text-context tie, so surviving text
  still wins after edits. The old 32-character writer omitted empty context
  properties; those absent properties remain unknown, never boundary markers.
  Legacy text-context scores are preserved. A previously arbitrary first-match
  tie becomes visible ambiguity; existing files are never migrated or rewritten.
- A comment is **created through the document door** (`insertEmptyChildBlock` and
  `setRaw` inside one `withUndoUnit`), so it is one undo step and uses the audited
  save path; there is no margin-specific write path. The reader of the properties
  is the parser (facets), never a regex.
- Surfaces: Ctrl/Cmd+R in the block editor (consumed, so the webview never
  reloads), the palette command **Comment on selection**, and the block menu's
  **Comment on selection** for words selected in a block's rendered text.
  The editor selection toolbar and the touch rendered-text selection toolbar
  also offer **Comment**. Within a thread this creates a direct child of its root,
  whose selection is a compact quote inside that thread.
  Each thread offers a local **Reply…** draft at its bottom on hover or focus
  (also while active in the margin). Focus expands it; Enter and Shift+Enter add
  a line, Ctrl/Cmd+Enter or the visible Send button submits, Escape/Cancel
  discards (D-20261107-MC3 §5). No block or save exists until a nonempty
  draft is submitted. The selection toolbar's speech bubble is rightmost on
  desktop and touch. A top-right delete control on each comment/reply removes
  that block and its replies as one document undo step
  (provisional margin-polish rule, 2026-10-23).
- The ordinary editor-hidden property projection also hides `quote`,
  `quote-prefix`, `quote-suffix` and `author` **only when the block's own primary
  properties include `quote`** (including an empty value). The kernel owns this
  discriminator, shared with `src/comments.ts`; a parent is additionally needed
  for card presentation. The existing split/reattach mechanism preserves the
  hidden lines on text edits. Handwritten `quote` stays typeable until edit exit
  and hides on reentry. `author` belongs here because the comment card presents
  it as a chip. Ordinary blocks keep these properties in their editor buffer,
  as on OG. Page-level property editing retains its own projection.
- **Presentation** (`render only`): a comment draws as a tinted card with the
  quote above it, and the quoted words are marked in the parent. Where the pane
  is at least 960 px wide a page that holds comments draws the threads in a
  right-hand column beside their passages; in any narrower pane, or when the
  viewer chooses **In a popup**, nothing sits beside the text and the highlight
  opens its thread in the hover-preview popover (a bottom sheet on phones). Inline
  cards are not a fallback anywhere, and the same width rule applies to journal
  days, splits, the sidebar and embeds (D-20261108-MC7/MC8, DESIGN §6). Threads
  follow passage order, retaining file order for ties; threads on one passage share
  a card. Focus pins a thread to its anchor line; neighbors cascade above and below
  it without overlap. Comment blocks draw **no bullet**. In a thread whose root has
  no `tine.date` every block reads at one level in file pre-order, whatever its
  stored depth; in a dated thread dated blocks are the messages (still one level)
  and undated descendants are each message's editable bullet outline (amendment
  above). Hand-nested files stay unchanged. No count badge sits on an expanded
  block; a collapsed block with open threads below shows an "N below" chip.
  The visibility command and Settings label are **Show/Hide comments**; the
  command id and shortcut remain unchanged.
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
- The rendered author chip can replace an `author` property row on any block;
  editor hiding is scoped to blocks with their own primary `quote` property.
  An ordinary quotation's `author::` remains visible and editable in its buffer;
  a page-level `author::` still shows.

## Amendment 2026-11-02: comment keys are `tine.*` (D-20261102-PNS)

Discussion #825 pointed out that `quote::` and `author::` collide with users'
own properties: a child bullet with a user's `quote::` rendered as a comment
card, and any block's `author::` row was replaced by an author chip. Martin
decided (D-20261102-PNS) that every property with a Tine-specific meaning is
`tine.<name>`. From this date the keys this ADR names are written and read as
`tine.quote::`, `tine.quote-prefix::`, `tine.quote-suffix::` and
`tine.author::` (Org: `:tine.quote:` …). The old names were Beta-only and have
no reader: a reader for them would keep the collision. The sections above keep
their original wording as the record of the decision then.

Consequences replaced: a page opened in OG shows comments as nested bullets
with a `tine.quote::` property; the author chip replaces only a `tine.author`
row (comments, replies and blocks the command line added), so a user's own
`author::` stays an ordinary property row everywhere. `tine.*` block keys are
render-hidden like other Tine keys. The key names live in
`crates/tine-outline/src/tine_keys.rs`; `src/tinePropertyKeys.ratchet.test.ts`
fails if Tine writes a key that is neither `tine.*` nor OG-owned.
