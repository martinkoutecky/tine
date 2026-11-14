# Margin comment interaction

Authority: D-20261107-MC3, design §§2,4–7, 9 and 12; D-20261108-MC4, MC5, MC6, MC7, MC8;
D-20261109-MC9; D-20261111-CSL; internal IDs UI-MC010-KEYBOARD, UI-MC010-BELOW,
UI-MC010-GROUPS, UI-MC010-VISIBILITY, UI-MC010-RESOLVE, UI-MC010-DATES, UI-MC010-MEDIUM,
UI-MC010-PHONE, UI-MC010-CLICK-DISPLAY, UI-MC5-MESSAGES, UI-MC5-COLLAPSE, UI-MC9-REVIEW,
UI-MC12-REVIEW-QUERY, UI-MC12-UNREAD; GH #891 review findings 1 and 2; GH #929.

Not yet built: the page-level discussion thread (design §13) and the search-hit comment cue.
Output follows D-20261113-CO/CO2 (H9TvA).

## Counting, listing and output surfaces

Comments require a parent block and the parser-recognized `tine.quote`
property. All descendants of that comment belong to its thread. A root block
carrying that property is ordinary body content, not a page discussion thread.
The discriminator is Rust's `tine_outline::editor_text::comment_quote`, exposed
through wasm to `isCommentBlock`; `threadRootOf` supplies document ancestry.
The surface comment index supplies root membership and resolution counts.

Output and unchanged counting/search behavior are covered by:
`src/document/commentSurfaces.test.ts`,
`crates/tine-graph-features/tests/comment_surfaces.rs`, and the existing comment
count, resolution and rendering suites.

| Surface | Treatment of comments |
|---|---|
| Body navigation | Skips the whole thread, including unquoted replies. |
| Empty page / typing target | Seeds a bullet when there are no roots, or after a Markdown properties-only root. A blank body root with a thread remains a body root and is not replaced. A root with `tine.quote` is not excluded. |
| Comment controls | Counts thread roots rather than messages: resolved count and hidden-descendant “N below” use the comment index; resolved roots are excluded from open counts. Expanded margin and popup have no count badge; phone chip has no visible count. |
| Text / Markdown export modal | Included by default. Text prefixes each thread block with `[Comment]` (author/date where known); source retains comment properties, including with Remove properties. Include comments off removes the whole thread. |
| HTML / OPML export modal | Included by default. HTML uses an annotation with a Comment header; OPML prefixes text with `[Comment]`, with author/date where known. Include comments off removes the whole thread. |
| Copy outline / copy page as Markdown | Includes comment text, replies and source comment properties. The private clipboard retains the complete source subtree. Copying a block reference or link copies its identity rather than its body. |
| Print / PDF | Included by default as labelled annotations with author/date where known. Include comments off removes threads from the print document used by PDF. |
| Static site | Included as labelled annotations by default, and present in the search index. `--no-comments` removes threads from pages and the index. |
| Live / query folder publication | Included by default. The snapshot retains source comment properties; the reader uses the app's margin. Static fallback uses labelled annotations. Include comments off removes threads before building all derived answers. |
| Quick share | Included by default; the reader shows the app's margin. Include comments off removes whole threads before creating the encrypted snapshot. |
| Full-text search | Comment text can produce ordinary block hits with the host in the breadcrumb; metadata is absent from display text. With page-content matching enabled, comment text can also produce a page hit. There is no separate comment result family. |
| Palette / search previews | Uses the search engine's ordinary block hits, display text and breadcrumbs. The result goes in the existing block section, without a comment label. |
| In-page Find | Includes comment and reply text in match counts, including threads outside the body outline. Hidden comment metadata is excluded. |
| Backlinks / All pages reference counts | References written in comment text and replies participate in membership and counts, like references in body blocks. These counts measure referring blocks, not page body size. |

There is no word-count feature or page-body-size badge in this inventory.
The export modal's block count is the requested selection/result count, not a
recursive count of the exported subtree. Query results and reference panels
retain engine membership; the live block renderer applies comment presentation
within a shown subtree. This inventory does not change query semantics.

Repeating folder exports and shares store `tine.comments:: false` on the definition
block when comments are excluded; absence means include. Re-export and Update all
read it. A dialog writes the property only for an explicit changed choice, through
the document's guarded save path. Update all displays mixed saved choices with an
indeterminate checkbox; changing it applies one choice to the batch and refreshes
its review. The reviewed fingerprint includes the effective choice.

