# Quick Switcher input

The Quick Switcher and command palette are global overlays. Opening them and
interacting with their controls preserves the focused content pane. A selected
command, page, block or create-page row acts in that pane, subject to the
existing explicit sidebar, background-tab and other-pane modifiers.

A mouse click, touch tap or pen tap activates a row once. Enter activates the
highlighted row. Page-result holds retain their context-menu behavior. Keeping
the input focused must not remove the pressed row between pointerdown and its
compatibility mousedown.

The editor's portalled slash and reference completion popup likewise preserves
its owning pane when a row is picked. Its selection callback runs once.

`QuickSwitcher.test.tsx` covers pointer activation with the real pane tracker,
keyboard activation and page-result holds. Native acceptance runs
`scripts/e2e-palette-pointer.mjs` and `scripts/e2e-code-wrap-toggle.mjs` on a
routed synthetic page with a focused split pane.
`EditorAutocomplete.pointer.test.tsx` covers completion callbacks and pane focus.
