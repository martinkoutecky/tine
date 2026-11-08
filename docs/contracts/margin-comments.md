# Margin comment interaction

Authority: D-20261107-MC3, design §§5 and 7; internal IDs UI-MC010-KEYBOARD,
UI-MC010-BELOW, UI-MC010-GROUPS, UI-MC010-VISIBILITY.

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
  existing surface fold. No resolve/reopen action or new property is introduced.
- Threads resolving to the same source range (or the whole block) share a card
  and passage header in the margin and existing inline surfaces. File order
  breaks ties. Stale and ambiguous selectors remain separate cards. Each root
  retains its messages, actions and reply composer; disk structure is unchanged.
  A composer remounted during regrouping or width changes retains its unsent
  draft for that thread and surface. The transfer lasts only through the current
  render; leaving the surface releases the state, and graph switches clear it.
- Commenting again on a resolved passage focuses its last thread's reply box.
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

There is no format, storage mode, transport, or write-path change. Resolve/date
semantics, author gutter runs and medium/phone layout tiers are separate work.
