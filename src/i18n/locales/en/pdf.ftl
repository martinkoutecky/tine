# Tine interface text. Graph names, paths and inserted syntax in variables stay unchanged.
pdf-copied-highlight-ref = Copied highlight ref
pdf-couldn-t-load-pdf-annotations = Couldn't load PDF annotations. ({ $detail })
pdf-couldn-t-capture-that-region-try-again = Couldn't capture that region — try again.
pdf-couldn-t-save-the-area-image-try-again = Couldn't save the area image — try again. ({ $detail })
pdf-finish-the-highlight-decision-or-area-image-cleanup-before-editing = Finish the highlight decision or area image cleanup before editing.
pdf-resolve-the-highlight-conflict-before-editing-more-highlights = Resolve the highlight conflict before editing more highlights.
pdf-cannot-close-or-switch-graphs-while-has-an-area-image-awaiting-cleanup-choose-retry-cleanup = Cannot close or switch graphs while { $name } has an area image awaiting cleanup. Choose Retry cleanup.
pdf-cannot-close-or-switch-graphs-while-has-a-highlight-conflict-choose-keep-mine-or-use-disk-version-or-discard-my-changes = Cannot close or switch graphs while { $name } has a highlight conflict. Choose Keep mine or Use disk version, or Discard my changes.
pdf-couldn-t-retire-the-unused-area-image-retry-cleanup = Couldn't retire the unused area image. Retry cleanup. ({ $detail })
pdf-couldn-t-save-notes-highlight-not-written-resolve-the-conflict-and-retry = Couldn't save notes — highlight not written. Resolve the conflict and retry.
pdf-highlight-saved-but-an-unused-area-image-couldn-t-move-to-trash = Highlight saved, but an unused area image couldn't move to trash. ({ $detail })
pdf-highlight-conflict-choose-keep-mine-or-use-disk-version = Highlight conflict — choose Keep mine or Use disk version.
pdf-couldn-t-save-highlight-it-remains-unsaved-in-the-pdf = Couldn't save highlight — it remains unsaved in the PDF. ({ $detail })
pdf-highlight-saved-but-notes-couldn-t-reload = Highlight saved, but notes couldn't reload. ({ $detail })
pdf-using-disk-highlights-local-changes-were-discarded = Using disk highlights; local changes were discarded.
pdf-couldn-t-use-disk-highlights-or-retire-their-area-image = Couldn't use disk highlights or retire their area image. ({ $detail })
pdf-couldn-t-reload-disk-highlights-for-keep-mine = Couldn't reload disk highlights for Keep mine. ({ $detail })
pdf-discard-your-local-pdf-highlight-changes-the-sidecar-will-be-left-untouched-any-unreferenced-local-crop-may-remain-on-disk = Discard your local PDF highlight changes? The sidecar will be left untouched. Any unreferenced local crop may remain on disk.
pdf-local-highlight-changes-discarded-disk-sidecar-was-left-untouched = Local highlight changes discarded; disk sidecar was left untouched.
pdf-couldn-t-confirm-highlight-discard = Couldn't confirm highlight discard. ({ $detail })
pdf-untitled = Untitled
pdf-area-image-cleanup-pending = Area image cleanup pending
pdf-unsaved-highlight = Unsaved highlight
pdf-previous-page = Previous page
pdf-page-type-a-number-and-press-enter-to-jump = Page — type a number and press Enter to jump
pdf-next-page = Next page
pdf-find-in-document-ctrl-f = Find in document (Ctrl+F)
pdf-zoom-out = Zoom out
pdf-zoom-in = Zoom in
pdf-fit-width = Fit width
pdf-fit-height = Fit height
pdf-area-highlight-drag-a-rectangle-to-capture-a-region-as-an-image = Area highlight ({ $shortcut }) — drag a rectangle to capture a region as an image
pdf-open-highlights-notes-page = Open highlights & notes page
pdf-notes = Notes
pdf-outline = Outline
pdf-more-settings = More settings
pdf-close-pdf = Close PDF
pdf-highlight-conflict-resolve-it-before-editing-more-highlights = Highlight conflict. Resolve it before editing more highlights.
pdf-keep-mine = Keep mine
pdf-use-disk-version = Use disk version
pdf-discard-my-changes = Discard my changes
pdf-area-image-cleanup-is-pending-retry-before-closing-or-switching-graphs = Area image cleanup is pending. Retry before closing or switching graphs.
pdf-retry-cleanup = Retry cleanup
pdf-pdf-settings = PDF settings
pdf-reader-tools = Reader tools
pdf-area-highlight = Area highlight
pdf-theme = Theme
pdf-theme-options = PDF theme
pdf-theme-label = { $theme ->
    [light] Light
    [warm] Warm
   *[dark] Dark
    }
pdf-theme-description = { $theme ->
    [light] Light PDF theme
    [warm] Warm PDF theme
   *[dark] Dark PDF theme
    }
pdf-document-outline = Document outline
pdf-loading-outline = Loading outline…
pdf-outline-too-large-partially-shown = Outline too large; partially shown.
pdf-no-outlines = No outlines
pdf-find-in-document = Find in document
# $index is the current match, $count the matches found. $truncatedSuffix is + when the search was capped, otherwise empty; keep it adjacent to $count.
pdf-find-count = { $index } / { $count }{ $truncatedSuffix }
pdf-no-results-in-scanned-text = No results in scanned text+
pdf-no-results = No results
pdf-previous-match-shift-enter = Previous match (Shift+Enter)
pdf-next-match-enter = Next match (Enter)
pdf-close-esc = Close (Esc)
pdf-couldn-t-open-this-pdf = { "Couldn't open this PDF: " }
pdf-copy-ref = Copy ref
pdf-linked-references = Linked references
pdf-remove-highlight = Remove highlight
pdf-outline-disclosure = { $state ->
    [collapse] Collapse { $label }
   *[expand] Expand { $label }
    }
