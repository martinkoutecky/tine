# The share dialog and its entry points (share step 1; docs/contracts/export-definitions.md).
share-dialog-title = Share
share-dialog-close = Close
share-dialog-what = What to share
share-dialog-this-page = This page
share-dialog-this-page-note = Shares only { $page }.
share-dialog-neighbours = This page and its neighbours
share-dialog-neighbours-soon = Coming with graph queries.
share-dialog-query = A query starting at this page
share-dialog-query-note = Edit the query to choose the pages. The pages it selects are exactly what is shared.
share-dialog-review = Pages that will be shared
share-dialog-resolving = Resolving pages…
share-dialog-pages = { $count ->
    [one] 1 page will be shared:
   *[other] { $count } pages will be shared:
}
share-dialog-where = Where to share
share-dialog-folder = Folder
share-dialog-folder-note = Exports a static site and read-only app into your graph folder. You name it on the next step.
share-dialog-link = Link
share-dialog-link-soon = Coming soon: quick share is the next step.
share-dialog-link-note = Uploads an encrypted copy to a secret gist in your GitHub account; anyone with the link can read it.
share-dialog-create-link = Create link…
share-dialog-saved-as-definition = After the export, Tine saves it as a definition on the Exports page, so it can be repeated.
share-dialog-saved-on-block = After the export, Tine records the folder on this query block (export:: property).
share-dialog-link-saved-as-definition = After the link is created, Tine lists it on the Exports page (a definition with tine.share::). The key that opens the link stays on this device.
share-dialog-link-saved-on-block = After the link is created, Tine records it on this query block (tine.share:: property). The key that opens the link stays on this device.
share-dialog-already-exported = Already exported to { $folder }.
share-dialog-cancel = Cancel
share-dialog-export = Export to folder…
share-dialog-definition-saved = Saved the export on { $page }.
share-dialog-definition-failed = The export was written, but saving its definition failed: { $error }
share-dialog-no-page = Open a page to share it
share-topbar-title = Share this page
share-page-title-button = Share this page
share-menu-item = Share…
share-query-icon = Share this query
