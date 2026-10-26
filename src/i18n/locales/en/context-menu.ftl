

# Remaining interface text (GH #107). Inline code and syntax stay unchanged.
context-menu-include-parent-block = Include parent block
context-menu-file-open-failed =
    { $action ->
        [show] Couldn't show the page file. ({ $error })
       *[open] Couldn't open the page file. ({ $error })
    }
context-menu-file-not-saved = This page has no on-disk file yet. Type something and let Tine save it first.
context-menu-child-view = { $active ->
        [yes] { "✓ " }
       *[no] { "" }
    }{ $view ->
        [grid] Grid
        [table] Table
       *[outline] Outline
    }
context-menu-delete-page-label =
    { $kind ->
        [journal] Delete journal
       *[page] Delete page
    }
context-menu-ui-couldn-t-copy-clipboard-write-failed = Couldn't copy: clipboard write failed.
context-menu-ui-page-actions = Page actions
context-menu-ui-show-children-as = Show children as →
context-menu-ui-automatic-heading = Automatic heading
context-menu-ui-auto = Auto
context-menu-ui-heading = Heading { $h }
context-menu-ui-remove-heading = Remove heading
context-menu-ui-no-background = No background
context-menu-ui-add-child-bullet = Add child bullet
context-menu-ui-zoom-into-cell = Zoom into cell
context-menu-ui-delete-row = Delete row
context-menu-ui-delete-column = Delete column
context-menu-ui-open-as-full-page = Open as full page
context-menu-ui-add-formula = Add formula…
context-menu-ui-edit-filter = Edit filter…
context-menu-ui-group-by = Group by →
context-menu-ui-no-grouping = { $choice1 ->
        [yes] { "✓ " }
       *[no] { "" }
    }No grouping
context-menu-ui-no-structural-actions = No structural actions
context-menu-ui-convert-to-pipe-table = Convert to pipe table
context-menu-ui-hierarchify-into-columns = Hierarchify into columns
context-menu-ui-hierarchify-by = Hierarchify by →
context-menu-ui-flatten = Flatten
context-menu-ui-open-in-sidebar = Open in sidebar
context-menu-ui-go-to-block = Go to block
context-menu-ui-copy-link = Copy link
context-menu-ui-copy-as = Copy as
context-menu-ui-copy-block-ref = Copy block ref
context-menu-ui-copied-block-ref = Copied block ref
context-menu-ui-copy-block-embed = Copy block embed
context-menu-ui-copied-block-embed = Copied block embed
context-menu-ui-a-template-named-already-exists = A template named “{ $title }” already exists.
context-menu-ui-template-created = Template “{ $title }” created.
context-menu-ui-make-a-template = Make a template…
context-menu-ui-template-name = Template name
context-menu-ui-create-template = Create template
context-menu-ui-tip-in-the-template-expand-when-it-s-inserted = Tip: in the template, <code><% today %></code>, <code><% current page %></code>, <code><% date: +3d %></code> expand when it's inserted (or via <code>/Template var</code>).
context-menu-ui-this-page-target-changed-reopen-the-page-actions-menu = This page target changed; reopen the page actions menu.
context-menu-ui-couldn-t-save-its-on-disk-file-was-not = Couldn't save “{ $name }”; its on-disk file was not opened.
context-menu-ui-has-an-unresolved-save-conflict-this-is-the-file = “{ $name }” has an unresolved save conflict — this is the file as it stands on disk. Your unsaved changes stay in Tine until you resolve it.
context-menu-ui-delete-the-file-moves-to-the-graph-s-tine =
    { $unsaved ->
        [discard] Delete "{ $name }"? The file moves to the graph's .tine-trash folder. Your unsaved changes to it are discarded too.
       *[none] Delete "{ $name }"? The file moves to the graph's .tine-trash folder.
    }
context-menu-ui-deleted = Deleted “{ $name }”
context-menu-ui-open = Open
context-menu-ui-open-in-new-tab = Open in new tab
context-menu-ui-open-in-new-window = Open in new window
context-menu-ui-remove-from-favorites = Remove from favorites
context-menu-ui-add-to-favorites = Add to favorites
context-menu-ui-copy-page-ref = Copy page ref
context-menu-ui-copied-page-ref = Copied page ref
context-menu-ui-copy-export-as = Copy / export as…
context-menu-ui-copy-page-as-markdown = Copy page as Markdown
context-menu-ui-copied-page-as-markdown = Copied page as Markdown
context-menu-ui-couldn-t-copy-page-as-markdown = Couldn't copy page as Markdown.
context-menu-ui-export-to-pdf = Export to PDF…
context-menu-ui-show-in-folder = Show in folder
context-menu-ui-open-with-default-app = Open with default app
context-menu-ui-find-in-page = Find in page
context-menu-ui-page-properties = Page properties…
context-menu-ui-carry-unfinished-tasks-today = Carry unfinished tasks → today
context-menu-rename-success =
    { $outcome ->
        [merged] Merged into “{ $next }”
       *[renamed] Renamed to “{ $next }”
    }
context-menu-ui-rename-failed = Rename failed: { $value1 }
context-menu-ui-rename-page = Rename page…
context-menu-ui-copied =
    { $count ->
        [one] Copied block
       *[other] Copied blocks
    }
context-menu-ui-zoom-into-block = Zoom into block
context-menu-ui-copy =
    { $count ->
        [one] Copy block
       *[other] Copy blocks
    }
context-menu-ui-properties = Properties…
context-menu-ui-insert-block-above = Insert block above
context-menu-ui-copy-block-refs = Copy block refs
context-menu-ui-copy-block-embeds = Copy block embeds
context-menu-ui-convert-to-grid = Convert to grid
context-menu-ui-cut =
    { $count ->
        [one] Cut block
       *[other] Cut blocks
    }
context-menu-ui-couldn-t-cut-clipboard-write-failed =
    { $count ->
        [one] Couldn't cut block: clipboard write failed.
       *[other] Couldn't cut blocks: clipboard write failed.
    }
context-menu-ui-remove-numbered-list = Remove numbered list
context-menu-ui-numbered-list = Numbered list
context-menu-ui-collapse-all = Collapse all
context-menu-ui-expand-all = Expand all
context-menu-ui-used-for-new-journals = ✓ Used for new journals
context-menu-ui-use-for-new-journals = Use for new journals
context-menu-ui-delete =
    { $count ->
        [one] Delete block
       *[other] Delete blocks
    }
