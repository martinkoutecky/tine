# Margin comment interaction

Authority: D-20261107-MC3, design §§4, 5, 7 and 9; internal IDs UI-MC010-KEYBOARD,
UI-MC010-BELOW, UI-MC010-GROUPS, UI-MC010-VISIBILITY, UI-MC010-RESOLVE, UI-MC010-DATES.

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
  existing surface fold.
- Threads resolving to the same source range (or the whole block) share a card
  and passage header in the margin and existing inline surfaces. File order
  breaks ties. Stale and ambiguous selectors remain separate cards. Each root
  retains its messages, actions and reply composer; disk structure is unchanged.
  A composer remounted during regrouping or width changes retains its unsent
  draft for that thread and surface. The transfer lasts only through the current
  render; leaving the surface releases the state, and graph switches clear it.
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

New comments and replies carry `tine.date:: 2026-11-07T13:37Z`: UTC, minute
precision, written once at creation from `appNow("instant")` (without the local
calendar's zone-skew correction). Edits retain it byte for byte. A quiet
localized date appears in each message header; missing or invalid dates show
nothing. Dates never change file order or passage grouping. Ordinary blocks
receive no date. All `tine.*` rows stay hidden in comment and reply editors.

These properties use the ordinary Markdown property / Org drawer writer and
the existing whole-file guarded save, with no new file or transport. With LF
endings, Resolve adds 21 bytes plus Markdown indentation on the root; a date
adds 30 bytes plus Markdown indentation per message. An unnamed Org reply
also needs its first property drawer (19 bytes), for 49 bytes total; a root or
signed reply already has a drawer. These byte deltas are independent of page
size; the existing save still rewrites the whole affected file. Displaying or
revealing dates/closed cards writes nothing. Editing adds no timestamp bytes.
