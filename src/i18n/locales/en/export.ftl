# Copy/export dialogs and reference occurrence controls.
# Keep Markdown, Org, OPML, HTML and Logseq syntax unchanged.
export-modal-plain-text = Plain text

export-modal-cleaned-the-text-as-displayed-without-markup-markers = cleaned — the text as displayed, without markup markers (bold, highlighting, links)

export-modal-markdown = Markdown

export-modal-formatting-preserved-block-references-resolved-and-embeds-expanded = formatting preserved — block references resolved and embeds expanded

export-modal-text = Text

export-modal-opml = OPML

export-modal-html = HTML

export-modal-dashes = Dashes

export-modal-logseq-outline-bullets = Logseq outline (- bullets)

export-modal-spaces = Spaces

export-modal-indent-no-bullets = indent, no bullets

export-modal-no-indent = No indent

export-modal-flat-no-bullets = flat, no bullets

export-modal-links-text = [[links]] → text

export-modal-remove-emphasis = Remove emphasis

export-modal-remove-tags = Remove #tags

export-modal-remove-properties = Remove properties

export-modal-newline-after-block = Newline after block

export-modal-resolve-refs-fully = Resolve refs fully

export-modal-copy-export = Copy / export{ " " }

export-modal-format = Format

export-modal-content = Content

export-modal-indent = Indent

export-modal-level = Level ≤

export-modal-close = Close

export-modal-resolving = Resolving...

export-modal-copy = Copy

# $kind is explicit for authored links or plain for unlinked text; $index is the one-based mention position.
reference-evidence-jump-to-mention =
    { $kind ->
        [explicit] Jump to explicit mention { $index }
       *[plain] Jump to plain mention { $index }
    }

reference-evidence-open-this-block-in = Open this block in { $page }

reference-evidence-open-this-mention-in = Open this mention in { $page }

reference-evidence-open-mention-in = Open mention { $span } in { $page }

reference-evidence-show-excerpt = Show excerpt

reference-evidence-show-full-block = Show full block

reference-export-chooser-copy-export = Copy / export { $subject }

reference-export-chooser-reference-selection = Reference selection

reference-export-chooser-all = All

reference-export-chooser-none = None

reference-export-chooser-entries = { $subject } entries

reference-export-chooser-no-entries-in-this-section = No entries in this section.

reference-export-chooser-cancel = Cancel

reference-export-chooser-open = Copy / export…

export-modal-block-count =
    { $count ->
        [one] { NUMBER($count, useGrouping: 0) } block
       *[other] { NUMBER($count, useGrouping: 0) } blocks
    }
reference-export-chooser-count = { $count } of { $total }
reference-evidence-mention-count =
    { $count ->
        [one] { NUMBER($count, useGrouping: 0) } mention
       *[other] { NUMBER($count, useGrouping: 0) } mentions
    }
reference-evidence-occurrence-label = Jump to mention { $index } of { $total }

# Helper-return and component-prop interface text (GH #107).
export-prose-plain-text-output-has-no-markup-markers-to-remove = Plain text output has no markup markers to remove
export-prose-only-applies-to-plain-text-output = Only applies to plain text output