Flat exports keep expanded embed content under its comment root. The root's
source properties and the embedded blocks' authors stay with their owners.
Detached replies carry an empty `tine.quote` in source exports so their comment
marking survives expansion and copies without transient thread metadata.
Computed tables, boards and grids show ordinary rows in their view and preserve
comment threads as annotations alongside it.

The comment discriminator remains `comment_quote`; `output_comments` carries
thread membership through publication planning, and the frontend export projection
uses the same discriminator through wasm. Shallow reference rows (linked, unlinked,
block referrers) and published-reader embed previews carry the same thread
membership on their detached root, so reference-panel Copy/Export and reader
exports label and omit comments like page exports (Iif8k). O(exported blocks), no added filesystem
reads, stats or IPC, no per-edit work. Search-hit presentation is a separate task;
reference counts and Find remain unchanged.

## Interaction

- Settings > Appearance > Comments offers **In the margin** (default) and
  **In a popup** (D-20261108-MC7). This viewer preference uses the same
  device-local display preference storage as Wide mode, under the key
  `logseq-claude.comments-in-popup`; the earlier On click value is not migrated
  (a viewer who chose it sees the default once). In a popup substitutes the
  medium tier for the wide tier, including in Wide mode; phone widths still use
  the bottom sheet. It applies live on every comment surface without changing
  the graph. Switching preserves unsent reply drafts by thread and surface, just
  as crossing tier widths does. Show/Hide comments takes precedence over either
  display choice. Changing the preference costs one local preference write
  and an O(1) tier comparison per mounted surface, with no graph I/O or IPC;
  rendering the newly visible threads costs O(visible threads).

- With In the margin, layout uses the containing pane's CSS width, including interface zoom. At
  960 px the existing margin fits; below that threads never fall back to inline
  cards. In a popup and the medium tier draw nothing beside the text: no
  per-thread rail markers and no count on any chip (D-20261108-MC8).
  Hovering a highlight with a mouse opens its thread through the hover
  preview's own trigger, delay (Settings hover-preview delay, default 1 s),
  100 ms close grace and popup-enter rules (`createPeekBridge`, shared with
  page-link and block-reference previews). A mouse click on a highlight places
  the caret and opens nothing; a press during the dwell cancels it. Touch and
  pen taps open the thread; Enter or Space on a focused highlight opens it.
  The thread opens in `PeekPopup`, sharing preview placement and outside-press,
  Escape, scroll and resize dismissal; it stays live and editable, and a
  focused comment editor or reply draft holds it open past pointer exit.
  Hovering another passage preserves the current popup while it contains focus
  or a non-empty reply draft. Only
  an open thread the renderer drew no highlight for gives its block one small
  glyph marker, labelled with the number of such threads, that opens the first
  of them. That covers a quote that changed, matches more than once or covers
  the whole block, and also one that resolves but cannot be marked (inside
  inline code, across bold or other markup). Marker eligibility reads the
  coverage the inline renderer reports (`QuoteCoverage`), never a second
  derivation from the source, so every open thread keeps an entry point. A
  thread sharing another's passage shares its mark. While the block is being
  edited nothing is rendered and the marker keeps its last answer. A thread opened by a request rather than a pointer (a new comment,
  Comment on existing, or an edit of one of its messages started elsewhere)
  anchors below the commented block's text, never over it. Cards, replies and
  document mutations are the same components and door
  as in the wide margin. Expanded wide blocks have no count badge.
- At 480 px or less, highlight or chip tap opens a bottom sheet capped at 55% of
  the viewport height, leaving source text above; hover does not open at this
  tier. Swipe horizontally or use the
  previous/next controls to move between passages; same-passage threads stay
  grouped. Only the shown passage's threads are mounted; the block's composers
  retain drafts across navigation through the entry's draft leases. The chip
  shows a comment glyph without a count (its accessible name still states the
  number of own open roots) and remains reachable when the
  block is folded. Show/Hide comments removes these entry points and surfaces.
  Page routes, journal days, zoomed blocks, splits, sidebar blocks, references
  and embeds follow the same width rule. Crossing the medium/phone boundary
  while editing changes placement without replacing the editor. A held sheet
  fits the visual viewport when a soft keyboard shrinks it; opening above an
  existing keyboard reveals the source in the visible text strip. Removing its source
  closes the compact surface; deleting its selected thread leaves any remaining
  thread visible and replyable.

