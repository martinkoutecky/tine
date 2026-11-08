# Margin comment interaction

Authority: D-20261107-MC3, design §§5–7; internal IDs UI-MC010-KEYBOARD,
UI-MC010-BELOW, UI-MC010-GROUPS, UI-MC010-VISIBILITY, UI-MC010-MEDIUM,
UI-MC010-PHONE; D-20261108-MC4, UI-MC010-CLICK-DISPLAY.

- Settings > Appearance > Comments offers **In the margin** (default) and
  **On click**. This viewer preference uses the same device-local display
  preference storage as Wide mode. On click substitutes the medium tier for
  the wide tier, including in Wide mode; phone widths still use the bottom
  sheet. It applies live on every comment surface without changing the graph.
  Switching preserves unsent reply drafts by thread and surface, just as
  crossing tier widths does. Show/Hide comments takes precedence over either
  display choice. Changing the preference costs one local preference write
  and an O(1) tier comparison per mounted surface, with no graph I/O or IPC;
  rendering the newly visible threads costs O(visible threads).

- With In the margin, layout uses the containing pane's CSS width, including interface zoom. At
  960 px the existing margin fits; below that threads never fall back to inline
  cards. Medium panes retain highlights and a slim per-thread passage rail.
  Markers, highlights and the block's own-thread chip open the live thread in
  `PeekPopup`, sharing preview placement and outside-press, Escape, scroll and
  resize dismissal. Editing holds the popup under the preview's existing rules.
  Cards, replies and document mutations are the same components and door as
  in the wide margin. Expanded wide blocks have no count badge.
- At 480 px or less, highlight/chip entry opens a bottom sheet capped at 55% of
  the viewport height, leaving source text above. Swipe horizontally or use the
  previous/next controls to move between passages; same-passage threads stay
  grouped. The block's composers retain drafts across navigation. The chip
  counts own open thread roots, never replies, and remains reachable when the
  block is folded. Show/Hide comments removes these entry points and surfaces.
  Page routes, journal days, zoomed blocks, splits, sidebar blocks, references
  and embeds follow the same width rule. Crossing the medium/phone boundary
  while editing changes placement without replacing the editor. A held sheet
  fits the visual viewport when a soft keyboard shrinks it; opening above an
  existing keyboard reveals the source in the visible text strip. Removing its source
  closes the compact surface; deleting its selected thread leaves any remaining
  thread visible and replyable.

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
  and passage header in the margin, popover and sheet. File order
  breaks ties. Stale and ambiguous selectors remain separate cards. Each root
  retains its messages, actions and reply composer; disk structure is unchanged.
  A composer remounted during regrouping or width changes retains its unsent
  draft for that thread and surface. Compact entry keeps drafts while its source
  row remains mounted, including closing and reopening the thread view; leaving
  the surface releases the state, and graph switches clear it.
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
semantics and author gutter runs are separate work.
