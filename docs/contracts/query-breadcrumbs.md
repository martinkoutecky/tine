# Query breadcrumbs

Simple query block lists expose **Show breadcrumbs** beneath the View buttons.
Default is shown. Disabling writes `tine.breadcrumbs:: false` through the normal
undoable block property edit; enabling removes the property. The shared list
renderer hides ancestor trails and grouped page headings (or globally sorted
page crumbs). Group membership and result editing are unchanged. The switch is
absent for page results and Table, Board or Search views, where it has no effect.
Collapse preserves the choice. Main panes, sidebars and embeds use the same
property. Logseq treats the unknown property as ordinary block metadata and
keeps its usual query context; this simple-query presentation is Tine-specific.

Advanced query lists honor the direct EDN map entry `:breadcrumb-show? false`.
Absent, true or nil keeps breadcrumbs enabled. Nested entries, strings, comments
and discarded forms are not options. Presentation reads the Rust EDN reader's
form spans; it does not change the authored source or query membership.

As in OG 6e7afa8eb, `query.cljs/custom-query-inner` and
`block.cljs/->hiccup`, disabling breadcrumbs hides ancestor trails but keeps
page headings in page-grouped lists. An explicit `:group-by-page? false` uses
ungrouped rows without page headings. Tine's existing globally sorted list
uses page crumbs in place of headings; disabling breadcrumbs hides those too.
True and absent preserve existing sorted-list presentation.

The same renderer serves main panes, right sidebar and embeds. Tables and
query collapse are independent. BEGIN_QUERY retains Tine's existing table
default; an explicit `:table-view? false` selects its list rendering so these
options can apply there.
