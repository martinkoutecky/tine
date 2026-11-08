# Advanced query breadcrumbs

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
options can apply there. A simple-query View switch and `tine.breadcrumbs`
persistence are awaiting a product decision and are not implemented.