- Surface order (GH #891 finding 2). One producer orders a block's thread roots:
  `groupCommentEntries` sorts by resolved anchor start (file order breaks ties,
  unresolved last) and groups by passage; `threadOrder` flattens the groups.
  The margin cards and the popover's grouped cards render that order, and the
  arrow keys in a thread consume the same list: past a thread's last message
  the caret moves to the next root in the order, above a root to the previous
  root's last message, and past either end to the commented block or the body
  block after it. The margin walks every group of the block; the popover and
  sheet walk only the group they show. Interleaved A (alpha), B (beta),
  C (alpha) present and navigate as A, C, B.
- Comment membership has one owner (GH #891 review round 3): the comment index
  of each resolution scope (page, journal day, sidebar item, reference group,
  zoomed block; `commentIndex.ts`). It answers which blocks are comments (with
  their selectors and resolution), which comments are thread roots rather than
  replies, and which roots lie under a subtree. The "N resolved" control, the
  folded "N below" count, and every block's comment children (outline filter,
  highlights, compact entry and popup, margin column) and margin eligibility
  read it. A structural change rescans the scope's trees once with untracked
  reads. The rescan compares old and new comment lists and notifies only the
  parents whose lists changed; unchanged parents evaluate no child-list memo.
  Child-list memos depend on a stable scan result for freshness while a comment's
  structure and metadata are published in one batch. A separate rescan signal
  notifies root readers; typing adds no scan reads to child-list accessor reads.
  Rows and grouped passage headers use the existing node-lifetime binding
  while deletion notifications settle, until their owners dispose them.
  Covering-index selection follows the block's ancestry rather than a global
  structural revision. A raw edit of any
  block on those pages runs one key screen (plus one facet lookup if it may
  quote): a comment whose selector or resolution changed is updated in place
  and notifies only its parent's readers and, for a root, the root readers; a
  change of membership (an ordinary block gains a quote, a comment loses it, so
  roots may become replies or back) rescans. Blocks outside every scope's trees
  (quick capture) share one index per top-level tree.
- Cost (GH #891 finding 1). Typing in an
  ordinary sibling therefore costs a constant number of store reads and at most
  one facet lookup, whatever the number of siblings (14 foreign raw reads and 1
  facet lookup at 4, 16 and 64 siblings in the counter test); the native typing
  p95 budget beside 64 siblings is 16 ms.

- In either block editor, Enter and Shift+Enter insert a line break in a comment
  or any reply. Ctrl/Cmd+Enter and Escape finish editing. These keys never split
  the message, create a reply, or run the task command. IME composition and an
  open completion popup retain their existing priority. Body Enter is unchanged.
- Reply drafts keep Enter as a newline; Ctrl/Cmd+Enter or the visible Send button
  posts the draft as a direct child of its thread root through the document door.
  An empty draft never writes. Escape/Cancel discards the draft.
- A collapsed body block shows an outlined “N below” chip for open thread roots
  on descendant body blocks. Own threads and all replies are excluded; a root
  with `tine.resolved:: true` is excluded from this count. The chip expands the
  existing surface fold. The count reads the comment index's roots
  (`createFoldedThreadCount`): O(roots × depth) when a root or the structure
  changes, nothing on a comment-free page. Typing in the folded block
  reads none of its hidden descendants (0 foreign raw reads at 4, 16 and 64
  hidden children in the counter test); the native typing p95 budget above 64
  hidden children is 16 ms.
- Threads resolving to the same source range (or the whole block) share a card
  and passage header in the margin, popover and sheet. File order
  breaks ties. Stale and ambiguous selectors remain separate cards. Each root
  retains its messages, actions and reply composer; disk structure is unchanged.
  A composer remounted during regrouping or width changes retains its unsent
  draft for that thread and surface. Compact entry keeps drafts while its source
  row remains mounted, including closing and reopening the thread view; leaving
  the surface releases the state, and graph switches clear it. Opening one
  passage mounts that passage's cards only (1 group and 1 card at 4, 16 and 64
  single-thread passages in the counter test, was 4/16/64); moving to another
  passage mounts it then.
- Commenting again on the same anchored passage focuses its last thread's reply box.
  It does not write. The group's explicit New thread action appends a distinct
  root on the same resolved passage, through the existing guarded save path.
  New roots use the existing context-to-uniqueness writer, including when an old
  root omitted context properties; that old root is never rewritten. Unresolved
  cards retain editing/replies and offer no New thread action without a passage.
  Read-only sources offer neither a composer nor New thread.
- The visible page actions menu offers Show comments or Hide comments on touch
  and desktop, using the same device preference as Settings and the command.
  Hiding removes cards, highlights, chips and markers. Pending drafts retain the
  existing refusal and remain available to send or discard.

## Resolved threads and creation dates (D-20261107-MC3 §§4,9)

Resolve writes `tine.resolved:: true` on the root only; Reopen removes it.
Both use the existing guarded document save and undo. A resolved thread loses
its cards, highlights and open counts, including the collapsed subtree count.
The page's **N resolved** control reveals greyed cards in their passage groups
with Reopen. Revealing never restores highlights or open counts. Replying to a
resolved thread removes its resolution in the reply's single undo unit.
At compact widths, revealing adds a muted **N resolved** entry beside each
source block with closed threads. It opens those same greyed live cards in the
popover or phone sheet, with Reopen and dates. This entry is separate from the
open-thread chip; closed roots never contribute markers or highlights. Secondary outlines and zoomed blocks own the same reveal scope.
Show/Hide comments also removes the resolved entry and any open compact view.

New comments and replies carry `tine.date:: 2026-11-07T13:37Z`: UTC, minute
precision, written once at creation. The window uses `appNow("instant")`
(without the local calendar's zone-skew correction); the CLI uses the UTC
system clock. Edits retain it byte for byte. A quiet
localized date appears in each message header; missing or invalid dates show
nothing. Dates never change file order or passage grouping. Ordinary blocks
receive no date. All `tine.*` rows stay hidden in comment and reply editors.

A root carrying `tine.date` opts its thread into message outlines: dated blocks
are messages, in file pre-order, and undated descendants form the nearest
ancestor message's ordinary editable bullet tree. The shared `Block` renderer
supplies bullets, nesting, references and editing, as in hover previews. Only
messages have comment headers and actions. Dates mark messages by property
presence; an invalid date still marks a message but displays no date. Threads
whose root has no date retain the flat legacy presentation, including replies
with dates. Opening either shape never rewrites it. Reply and in-thread Comment
date only their newly created message; CLI `add --under` inside a thread dates
the inserted fragment's roots, preserving its undated body descendants.

`collapsed:: true` (Org `:collapsed: true`) folds a thread root to its quote,
first displayed message line and **N replies**, or folds a message's body tree.
Expand restores it. Collapse controls use the ordinary property writer and
undo. The block menu, also reachable from a comment's author handle in the
margin, offers **Collapse all comments on this block**: it folds all thread
roots on that passage in one document transaction and undo step. Folds travel
with the graph and are read by Logseq. Collapsed bodies mount no hidden rows.
The root's message enumeration is O(thread blocks), with zero I/O or IPC;
ordinary rows retain their existing owner and render budgets.
Arrow navigation uses that message order, visiting each message's visible body
before the next message, even when a reply is nested by hand under a body block.
Folding a body does not hide an independent dated message in the flat thread.

These properties use the ordinary Markdown property / Org drawer writer and
the existing whole-file guarded save, with no new file or transport. With LF
endings, Resolve adds 21 bytes plus Markdown indentation on the root; a date
adds 30 bytes plus Markdown indentation per message. An unnamed Org reply
also needs its first property drawer (19 bytes), for 49 bytes total; a root or
signed reply already has a drawer. These byte deltas are independent of page
size; the existing save still rewrites the whole affected file. Displaying or
revealing dates/closed cards writes nothing. Editing adds no timestamp bytes.

## Open-threads review and unread comments (DESIGN §12)

**Review query.** **Review open comments on this page** and **Review open comments
in the graph** (command palette) open a `query` item in the right sidebar. It stores only
`{kind: "query", scope: "page"|"graph", page, pageKind, path?, author?, collapsed?}`
in the existing sidebar lists; the query text is derived from it by
`src/commentReviewQuery.ts` and never saved:

```
(and (property tine.quote) (not (property tine.resolved true))
     [(page "Name")] [(property tine.author "Ada")])
```

The item mounts `QueryMacro` without a block, as `TodaySections` does, so its
results are the ordinary live, editable blocks of the one query engine. There is
no second comment index, no regex or line scan and no bespoke panel. Resolved
threads and replies (no `tine.quote`) are not listed; an author filter is one
exact `tine.author` value (a free-text field; empty clears it). Two items never
share a slot: one per page and one for the graph, keyed `query:<pageKind>:<page>`
and `query:graph`, so reopening promotes and keeps the author. A collapsed item
mounts and runs nothing. With no page route the page command falls back to the
graph scope.

**Cost.** The query rides the indexed property path: `tine.*` keys are in the
key-to-pages table, so the candidate pages are the pages that hold `tine.quote`
(all of them for the graph; the one named page for page scope), independent of
how many comment-free pages the graph has. Pinned by
`open_threads_review_lists_open_roots_and_visits_only_pages_with_comments` in
`crates/tine-store/src/query/exec_key_tests.rs` (candidate pages (2, 1) at 30 and
at 120 comment-free pages, and the narrowed answer equals a full scan) and by
`crates/tine-store/tests/fixtures/comment-review-queries.json`, the query text
shared with the frontend test. The sidebar item adds no write path and no file.

**Restore.** `rightSidebarItems` (app-data session and the `logseq-claude.rightSidebarItems`
copy) now may hold `query` items; both pass `validSidebarItem`, which drops any
kind it does not know and any malformed item without throwing, so a build from
before this change restores the other items and silently loses the review item,
and this build does the same for a kind from a newer one.

**Unread.** A comment is unread when its message changed *since you last looked*:
the existing `src/seen` mark (ADR 0079), from the page's one `getPageChanges`
read. A comment card draws the same change bar as an ordinary row when any message
block of its thread is marked (`src/seen/threads.ts`, an O(thread) walk with no
IPC), and a collapsed block's **N below** chip reads **N below, M changed** with
the change colour when M of its open threads contain a mark. The margin column
shares the page's seen context. Git graphs only: with `available: false` (no git,
no history) no mark or changed count appears. **Mark page seen** clears the bars as
for every block; there is no second unread record, no new file and no per-row IPC.
Nothing in the graph changes.

## Next and previous comment (D-20261109-MC9)

**Next comment** and **Previous comment** (command ids `comment/next` and
`comment/previous`) are palette commands and rebindable shortcuts with no default
key. They walk one resolution scope's open threads in the order the margin draws
them: `commentReview.ts` takes each parent block of the scope's comment index
in turn and flattens its `groupCommentEntries` groups with `threadOrder` (anchor
start, file order for ties, unresolved last). Resolved threads join only while the
page's **N resolved** control reveals them. That one list is also the order the
cards, popover and sheet use, so no surface derives its own. The scope is a page,
or one journal day; a step never crosses days, panes or windows. In a journal feed
the scope is the day last pointed at or focused, else the first visible day.

A step with no current thread goes to the first thread (forward) or the last
(backward); it does nothing past either end. It ends any block editing, shows the
comments if **Hide comments** was on, expands the collapsed ancestors of the
commented block through the ordinary reveal (one undo step writing the same
`collapsed::` state as clicking their arrows; the commented block's own fold and
every other block stay as they are, since a collapsed block keeps its own threads
reachable), scrolls the target's passage to the centre of its pane and focuses that thread's
reply composer, opening the popover or phone sheet at the narrower tiers. For a
grouped passage the passage itself is scrolled, for a resolved thread with no
painted highlight the passage's position is scrolled, and the popover follows the
focused thread. Clicking or focusing any thread makes it the current one.

The popover and the phone sheet carry previous and next buttons and the label
**Comment N of M**; with a review scope these step the whole scope, without one
(no registered scope) they step only the displayed group and read **Passage N of M**.
Pinned by `src/components/CommentReview.test.tsx` (backend requests and scrolls
counted per step, ancestor reveal, journal-day scope, palette entries without
default keys, hidden comments shown by a step) and the native journey
`scripts/e2e-comment-review.mjs`.

**Cost.** A step walks at most the scope's thread order, already cached by the
comment index, and makes no backend read or write other than the ancestor
reveal's own save (none for a visible target; counted in
`CommentReview.test.tsx`). Nothing is stored; the current thread is a view signal.

## Full-text search (COMMENT-OUT-V2, D-20261113-CO2)

Full-text search retains comment roots and every descendant reply as ordinary
block hits. The palette, search workspace and search result excerpts label them
**comment**, keeping the host block in their existing ancestor breadcrumbs.
A page whose only matching text is a comment remains a content-search page hit;
its matching comment also remains a labelled block hit. In-page Find and reference
counts retain their existing inclusion of comments (V3).

The result-only `QueryHit::Block.is_comment` fact comes from
`tine_outline::editor_text::comment_quote` over parsed own properties, with a
parent required for a thread root. The search walk carries membership to all
descendants. Projection and rendering read that boolean in O(1) per hit, with
zero additional file reads, writes, stats or IPC calls. Navigation uses the
existing comment-target reveal path.

## Styling layer (D-20261111-CSL, GH #929)

The class names users restyle comments with, `data-author` on every block
carrying `tine.author::`, and the two comment tokens
`--tine-comment-highlight-bg` and `--tine-comment-card-bg` are public hooks. Their
list, the `data-author` rules and the guard test live in
`docs/contracts/theme-tokens.md` §7; this contract only promises that the
components named here keep drawing those classes.
