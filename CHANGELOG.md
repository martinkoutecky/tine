# Changelog

All notable changes to Tine are documented here. Tine is a fast, local-first
outliner that reads and writes a real Logseq Markdown (and now Org) graph.

The format follows [Keep a Changelog](https://keepachangelog.com/); versions use
[Semantic Versioning](https://semver.org/).

## [Unreleased]

- Added open-only `tine://` graph, page and block links, with Copy link actions and lazy stable graph identity (GH #181).

- Code blocks show line numbers and a language label; Settings → Appearance offers one device-wide wrap preference shared by reading and editing (GH #474).

- Beta releases now include the experimental Windows 32-bit installer and portable ZIP, with manual updates as on stable (GH #275).
### Added
- Guide page **Reference/Tine query model**: every way a query's answer can differ from Logseq's, and why — the "find more, never less; browsing never writes" rule, the one exception, what Tine refuses, and how Tine 0.6 differs (GH #422).
- Added TQL `parent`, `ancestors`, and unbounded `descendants` relations with `any`, `none`, and `every` quantifiers (GH #551).
- `/Date picker`, `/Tomorrow`, and `/Yesterday` insert journal date links in the graph's configured title format (GH #485).

- Wide Markdown, Org, query and sheet tables now use free space on both sides of the text column before scrolling at the pane boundary (UI-OG-QBG-TABLE-BLEED).
- Added remappable, initially unbound commands to grow/shrink either sidebar width (GH #425) and open a persistent search tab directly (GH #437).
- Truncated left-sidebar page titles now show their full title on hover; fitting titles stay quiet (GH #563).

### Fixed
- A PDF link into a nested assets folder (`../assets/nested/report.pdf`) opens that PDF, not a same-named one in the assets root, and highlights made in it are saved under the nested PDF's own page and sidecar; opening the PDF from a highlight's page does the same (UI-OG-C5-P6-PDFLINK).
- Image `{:width ...}` / `{:height ...}` metadata is read as EDN, so a quoted title that contains `:width 999` no longer sets the image width (UI-OG-C5-P6-IMAGEMETA).
- PDF Find shows an error and clears stale results when a page cannot be read, and closing Find or changing the query stops a pending search from moving the reader (UI-OG-C5-P6-FIND); a PDF text layer that finishes after its page was re-rendered or the reader closed no longer installs, and a failed text render shows the PDF error (UI-OG-C5-P6-TEXTLAYER).
- A rejected native window request (minimize, maximize, close, resize drag) now shows an error instead of failing silently, and a resize listener registered after the window chrome was removed is released (UI-OG-C5-P6-WINDOW).
- Page rename publication skips reparsing the old document and repeated transaction-record searches; loaded-page refresh uses one path lookup per touched page (GH #623).
- Closing or switching PDFs no longer shows an annotation-loading error from a retired reader (GH #557).
- Image picker and Upload keep the initiating editor through asset import, so delayed native blur does not discard the saved image link (GH #622, GH #493).
- Journal date-format parity: honor legacy `:date-formatter` after the modern key, offer all Logseq formats, and render `E`/`EE` as abbreviated weekdays while keeping old titles readable (GH #332 follow-ups).
- Plain-text search skips Unicode normalization for ASCII text while preserving search results and highlight spans (GH #623).
- Query property commas now follow Logseq's configured keys; query text matches raw content with exact case and accents, while Search keeps folding (GH #624). Imported `query-table::` and trailing `table` choices are honored without writing Logseq view properties.
- GH #510: edit a code region inside a mixed text block without exposing its fences; boundary navigation returns to source editing and preserves surrounding text.
- Find searches main and split panes plus expanded right-sidebar items, with next/previous traversing them in order (GH #559).
- Search-tab results open in new tabs while keeping Search; middle-click and modifiers choose background tabs, sidebar or another pane, and page results share the Quick Switcher context menu (GH #416).
- Opening and reading graph PDFs leaves annotation files untouched; create the highlight sidecar and notes page on the first annotation (GH #577).
- Date pickers keep a selected day open for time/repeat edits; Done or clicking outside applies the complete choice, while Escape cancels (GH #30).
- GH #407: Markdown/Org copy export preserves formatting while resolving block refs and expanding embeds; HTML/OPML share resolution, hidden IDs are omitted, and plain text retains user property separators.
- Changing only page-name capitalization updates filename spelling, title and references in one guarded rename (GH #609).
- PDF `file://` links now open in the desktop's default viewer with their full path, without redirecting to graph assets or creating annotation pages (GH #577).
- Android and iOS startup no longer shows “Couldn’t read the window state” from an unsupported desktop maximize query (GH #621).
- Journals feed days now show Linked References with counts and the shared collapse/filter controls, loading as you scroll and hiding empty sections (GH #481).
- Code cards keep their surrounding layout on entering/leaving edit, including empty code and long lines (GH #474, layout only). Empty code wrappers can be removed with Backspace, and right-click at a desktop code caret offers the existing block deletion action (GH #488).
- Android's keyboard toolbar shows distinct return arrows for Undo and Redo (GH #599).
- Linux deb/rpm installations no longer gain a second visible launcher on startup; the app-ID entry remains available for Wayland icon lookup (stable and Beta, GH #626).
- Deleting a page or journal now succeeds if its file was deleted externally while the confirmation was open; a changed title or replacement file still cannot be deleted through a stale page target (GH #620).
- First opens of large pages skip unnecessary offscreen body parses for property-free blocks, using the properties already computed by the backend (GH #623).
- Mobile block swipes reveal distinct indent, outdent and more icons before release; left swipes now have a wider outdent band (40–139 px), with actions at 140 px, so a natural swipe is less likely to open the menu (UI-OG-QBF-SWIPE).
- Render block properties as linked keys on separate rows with OG duplicate/group precedence, shade block embeds in both themes, and show pages tagged with the current page before references (GH #612, GH #610, GH #214).
- An unexpected rendering failure in a PDF pane, sidebar item, query result, plugin settings view or dialog now reports its error within that surface with **Retry** and **Copy details**, while sibling surfaces remain usable (GH #490, GH #332; UI-OG-GH490-QBE).
- Contributor instructions, harness scripts and historical records use portable paths, with a tracked-file guard against personal checkout paths (GH #579).
- Multi-block context-menu copy, cut, references, embeds and delete now act on the whole selection and use plural labels (GH #591).
- Scheduled and Deadline slash calendars take keyboard focus for date navigation and return to the editor after picking or cancelling (GH #596).
- Fixed query grouping summaries to name their field and offer Clear, hide a sole missing-value group table, and keep Search/List page results uncluttered with properties one click away (UI-OG-QBV-QUERY-DISPLAY).
- External asset replacements, creations and deletions made while the graph watch is being installed now refresh the open graph (REG-OG-QBW-ASSET-WATCH-STARTUP).
- Linux YouTube embeds now provide native client identification and initialize without configuration error 153 (GH #600); video playback still requires supported system codecs.
- Query blocks with title options no longer leave a stray `}` below their results. A damaged macro now reports that its source could not be parsed and stays editable, instead of showing All pages or failing a builder edit with “The block changed while saving” (UI-OG-QUERY-MACRO-BRACE).
- A page containing a lone carriage return (a CR not followed by LF, for example `key::` ended by CR) no longer shows "This page could not be displayed. Unreachable code should not be executed" and no longer breaks every later page: the lsdoc parser (v0.5.8) now handles bare CRs as Logseq's parser does, and any future parser panic is confined to the one call that caused it.
- Saving on a network or FUSE filesystem that cannot sync directories (some NFS mounts, Android shared storage) no longer fails: as on master, the errors such filesystems give for a directory sync (EBADF, EACCES, EISDIR, EINVAL) are tolerated, while a real I/O failure or a full disk still fails the save.
- A save that fails for a passing reason (a disk hiccup, a file briefly locked by another program) is retried automatically after 100 ms and 300 ms before Tine reports it; the page stays marked unsaved until it is saved (REG-OG-XPORT-SAVE-TRANSIENT-RETRY).
- Creating a page is refused, naming the file, when a file Tine cannot read (for example one with broken text encoding delivered by sync or an editor) may already be that page, instead of creating a second file for the same page; your edits stay in the editor (REG-OG-XPORT-CREATE-UNREADABLE-OWNER).
- A graph that fails to open now shows a red notice that stays until dismissed, with a **Retry** button that reopens the same graph (from the folder picker, the graph list or its right-click menu) (REG-OG-XPORT-GRAPH-OPEN-RETRY).
- Opening or switching graphs no longer freezes the window: the graph now opens in the background, and closing the previous graph (about 0.2 s) no longer holds up other requests (REG-OG-XPORT-LOAD-GRAPH-OFF-UI).
- The linked and unlinked references error banner says what actually went wrong instead of a generic message (REG-OG-XPORT-REFS-BACKEND-ERROR-TEXT); a reference filter whose page-header tags were cut short now says the list is incomplete (REG-OG-XPORT-PAGEPROP-FILTER-TRUNCATED).
- Browsing no longer writes to your graph (GH #623): zooming into a block, jumping to one from Ctrl-K, and opening a block in the sidebar, a tab or a pane used to stamp an `id::` property into the block's page file just to remember where you were. Those places are now remembered by their position in the page (kept in the session file, not the graph), and a block gets an `id::` only when something actually references it. A saved session with an older block reference still restores.
- Opening a page on a large graph no longer stalls (GH #623): the first page open after launch or after a page was created, deleted or delivered by sync read every page file's header under the store's lock (about 2.8 s on an 11,000-page graph on Windows); it now reads only that page, and a page whose file is named for it opens while the graph is still loading. Folding or unfolding a block saves the file without rebuilding graph-wide state or rewriting the launch checkpoint (a save went from about 20 ms to about 3 ms on a 13,000-page graph). Launching no longer freezes while Tine lists the installed spell-check dictionaries: that ran a helper program on the window's main thread and held every other request behind it (seconds on a slow disk or network drive); it, and opening links, assets and folders in other programs, now run in the background.
- Launch and focus return on large graphs (GH #623): the conflicts list no longer reads every page file (the store remembers which pages carry merge-conflict marker lines, kept in the launch checkpoint), the asset scan no longer opens every asset file while holding the store's writer lock, and the "Refreshing changes from disk…" notice moved beside the help button, dimmed and shown only after half a second; the diagnostic report now also carries per-command latency histograms and the focus-return phase split (numbers only).
- Query builder usable again (GH #619, UI-OG-GH619-*): presses and keys inside the open query sheet no longer reach the block behind it (before, a press entered edit mode and tore the sheet down, so the Find pages/blocks control and the other buttons never acted); every floating panel now shares one portal that keeps events inside it, guarded against a bare portal; field-chooser rows no longer shrink to slivers and the list scrolls, with dropdowns that flip or clamp to the viewport; the sheet opens beside its query instead of at the top-left and stays inside the window (it flips above a query near the bottom); typing `/query` opens the sheet on its condition list without also opening the field chooser; and the drag-reorder bar is no longer drawn at slots where a drop changes nothing.
- Queries answer like Logseq, round two (Discussion #617 query audit): an advanced query with any clause Tine cannot read is refused whole, with a notice naming the clauses, instead of running part of it and showing a wrong answer (REG-OG-Q617-QFIX-WHOLE-REFUSAL); printing a query back to Logseq text keeps `page-property` as page-level (REG-OG-Q617-QFIX-ROUNDTRIP, with a permanent 760-form round-trip test); `(all-page-tags)` lists the pages some page uses as a tag; `(task [TODO DOING])`, `(priority #{A B})` and `(page-tags (x y))` accept set and list forms; `not` takes several operands; `sort-by` is descending unless `asc`, `sample` picks a random subset before sorting, and a query made only of those directives shows nothing; `(between created-at -7d now)` and `last-modified-at` filter by timestamp properties; the page-property header block counts as a referrer and in block-reference counts, and its properties appear in the property picker; `(namespace X)` matches only the pages directly under `X` (advanced queries stay recursive); journal result groups run newest day first; and a legacy `{:table-view? true}` table honours the block's `query-properties`, `query-sort-by` and `query-sort-desc` (REG-OG-Q617-QFIX-*).
- Queries answer like Logseq (Discussions #617/#619, issue #624): `(priority [A])`, `(task [TODO])` and `(page-tags [x])` accept Logseq's vector form; a plain text search also finds text that sits in a property line; sibling matches under one parent show that parent's breadcrumb once and sit together; the query builder's pages/blocks choice now sticks (REG-OG-Q617B-VECTOR-FORMS, -PROPERTY-LINE-TEXT, -GROUP-BY-PARENT, -BUILDER-ANCHOR).
- Simple queries such as `(property type [[Person]])` now find pages whose properties are in the page header, as Logseq does: the page-property block takes part in `property`, `page`, `between`, page-reference and boolean queries and appears as a read-only page-property row (REG-OG-Q617-PREBLOCK-QUERY, Discussion #617).
- **Raw HTML resources (UI-OG-DUPBL2-D29):** local images and sandboxed iframes use actual DOM attributes, so image text in comments and `data-src` lookalikes cannot select the wrong resource.
- Uppercase `.ORG` files retain Org format in conflict diffs, PDF annotations and page rename/merge (REG-OG-DUPAL2-F12).
- Keep code examples in a user’s Favorites page from being adopted as favorites arrangement metadata (UI-OG-DUPBL1-FAVORITES-LITERAL-MARKER).
- Preserve code literals during text, HTML and OPML export cleanup; recognize standalone embeds from parsed content; preserve early journal years and advance timed repeaters (REG-OG-DUPD2-D15–D19).

### Added

- Touch gestures on a phone (GH #492, GH #501): swipe a block right to indent it, a short way left to outdent it, or a long way left to select it and open its action menu; swipe in from the left edge to open the left drawer (a mostly-vertical swipe is ordinary scrolling and does nothing); on iOS the same edge swipe goes Back when there is something to go back to, following your finger and snapping back if you let go early; and the image viewer takes pinch-to-zoom, double-tap zoom, drag-to-pan, a sideways swipe between the page's images and a drag up or down to close (modelled on Logseq's PhotoSwipe viewer: a tap shows or hides the buttons instead of closing, a new × button closes, and dragging a zoomed image past its edge turns to the next image). Swiping is off while typing, in tables, code, queries, drawings and media, and with more than one finger. Logseq's `:mobile {:gestures/disabled-in-block-with-tags [...]}` setting in `config.edn` also turns the block swipe off in any block that references a listed page, and in the blocks nested under it. Documented in Reference > Platforms and mobile.
- Windows: the diagnostic report now records whether Windows Defender real-time protection is on (`windowsDefenderRealtime`: on, off, unknown or not-applicable; a flag only, never a path). When a graph's first open is slow and real-time protection is on, a one-time notice per graph says so and offers **Add an exclusion for this graph folder**, which asks for administrator approval and only then runs `Add-MpPreference -ExclusionPath`; nothing changes without that click, and a declined or refused attempt is reported (GH #623).
- Settings → Help & diagnostics: the diagnostic report now carries launch timings (listing, file dates, reading vs parsing, index build, ready), recent full rescans and saves, and statistics-only graph-shape numbers (sizes and quantiles, never names or text), so a slow graph can be diagnosed without sharing it; **Rescan graph** forces a full rebuild: it ignores every remembered file date, size and checksum, re-reads and re-parses every file, and rebuilds the page, link and search state without blocking the app or touching what you are editing, then shows when it finished; the quick check on returning to the window is unchanged (GH #623).
- Query sheet, from hestratos's design list (GH #619, UI-OG-GH619-*): the Task field has **Any status** (writes every task marker OG knows) and a new **In a journal page** condition (writes `(between -2000y +2000y)`, which Logseq reads as journal-only); a condition can be dragged into another group and a group left with one condition dissolves; the query text is hidden behind an **Edit as text** toggle that is remembered across restarts (stored in the app settings, not localStorage); results show and follow your edits inside the open sheet before you save (debounced, latest answer only); page results show the page's own properties and a pencil that edits them through the page's property panel; and the Find menu has **Pages and blocks**, showing both answers with Pages above Blocks (the sentence reads *Pages and blocks where …*, the Blocks section follows your List/Table/Board choice, a sample that cuts a section short says so, and the open sheet previews both), stored as `tine.result-kinds:: pages-and-blocks` which Logseq ignores.
- Settings → About: **Check for updates automatically** can disable startup update checks and notifications on this device; manual checks remain available (GH #618).

### Changed

- Android Back follows the one Back ladder used everywhere (GH #492): it dismisses an open dialog, menu or image first, then closes an open drawer, then steps back through page history, and exits only at the root; a failed exit keeps the screen shield up and the next Back retries only the exit.
- Launch: Tine reopens a graph from a checkpoint of its last state in app data, then checks every file's date and size and rereads only what changed before the graph is ready; the checkpoint includes remembered query and backlinks answers, and is written about a minute after editing pauses (at most every 5 minutes), or 5 seconds after a launch that had none (GH #623).
- Query sheet rows no longer say *advanced* for anything the builder wrote, and Scheduled/Deadline/journal date ranges read as Scheduled, Deadline or journal date instead of *Between dates* (GH #619, UI-OG-GH619-NO-ADVANCED).
- REG-OG-P12B-PAGE-HEADER: A Markdown page header is now exactly the leading properties the parser accepts, so a no-space `key::value` line is page text rather than a property, in the editor, page promotion on save and alias lookup alike.
- REG-OG-P12B-FENCES: Fence-aware editing (property and language autocomplete, code-body editing, calc blocks, hidden metadata) follows Logseq's parser instead of CommonMark: any fence run closes a fence, whatever its length.

### Fixed

- Sheet date footers skip impossible calendar dates, and typed cells and formulas preserve years below 100 (UI-OG-DUPD3-DATE-FOOTER, REG-OG-DUPD3-CALENDAR).

- REG-OG-DUPF05-UNICODE-IDENTITY: Page-name lookup and query grouping use the native Unicode whitespace policy in the frontend too, so U+0085 and BOM no longer produce different identities or accepted grouping fields.
- Annotation editing and PDF targets respect parsed properties, including empty results, and ignore metadata inside code (UI-OG-DUPD1-D08).
- Outline insertion no longer treats literal `id::` examples as live block identities; merge identity recovery also uses accepted properties (REG-OG-DUPD1-D09).
- Caret link navigation ignores code and follows parser-accepted tag boundaries, including punctuation (UI-OG-DUPD1-D10).

- REG-OG-P12B-CONFLICT-PREAMBLE: Merging a conflict copy's page preamble no longer treats property-looking lines inside code blocks as page properties, and keeps differing code blocks whole instead of merging them line by line.
- REG-OG-P12B-MARKERS: Task markers and priorities share one parser-derived span for reading and writing, and the demo graph reads them once the parser is ready.
- Published block previews copy only the requested nodes, avoiding full-subtree allocation while keeping snapshot metadata independent (REG-OG-DUPF03-PREVIEW).

- Property readers and sheet field rename now use accepted parser properties and source spans, including Unicode keys and literal-code protection (REG-OG-P12-PROPERTY-SPANS).
- Page aliases and titles ignore metadata-looking source-block contents (REG-OG-P12-PREAMBLE-LITERALS).
- Page-reference candidates share native target classification, exclude unlabeled local assets, and include nested links; Org filename candidates retain their existing selection policy (REG-OG-P12-REFERENCE-TARGETS).

- REG-OG-P11B-BEGIN-QUERY: Static BEGIN_QUERY export uses the live EDN inspector, preserving escaped titles, discarded forms and typed positional inputs.
- REG-OG-P11B-OUTLINE-LITERALS: Outline paste keeps parser-owned literal blocks and bullet continuations intact while preserving ordered list recognition.

### Tests
- Added native coverage for Ctrl+K alias navigation with ASCII/fullwidth commas and LF/CRLF headers (GH #623); the reported Windows failure was not reproduced on current og under Linux.
- Added native Shift+click coverage across journals, main, sidebar and split panes, plus rendered modifier checks for page refs, tags, property values and block refs (GH #623); current og passes without a production change.

## [0.7.0-beta.1] - 2026-09-30

### Changed
- UI-OG-BETA-VERSION: About and copied reports identify **Tine Beta**; Copy version includes the full version, and update notifications retain the Beta sequence. The updater refuses a stable payload before offering or installing it.
- REG-OG-BETA-CHANNEL: The separate preview app is now **Tine Beta**, with the `beta` update/release channel and `-beta.N` prerelease versions. Stable Tine and Beta keep separate app data and update channels; the local deploy filename remains `tine-og`.
- REG-OG-R6-PREVIEW: Preview candidates and AppImage update metadata stay on `og-preview`; manual candidate builds do not publish, and preview publication cannot become the stable latest release.

### Fixed
- UI-OG-E2EB-WORKSPACE-LIFETIME: Editing a query workspace's source, presentation or Display keeps its live answers; replacing the route object no longer closes the active workspace's search.
- REG-OG-P11-ASSET-LIVENESS: Unused-media discovery and trash protect linked filenames containing parentheses and percent escapes, using parser-owned targets.
- REG-OG-P11-PROPERTIES: Cross-format paste preserves parser-accepted Unicode and custom property keys, duplicate entries, and their order.
- Title rename avoids reading temporary navigation destinations while refreshing the graph (UI-OG-P10C-RENAME-READS).
- REG-OG-P10B-IDENTITY-COST: Loaded blocks reuse parser-owned identity absence through the shared answerer for editor and reference badges, avoiding offscreen identity parses on large pages.
- REG-OG-P10-QUERY-PRIORITY: Background List mounting yields while the search picker covers it, retains existing rows, and resumes on a later frame when the picker closes.
- REG-OG-P10-QUERY-FRAMES: Broad List queries mount at most 32 new keyed groups per frame, reserve pending scroll height, and cancel queued rendering when their owner retires.
- REG-OG-P10-QUERY-VIEWPORT / REG-OG-P10-DATE-PARSE: Large List queries defer group headers and live subtrees until viewport approach; compiled journal date patterns parse without allocating temporary token strings.
- REG-OG-P10-LAZY-GROUPS / REG-OG-P10-JOURNAL-SCAN: Offscreen query and reference groups defer row state until they approach the viewport; journal inventories validate relative paths without repeatedly allocating configured directory prefixes.
- REG-OG-P10-BLOCK-MEMORY / REG-OG-P10-QUERY-PRESENTATION / REG-OG-P10-JOURNAL-FORMAT: Large graphs share sparse parser edit regions and compiled journal formats; List queries avoid building excerpts for inactive presentations.
- REG-OG-P10-HOSTED-PERF: Hosted OG performance checks retain the fixed v0.6.5 campaign anchor after stable releases, with unchanged performance budgets.
- REG-OG-R6B-PREFLIGHT: Preview preflight uses the OG v0.6.5 performance baseline with unchanged budgets.
- UI-OG-R6B-GRAPH-NOTICES / UI-OG-R6B-MIGRATION-TOAST: Query notice dismissals follow the current graph, and startup explains completed legacy app-data migration once.
- REG-OG-R6-IDENTITY: Read released Tine’s graph notice dismissals, restore legacy desktop app data before startup, and seed desktop preview browser preferences without changing the released source.


### Fixed

- Windows validation uses portable structural-ratchet keys and exercises page-merge races across alternate path spellings (og-W1; test harness only).

- **Page saves reuse their checked parse when publishing, and renames use the known page-name index** (og-R5; REG-OG-R5-SAVE-PARSE, REG-OG-R5-RENAME-COST). Saves retain their existing byte-preservation checks; a rename no longer rereads unrelated page headers for its same-name check.
- **Browser and native query rewrites use the same macro extent reader**, preserving authored arguments and rejecting malformed closing braces consistently (og-R4C; REG-OG-R4C-MACRO-EXTENTS).

- **Org typing keeps hidden IDs in the parser-accepted drawer after CLOSED planning and leaves source blocks intact** (og-R4C; UI-OG-R4C-ORG-REATTACH).

- Query table footers show complete backend totals and save Count, Sum, Average, or None through Display without reordering repeated entries. Static export totals summarize exported rows (UI-OG-R3A3-QUERY-FOOTER).

- **Splitting an Org block keeps its own ID on the original block after CLOSED planning** (og-R4B; UI-OG-R4B-OWN-PROPERTIES). The editor hides properties from the parser's own property region and keeps body drawers visible.

- **Nested queries in a query export no longer report how many matches were left out on other pages** (og-R3C; REG-OG-R3C-NESTED-COUNTS). This applies to the static fallback as well as the read-only app.

### Added

- **Query export writes a movable site inside the graph** at `published-queries/<folder>/` (og-R3C2; UI-OG-R3C2-QUERY-DESTINATION, REG-OG-R3C2-QUERY-PORTABILITY). A collision offers Replace or a separate folder; replacement preserves and reports the previous export in recovery. Referenced assets travel with the folder, missing assets warn, and Settings → Graph → Query export size limit sets a device-local copied-asset budget (default 1 GiB), with a direct Settings action when it is exceeded. Live and command-line exports keep their external destinations.

- **An old system web engine shows an update card before startup** (GH #572; og-R2). On macOS it explains the macOS 12.3 requirement and why updating Safari alone does not update the embedded engine.

- **Missing page links are muted with a dotted underline** (tags are not) (og-R2; UI-OG-R2-MISSING-LINKS). They still open their destination, and become live when a page or alias is created; deletion dims them again without reloading. This deliberately differs from Logseq, which dims only untitled pages.

- **Parser-owned structural edits** (og-D1): metadata edits and template/conflict copies share native/wasm block regions, preserving parser-recognized literal contents. Empty code cards retain their body/wrapper separator after typing, code-body typing preserves literal typography, and calendar edits retain glued body text. Published block identities, logbook and repeaters use the block's format.
- **An image replaced outside Tine refreshes where it is shown** (master d017d1afc, 2f54a8d5e, og-J2).
  A picture changed by an editor, Syncthing, Dropbox or another Tine window updates in place without reloading the page,
  and one deleted outside shows its missing-image placeholder. This includes an `assets` link to an approved folder outside the
  graph, even when it is reached through symlinked folders. An open PDF, audio or video is not swapped while you use it;
  it shows the new file the next time you open it. Tine's own asset writes never echo back as outside changes, and the extra
  watch is released when the graph closes.
- **A query shown as a table or board exports as that table or board** (og E, 22c open item).
  A block whose whole body is one `{{query …}}` with `tine.view:: table` or `board` (a query `as table`/`as board` may override it)
  now publishes with the query's own columns and rows, on the page each row came from, computed by the
  app's own sheet code; a result on a page the export does not publish is left out (and never counted), and if the results changed while the
  export was prepared it stays the usual result list. The command-line export still writes the outline.
- **A TQL query with a page reference after a comma saves as a macro** (og E).
  A form such as `any(children, [[a]])` used to be refused when Tine wrote it into a
  `{{tine-query …}}` block, because the document parser read the comma-separated `[[a]])` as a
  page reference; the saved macro now spells that operand in parentheses, and it reads back as the same query.
- **Verify a synchronized graph** (master 749bfb2b1, og-D). Settings → Help &
  diagnostics can now fingerprint the exact bytes of your Markdown and Org files
  and compare them with the report from another device, naming every file that
  exists on only one side or differs. It reads files only, reports paths and
  checksums (never contents), can be cancelled, and never confirms a match from
  a scan that was disturbed while it ran.
- **Recent error messages in Diagnostics** (og-D). Settings → Help & diagnostics
  lists the last 20 red error messages of this session, newest first, with the
  time, a ×N count for repeats and a Copy button, so a message can still be
  reported after you closed it. It lives in memory only: never written to disk,
  and not part of the diagnostic report, because messages can name pages.
- **A command line you can rely on** (master e7af4db9c, og-D). `tine --help`,
  `tine --version`, `tine open GRAPH`, `tine capture`, `tine doctor GRAPH` and
  `tine export static|live GRAPH --output PARENT` share one documented schema,
  and Linux `.deb` and `.rpm` packages install a generated `man tine`. On Windows
  the terminal commands now print into the console that launched them. See the
  new Guide page Reference/Command line.
- **A conflict can no longer scroll out of sight** (master 61ea6600c, og A).
  Once the in-page review scrolls above the window, a slim bar pins to the top
  of the pane; tapping it unrolls the same review in place, with your choices
  kept. Tap again, press Escape, or scroll back up to fold it away.
- **A duplicate journal day resolves on the day itself** (master 9dc54e4a7, og A).
  A day with two files (a date-named one and a title-named one) joins the
  conflict badge and the Conflicts page, and its page compares the two files
  block by block: the pre-selected choice keeps everything, and applying folds
  the other file in and moves it to the recoverable trash. Each file keeps its
  Open, Rename and Trash actions; the startup toast is gone, and Settings keeps
  the list as a fallback.
- **Right-sidebar items can be reordered by dragging their header** (GH #211, og 22b); a short press still opens the item.
- **External changes stay fresh without interrupting you.** Returning to Tine
  asks the watcher for a fresh pass before you type; a large checkout or sync
  burst arrives as one change; `.git/` and `.stfolder/` are ignored; and if the
  system refuses live file notifications, Tine says so and checks every 3
  seconds instead. An optional "always ask" policy (Settings → Backups &
  recovery) holds even clean external changes for **Reload from disk** /
  **Keep mine**.
- **A live-draft conflict offers Review, not a blind overwrite.** Its banner opens
  the in-page comparison, and the Conflicts page lists such pages under
  **Unsaved drafts**.
- **Sheets come out of Export as the tables, boards and grids you see in the app.**
  Export HTML and read-only app, a query's Export and Export to PDF write each
  `tine.view` table, board or grid with its rows, groups, formula columns and
  aggregate footers. Tine computes them with the app's own sheet code and hands
  the publisher plain data, so the export cannot disagree with the screen. The
  command-line export has no app to compute them and keeps the plain outline; a
  sheet that cannot be computed is written as the outline with a visible note
  instead of failing the export.
- **A "changed on disk" conflict can be merged block by block at the page.** The
  panel at the top of the page compares your unsaved edits with the file as it is
  now, pre-selecting each side's own change when Tine still has the version you
  started from. Apply writes only if the file is still the version shown; a newer
  change refreshes the comparison. If Tine closes first, the draft is kept and the
  same comparison appears after the next start, even when the page cannot be
  opened (GH #541).
- **Alt+click opens an internal link in the other pane** (GH #438), and **Ctrl/Cmd+click opens a background tab** (GH #283), on page links, block references, reference headers, namespaces, the sidebars, zoom breadcrumbs and query results, next to Shift+click (right sidebar) and middle-click (background tab). Middle-button autoscroll and Shift range-selection are suppressed on every one of those surfaces (GH #207). Ctrl/Cmd+click no longer opens the other pane; that is now Alt+click.
- **Ctrl+Y also redoes** on Windows and Linux (GH #491). It is a second default for the same Redo command, shown beside the binding in the shortcuts list; remapping or unbinding Redo replaces both chords.
- **Dropping a block on the right of another block's text nests it** as that block's last child (GH #326), with a drop marker showing where it will land.
- **The Guide grows thirteen task pages.** Start (Where things are, Bring an
  existing graph), Workflows (Keep context visible, Structure repeated
  information, Capture and plan your day, Find and revisit, Research a document,
  Extend Tine) and Reference (Files, external edits and backups; Troubleshooting
  and recovery; Journals, tasks and scheduling; Pages, links, references and
  search; Platforms and mobile). Each describes only what this build does.
- **A part of Tine that fails now says so, where it failed, with a Retry — instead
  of leaving the window blank.** The page, the sidebar, Linked and Unlinked
  References and the conflict panel each fail on their own; the message says how
  many backend operations are still outstanding and for how long when that is the
  likely cause (GH #490, GH #332).
- **When the graph you last used will not open, the Welcome screen says which one
  and why**, with Try again, Copy details and Open another graph.
- **Page width is configurable per device** (Settings → Appearance → Advanced):
  a standard reading width, and a fill-pane or custom maximum for Wide mode
  (GH #382).

### Fixed

- **PDF highlight reads run on a blocking worker** (og-R4A; REG-OG-R4A-HIGHLIGHT-READ-WORKER), returning worker panics as errors instead of aborting the app.
- **Query title edits preserve unrelated authored EDN** (og-R4A; UI-OG-R4A-QUERY-TITLE-SPANS). Nested titles and discarded forms stay intact; unreadable options refuse the edit visibly. Option reads and edits share one Rust span reader across native and wasm.

- Sheet field rename preserves ordered query aggregates, including repeated keys, average, whole-result count and unrelated configuration (UI-OG-R3A2-AGGREGATE-RENAME).
- Starting another PDF export supersedes pending preparation; graph changes and window teardown discard stale output (UI-OG-R3A2-PRINT-SUPERSESSION).
- Guide explains table-only sort clearing, search Retry and block-picker states, aggregate rename preservation, and Print query-limit refusal.
- **Cold Quick Capture waits for its graph before showing the editor** (UI-OG-CAPTURE-COLD-R3B): graph-backed suggestions and the first captured entry work on `tine --capture` startup; superseded shows cannot focus or retarget the window.
- **Launch backups wait until startup is idle** (REG-OG-LAUNCH-BACKUP-IDLE-R3B): warm completion signals the background snapshot after a quiet period, with a safety deadline and immediate graph-switch cancellation.

- **Custom journal titles apply before graph activation and live settings refresh** (REG-OG-CUSTOM-JOURNAL-TITLE-R3B): journal-template lookup reads a sync-delivered journal under its configured title. Missing-baseline saves remain guarded.
- **Unicode regex search agrees across native queries and browser filters** (REG-OG-R1-REGEX-001): matching and highlights use the same bounded Rust engine. `\d`, `\w`, and `\b` recognize Unicode text; inline flags such as `(?i)` work in friendly search, while backreferences, look-around, and programs above 1 MiB are refused.
- **Caret selection follows code editors with wrapping disabled** (og-R3A, master 587bdc431).
  The shared caret mirror copies shaping and wrapping styles instead of inventing wrapped rows.
- **Search failures can be retried in place** (og-R3A, master 371241774).
  The quick switcher offers Retry, and the block-reference picker distinguishes Searching, a failed
  read with Retry, and an empty answer while preserving the editor text.
- **PDF export stops at the Print query limit** (og-R3A, master e04b0e7f).
  A renderer-declared source, nesting or match limit now stops preparation with its reason instead
  of opening the print dialog over a placeholder.
- **A local query-table sort identifies itself as Table-only sort** (og-R3A, master a9c8b2596).
  Its label clears that local order without changing the saved query sort.

- **Percent-escaped Org file links resolve to their page names** (REG-OG-PAGE-FILENAME-001, og-B-TAIL). Reference extraction now uses the native filename codec through wasm, including escaped punctuation and namespace separators.
- **Saves keep reference counts and names live without full refetches** (og-B-SIG2): native save and watcher signals update changed count targets and refresh names only when their sources change; ordinary text edits avoid both graph-wide reads.
- **Published queries use their owning page** (og-B-SIG2): current-page queries, template substitutions, and query sheets share the baked publication context; identical queries on different pages keep separate static cache answers.
- **Saves reuse parser-owned formatting** (og-B-SIG2): old page content and serialization layout come from one parse, removing two redundant parses from ordinary saves.

- **Code examples do not set a page's saved indentation** (REG-OG-SIG-LAYOUT-001).
  Formatting detection now uses the parser's actual outline headers, so a
  bullet inside a literal code block cannot supply a false indentation unit.

- Failed journal scans report incomplete reads, and a page file whose name cannot be read is reported while the rest of the graph stays usable; unreadable graph config opens read-only, unreadable custom CSS reports its safe fallback, failed conflict refreshes keep their last inventory, and verification cancellation stays silent through a typed outcome.

- **Graph read failures preserve user data** (REG-OG-B-FAIL2-CONFIG-WRITE, REG-OG-B-FAIL2-ASSET-TRASH, REG-OG-B-FAIL2-VERIFICATION-REPLACE): an unreadable config blocks writes into guessed directories, asset trash rechecks published references under the writer, and graph verification detects a source replaced while its old descriptor is being hashed.
- Long flat TQL boolean queries no longer exhaust the stack; genuine nesting is bounded (REG-OG-B-DOOR2-TQL-FLAT-DEPTH).
- Org drawer and directive properties retain backlinks; page icons ignore prose and literal examples (REG-OG-B-DOOR2-ORG-PAGE-PROPERTIES, REG-OG-B-DOOR2-PAGE-ICON-OWNERSHIP).
- Org blocks documenting VCS conflict markers remain editable, and plain references keep decomposed Unicode accents with their letters (REG-OG-B-DOOR2-ORG-MARKER-LITERALS, REG-OG-B-DOOR2-NFD-REFERENCE-BOUNDARY).
- Fullwidth commas separate tag references consistently with Logseq (REG-OG-B-DOOR2-FRONT-TAG-SEPARATOR).
- **Saving with retained graph views avoids graph-wide copies** (REG-OG-B-COST-SNAPSHOT, REG-OG-B-COST-NAMES): page slots, timestamps, reference counts and name/signature/icon indexes share untouched tree branches. Save preservation checks also reuse the old parsed document (REG-OG-B-COST-PARSE).
- Published query snapshots retain reviewed source documents across external edits; draft loading and CSV/TSV imports bound their reads before decoding (REG-OG-B-W3-PUBLICATION-SNAPSHOT, REG-OG-B-W3-BOUNDED-DRAFT-READ, REG-OG-B-W3-BOUNDED-CSV-READ).
- Sheet row virtualization clears its visited-row state on graph reset (UI-OG-B-W3-SHEET-BINDING-RESET).

- Sheet formulas memoize shared references, reject inherited member names, and refuse excessive field-rename depth without crashing (UI-OG-B-W3-FORMULA-DAG, UI-OG-B-W3-FORMULA-OWN-MEMBERS, UI-OG-B-W3-FORMULA-RENAME-DEPTH).


- **Published permalinks share page and block identity** (UI-OG-PERMALINK-IDENTITY-001, UI-OG-PERMALINK-BLOCK-ID-001): equivalent Unicode page/alias names resolve, and Markdown/Org block links ignore code examples of IDs.
- **Expanded audio shares inline asset decoding** (UI-OG-AUDIO-ASSET-PATH-001), including normalized asset-directory case and separators. Native streams, cached images and media fallbacks use one Rust MIME answer through the existing wasm module.
- **Export retains literal metadata and Org body drawers** (UI-OG-EXPORT-LITERAL-METADATA-001): HTML, OPML and source Text strip canonical properties through the parser-owned region door.
- Fixed namespace and published-backend identity drift, published search highlights and block-ID lookup, and deeply nested namespace/snapshot handling (OG-B-FRONT).
- Fixed recursive embed exports, oversized scientific-zero output, rejected registry response cleanup, broad table rendering, and media labels containing a literal percent. Search excerpts and query-macro scans avoid repeated input work (OG-B-FRONT).

- **Exported grids show the same bounded rows their footer totals**, and count their full area against export limits (og B-SHEET).

- **Sheet cell operations preserve their own configuration**, query sheets keep the physical page already open, and field rename accepts Unicode property siblings (og B-SHEET).
- Kept-draft conflict resolution and publication remain with their original graph; saving retires a pending first-write crash-safe draft in order, and uninstall stops a plugin still starting.
- Asset versions, rendered block and board markers, readiness listeners, audio scrub gestures and updater handles retire with their owner. Query builder edits compose while print/parse work is pending, completed workspace searches release their cancellation lanes, parser comparisons stop on retirement, and visible PDF pages share a bounded tile budget.
- **Failed native reads keep their errors and recovery details** (OG-B-FAIL): plugin and asset inventories
  propagate disk failures, unreadable sessions cannot create blank workspaces, and incomplete transaction recovery
  remains visible through conflict resolution. Launch-backup failures appear in the graph window.

- **Failed reads stay visible** (OG-B-FAIL): page and reference refreshes keep their last successful data,
  journal-feed and session reads report failures, template creation refuses a failed name check or a read-only block,
  and the journal-template picker offers Retry. Quick Capture now shows block-action feedback with copyable errors.

- Org pages with long directive preambles open without reparsing every growing prefix (REG-OG-ORG-DIRECTIVE-OPEN-001). Page moves sync each affected directory once, and both journal navigation doors share the absent-path rule (og-B-STORE).

- Graph settings read their top-level EDN entries and decode authored string escapes consistently (REG-OG-CONFIG-ROOT-READ-001, REG-OG-CONFIG-EDN-ESCAPES-001).

- **Query memo limits include statistics and compiled patterns** (REG-OG-QUERY-MEMO-001): oversized entries are returned without caching, and program reservations count toward both entry and total memory budgets.

- **Large query property lists and macro blocks stay responsive** (REG-OG-QUERY-ATOMS-001, REG-OG-QUERY-MACROS-001): atom uniqueness uses one ordered set, and raw query macros are scanned once. TQL LIKE prefix shortcuts now share the matcher’s escaping rules, including `\a` (REG-OG-QUERY-LIKE-001); query printers share literal escaping and numeric spelling.

- **Faster graph startup and first search** (og-P1). Journal-first reads avoid ordinary-page title discovery,
  completed claimant discovery is reused by direct reads, and a search without page results skips page-property hydration.
  Startup graph loading begins in the background while the frontend starts; open failures still reach the Welcome error card.

- **Delayed actions retain their original targets and newer input** (OG-K3).
  Quick Capture refuses delivery after its destination graph changes and keeps
  scratch/title edits made while saving. Page-menu deletion retains its file
  target through confirmation, alias saves preserve active editor/IME drafts,
  and cancelled or unmounted image/video resizes cannot edit another graph.
- **Content survives disabled-query edits, Unicode Org conflict resolution, and raced page merges** (OG-K1).
  Editing another query operand preserves rejected conditions inside `off(...)` byte for byte.
  Keeping both Org block versions and renaming an Org reference tolerate non-ASCII prefixes.
  An unchanged transaction step checks its revision before a merge can retire the source page.
- **Edits preserve unrelated authored content** (OG-K2). HTML paste keeps blank code lines; query-builder edits keep untouched empty boolean groups; sidebar arrangement edits keep the Favorites page preamble; sheet Flatten keeps group notes and properties as rows.
- **A second file holding a page's name is never shown or written as that page** (master 7bd793bd0, og J1).
  When a duplicate journal day (left by sync or a date-format change) or a same-named page opened by path
  was open with unsaved input, the journals feed showed it as the requested day, so what was typed there
  saved to the wrong file; quick capture and "add to today" could append into it and report success; a
  sidebar item pinned to the other file stayed blank; and a PDF-notes refresh declined during an edit was
  dropped, leaving the notes stale. The feed now keeps its previous days (or, on first open, says why in place) and fills in once
  that input is finished; capture and carry go into today's real file, or, when the other file has unsaved
  input, refuse and name both files (capture keeps its text in the capture window); the sidebar says why and retries;
  the notes refresh applies once the edit ends.
- **The preview build never offers the released Tine as an update** (og J3). The updater and the
  Check for updates button ask the Tauri updater plugin what the `og-preview` release offers (its
  `releases/download/og-preview/latest.json`, the same manifest the installer uses), never `releases/latest`,
  whose higher version number would have offered to replace the preview with the shipped app. With no
  `og-preview` release, an unreachable or invalid manifest, or nothing newer, there is no toast and About says it
  could not check or that you are current. The manual releases link opens the `og-preview` page.
- **Plugin settings: an operation finishing no longer re-enables controls under a later one.** The Plugins
  tab's busy state is held per operation (also while the uninstall confirmation is open) and released only by
  its owner (master cdd0eda4b).
- **A `file:` link opens the file** (GH #444, master c817fb150, og I3). A link written
  `[Test](file://D:\test.txt)` (Logseq) or `[Test](<file:///D:\test.txt>)` (Obsidian) rendered
  as a link and did nothing; it now opens that file or folder in the application your system uses
  for it, on desktop. A file that is gone, a link that names no local file, and any other refused
  link now show an error message instead of doing nothing. Other schemes stay refused.
- **Clicking or dragging a code block's own scrollbar no longer starts editing it** (master
  8d404e9c1, og I3). Scrolling a wide code block by its native horizontal scrollbar entered edit
  mode on press.
- **Published exports ask for no refused writes; the workspace name field survives typing**
  (GH #549 siblings and GH #498, master 4aadb1407, og I3). A read-only published export no longer
  attempts the tag-table toggle query or the PDF view-position save it must refuse, and the
  workspace name field is no longer rebuilt on every keystroke, so an input-method composition
  is not dropped.
- **Unlinked References: the highlighted mention is the jump control** (GH #200, master
  552d988c9 and 54c00e2d6, og I3). Clicking the highlighted mention opens its page with that
  mention selected; the numbered jump row appears only for mentions beyond the excerpt, and Show
  full block marks every mention.
- **Faster large pages and lists** (master a2789cc0f, 0350c00b6, 023c986b0, ce9a796fb, b3fc9c813,
  8c495c1ce, og I3). The All Pages list labels its rows in one pass; each block derives its
  heading, task and macro state once and only enters the marker/priority chip groups when it has
  one; leaf blocks skip plugin thread-line subscriptions; indenting or outdenting a selection, and
  redoing or pasting cut blocks that keep their ids, do one pass of work instead of one per block.

- **Plugin calls survive a rename or a format change** (master cdd0eda4, og-I2).
  A plugin's ownership of the open graph followed the render epoch, which a page rename, a typography
  change or a journal-title-format change bumps, so a call in flight across one of those was dropped as
  if the graph had changed. It now follows the graph binding only; switching graph still retires it.
- **Creating a page from the Quick Switcher after a graph switch says so** (og-I2). Alt+Enter and the
  new-pane paths used to open nothing, or the wrong graph's page, when the graph changed under a
  create in flight; they now show "The graph changed before the page was created. Try again."
- **Restore can no longer be clicked twice while its confirmation is open** (og-I2).
- **Arrow-key pane selection steps across the seam between panes correctly** (master 91d102fb, og-I2):
  the candidate's near edge, not its centre, is tested against the current pane's boundary.
- **Search regular expressions accept `\d \w \s \b`** (master f02af3ef, og-I2). Only back-references
  `\1`–`\9` are refused. Note: the native search reads `\d \w \b` as Unicode classes and the frontend
  as ASCII, so a non-ASCII digit or letter can match on one and not the other.
- **The HTML export no longer prints a sheet's `tine.*` view settings as property chips** (og-I2).
- **The reference filter summary says filtering is pending while the descendant index loads** (og-I2):
  "Indexing N references… the filter applies when this finishes" instead of a count over an unfiltered list.
- **The graph switcher says when it could not load the other graphs, and offers Retry** (og-I2).
- **Diagnostics report watcher-batch timings, and `__tineWatcherLatency()` prints them in the devtools console** (og-I2).
- **The Guide names the left and right sidebar link areas, the embed root-bullet drag versus click, and the
  search-fold examples** (`か`/`が`, `и`/`й`, `क`/`कु`, `елка`/`ёлка`) (og-I2).

- **`tine doctor` reports unreadable pages and duplicate page names** (og I1f, GH #35, master e7af4db9c).
  It used to print OK and exit 0 while a page could not be read, or while two files claimed the same page name.
  It now lists each, prints the configured home page, and exits 1 when it finds a problem.
- **A live export opens on the graph's home page** (og I1f, GH #35). The published app, from the command line
  or the app, now starts on the configured home page when that page is exported, then Welcome to Tine, then the
  first page; `tine export live --home "Page name"` chooses one explicitly and refuses a page that is not exported.
- **Tine starts when its usual application-data folder cannot be written** (og I1a, master 8e1ea0bfd).
  On Linux, a root-owned or read-only `~/.local/share` made every launch crash before a window appeared.
  Tine now keeps settings and the web view's data in `~/.tine-data` (or another private writable
  folder) for that session and says so; if nothing is writable it prints one sentence explaining why.
- **A conflict banner goes away by itself once the file is back to what you loaded** (og I1c, master c68c0b6e7).
  Some editors and sync clients briefly remove a file while replacing it; Tine treated the gap as a
  conflict that only a click could clear, and held the edit unsaved behind it. If the file comes back
  matching what your editor started from, the banner clears and your edit saves normally; a file that
  comes back changed still raises the conflict.
- **A page with an unresolved save conflict can be opened on disk again** (og I1d, master 6f8531344, GH #490).
  "Open with default app" and "Show in folder" refused while a conflict was pending. Both now open the
  file as it stands on disk and say so; your unsaved changes stay in Tine until you resolve the conflict.
- **Carrying tasks no longer moves them into a second file for today** (og I1e).
  When two files existed for today (a duplicate day from sync or a changed journal date format) and the
  other one was open under today's name, carry moved the tasks into it and said "Carried", while
  today's journal in the feed never got them. Carry now stops and names both files.
- **Closing no longer offers to throw away a save that is still running** (og I1b, master fea3c314b).
  If writing your changes took more than four seconds — a slow or network drive is enough — the close
  asked whether to lose them. It now says it is still saving and waits up to another 26 seconds; it
  only asks once the save has failed or stalled, in words that say which.
- **Ctrl+F scrolls to the exact occurrence, not just its block** (GH #253, master
  46a5290a2, og H). In a block taller than the window, in-page find used to center
  only the block, leaving the match off-screen; it now centers the active occurrence
  in the pane, falling back to the block while it is being edited.
- **An HTML export of 0 pages says why** (GH #560, master 350efef1f, og H). With no
  `public:: true` page (and "Include every page" unticked), Settings → Graph →
  **Export HTML and read-only app…** now reports "only pages with `public:: true` are
  exported" instead of a bare "Exported 0 pages".
- **A phone toolbar keeps calendar, journals and theme on the bar** (GH #205,
  master a006f1308, og H). On a 390px phone they had moved into the "…" menu; they
  now collapse only below a 345px toolbar. A narrow desktop window, whose tab strip
  needs room, still collapses them at 460px and below a 440px window also moves
  Back/Forward into "…", so the tab title stays readable instead of shrinking to a
  couple of letters, and the "Show all tabs" button never covers the next button.
- **The parser comparison no longer offers known intentional differences as bugs**
  (master c0c2ff11b, og H). Dollar math inside Markdown emphasis (`**…$x$…**`),
  which Tine keeps on purpose, is re-verified in isolation and listed under a
  "known intentional parser difference(s) suppressed" count instead of a reportable
  divergence.
- **The "new sync conflicts need review" notice goes away once they are resolved**
  (master 042054c1b, og H). It used to stay beside the green "Merged" confirmation
  until dismissed by hand; it now closes when none of the conflicts it announced is
  left, whether resolved in Tine or cleared on disk.
- **A voice memo stopped from another block is kept** (master b3d64add, og H). On
  mobile, starting a recording in one block and tapping Stop while editing another
  used to discard the recording silently; it is now saved to `assets/` and Tine says
  it was not inserted. A failed stop or import is always reported. A recording or
  photo that finishes after you switched graphs is saved to the `assets/` of the graph
  it was started in, never the new one, and Tine names that graph and the file.
- **An edit typed while another graph is opening is kept** (og-T). It is saved
  as a draft of the graph you left and offered for review when you reopen it,
  instead of being dropped by the switch.
- **Editing a block keeps its blank and under-indented lines byte for byte**
  (og-T). A whitespace-only line inside the edited block was written empty, and
  a continuation line indented less than usual was re-indented.
- **A broken `:hidden` setting fails safe** (master, og-T). If `:hidden` in
  `config.edn` cannot be read, Tine shows no graph pages until it is fixed,
  rather than showing pages you meant to hide; entries with Unicode spaces at
  either edge are ignored.
- **A page's `title::` is read from the page's own preamble only** (og-T).
  Properties after a leading `# heading` belong to that heading block, so
  opening, renaming and linking agree on the page's name.
- **Android follows external file changes with native events by default**
  (master, og-T), like every other platform; polling stays a setting.
- **The conflict badge is always reachable, and closes the phone drawer** (og-F).
  The sidebar's footer (the conflict badge and New page) sat one header-height below
  the bottom of a short window, so a pending conflict could be invisible until you
  scrolled the sidebar. It now fits, and on a phone tapping the badge closes the
  navigation drawer so the Conflicts page opens in view instead of behind it.
- **A failing tag table or conflict comparison no longer blanks the page** (og-F).
  If reading them fails, the page shows a retryable error in that panel instead of
  throwing while it renders.
- **Query board and table settings survive and apply** (master P5A/P5B, og E).
  A board now groups by the query's own Display grouping (it ignored it before), and its Group by
  control changes that setting. A `tine.fields` list of column names on a query block is no longer
  mistaken for a typed schema: its columns are not marked stray, and declaring a schema moves the
  list to `tine.columns` in the same undo step. A saved query tab or window state with one malformed
  display setting keeps the tab and drops only that setting. "No grouping" on a query board now shows one column of every result
  instead of falling back to the task-state columns.

- **Red error messages stay until you close them, and offer Copy.** Every error
  is also noted in the privacy-safe diagnostic report (not its text) and, with
  `TINE_DEBUG=1`, written in full to the debug log (og OG-TOAST).
- **No false "couldn't finish checking for external changes" at launch.**
  Returning to the window before the graph finished opening (or on the Welcome
  screen) no longer reports the not-yet-open graph as a failure; the same holds
  for the session save and inline page icons (og OG-TOAST).
- **A panel whose data fails to load no longer blanks itself** (master c5279d186,
  og-D). When a background fetch fails, backlinks, the Quick Switcher, the
  calendar, Settings, the sidebar and the other panels that read one now keep
  what they had or show "Couldn’t load …" instead of throwing; the block
  references panel and Search add a Retry. Details go to the debug log only.
- `tine open GRAPH` while Tine is already running now opens that graph in a new
  window; the second launch used to be read as a page called "open" and opened the
  wrong folder (master e7af4db9c, og-D).
- **An edit made while a graph switch was already under way is no longer
  dropped** (og A). Tine saves once more right before it opens the other graph,
  and stays on the current graph if that edit cannot be saved.
- **Files with very long names can be imported, deleted and resolved.** An asset
  or page whose name is close to the 255-byte limit (an 84-character CJK title)
  could not be imported, re-imported under a numbered name, moved to trash,
  resolved after a VCS merge, or created: the helper files and trash copies Tine
  names after it were too long. Trash copies now shorten the name and keep its
  extension. Merging a page also no longer repeats an alias written after a
  full-width comma, and a drag that would nest the outline too deep now says so.
- **A page marked `public:: false` stays out of every export.** With
  `:publishing/all-pages-public? true` in `config.edn`, or **Include every page**
  ticked, Tine published such pages anyway; like Logseq it now leaves them out,
  and their blocks no longer reach another page's embeds, queries or references.
  A query's own **Export…** is unchanged.
- **An outside edit to `logseq/config.edn` now applies while the graph is open**
  (og 22b). A change made in Logseq, a text editor or delivered by a sync
  provider was taken in only after a rename or a reopen; the store watcher now
  reloads it in both notify and poll modes, only when its bytes changed.
- **A settings change edits only its own top-level entry of `config.edn`** and is
  refused, leaving the file untouched, when the file is half-written or not a map;
  a key nested inside another setting is never overwritten (og 22b).
- **Renaming the home page keeps it the home page**, as in Logseq; merging it
  into another page does not (og 22b).
- **Choosing "Keep mine" after a conflicted alias save no longer duplicates the
  alias's content on its owner page.** The retry replaces what already landed,
  and refuses rather than guessing if the owner changed meanwhile (og L13).

- **Hostile content cannot crash a table formula or navigate the app away** (I-22): formulas that nest past 128 levels, counting references, show an error in the cell instead of overflowing the stack, video and tweet macro links open in the system browser, and on Android and iOS a link with any other scheme (for example `intent:` or `javascript:`) no longer navigates the app.
- **A save that fails on the disk now says which step failed** (GH #538, #590): the message names the platform call (for example the no-replace rename, or syncing the temporary file) and its OS error number, so a report can be acted on. Diagnosis only; it does not make such a save succeed.
- **The vendored parser WASM matches a clean source rebuild again** (GH #392), so F-Droid's from-source build reproduces the shipped parser; CI now rebuilds it and fails on any difference.
- A page rename no longer rewrites links inside a file that still carries
  unresolved git or Fossil merge markers; the file keeps its bytes and a message
  lists it.
- Tine's Concord ledger now lives in its own app-data folder, so it no longer
  deletes, or is deleted by, the ledger of a mainline Tine build on the same machine.
- **Middle-clicking a link opens a background tab without switching panes** (GH #87): the click no longer makes the pane under it the active one, so the tab opens where you are working.
- **Ctrl/Cmd+click and Alt+click on an outline bullet** go where the same modifier on a link goes (GH #456): a background tab and the other pane. Bullets in linked references do the same.
- **A split pane's only tab keeps its close button** (GH #207), unless that tab shows the journals; closing it closes the pane.
- **A file change that arrives while you are editing that page is applied when you finish** (GH #337), instead of being dropped until the next change.
- **Block embeds follow their source's fold state live** (GH #360). Folding the embedded block inside one embed is remembered by that embed alone, and folding rows inside an embed yields to the next fold of the source.
- **Filter chips and groups in linked and unlinked references** treat differently-normalized spellings of one page name as the same page.
- **Saving a search or query as a page in an Org graph writes its view properties where Org reads them** (a `:PROPERTIES:` drawer, not a body line that was never read back), and a save the workspace has since moved on from is refused instead of publishing an obsolete search: if the write had already begun, the page is kept and the workspace stays put with a note.
- **Dragging a block embed by its bullet moves the embed itself** (GH #514), not the source block it shows. Rows nested inside the embed still drag as the source's own outline.
- **Arrow keys move between bullets inside linked references, block references, query results and embeds** (GH #341). Up/Down and Left/Right at a block edge now step through the blocks as rendered in that view instead of jumping the caret to the source page's outline, where the editor is usually not on screen; merges and indents there still act on the real page outline.
- **Code blocks**: the caret no longer jumps when you click into a code block or leave it with the arrow keys, the language line and fence markers are editable as text, and clicking a code card places the caret where you clicked (GH #489).
- **Property autocomplete** follows Logseq's keyboard flow (GH #306): typing `::` at the start of a line puts the caret before the delimiter so you type the name, "Create" is offered first, values already on the line are not suggested again in a comma-separated list, and a failed facet lookup no longer raises an error toast.
- **Context-menu submenus** open on the side that stays on screen near the window's right edge (GH #471).
- **Block text and bullet share one first line**, so the bullet, the reference-count badge and the first text line line up in the default and serif themes (GH #459, GH #454).
- **Default task styling**: done tasks are struck through and dimmed, and the checkbox colours follow the theme (GH #394).
- **Typing into a page in the right sidebar** no longer reseeds an empty trailing block over your text (GH #483).
- **Clicking into a block** starts editing on mouse-down, so the caret lands under the pointer even when a re-render follows (GH #368).
- **IME composition** keeps a block in its edit transaction when the composing input arrives without a `compositionstart`, so a composed character is not lost.
- **Android**: the on-screen keyboard no longer covers the block being edited (the app's bottom inset accounts for the IME).
- **The static HTML export shows a block's clocked time and the numbering of
  numbered blocks.** A block with LOGBOOK clock rows keeps its elapsed-time badge
  (the drawer itself stays hidden), and blocks marked as own-numbered lists show
  their `1.` / `a.` / `i.` markers as the app does.
- **Journal days and "Today" follow the calendar zone the backend uses**
  (GH #607). The frontend no longer trusts a WebView clock that can disagree
  with the system zone; it applies the measured backend offset, refreshed
  periodically and on focus.
- **Opening a page by a differently cased name lands on the spelling that
  exists on disk** (GH #597), and renaming an Org page keeps `file:` links to
  it valid under the graph's filename format.
- **Switching Settings sections no longer makes the whole dialog vanish and
  reappear**, and the maximized size is remembered across restarts (GH #409,
  GH #427).
- **The right sidebar's spare header width is no longer a link** and the left
  sidebar's whole row opens its page (GH #464, GH #468).
- **The fold arrow on a block is no longer partly covered by the fold-all guide**
  (GH #423), and **the left sidebar's scrollbar can be grabbed** with the resize
  strip beside it instead of over it (GH #435).
- **Application chrome no longer starts a text selection** while page content
  and editors stay selectable.

## [0.6.5] - 2026-07-22

### Added

- **AppImage builds now carry update information** (GH #222), so
  `appimageupdatetool` (and AppImageUpdate) can update Tine in place. Each
  release also publishes the matching `.zsync` file. Delta updates work from
  one release that has this metadata to the next, so the first usable update
  is the one after this release.

### Fixed

- **Large graphs no longer do reference and rename work on the UI thread**
  (GH #233, #235, #236). Page-title blur now commits the same conflict-safe
  rename as Enter, repeated commits are suppressed, and reference lookups use a
  reconstructible in-memory candidate index with exact verification and a safe
  full-scan fallback. Linked References use Logseq-compatible disclosure
  defaults; Unlinked References precompute in the background while remaining
  collapsed.
- **Opening Backups no longer hashes every saved snapshot** (GH #234). The list
  reads cheap manifest metadata in a blocking worker, while restoring still
  verifies the selected snapshot before changing graph files. The panel now
  exposes loading and retryable error states.
- **File-backed block rows now keep stable runtime identity** (GH #232) across cold loads, cache rebuilds, references, merges, and publish snapshots without adding synthetic `id::` properties.
- **IME composition now commits finalized block text once** (GH #237), avoiding
  intermediate graph writes and duplicate trailing-input commits.
- **Template date expressions now understand natural language** (GH #226), so
  `<% next monday %>`, `in 5 days`, and similar English expressions expand to
  journal links using the graph's configured journal-title format. Unknown
  placeholders remain intact.
- **Reference-only page names now contribute namespace descendants** (GH #229),
  so the namespace sidebar, `{{namespace}}`, and a page's Hierarchy section show
  linked child paths even when no child file exists. All Pages remains file-only.
- **Android 9 can load Tine's native library** (GH #192). The earlier fix
  covered the backup path but missed a second `renameat2` call in the trash
  path, so v0.6.4 could still fail before launch. Both paths now use the
  API-compatible syscall entry point, and release CI checks the final APK for
  this class of loader regression.
- **Ctrl+F no longer reselects the query after every typed character**
  (GH #224). The field still selects its contents when Ctrl+F is invoked again,
  but ordinary debounced search updates leave the caret and selection alone.
- **Task slash commands replace the block's task marker** (GH #225), rather
  than appending literal text such as `DONE` after an existing `TODO`. The
  shared marker operation preserves priority and block properties.

## [0.6.4] - 2026-07-21

### Added

- **Drag table columns into the order you want** (GH #217): grab a property
  column's header in a table view and drop it where it belongs. The order is
  stored with the sheet, so it survives a restart. Computed formula columns
  stay pinned at the end.
- **Cut & paste keeps block identity, and copy & paste is block-exact**, like
  Logseq: cutting blocks (Ctrl/Cmd+X or the context menu) and pasting them
  back now preserves their `id::`, so `((...))` block references and embeds
  pointing at them keep working. Pasting your own copied blocks inside Tine
  is now a true block-level paste — exact text, hidden properties like
  `collapsed::`, and the full subtree survive — while plain-text pastes into
  other apps are unchanged. Copy-paste never duplicates block ids, nothing
  about block identity is written to the OS clipboard, and pasting can never
  create two blocks with the same id.
- **Full calculator language in ` ```calc ` blocks**, like Logseq: hex/octal/
  binary numbers in and out (`:hex`, `:oct`, `:bin`, `:decimal`), scientific
  and mixed-number literals (`3 1/2`), inverse trig and factorial, output
  format directives (`:format fix/sci/normal/fractions/improper`), exact
  decimal arithmetic (`0.1 + 0.2 = 0.3`), and Logseq's error-to-`last`
  behavior. Tine's `floor`/`ceil`/`round` keep working as an extension.
- **Org image links render as images**, like Logseq: in Org pages, a page
  reference pointing at a local image asset (e.g. `[[../assets/pic.png]]`)
  now shows the image instead of a page link.
- **Direct Hiccup and raw media render**, like Logseq: a Hiccup form typed
  directly in a block (e.g. `[:span.highlight "text"]`) now renders as real
  (sanitized) HTML instead of literal source, and raw HTML `<audio controls>`
  / `<video controls>` play natively. Scripts, iframes, event handlers, and
  autoplay remain stripped; base64 `data:` images keep working, matching
  Logseq's sanitizer exactly.
- **Admonition icons and custom-block styling**, like Logseq: NOTE, TIP,
  IMPORTANT, CAUTION, WARNING, and PINNED admonitions now show a per-type
  icon, and other `#+BEGIN_X` blocks (like VERSE) keep a wrapper with their
  name as a CSS class so themes can style them.
- **Table v2**, like Logseq: a table in a block with
  `logseq.table.version:: 2` renders Logseq's grid presentation with compact
  mode and header transforms, and the `logseq.table.*` configuration
  properties no longer show as visible text.
- **YouTube timestamps**, like Logseq: `{{youtube-timestamp}}` stamps are now
  clickable and seek the embedded YouTube video, and a new "Embed Youtube
  timestamp" slash command inserts the current playback time. Works offline
  too — without a player the stamp stays a plain label.
- **`<` advanced commands**, like Logseq: typing `<` at the start of a line
  opens the advanced-section menu (Quote, Src, Query, Note, Tip, Warning,
  Example, Verse, Center, Export variants, Comment…); picking one inserts the
  paired `#+BEGIN_…/#+END_…` section with the caret ready to type — and on
  Markdown pages, Src inserts a ``` code fence, exactly as Logseq does.

### Fixed

- **On phones, a PDF can always be closed again.** The reader took over the
  screen with a toolbar that pushed its own Close button off the edge, and the
  Android back gesture ignored it — leaving no way out. The reader now fills
  the screen deliberately, Close sits at the front of the toolbar, the
  secondary tools move into the "⋯" menu when space is tight, and both the
  back gesture and Escape close the reader (any open find bar, settings or
  outline panel closes first).
- **Splitting a pane from the journals feed no longer opens a random other
  day's bullet.** Because a second journals pane isn't allowed, the new pane
  was filled with the last page you had visited in that tab — reusing its
  zoom and its pinned state, which is why an unrelated day's bullet could
  appear zoomed and pinned. It now opens the day page of the bullet you had
  selected, plainly.
- Editors re-measure their height when a pane split changes their width, so a
  multiline block clicked in a freshly created pane is no longer clipped.
- **Typing at the end of a block that ends with a `SCHEDULED:`/`DEADLINE:`
  line no longer breaks the date.** Clicking at the visual end of such a block
  puts the caret after the planning line, and the text you typed there used to
  turn the deadline into ordinary text. The date now keeps working as long as
  the timestamp starts its line — matching Logseq, which accepts the same
  shape — and your text stays exactly where you typed it; nothing rewrites the
  block. Re-picking a date from the calendar also keeps that trailing text
  instead of deleting it.
- A page containing a malformed HTML fragment (for example `- <div </div><`)
  no longer breaks the parser or disappears from search — it loads and stays
  searchable (GH #221, via the lsdoc 0.5.4 update).
- The `/Today` command now inserts the date in the journal date format your
  graph is configured with (`:journal/page-title-format`) instead of always
  `MMM do, yyyy`, so the link points at the actual journal page (GH #220).
- PDF annotation files written by Tine now match Logseq's exact field shape
  (text highlights omit the empty `:image` key; area highlights write
  Logseq's `"[:span]"` text sentinel), so a graph annotated in Tine looks
  byte-familiar to Logseq and vice versa. Deleting an area highlight now
  also moves its cropped image to the graph's recoverable trash (Logseq
  deletes it outright; Tine previously left it orphaned) — only after the
  annotation save fully commits, and never when another highlight still
  references the image.
- **Automatic headings**, like Logseq: a new "Auto" option (context menu and
  `/Heading (Auto)`) sizes the heading by the block's nesting depth and updates
  live on indent/outdent; explicit H1–H6 still win, and switching between the
  two cleans up the other representation in both Markdown and Org.
- **Paste parity with Logseq**: Ctrl/Cmd+Shift+V now pastes the literal
  clipboard text for every payload (not just multiline) — rich HTML and URLs
  included, with no formatting applied; formatted paste keeps HTML images
  (`![alt](src)`, org equivalent, unsafe `data:` URLs declined safely); and
  pasting a bare YouTube/Loom/Vimeo/Bilibili link inserts a playable
  `{{video}}` embed.
- **Numbered lists work like Logseq's**: typing `1. ` in an empty block turns
  it into a numbered-list block; Enter continues the numbering into the new
  sibling and Enter on an empty item stops the list; Backspace at the start
  removes just the numbering; and blocks dragged or pasted into a numbered
  list pick up the numbering automatically (already-numbered blocks keep
  theirs).
- **Document-mode Enter**, like Logseq: with Document mode on, plain Enter now
  inserts a line break and Shift+Enter creates a new block (the reverse of
  outline mode), with a config switch
  (`:shortcut/doc-mode-enter-for-new-block?`) to keep the outline mapping.
- **Logical outdenting**, like Logseq: a new Editor setting; when on, Shift+Tab
  moves a block out one level and leaves its following siblings where they are
  (Roam-style) instead of nesting them under it. Both settings live in the
  graph's `config.edn`, so they travel with the graph.
- **Undo/redo modes and context restore**, like Logseq: a new palette command
  "Toggle undo/redo mode" switches between the default Global history and
  Page-only history (undo/redo affects only the current page's latest change).
  Undo/redo now also restores where you were — the route/pane, right-sidebar
  state, and the editing cursor position — instead of just the text.
- **Property autocomplete**, like Logseq: typing `::` at the start of a line
  (or a typed key ending in `::`) opens a picker of the property names already
  used in your graph; picking one inserts the canonical `key:: ` and
  immediately offers that property's known values. `::` inside ordinary prose,
  references, or code fences never triggers it.
- The Copy/export modal gained Logseq's **maximum-depth control** (`Level ≤`
  all/1..9): descendants deeper than the selected level are omitted from the
  exported text, in both Rendered and Source modes.
- **OPML and HTML export**, like Logseq: the Copy/export modal now offers
  Text/OPML/HTML for a block selection, and the **page menu gained
  "Copy / export as…"** opening the same dialog for the whole page. Each format
  shows Logseq's option set (cleanup + max depth for OPML/HTML; the full set
  for Text); OPML/HTML always omit property lines.

### Changed

- **The back and forward buttons now stay on the top bar on phones.** The bar
  used to move them into the "…" menu at the same width as the calendar,
  journals, theme and right-sidebar buttons; navigation now keeps its place and
  only collapses on extremely narrow windows (GH #205).
- **Multiline plain-text paste now matches Logseq**: pasted text only becomes
  multiple blocks when it looks like an outline (`-`/`+`/`*`/`#` lines, org
  stars) or contains blank-line-separated paragraphs; ordinary prose —
  addresses, log excerpts, indented text — stays in one block, replacing the
  selection literally. (Previously every multiline paste was split into
  blocks.)
- Parser updated to **lsdoc v0.5.4**: correctness, panic, and performance
  fixes from the GH #209 audit (split-title math chains, latex-env tails,
  raw-HTML scanning, 32-bit cookie/timestamp bounds, refs-indexing parity).

### Fixed

- **Property keys now create backlinks**, like Logseq: a page or block with
  `author:: something` shows up in the `author` page's Linked References even
  when the value contains no `[[reference]]`. Built-in/hidden properties and
  the `:property-pages/excludelist` are exempt, and
  `:property-pages/enabled? false` turns it off — matching Logseq's rules.
- **Exported HTML sites now run `#+BEGIN_QUERY` blocks** the same way they
  already run `{{query}}` — showing the authored title and results, restricted
  to public pages; when matches on non-public pages were filtered out, the
  query notes how many were omitted instead of silently under-reporting.
- **Custom macros with rich output now render it**, like Logseq: a `:macros`
  entry whose body is a Hiccup form (e.g. `[:span {:class "x"} "text"]`) shows
  the styled result instead of the literal bracket source — on the page, in
  linked references, and in reference previews. Output goes through the same
  HTML sanitizer as pasted raw HTML; Hiccup typed directly in a note (outside a
  macro) still renders literally.
- **`#+BEGIN_QUERY` advanced query blocks now render** their authored title and
  a bounded result table — on the page, in Linked References, and in reference
  previews — instead of showing the raw `#+BEGIN_QUERY … #+END_QUERY` source,
  like Logseq. The common page-property Datalog form
  (`[?p :block/properties ?props]` + `[(get ?props :key)]`) is understood;
  malformed or unsupported queries show a clear "Unsupported BEGIN_QUERY"
  notice rather than a guessed partial result.
- **YouTube (and Vimeo) embeds now play** instead of failing with the player's
  "error 153". A `{{youtube …}}`/`{{video …}}` embed — and a pasted raw
  `<iframe>` pointing at a video host — now sends the app origin as its referrer
  (`referrerpolicy="strict-origin-when-cross-origin"`) and the standard media
  `allow` permissions, matching Logseq; YouTube rejects an embed that arrives
  with no referrer. Raw `<iframe>`s to any other host keep `no-referrer`, so an
  arbitrary embed still can't see where it was opened from.
- A bare remote media URL now **renders inline**, like Logseq: a plain
  `https://…/photo.jpg` (or `.png`/`.gif`/`.webp`/…) shows the image, and a bare
  `.mp4`/`.webm`/`.mp3`/… URL shows a video/audio player, instead of a raw link.
  Labeled links (`[text](…)`) and non-media links are unchanged.
- `[[`, `#`, and `((` autocomplete now match what you type **literally**, like
  Logseq. Previously the in-progress text was run through the Ctrl+K search query
  language, so typing a bare `OR`, a leading `-`, a quote, or a `/…/` was read as
  a boolean/negation/regex operator and made valid pages or blocks silently
  disappear (the "works at 1 char, vanishes at 2, returns at 3" effect). Ctrl+K
  keeps its full query language (GH #186).
- Made equal-ranked page autocomplete results **deterministic**: ties now break
  on a stable key (file path / canonical name) instead of filesystem/enumeration
  order, so the same query on the same graph always returns the same set and
  order (GH #186).
- On Windows, clicking an external `http(s)`/`mailto` link now opens your default
  browser/mail client instead of a File Explorer window. The opener no longer
  hands the URL to `explorer.exe` (which treats it as a shell item); it uses the
  system URL handler, the same path already used on mobile (GH #215).
- The top bar no longer collapses its calendar/journals/theme/right-sidebar and
  history actions into the "…" overflow menu while there is plenty of room for
  them; the collapse threshold now reflects the actual space the buttons need
  (GH #205 follow-up).
- The top bar "…" overflow menu now closes when you click anywhere outside it
  (GH #205 follow-up).
- Editing a cell in a field table (`/table`) no longer reorders the columns.
  Updating a block property now keeps the property on its original line instead
  of moving it to the end, so the edited column stays put and the saved file
  keeps a stable property order (GH #216).

## [0.6.3] - 2026-07-20

### Fixed

- Ctrl+K search now tells you when more matches exist beyond its bounded result
  window and prompts you to narrow the query, instead of presenting a truncated
  list as if it were complete (GH #209).
- Page aliases are now scoped to the physical file that declares them, so an
  exact alias search cannot return a same-named sibling file (GH #209).
- One page containing unsupported search-index syntax can no longer make its
  whole cache worker shard disappear silently; other pages remain searchable and
  the skipped page is reported as an indexing failure (GH #209).
- The mobile/narrow top bar no longer pushes action buttons off-screen: the
  workspace switcher now lives in the left-sidebar header (with a compact one-tap
  fallback in the toolbar when the sidebar is closed), and lower-priority toolbar
  actions collapse into a "…" overflow menu when the bar is narrow — driven by the
  toolbar's real width (zoom/DPI-aware), so narrow desktop windows adapt too
  (GH #205).
- Page-property backlinks (e.g. `tags:: blah`) now render their linkified value in
  Linked References through the same renderer used in-page, instead of showing raw
  `key:: value` markdown (GH #212).
- Recursive and deeply nested block/page embeds and queries now stop with an
  "Embed depth is too deep" notice (OG's depth-5 guard) instead of expanding
  forever; a page can no longer embed itself (GH #206).
- Pressing Enter while typing page-header properties no longer shows a validation
  error or jams later saves for that page; the transient trailing newline remains
  in the editor while the persisted header stays canonical (GH #210).

## [0.6.2] - 2026-07-19

### Added

- Named workspaces can save and switch the complete window context in place,
  persist per graph across restarts, and leave graph files untouched (GH #104).
- Show brackets around page references can now be toggled in Appearance settings
  or with `mod+c mod+b`, with the OG-compatible value saved to the graph's
  `logseq/config.edn`.

### Changed

- Linked/Unlinked/query reference groups prune their per-group collapse state
  only when the result set actually changes, instead of re-walking the entire
  reference subtree on every structural edit anywhere inside it. Collapse
  behavior is unchanged; large reference sections do less bookkeeping work while
  you edit (GH #185).

### Fixed

- Property names now fold to lowercase with spaces and underscores shown and
  matched as `-`, matching Logseq while preserving the file's original bytes.
- `#+BEGIN_QUOTE`/`#+BEGIN_EXAMPLE` (and other org container) blocks whose body contains a `- ` list were split into separate blocks — corrupting the block structure on save and leaking the raw delimiters in rendering; they now stay one block.
- Pages whose page-header properties came from an older version (e.g. a page that
  is only `title::`/`tags::` metadata) no longer get stuck with a repeating
  "Couldn't save … — will retry. (refusing to drop an existing page preamble
  while authoring page-header properties)" error. The data-preservation firewall
  was misfiring on the legitimate save of such a page once its properties had been
  canonicalized to disk; the page now saves normally with no change to the file on
  disk. Restarting is no longer needed to clear the error (GH #198).
- The page-bottom "+ Add block" target now always adds a new writable block, even
  when the page already ends in an empty bullet. Previously it re-focused the
  existing trailing empty block, so if that block was indented you could never get
  a fresh unindented block below it and clicking appeared to do nothing. Stacking
  empty last blocks is now allowed (GH #158).
- Editing inside an existing `[[page]]` or `((block))` reference — for example
  inserting a word in front of the current text — and accepting a completion now
  rewrites the whole reference instead of leaving a stray `]]`/`))`, matching
  Logseq (GH #199).
- Linked References no longer show a redundant "1 mention" label and jump button
  on a block that mentions the page only once; the occurrence count and
  jump-to-occurrence controls now appear only when a block mentions the page more
  than once, matching Logseq (GH #200).
- Clicking a Markdown external link on Linux now opens the browser with a
  browser-appropriate environment instead of failing with a KIOExec file error;
  only bundle/loader environment variables are scrubbed, desktop/session state is
  preserved (GH #195).
- Android versions below 11 (API < 30) no longer crash on launch with an
  `UnsatisfiedLinkError` for `renameat2` (GH #192).
- Linked and Unlinked References now match Logseq: Unicode-canonical (NFC) page
  and alias identity, plain (unbracketed) property text counted as an unlinked
  mention, same-named pages from different folders merged into one reference
  group, and a real page title no longer shadowed by another page's alias
  (GH #137).
- Reference panels now show result truncation ("showing N of M") and a bounded
  error state instead of an empty panel when limits are exceeded (GH #137).
- Linked and Unlinked References now use the complete transitive, bidirectional
  alias component, including every owner of a duplicate alias (GH #137).
- Per-block reference labels now report the true mention total while keeping
  the occurrence jump-target list bounded (GH #137).
- Autocomplete and Ctrl+K search now rank all matching blocks globally before
  applying result caps, so a strong block match is no longer omitted because of
  where the block sits in the graph; inline and Ctrl+K pools match Logseq's
  sizes, and autocomplete ordering uses the same Unicode (NFC) identity as the
  rest of search (GH #186).
- Settings now scroll on narrow and mobile viewports, so lower settings are
  reachable instead of being clipped by the modal.
- The mobile sidebar drawer can now be closed by swiping it toward its edge, in
  addition to the existing back gesture/button and close button.

## [0.6.1] - 2026-07-18

### Fixed

- Mouse side buttons (back/forward, also called buttons 4/5) now navigate page
  history.
- Copy-pasting block text that contains square brackets no longer backslash-
  escapes them (`[ref]` stays `[ref]` instead of becoming `\[ref\]`).
- Deleting a block selection, and returning to a pane from pane-selection, now
  keep a block selected (and keyboard-navigable) instead of clearing it.

## [0.6.0] - 2026-07-17

### Added

- Begin an experimental Tine-native plugin platform: capability-limited WebAssembly
  guests, host-owned contribution points, explicit desktop/mobile declarations, and
  a public-registry safety model. This is not Logseq or Obsidian API compatibility.
- Add disabled-by-default local and signed-community installation, explicit
  capability review and enable/disable controls, registry revocations, immutable
  manifest/WASM/report digests, expandable safety findings, automatic-versus-manual
  publication labels, and desktop/mobile plugin catalogue layouts.
- Publish the first AI-primary examples (bullet threading, query-filter shortcuts,
  and a Logseq heading-shortcut behavioral port), a Rust guest SDK/template,
  deterministic package checker, and developer/security documentation.
- Add a credential-separated local registry auditor: rootless hostile builds,
  no-tools Codex source review, quarantine/manual approval, signed catalogue
  publishing, and symlink/path/digest fail-closed checks.
- Add per-version plugin uninstall controls that remove only app-local packages
  and clear plugin settings after the last installed version.
- Add plugin API 0.2 declarative settings: bounded host-rendered controls,
  device-local validated persistence, live settings-change events, reset behavior,
  nested plugin detail pages, and immutable behavioral-port provenance.
- Add a separate theme API 0.1 with inert token-only packages, strict literal-color
  validation, local and signed-catalogue installation, immutable provenance, and
  Appearance-owned selection that remains subordinate to graph `custom.css`.
- Add a machine-checkable port-gap report and a current popular-plugin compatibility
  matrix so AI-assisted ports must distinguish faithful subsets, reusable host API
  requests, core features, and inherently privileged integrations.
- Let host-rendered decoration plugins respond live to their declarative settings,
  and let command plugins declare ordinary remappable default shortcuts without
  receiving keyboard, DOM, or global-input authority.
- **Search can now stay on the current page or send a result to the sidebar.**
  Ctrl/Cmd-Shift-K searches only blocks owned by the focused routed page,
  including collapsed descendants, while Shift-Enter in Ctrl-K opens a page or
  block result in the right sidebar without navigating away.
- **Page titles now expose a discoverable, accessible actions menu.** The
  ellipsis opens the same file, navigation, copy, export, properties, rename,
  carry, and delete actions as title right-click; keyboard navigation, touch
  geometry, and focus restoration are built in. This is the bounded first page
  menu phase of GH #182.
- **Children-backed Sheet fields can now be renamed in place.** Right-click a
  column header or double-click its name to update the local schema and its
  dependent filter, grouping, aggregate, and formula configuration as one
  undoable, persistence-safe edit. Ambiguous or colliding renames are rejected.
  (GH #175)
- **Linked References can now be filtered without loading complete subtrees.**
  The panel combines bounded content search with page, tag, property, and task
  facets while preserving reference counts and lazy result expansion. (GH #173)

### Changed

- **Broad CI now runs once for a frozen release candidate instead of after each
  merge.** Pull requests retain a lightweight Linux validation path, while
  Windows, Android, performance, UI E2E, and Flatpak proofs remain manually
  dispatchable between releases. Release packaging fails closed unless all full
  CI jobs succeeded on the exact candidate commit.
- **F-Droid builds now meet the store's no-runtime-code-download policy.** They
  omit the network-backed community plugin and theme catalogue. The
  capability-limited plugin host, local plugin/theme package installation,
  already-installed plugins, and built-in themes remain available; other Tine
  distribution builds retain the signed community catalogue.

### Fixed

- **Explicitly selected pages and blocks now keep their exact physical file
  owner through follow-on actions.** Ctrl-K, page titles, block zoom, sidebar,
  tabs, Recent, friendly query results, menus, and session restore preserve the
  selected graph-relative path. Stale or ambiguous rename/delete targets fail
  closed instead of touching a same-name sibling, while older logical/pathless
  links, Favorites, and sessions remain compatible.
- **Signed plugin registry cache updates are now failure-atomic and revocations
  remain durable.** The verified index and signature share one native envelope,
  legacy split keys migrate through a guarded transaction, torn or unreadable
  cache state holds guest activation, and cached/live revocations clear the
  installed enable bit before any guest bytes or runtime can be used.
- **Delayed plugin results now stay with the graph and editor that invoked them.**
  Switching or refreshing a graph while a command or slash completion is pending
  drops the stale result, even when the new graph contains the same block UUID
  and text, without disabling the healthy plugin worker.
- **Plugin launch verification now works from a standalone Tine checkout.**
  Documentation launchers use the checkout's own Vite and bundled community
  plugins instead of depending on an untracked sibling development repository.
- **Cached signed plugin and theme revocations now take effect before startup
  activation.** A stalled catalogue refresh is abort-bounded, one broken plugin
  no longer blocks the rest, and a newer verified revocation immediately stops
  an affected active plugin without restoring older cached state afterward.
- **Pending PDF work now stays with the graph that owns it during graph changes.**
  In-place graph switches and safe close drain the old graph's PDF work first,
  stale callbacks cannot write into the new graph, and drain failures abort the
  transition with the old graph still open.
- **PDF area selection now follows Logseq's platform gesture and confirmation
  flow.** Shift-drag on Linux and Windows, or Command-drag on macOS, must exceed
  10 pixels in both dimensions and opens the color chooser before anything is
  written; dismissing the chooser leaves the graph unchanged.
- **The PDF reader now has persistent Light, Warm, and Dark themes plus document
  outlines.** Nested outline entries expand independently and navigate through
  both named and explicit PDF destinations, while theme preference remains
  application-local rather than entering graph or annotation files.
- **Search now treats canonically equivalent Unicode spellings as identical.**
  Composed and decomposed page names, aliases, and block text share membership,
  exact-page detection, ranking identity, and source-accurate highlights without
  adding accent folding or transliteration.
- **Markdown page-header properties are now directly editable and stay
  unbulleted on disk.** Clicking an existing header, or crossing into it with
  the arrow keys, uses the ordinary block editor; newly authored custom and
  Unicode properties reopen as canonical Logseq page metadata without changing
  body blocks or unsafe preambles. (GH #163)
- **Linked References and list-query results now keep deep matches concise and
  understandable.** Each hit shows its final ancestor context, while deeper
  descendant branches start folded in a view-local copy that never changes the
  source block's collapse state.
- **Mixed-case page links now open the existing canonical page.** Wiki links,
  tags, aliases, tabs, and sidebar navigation share the same case-insensitive
  page identity instead of opening a blank, non-editable case variant. (GH #179)
- **Bare `tags`, `alias`, and `aliases` property values now create Linked
  References.** Page and block properties use the same canonical reference
  evidence as wrapped page links and hashtags, including after an in-place edit.
  (GH #180)
- **Selection formatting no longer wraps selected outer spaces.** Bold, italic,
  strike, and highlight actions keep leading and trailing selected whitespace
  outside their Markdown or Org delimiters, whether invoked from the keyboard
  or toolbar. (GH #178)
- **Nested WebView scroll regions no longer overscroll the Tine window.** Scroll
  gestures stop at the viewport boundary while panes, sidebars, and drawers
  retain their own scrolling. (GH #177)
- **Every foreground page activation now updates graph-global Recent pages.**
  Opening or focusing a page through the main pane, split panes, sidebar, or
  history uses the same RECENT ordering instead of tracking only some routes.
  (GH #170)
- **Simple queries now match Logseq's membership and journal-date semantics.**
  Bare page references include inherited page membership, date bounds are
  inclusive and order-independent, and Search preserves the same result
  identities as List, Table, and Board for supported simple queries.
- **Line-leading inline code containing `::` remains visible code.** It is no
  longer misclassified as a property drawer, while actual properties and
  references outside the code span keep their existing behavior.
- **Escape and Android Back now close every visible popup before the surface
  beneath it.** Calendar Jump, selection formatting overflow, PDF Find and
  highlight actions, QueryBuilder menus, and formula value pickers all join the
  shared one-gesture/one-layer dismissal order without losing selections,
  drafts, or reader state. (post-GH #161 follow-up)
- **Tab close buttons work on Windows again.** The visible X keeps its native
  pointer action instead of handing the pointer to the parent tab-drag capture
  session, while ordinary tab activation and drag-to-reorder stay unchanged.
  (GH #174)
- **Table cell values now commit before Tab advances to the next cell.** Typing
  the next value no longer overtypes the cell that was just saved, and formula
  columns react to the preserved inputs as expected. (GH #176)

## [0.5.10] - 2026-07-16

### Added

- **At viewport widths below 640 px, sidebars now behave as modal drawers.**
  They overlay instead of squeezing the page, isolate background controls, and
  dismiss safely via the scrim, Escape, or Android Back while restoring focus.
  At 640 px and wider, including tablets, persistent sidebar and split-pane
  behavior is unchanged. (GH #161)

### Fixed

- **Split-pane Back and Forward stay with the pane you focused.** Clicking the
  global navigation toolbar no longer retargets history to the main pane before
  the action runs; pane-targeted Search and Journals controls preserve the same
  focused-router contract. (GH #170)
- **Existing PDF highlights now expose their reference workflow.** On desktop,
  text and area highlights offer **Copy ref** and **Linked references** from the
  same click or right-click menu; both actions safely ensure the annotation
  block before copying or opening it with its ordinary referrers visible.
  (GH #168)
- **Search tabs can now be opened before entering a search.** Empty virtual
  search tabs focus their own input and remain independent until a valid search
  is explicitly named and saved. (GH #172)

- **Future-dated journals no longer displace today from the Journals feed.** They
  remain intact and directly reachable through search, links, the calendar, and
  All pages. (GH #171)

- **Mobile disclosure controls stay separate from bullets without stealing text
  taps.** Foldable blocks keep a wide trailing touch target on narrow Android
  layouts, while leaf blocks no longer retain an invisible right-edge disclosure
  hit area. Nested outlines, headings, live embeds, and sidebar rows share the
  same touch-geometry regression. (GH #159)
- **Bare `/` now defaults to Page reference.** `/` then Enter, Tab, or pointer
  selection inserts `[[]]`, leaves the caret inside it, and continues directly
  into page completion without changing typed slash-command ranking. (GH #155)
- **Page and tag completion now use OG's adaptive default.** Exact pages remain
  exact; strict-prefix candidates lead deterministically with Create immediately
  after the leading match, while fuzzy-only matches leave Create first. Advanced
  Settings also offer explicit existing-first and typed-first policies. Rapidly
  accepting a visible result now replaces the complete current trigger, and a
  slower older lookup cannot overwrite results for newer input.
- **Mod-L now inserts a format-aware external link.** Markdown and Org handle
  empty text, selected labels, and selected parser-recognized links/references
  through the same command, toolbar, and simple slash-Link boundary.
- **Native form fields now retain Tab and Shift+Tab focus traversal, including
  their blur commits, while outline and Sheet-cell editors keep their
  application-owned indentation, autocomplete, and cell-navigation behavior.**
  (GH #157)
- **The page-bottom Add block target now opens one focused, writable editor in
  the originating pane.** It reuses only a rendered empty structural leaf;
  collapsed and opaque Sheet storage tails create at the normal page or zoom
  boundary instead of selecting an unmounted descendant. (GH #158)
- **Bare hashtag autocomplete stays open for Unicode IME input.** CJK, Kana,
  Hangul, Thai, accented, emoji, and namespaced tag prefixes now use the same
  hard-stop contract as the parser instead of JavaScript's ASCII-only word
  class, while punctuation and embedded-hash boundaries still close the picker.
  (GH #167)
- **Static publication is now a closed capability boundary.** Ambiguous
  public/private source identities fail closed, generated anchors are escaped
  separately for HTML attributes and URL fragments, ordinary links and media
  macros share a safe-scheme policy, and the site CSP no longer permits inline
  script handlers.
- **PDF resources, highlight navigation, and Find have the right lifetimes.**
  Changing assets tears down the old viewer before mounting the new identity,
  including delayed state writes and late pdf.js loads; references into the
  already-open asset keep it mounted and scroll to the exact highlight rather
  than only its page, while a targetless direct reopen preserves the current
  reading location, with both Markdown and Org annotation-page metadata.
  Find retains a bounded text LRU, caps page text and occurrences, and drops
  cancelled work. (GH #169)
- **Graph-open background work and result construction have hard ceilings.** A
  replaced graph binding cancels warm-cache and backup work between files,
  process-wide permits prevent I/O amplification, failed `.partial-*` backups
  are removed, and queries, references, facets, block resolution, publishing,
  and query export enforce row/byte limits while constructing—not after cloning
  a complete result. Reference occurrence evidence is capped while scanning,
  all live bounded result families retain warm caches across unrelated edits
  (including pages with unchanged aliases), semantic alias transitions still
  invalidate them, and overflow metadata is never retained across an unknowable
  negative transition. Persisted simple and advanced query sources fail closed
  at shared byte and nesting ceilings before parser recursion or cache-key
  construction, including static publication's now-bounded query memo.
  Unlinked-reference edges follow Logseq's ASCII boundary rule.
- **Clipboard image paste validates dimensions before decoding RGBA.** Pixel,
  raw-buffer, PNG, frontend IPC, and native base64 limits now form one bounded
  ingress path, avoiding several simultaneous unbounded image copies.
- **PDF export now bounds image bytes before crossing the native/WebView
  boundary.** Each image has a 12 MiB ceiling and one export shares a 32 MiB
  source-byte budget; missing, remote, oversized, and over-budget images become
  inert omission markers instead of being read, base64-expanded, copied through
  IPC, and materialized in the print DOM without a limit.
- **Long high-zoom PDF sessions have a real memory ceiling.** Canvas admission
  now uses aggregate backing-store pixels (with a lower mobile budget) instead
  of retaining up to 24 maximum-size pages, evicts before allocating, and zeroes
  each canvas before removal so WebKit releases its bitmap promptly.
- **Help improve Tine now fails closed when a parser reproduction cannot be
  irreversibly anonymized.** The reversible fallback was removed, non-ASCII
  content and custom Org identifiers are always scrubbed, only fixed public
  grammar tokens may survive, and the UI no longer makes an absolute sharing
  guarantee.
- **PDF export documents no longer inherit Tine's native privileges.** Math and
  code highlighting are rendered from bundled libraries before printing; the
  resulting document is script-free, carries a restrictive content-security
  policy, and runs in a sandbox without script permission instead of loading
  executable code from a CDN inside the app origin.
- **Nested query, reference, and block-resolution results no longer amplify
  overlapping subtrees quadratically or omit valid nested occurrences.** Query
  shaping now transcribes Logseq's actual rule—suppress a match only when its
  immediate parent also matched—while reference panels retain every independently
  countable occurrence. All native result rows stay shallow; hover previews are
  bounded by nodes and bytes before transport, and all query macros in one
  Copy/Export session are hydrated natively under one shared root/node/byte
  budget without transferring their complete source pages to the WebView.
- **The release performance gate now rejects noisy measurements instead of
  changing its verdict on retry.** Candidate, v0.4.7, and the previous release
  run in three order-rotated rounds; decisions use the median round result, keep
  every sample as evidence, and fail reliability when an individual metric's
  cross-round spread exceeds its declared limit.
- **Backup restore stays inside the selected graph under symlink and directory
  races.** Recovery areas and live-file publication are now bound to opened
  directory capabilities, use create-without-replace semantics, and refuse a
  replaced ancestor instead of following it outside the graph or approved
  assets root.
- **Android photo capture and picking are memory-bounded.** Camera and picker
  results are checked for byte and pixel limits, streamed through a native cache
  token, and then streamed into the graph without whole-file or base64 copies
  across the Kotlin/WebView/Rust bridge.
- **Static publishing now treats the public page set as a hard privacy
  boundary.** Queries, page/block embeds, and namespace macros cannot expand
  private content; each export is assembled in a guarded staging tree and then
  swapped as one unit through bound directory capabilities, so formerly public
  pages disappear and concurrent staging, recovery, or `publish/` symlink and
  junction swaps cannot redirect generated writes outside the graph. The
  previous output remains in Tine's recoverable conflict trash.
- **Voice memos have one bounded, reachable recorder.** Desktop recording is
  process-owned, cancels when its editor disappears, rejects concurrent starts,
  and stops at 30 minutes or 32 MiB; Android applies the same duration/size
  ceilings and streams the native temp directly into the graph instead of
  multiplying a valid recording through Kotlin, JavaScript, and Rust base64
  buffers. Failed native setup also releases its recorder and temp file.
- **Android long-press text selection keeps the native selection UI.** Tine no
  longer intercepts textual `contextmenu` gestures with desktop menus, including
  page links, block references, reference panels, namespaces, embeds, and query
  results; the bullet remains the explicit mobile block-action target.
  (GH #162)
- **Fresh block references use their durable identity immediately.** Copy block
  ref now gives a new target one UUID shared by counts, referrer panels, routes,
  and the sidebar before reload, with Markdown and Org persistence. (GH #154)
- **Inline block-reference text follows every landed source transaction.** Loaded
  targets update immediately through their reactive editor node; visible UUIDs
  whose source was never loaded are batch-refreshed after external edits and
  become missing after deletion, without graph-wide work on each keystroke.
  Block embeds, previews, referrer panels, and count badges share the revision
  invalidation contract. (GH #166)
- **Page-property settings preserve the literal page-header structure.** New
  properties follow Logseq's prepend behavior, updates stay in place, and the
  real UI-to-disk round trip preserves CRLF, blank separators, and all unrelated
  lines. The guarded native writer rejects even a forced save if an existing
  header property has been reclassified as outline content. (GH #163)
- **Large Search result sets remain inside persistent and inline query panes.**
  The full workspace/grid/item chain can shrink around long unbroken content,
  including the Filters/Advanced path with hundreds of page hits. (GH #140)
- **Help-with-Tine anonymization now preserves the structural identity of a
  parser divergence.** A safe scrub tier is accepted only when it retains the
  original mismatch paths and classes; a different surviving mismatch is not
  treated as the same report. (GH #82)
- **Ctrl+K now includes favorites in its bounded adaptive tie-breaking.** A
  favorite can rank first only within the same objective relevance class, just
  like local selection history; neither signal can promote a weaker match over
  an exact or prefix result. (GH #143)
- **Graph writes are safer under sync and filesystem races.** New pages, PDF
  artifacts, and demo files use no-replace publication when no baseline exists;
  PDF highlight sidecars are restored or quarantined if their paired annotation
  page fails; config creation merges rather than overwrites a concurrent creator;
  rename rollback and Copy Guide withdrawal preserve files replaced during their
  final syscall race; and Copy Guide rechecks page and asset containment at write
  time.
- **Settled edits avoid two graph-sized background costs.** Tine's own atomic-save
  temp events stay on the incremental watcher path and are scoped to their owning
  graph, while edits that do not alter block references reuse the existing badge
  count index; any necessary rebuild now runs off the command thread.
- **Broken audio and MKV fallback is memory-bounded.** Inline and expanded-player
  fallbacks share one process-wide budget, cancel and release work when closed,
  use lower size ceilings, and avoid a redundant JavaScript copy. Expanded audio
  now keeps a streaming scrubber instead of fetching and decoding the entire
  track into potentially gigabytes of PCM; normal media remains range-streamed
  and larger files retain the external-player escape hatch.
- **Plasma Wayland task switchers now resolve Tine's icon for standalone
  binaries.** Tine replaces GTK's executable-name fallback only after the
  Wayland top-level exists, while retaining the compatible post-map update for
  older GTK 3.24 runtimes; the advertised ID now matches the installed desktop
  entry before the first visible buffer.
- **Linux Quick Capture secondary launches no longer risk an Xlib/XCB abort.**
  Xlib's process-wide thread mode is initialized before GTK or Tauri, so the
  short-lived global-shortcut forwarder can hand off safely while the primary
  app is active.

## [0.5.9] - 2026-07-14

### Added

- **Linked and unlinked references now share exact source evidence.** Each
  matching block carries parser-owned explicit or plain occurrences, so a block
  with both kinds appears correctly in both panels, code and syntax boundaries
  stay consistent, and target-scoped diagnostics explain the same engine rather
  than running a second matcher. (GH #137)
- **Large reference panels now show bounded, highlighted excerpts.** Several
  mentions remain one block row with a count and exact jump actions; each source
  page can be collapsed independently, with bulk controls when several groups
  are present. Excerpt windows preserve Unicode graphemes and full blocks remain
  available on demand. (GH #144, GH #145)
- **Ctrl+K can learn repeated deliberate choices without changing search
  truth.** Page results expose exact, prefix, substring, and fuzzy objective
  classes (including aliases); device-local, graph-scoped frecency may reorder
  only ties inside one class after repeated activation. The bounded history can
  be disabled or reset, and saved searches and queries remain deterministic.
  (GH #143)

- **Opening fenced code blocks now offer language completion.** Typing at least
  one language character after backtick or tilde fences searches only the
  languages bundled for highlighting, accepts common aliases while writing the
  canonical identifier, and never activates on closing fences. `/Code block`
  opens the same bounded picker immediately; bare and unsupported fences keep
  their previous Enter behavior. (GH #94)
- **Ctrl/Cmd+Enter now cycles every selected block's task state in one step.**
  Mixed selections advance independently through the configured workflow,
  repeaters keep their existing rollover behavior, blank blocks stay blank, and
  the complete change is one atomic Undo while the selection remains active.
  The command remains remappable. (GH #136)
- **Tabs can now be reordered directly in the overflow menu.** A visible drag
  handle and Alt+Up/Down keyboard actions update the pane's canonical tab order
  while preserving active, pinned, split-pane, close, and persistence behavior.
  (GH #141)
- **The selection toolbar can now toggle page links and inline code.** The
  actions preserve the inner selection, unwrap existing syntax, participate in
  Undo, and keep the toolbar compact through a narrow-layout overflow. (GH #142)
- **Page-valued properties now provide direct navigation.** Bare values in
  `tags`, `alias`, and `aliases` are rendered as page links (including
  comma-separated values), while custom and wholly quoted properties stay
  literal unless they contain an explicit page reference. (GH #139)

### Changed

- **PDF uploads and annotations now follow Logseq OG's file-graph contract.**
  Upload links retain the original source name while Tine's configurable
  filename template controls the stored asset, resolve from the actual page
  path, and use the correct Markdown or Org syntax. The viewer restores and
  persists page/scale state, creates `hls__` pages in the graph's preferred
  format, copies a new highlight's block reference, and writes OG-shaped area
  metadata while retaining Tine's guarded merge and foreign-data protections.
- **Search now has one visible home beside the primary navigation controls.**
  The duplicate read-only sidebar field is gone; the labelled toolbar button,
  Ctrl+K shortcut, complete switcher, and “Open search tab” flow are unchanged.
  (GH #100)

### Fixed

- **Block reference-count badges now refresh after a reference is saved.**
  Creating or removing a `((block reference))` updates the source block's badge
  without requiring the graph to be reopened. (GH #154)
- **Linux windows now advertise Tine's stable desktop identity.** Main, graph,
  and Quick Capture windows use the packaged application ID, and standalone
  binaries provide the matching desktop entry and icon without interfering with
  single-instance shortcut forwarding. A remaining Plasma task-switcher lookup
  problem is tracked separately rather than being treated as covered here.
- **Linux system titlebar controls work when native window decorations are
  enabled.** GTK now propagates pointer events to the window-manager frame, so
  its minimize, maximize, and close buttons are interactive; close still runs
  through Tine's guarded save-and-session flush path.
- **Quick Capture accepts typing on its first show and has a visible frame.**
  Its scratch bullet now has a real block identity, allowing the existing
  activation path to enter edit mode immediately instead of waiting for a first
  click. Plasma users can invoke the shortcut and type directly into the bullet,
  and the frameless window now draws a subtle theme-aware border.
- **Page property settings preserve the surrounding Markdown layout.** Editing
  one field now updates it in place without moving it below other properties or
  deleting blank separators, so unrelated page-header metadata remains intact.
  (GH #163)
- **Logseq PDF highlights open safely and round-trip between both apps.** The
  bounded EDN reader now consumes Logseq's UUID tags and list-shaped rectangles
  without runaway allocation, preserves creation-zoom coordinates for correct
  placement, and writes Logseq's current sidecar shape back without erasing
  foreign metadata. Newly inserted PDFs also use Logseq's compatible embed form.
  (GH #61)
- **Linux Developer Tools now detach reliably where the native backend supports
  it.** On X11/XWayland, the old implementation asked an asynchronously-created
  inspector to detach too early, so the request was normally a no-op. A one-shot,
  timer-free lifecycle hook now detaches after WebKit's actual attach event and
  leaves later manual reattachment alone. Native Wayland remains docked because
  current Fedora/WebKitGTK renders the detached inspector black; its docked
  inspector is correctly scaled. AppImage mixed-DPI rendering remains a separate
  packaging diagnostic rather than an unverified scaling change. (GH #31)
- **Help with Tine now canonicalizes optional parser fields before classifying
  known oracle artifacts.** A harmless `undefined`-versus-omitted field can no
  longer make a backtick-state-only mismatch look like a new divergence.
  (GH #82)
- **Deep outlines keep a useful text column on Android.** Coarse-pointer phone
  layouts use a tighter nesting step, keep guide lines under their parent
  bullets, and expose folding as a visible trailing touch action; desktop
  geometry is unchanged. (GH #150)
- **Android status and navigation icons now follow Tine's selected theme.** The
  native edge-to-edge bars restore the persisted appearance during launch and
  resume, then stay synchronized across repeated light/dark switches. (GH #149)
- **Persistent Search results now fit their pane and retain their evidence.**
  Search, List, Table, and Board keep the matched terms highlighted; result
  rows wrap instead of widening a narrow pane; and Ctrl+F searches the visible
  query results as well as linked and unlinked reference rows. (GH #140)
- **Enter now adds another page property when editing the first properties-only
  bullet.** A second Enter on the trailing empty line exits cleanly to a normal
  body bullet, matching Logseq without splitting the property list. (GH #138)
- **Android's Interface size setting now scales the complete application.** It
  uses the document-level Chromium path on Android, where Wry's native zoom API
  is a no-op, while desktop and iOS retain native webview scaling. (GH #133)
- **Desktop startup no longer exposes intermediate unthemed layout frames.**
  The main window is revealed only after the themed app has painted, with a
  bounded native fallback so a frontend failure cannot leave Tine invisible.
  (GH #132)
- **Arrow navigation and empty-block deletion inside a block embed keep the
  caret in the visible embed.** The underlying source outline is still edited,
  but structural focus no longer jumps to the source block. (GH #134)

## [0.5.8] - 2026-07-13

### Added

- **Search and queries now share a persistent result workspace.** Ctrl+K can
  open its complete page-and-block result set in a graph-scoped tab, switch
  between search, list, table, and board presentations, survive an app restart,
  and become one ordinary query page when named—without writing temporary graph
  files. (GH #99)
- **Query creation has a friendly primary surface and an optional deeper one.**
  Plain search syntax remains editable as plain text; a Gmail-style filter
  dialog can build richer searches or hand off losslessly to the visual query
  builder and raw DSL, while on-demand explanations and diagnostics show what
  the engine interpreted. (GH #69)
- **Search results now show bounded, useful evidence.** Block results separate
  page/breadcrumb context from a two-line excerpt and highlight every positive
  term that actually caused the Rust engine to match; negated terms are never
  presented as evidence, and the combobox exposes its active result to
  assistive technology. (GH #98)
- **Primary panes now share quiet, theme-aware scrollbar styling.** The left
  sidebar, page/split scrollers, and right sidebar use the same semantic thumb
  colors without forcing overlay scrollbars into layout-consuming geometry;
  forced-colors and coarse-pointer environments retain native controls. (GH
  #103)
- **Clicking an outline guide now expands or collapses the complete descendant
  subtree.** If any collapsible descendant is folded, the guide expands them
  all; otherwise it folds them all while leaving the guide's parent open. The
  forgiving hit target is keyboard-accessible, normal pages persist the change
  as one Undo step, and embeds/references keep it local to that surface. (GH
  #128)
- **Overflowing tab strips now keep titles readable and provide a complete tab
  overview.** A pane-local button appears only when its tabs no longer fit,
  lists every full title with active, pinned, and close controls, and supports
  keyboard navigation. Activating a tab reveals it in the horizontal strip;
  ordinary tab closing, pinning, and drag-and-drop behavior remains intact. (GH
  #105)
- **Right-sidebar items can now be collapsed independently.** Each page or block
  has an accessible disclosure that parks its body without mounting its outline
  or references; a compact menu provides Collapse all, Expand all, and Close
  all. State is local to this installation and graph, survives restarts and
  renames, and active edits commit before a body is removed. (GH #106)
- **Block embeds have a restrained, theme-aware identity cue.** The embedded
  root bullet and its heavier descendant guide share a muted accent derived
  from the active theme; ordinary bullets, guides, text, and backgrounds remain
  unchanged, and custom CSS can override the semantic token. (GH #125)
- **Favorites and Recent can now be collapsed independently in the left
  sidebar.** Both sections default open, retain their item counts while folded,
  work as keyboard-accessible disclosures, and remember their state separately
  for each graph across restarts. (GH #101)

### Fixed

- **The `/Calculator` slash command now activates the live calculator on first
  insertion.** The new block immediately shows its fence-stripped editor,
  line-number gutter, and live results instead of requiring a blur and second
  click. (GH #57)
- **Typing a page alias into the first bullet no longer interrupts the editor at
  `alias::`.** The property block stays mounted until editing ends, then adopts
  the compact page-property presentation; the completed alias persists and
  resolves links and backlinks normally. (GH #62)
- **Android backup restore no longer fails when app data and the selected graph
  live on different filesystems.** Pre-restore recovery files now stay beside
  the live graph or external assets they protect, preserving the atomic safety
  move without hitting a cross-device error. (GH #130)
- **Switching an ordinary query to Search view no longer hides its results.**
  Search, List, Table, and Board now preserve the query engine's membership;
  DSL results use the same bounded search rows without inventing text-match
  highlights that the query did not produce.
- **Graphs with an external `assets` symlink or Windows junction can be opened
  safely.** Tine shows the resolved directory for explicit, device-local
  approval, then confines every asset read and write to that exact canonical
  target. Declining leaves the graph closed with a useful explanation, while a
  stale or retargeted link fails closed without widening access to pages,
  journals, configuration, or other managed files. (GH #127)
- **Linked and unlinked references now use the complete page identity.** Plain
  text mentions of a page alias appear under the canonical page's unlinked
  references, while explicit links in page-level properties appear as exact,
  read-only backlink rows. Scoped cache invalidation follows the same rules, so
  edited references update immediately. (GH #126)
- **Block embeds now behave as live editing surfaces.** Real disclosure clicks
  fold same-page and cross-page embedded branches locally without editing the
  macro host or changing the source block's collapse state, and Enter keeps the
  new block and caret inside the visible embed while persisting one source edit.
  (GH #124)
- **Help with Tine no longer exports a scrubbed reproduction that has lost the
  original actionable parser delta and retained only mldoc's known backtick
  state artifact.** The anonymizer now tries its remaining privacy tiers and
  omits the case if none preserves a non-artifact divergence. (GH #82)

## [0.5.7] - 2026-07-12

### Fixed

- **Alt-modified literal delimiters now retain Logseq selection-wrapping
  behavior.** On layouts where `Alt + [` still produces a literal `[`, two
  presses wrap selected text as `[[text]]` and open page completion. Layouts
  where Alt/Option produces another character keep native text input, and an
  explicitly configured editor shortcut takes precedence. (GH #83)
- **The shared parser is updated to lsdoc 0.5.3.** Native and browser-WASM
  parsing now include the final issue #82 state-parity corrections, while the
  Help with Tine oracle remains pinned to the exact released sources. (GH #82,
  GH #111)
- **Help improve Tine now version-locks the complete lsdoc comparison oracle.**
  The mldoc parser, AST normalizer, comparator, and reference extractor are
  pinned and checked as one bundle, preventing stale helper files from being
  reported as real graph divergences. Context-dependent differences that reduce
  to mldoc's known failed-double-backtick state leak are rechecked in fresh
  parser realms and shown separately instead of counted as lsdoc bugs. (GH #82)
- **Double Enter now exits a trailing fenced code or calculator block.** The
  first Enter adds a blank code line; the second removes that sentinel and opens
  a normal sibling block below. One Undo restores the entire pre-exit state. (GH
  #93)
- **Imported preamble text, first-block page properties, and split middle-click
  navigation now match the page that owns them.** Ordinary Markdown before the
  first bullet is visible without rewriting the file and becomes a block only
  when edited; a properties-only first block uses the same page-property UI and
  gear editor as an unbulleted pre-block; and middle-clicked page links open in
  their source pane rather than whichever pane was focused earlier. (GH #85,
  GH #86, GH #87)
- **Returning to a previously loaded large page no longer mounts it twice.** A
  pane now renders only the route whose asynchronous load actually completed;
  obsolete load failures cannot replace a newer page, and the performance gate
  compares every candidate on one machine with both an immutable long-term
  anchor and the previous release.
- **Clicks inside inline code now put the caret on the clicked character.**
  Literal delimiters are mapped separately from their content instead of
  snapping clicks to the start or end of the formatted span. (GH #114)
- **Quick Capture now requests native activation only after its editor is ready,
  with bounded retries for newly mapped Linux windows.** A missed initial show
  event is reconciled without creating a focus feedback loop. (GH #117)
- **Table arrow-key navigation is now covered through the real global keyboard
  path.** The deployed app already had the Grid-equivalent behavior reported in
  GH #113; component and Linux real-app regressions now guard it.
- **MKV videos play inline again on Linux.** When WebKitGTK rejects Matroska from
  Tauri's range protocol, Tine retries supported files through a graph-scoped,
  size-bounded Blob; oversized or unsupported files retain the external-player
  fallback. (GH #119)
- **System media players are launched outside Tine's runtime session.** Linux
  openers now inherit the KDE/Plasma session identity needed by `xdg-open`,
  exclude AppImage loader paths, and start in a new session so VLC cannot load
  Tine's bundled libraries or die with its parent process group. (GH #118)
- **MP3 and other graph audio play inline again on Linux.** WebKitGTK protocol
  failures retry through the same graph-scoped, size-bounded Blob path as MKV,
  while expanded playback and external-player actions remain available. (GH
  #121)
- **Page titles can reveal or open their exact source file on desktop.** The
  right-click menu flushes edits first, refuses save conflicts, preserves nested
  and path-pinned Markdown/Org identity, and never exposes the actions for the
  bundled Guide. (GH #84)
- **Published Guides now open on Welcome to Tine and preserve block-reference
  navigation.** Home links target the Welcome page, the alphabetical list remains
  at All pages, and public reference targets expose keyboard-accessible counts
  with links to public same-page and cross-page referrers. (GH #115, GH #116)
- **Published outline guides line up with their bullets.** Inline block embeds
  now use a single root marker instead of stacking host, list, and embedded
  connector lines. (GH #122)
- **Mobile outlines use substantially more of the available screen width.** At
  phone widths, page gutters shrink from 48px per side to 12px per side while
  retaining the device safe-area insets.
- **Writable pages have a quiet continuation target below their content.** It
  focuses an existing empty trailing leaf or creates exactly one root (one Undo);
  zoomed outlines append within the zoom root, while Guide and read-only pages
  remain immutable. (GH #96)
- **Ctrl+K now explains its search grammar in place.** A keyboard-accessible
  Search syntax button documents AND, OR, exclusion, phrases, and regex; Escape
  closes the help before closing search, and every displayed example is executed
  against both frontend and Rust matchers in tests. (GH #97)
- **Settings now has progressive disclosure and cross-tab search.** Niche and
  experimental controls live in persisted, accessible Advanced sections; search
  covers labels, descriptions, and aliases, identifies the tab/section, and
  temporarily reveals matching hidden controls without changing the saved
  disclosure state. (GH #112)
- **Pasting selected structured content preserves its explicit outline.** Safe
  clipboard HTML is deterministically converted into nested lists, headings,
  paragraphs, quotes, fenced code, links, emphasis, and one-block GFM tables;
  malformed, semantic-free, or bounded-out payloads use the existing plain-text
  path. The import is one normal persistence transaction and one Undo, while
  Ctrl/Cmd+Shift+V remains literal plain-text paste. (GH #58)

### Changed

- **The frontend build and test toolchain has been security-updated.** Vite 6
  and Vitest 3 replace vulnerable development-only versions, with deterministic
  SolidJS test resolution and zero known npm audit findings.
- **Block embeds now begin with one interactive root bullet instead of two.**
  The referenced root keeps its collapse, zoom, sidebar, navigation, and editing
  behavior, while a slightly heavier descendant guide marks the embedded outline
  without adding a surrounding box. (GH #88)
- **Bug reports now feed a durable regression and follow-up workflow.** The issue
  form asks for exact steps and an anonymized minimal graph, UI and non-UI bugs
  share one indexed catalog, and a reporter's comment on a closed issue reopens
  it automatically for triage.
- **Release publication now fails closed on an incomplete platform set.** Tagged
  releases require Android signing, a successful real offline Flatpak build,
  lockstep version/changelog metadata, cross-platform-stable vendored oracle
  bytes, all 21 expected artifacts, and all 12 updater platform entries before
  the draft can become public. All expensive platform builds now run in parallel
  into immutable workflow artifacts; one short publisher assembles the updater
  manifest and performs the only GitHub Release mutation.

## [0.5.6] - 2026-07-11

Parser-integration and release-recovery patch: lsdoc 0.5.2, private and
reproducible Help-panel reports, and complete cross-platform release guards.

### Changed

- **The shared parser is updated to lsdoc 0.5.2.** Both the native core and the
  vendored browser WASM parser use the same released parser build.

### Fixed

- **Help improve Tine uses the same OG-faithful reference oracle as lsdoc.**
  Property, nested, file-label, Org, embed, and block-reference semantics no
  longer drift between the two sides of the comparison, eliminating false
  divergences such as Markdown links in property values. CI now binds the
  vendored oracle to the pinned lsdoc release and its exact source hash.
- **Help improve Tine reports no longer expose page names or private URLs.**
  Source files use neutral labels, URL schemes remain parseable while hosts and
  paths are scrubbed, URL-sensitive divergences survive anonymization more
  reliably, and copied reports record the Tine version used for the comparison.
- **Release CI catches platform-only compilation and stale Flatpak sources before
  tagging.** Windows and Android compile guards now run on ordinary CI, the
  Flatpak offline npm and Cargo manifests are checked against their lockfiles,
  and a release remains draft unless every required artifact job succeeds.

## [0.5.5] - 2026-07-11

Correctness and interaction release for Sheets, caret navigation, edit-mode
rendering, Windows graph and clipboard behavior, and read-only Org safety.

### Added

- **Ctrl/Cmd+Shift+V pastes multiline plain text into the current block.** Normal
  multiline paste keeps Logseq's outline-building behavior, while the modified
  shortcut preserves embedded newlines at the caret. (GH #81)

### Fixed

- **Arrow Down leaves a wrapped block at the caret's visual column.** Crossing
  into the next block no longer measures from the beginning of the wrapped
  source line and clamps the caret to that block's end.
- **Sheets remain identity-safe across split panes, sorting, pagination, and
  asynchronous query hydration.** Selection and mutation targets are scoped to
  their grid surface and source block, stale query results cannot overwrite a
  newer view, formula/aggregate dependencies invalidate correctly, and large
  Grid/Table/Board views keep bounded lookup and rendering work.
- **Board card drags stay bound to one pointer and one rendered Board.** Starting
  another drag cancels the previous document-wide session, unrelated pointer
  events are ignored, and a column in a duplicate split-pane Board cannot be
  accepted as the drop target.
- **Raw block punctuation and numbers use normal text metrics in edit mode.**
  Inter or the configured monospace face now handles `#`, `*`, brackets, and
  digits before the bundled emoji fallback, while actual emoji remain protected
  from WebKitGTK's unsafe system COLRv1 path.
- **Arrow Up enters a wrapped previous block on its bottom visual row.** The
  caret keeps its horizontal source column instead of jumping to the matching
  position on that block's top row.
- **Windows graph windows are created off synchronous Tauri event handlers.**
  Shift-opening a second graph no longer takes the WebView2 deadlock path that
  could leave the new window blank and the original window uneditable. (GH #70)
- **Windows screenshot paste reaches the image-byte path again.** WebView2 image
  clipboard payloads no longer fall into native file-list import and report a
  spurious skipped item; byte-only images retain the 64 MiB safety bound and
  mixed copied files still use path-based import. (GH #78)
- **Page rename and alias navigation keep sidebar state live.** Successful
  renames re-key and deduplicate Favorites and Recents, while alias favorites
  resolve to their canonical page for ordinary, sidebar, new-tab, and context
  actions. (GH #79, GH #80)
- **Read-only Org pages now reject every frontend mutation path.** Collapse,
  context-menu, selection, drag/move, sheet, property, durable-ID, dirty-state,
  and persistence entry points enforce the round-trip safety boundary rather
  than relying only on the hidden textarea.
- **Zoom navigation and editing stay inside the rendered subtree.** Arrow and
  shift-selection order includes children revealed by the zoom-only collapse
  override, excludes invisible page siblings, and keeps Enter-created blocks and
  their caret mounted without changing durable collapse metadata.
- **Variable-length code fences no longer close on a shorter delimiter run.** A
  shared backtick/tilde scanner now drives property hiding, planning
  normalization, Enter, and multiline-paste decisions.
- **Org editing mutations remain in Org syntax.** Collapse and ordered-block
  splits use property drawers, Org subtree copy strips durable IDs while retaining
  OG's Markdown clipboard outline, and multiline paste replaces visibly-empty
  metadata-only blocks without leaving a ghost bullet.
- **Query collapse state no longer leaks between identical queries on different
  pages or graphs.** Overrides are keyed by graph and block identity, and an
  explicit expanded choice now survives a source `:collapsed? true` default.
- **Zooming into a collapsed block reveals its children without expanding the
  block on its parent page.** The zoom root temporarily ignores only its own
  stored collapse state; descendant blocks retain their individual folds. (GH #77)
- **Emoji in editable fields no longer trigger WebKitGTK's COLRv1 crash.** Native
  inputs and textareas use a bundled monochrome Noto Emoji font, covering page
  properties, page-title rename, block editing, and other raw-text controls while
  display surfaces continue to use Twemoji SVGs. (GH #76)
- **Default Windows draw.io installations now autodetect and launch correctly.**
  External-editor command templates accept double-quoted executable paths such as
  `"C:\Program Files\draw.io\draw.io.exe" {}`, and autodetection checks both
  `%ProgramFiles%` locations in addition to the per-user install directory. The
  command is still spawned directly without a shell. (GH #71; follow-up to #38)

## [0.5.4] - 2026-07-10

Focused bug-fix release for journal templates, linked-reference filters,
imported collapsed headings, planning-date rendering, and mobile update UI.

### Fixed

- **Default journal templates appear on the initial Journals view without a
  manual refresh.** Template content is persisted before graph resources reload,
  including when an empty journal file already exists. (GH #73)
- **Linked References filters include task states, tags, and page references
  from descendant blocks.** Facet counts and include/exclude filtering now match
  each complete displayed backlink tree. (GH #59)
- **Collapsed heading blocks produced by importers no longer lose their parent
  heading.** A heading immediately before the first list is narrowly recovered
  as the collapsed parent when it carries `collapsed:: true`; ordinary Markdown
  introductions and page properties remain unchanged. (GH #67)
- **Scheduled and deadline dates remain rendered as clickable date chips when
  body text follows the planning line.** The trailing body stays visible, while
  mid-text and code lookalikes remain ordinary content. (GH #75)
- **Android and iOS no longer expose the desktop self-updater.** Mobile builds
  skip the startup update toast, hide the About tab's manual update action, and
  direct users to their app distribution channel instead. (GH #48)

## [0.5.3] - 2026-07-10

Multi-window graph management, direct file-manager asset paste, PDF and query
fixes, Android/Flatpak release repair, and comprehensive graph data-safety
hardening.

### Added

- **Multiple graphs can stay open in independent desktop windows.** The graph
  switcher now keeps a durable removable MRU list; click switches the current
  window and Shift-click opens another OS window. Each window owns its graph,
  watcher events, warm cache, backups, and persisted tab/pane session, while
  quick capture safely targets only the last-focused graph. A second
  `tine <graph>` launch opens or focuses that graph in the existing process.

- **Files copied in the OS file manager can be pasted directly into a block.**
  Tine imports regular files into `assets/` and inserts Logseq-compatible links;
  multiple files are supported, directories are skipped, native file paths avoid
  loading large files into the webview, and byte-only clipboard payloads are capped
  at 64 MiB per file.

### Fixed

- **Graph and recovery operations now stay inside the selected graph.** Unsafe
  configured page/journal paths and escaping journal filenames are rejected,
  overlapping graph windows are refused, and every graph-scoped IPC is pinned to
  the window binding that issued it.
- **Backups are root-bound and complete before they become restorable.** Snapshot
  namespaces use a canonical-root digest, complete snapshots carry a hash-verified
  v2 manifest, partial/legacy-unverified directories are hidden from normal restore,
  and restore rebuilds the live graph using the snapshot's recorded directories.
- **Exact duplicate-journal navigation cannot edit the canonical file by mistake.**
  Loading a path-pinned file replaces a same-name working-set slot and preserves
  that exact path through save and undo.
- **Captured media is durable before its Markdown link is inserted.** A crash can
  leave a recoverable orphan, but not a saved note pointing to bytes that only
  existed in WebView memory.
- **Configuration updates and rename rollback preserve concurrent/failing work.**
  Config read-modify-write retries external changes, and rename rollback now
  includes the move whose source removal failed.
- **Android release builds use the stable `page.tine.app` application ID.** The
  desktop-only app-ID rename no longer makes Tauri search for a nonexistent Java
  package, which had prevented the signed APK from being produced for v0.5.1 and
  v0.5.2.
- **Flatpak's offline dependency bundle is current and checked before releases.**
  Dependency-lock changes now trigger the Flatpak build-test on `master`, while
  release tags no longer start that separate non-release workflow.
- **PDF highlight block references now open the source PDF at the highlighted
  page.** Plain-clicking an annotation `((block-ref))` follows OG Logseq behavior,
  including PDF filenames containing spaces; modifier-click navigation remains
  available. (GH #61)
- **PDF viewing is bounded against malformed or extreme files.** Tine rejects PDFs
  over 256 MiB before reading them into memory, caps page/layout and canvas
  allocations, validates page dimensions, downsamples unusually large valid pages,
  and releases pdf.js resources on failure instead of risking a blank runaway
  viewer. (GH #61)
- **Area highlights now round-trip OG Logseq's `hl-stamp::` metadata.** Newly
  created area annotations copy the EDN image timestamp exactly, while text
  highlights correctly omit the property and existing foreign properties remain
  untouched. (GH #61)
- **Deleting a page now refreshes live queries.** After deleting a page, open
  `{{query}}` panels re-run immediately and drop the deleted page's rows, instead of
  lingering with a stale result until the next edit.

## [0.5.2] - 2026-07-10

In-app Guide link/reference fixes, context menus that stay on-screen, faster
sheet mounting, and a parser refresh (lsdoc 0.5.1). No new features.

### Fixed

- **Context menus no longer open off-screen.** A right-click menu near the bottom of the
  window (e.g. deleting a namespace low in the sidebar) now opens *upward* when there isn't
  room below, and is clamped horizontally, so all of its items stay reachable.
- **Links and block references now work on the in-app Guide.** Guide pages linked to
  `[[Welcome to Tine]]` and `[[Project/Roadmap]]`, which weren't part of the bundled
  guide set, so those links opened a blank page; and block references / embeds
  (`((…))`, `{{embed …}}`) never resolved because the Guide is virtual (never written
  to disk) while resolution only scanned the on-disk graph. The guide set is now closed
  under its own links (a test enforces it), and refs/embeds fall back to the loaded
  guide pages. Everything resolves consistently in the in-app Guide, in the
  copied-into-graph copy, and in the published website demo.
- **Page aliases typed as the first bullet now work.** Writing `alias:: book` as the
  first bullet on a page (the natural outliner action, matching Logseq) now registers
  the page alias, so `#book`/`[[book]]` references resolve to that page and appear in
  its backlinks — previously the alias only took effect when set via the page
  properties panel. (GH #62)
- **Shift-click in the left sidebar opens the page in the right sidebar.** Shift-clicking
  a favorite, recent, all-pages, or namespace-tree entry now opens it in the side panel
  (as inline links already did) instead of navigating in the center pane and selecting
  text. (GH #63)
- **Query-builder dropdowns no longer render behind the backlinks section.** (GH #64)
- **Enter inside a fenced code block inserts a newline** instead of splitting off a new
  bullet and breaking the fence. (GH #66)

### Performance

- **Large sheet tables and boards mount much faster.** A row's / card's heavy content
  (title parsing, value chips, formula results) is now rendered lazily as it scrolls
  near the viewport instead of all at once, mirroring the existing block-body
  virtualization. Selection, keyboard navigation and drag still work over the whole
  sheet. On a synthetic 2000-row table this cut initial mount cost by ~2.6×. (The grid
  view gets the same treatment in a follow-up.)

### Changed

- **Parser updated to lsdoc 0.5.1.** Page-reference and backlink extraction now follow
  Logseq/mldoc semantics more closely, alongside lexer performance improvements. Purely
  a parser refresh — your files are unchanged.

## [0.5.1] - 2026-07-10

Data-safety hardening, an application-ID correction, and PDF fixes. No feature changes.

### Changed

- **Application identifier corrected to `page.tine.Tine`** (was `page.tine.app`).
  Flathub forbids IDs ending in `.app`, so the desktop/Flatpak identifier was
  renamed. Your data directory (settings, session, backups) migrates automatically
  on first launch and remains reversible via backups. The Android application ID is
  unchanged.
- **"Empty trash" for orphaned media now deletes only asset files.** Deleted pages,
  duplicate journals, and sync-conflict copies are kept in typed trash subfolders and
  are never swept by the asset cleanup; the Settings action is relabeled and shows the
  protected recovery counts.

### Fixed

- **Pasted/captured media durability.** The app now waits for asset bytes to be
  written before it can close, and rolls back the inserted link if the write fails —
  a note can no longer end up referencing an asset that never reached disk.
- **Journals are snapshotted before launch-time filename migration.** The safety
  backup now captures the original filenames first; if the snapshot cannot be taken,
  the rename is skipped rather than mutating journals without a recoverable copy.
- **HTML export no longer overwrites pages on slug collisions.** Titles that differ
  only in punctuation (`Foo!` vs `Foo?`) or use non-ASCII scripts now get unique,
  non-empty filenames, and all internal links point at the file actually written.
- **PDF links.** Image-syntax PDF references (`![](…file.pdf)`) render as a PDF link
  instead of a broken image; backslash paths are normalized; an unloadable PDF shows
  an error instead of a blank viewer with runaway memory. (GH #61 — the highlight
  block-reference click-to-open behavior is still being finished.)

## [0.5.0] - 2026-07-09

### Added

- **Sheets: grids, databases, and boards over plain bullets.** Blocks can now
  render as recursive grids, field tables, or boards with spreadsheet navigation,
  typed `tine.fields::` schemas, editable task/property cells, tag boards with
  write-back, aggregates, markdown pipe-table conversion, and CSV/TSV file-drop
  import — all stored as ordinary Logseq markdown/org outlines plus `tine.*`
  properties. Phase 7 adds typed `tine.formula.<name>::` computed columns and
  formula group-by axes, `tine.filter::` table/board filters that fail open with a
  visible chip, and a right-click formula/filter editor.

- **Sheets: grids grow from their edges, and boards have a group-by picker.** A
  grid is never a dead end — an empty grid shows a clickable placeholder cell
  instead of inert "empty grid" text, and hovering a top-level grid reveals **+**
  affordances on its right and bottom edges that add a column or row (one undo,
  cursor lands in the new cell). Boards now expose their grouping: a **Group by**
  dropdown above the columns and a matching **Group by →** submenu in the board
  right-click menu let you regroup by State, Priority, Tags, or any field —
  previously the axis was fixed to `state` at creation and only changeable by
  hand-editing `tine.group-by::`.

- **Sheets: paste nests or splats depending on mode.** Pasting a copied grid
  region while cells are **selected** now **splats** it into the surrounding grid
  (anchored at the selection's top-left, growing/padding/overwriting the footprint
  in one undo, with a toast to undo if it replaced non-empty cells) instead of
  burying it as a nested grid. Pasting while **editing** a cell still **nests** the
  copy as a subgrid at the caret. This fixes the accidental double-nested grid and
  needs no modifier — the paste mode is the signal (ADR 0037).

- **Turn an outline into a grid/table from its bullet.** Right-clicking a plain
  outline bullet that has children now offers **Show children as → Outline / Grid /
  Table** — the convert-in-place gesture the Guide describes, which previously existed
  only inside a sheet's own row menu. (Shared with that menu so both stay in sync.)

- **Add formula… from a column header.** Right-clicking a table column header now
  offers **Add formula…** (it previously lived only on the table's ⋮/body menu, so
  the Guide's "right-click a column header" instruction pointed at a command that
  wasn't there). Works whether the header is a plain field or an existing formula
  column.

- **In-app Guide.** Help → Guide and the *Open Guide* command now open bundled,
  read-only how-to pages for Sheets, quick capture, PDF annotation, tips, and the
  feature showcase. Guide pages live only in memory under `Tine-guide/` until you
  explicitly use **Copy the guide into your graph**, which creates the complete
  editable `tine-guide/...` namespace, rewrites inter-guide links to the copied
  pages, includes referenced guide assets, and skips existing copied pages without
  overwriting user edits. A from-zero **Features/Formulas** page covers what a
  formula column is, right-click a column → Add/Edit formula, the IF/THEN/ELSE and
  value-picker faces, the `</> raw` toggle, and honest limits (single-level `if`,
  nested arithmetic needs raw); the Sheets guide's "Create one yourself" sections
  teach `/Grid`, `/Table`, `/Board`, **Show children as →** conversion, edge-grow,
  ghost Add-row/column buttons, and the board **Group by** picker rather than
  telling you to hand-type `tine.header::` / `tine.fields::`.

- **Split view.** Panes now have their own tabs and history, TreeSheets-style
  pane/seam keyboard navigation with type-at-a-seam-to-split, `Ctrl+click` opens
  links in another pane, tabs can be dragged to another pane or seam, and the
  layout persists across launches. Pane-select mode (Esc from block-select, or
  the palette) shows a hint pill and tints its target; arrows step strictly
  directionally across panes, seams, per-pane edge segments (split just that
  pane) and whole-window edges (split everything); selecting a pane focuses it,
  `Delete` closes it, and `Ctrl+K` opens a page right there.

### Changed

- **New parser (lsdoc v2).** Tine's block and inline parser was rebuilt from scratch
  as a two-phase, linear-time parser transcribed directly from Logseq's mldoc,
  replacing the previous optimistic scanner. It is more faithful to Logseq on
  real-world graphs and parses in guaranteed linear time; on any construct it has not
  yet transcribed it is designed to fail safely rather than silently mis-parse.

- **Richer link hover previews.** Hovering a `[[page]]`, `#tag`, or block reference
  now shows the target's real, read-only block tree — bullets, nesting, task markers,
  priority, full multi-line bodies, and inline formatting — in a floating popup you can
  move into and scroll, matching Logseq's page preview. (Previously it showed only the
  first line of each block as plain text.) Block-reference previews now open after the
  same short hover delay as page previews instead of instantly. Hovering never modifies
  the graph.

- **Desktop app identifier is now `page.tine.Tine`** (was `dev.tine.app`, then
  briefly `page.tine.app`). This lets Tine prove domain ownership (`tine.page`)
  for Flathub. On desktop the change is
  invisible: on first launch Tine moves your existing settings, backups, open-tab
  session **and your last-opened graph** from the old location to the new one, then
  shows a one-time note that a few app-level preferences (e.g. window size) may need
  setting again. (Android stays `page.tine.app` and keeps its existing APK data.)

### Fixed

- **Backups now include nested pages.** Graph backup and restore copied only the
  top level of `journals/` and `pages/`, so pages inside sub-namespace folders were
  silently omitted — and the backup still reported success. Both now recurse the
  whole tree (skipping hidden and symlinked directories), and the completeness
  check counts every Markdown/Org file.

- **PDF highlight migration can no longer clobber another PDF's data.** When two
  PDFs had asset filenames differing only by case or space-vs-underscore, migrating
  one PDF's highlights to the new storage key could read and then delete the *other*
  live PDF's `.edn` and highlight files. Migration now skips the legacy key whenever
  it belongs to a different PDF still in the graph.

- **`##` headings render on every line of a multi-line block.** In a block spanning
  several lines, only the first line's heading was styled; `##`/`###` on later lines
  rendered as plain text. Each heading line now renders at its level.

- **Typing in a very long block no longer jumps the caret to the bottom.** In a
  block taller than the window, each keystroke scrolled the view so the caret sat at
  the bottom edge. The editor now holds the scroll position steady as the block
  resizes.

- **Sheets: removing a just-added table column takes effect immediately.** A column
  added via *Add column* lived only in an in-memory signal, so removing it from the
  schema left it on screen until an app restart. It's now cleared on removal, and an
  added-but-undeclared column gets its own **Remove column** in the header menu.

- **Sheets: long cell text wraps instead of stretching the whole table.** Sheet
  columns are capped with `fit-content()` and cells wrap, so one long note grows its
  row taller rather than blowing the table out horizontally. The in-cell value editor
  no longer overflows a narrow column (e.g. a numeric cell) past its right edge.

- **An empty day (or page) shows a bullet to type into again.** Deleting the last
  block via *Delete block* / a multi-block selection (which bypass the Backspace
  last-block guard) left the page with nothing to click. It now re-seeds the same
  phantom empty bullet a brand-new day gets — present to type into, but only written
  to disk once you actually type.

- **A conflicted page can be deleted again.** When a page's on-disk copy changes
  underneath an open edit (e.g. a Syncthing-delivered update), its save is refused
  until the conflict is resolved — but deleting it also flushed-first and aborted on
  that impossible save, so the page could be *neither* saved *nor* deleted. Delete is
  itself a resolution now: the on-disk version still moves to `.tine-trash`
  (recoverable) and the page is removed.

- **Query builder: a way back from "advanced".** The visual query builder's
  "⚙ advanced" switch to raw Datalog was one-way — advanced query blocks now show a
  **← Simple** control that returns to the visual builder. Within a session it
  restores the exact pre-conversion query (including the sort/aggregate/group-by
  clauses the Datalog form drops); for a query authored directly as raw Datalog it
  reverse-parses the recognized clause set, disabling the toggle with an explanation
  when the query can't be represented visually.

- **The identifier migration now actually runs.** The first cut migrated too late —
  after WebKitGTK had already created the new (empty) data directory — so it backed
  off and left you on the Welcome screen with your graph "forgotten". Migration now
  runs before the webview starts, backfills over an empty new directory, and also
  recognises the older `dev.logseqclaude.app` layout.

- **Android: external links now open.** Links on the About page (Changelog, Report
  an issue, Website, Ko-fi, …) and the Help/Releases links did nothing on Android —
  they tried to spawn a desktop opener that doesn't exist there. They now open via
  the platform (an `ACTION_VIEW` intent). (GH #49)

## [0.4.7] - 2026-07-08

### Fixed

- **Enter nests when you're zoomed into a leaf block** ([#46](https://github.com/martinkoutecky/tine/issues/46)).
  When zoomed into a block that has no children, pressing Enter created a new
  block as a *sibling* — outside the zoomed view — instead of a child. It now
  creates a child, matching Logseq. Applies to both Markdown and Org graphs.

- **The Command key no longer resizes the interface after scrolling on macOS**
  ([#27](https://github.com/martinkoutecky/tine/issues/27)). A trackpad scroll
  leaves a brief momentum "tail"; pressing Command during it was misread as a
  Command-scroll zoom, shrinking or growing the whole UI. Tine now zooms only when
  Command/Ctrl is held *before* the scroll gesture begins.

- **"Edit in draw.io" reliably appears and opens your editor** ([#38](https://github.com/martinkoutecky/tine/issues/38),
  reported by @nataloko). A second `/drawio` diagram could be saved under a mangled
  name that lost the edit affordance, and an unconfigured editor fell back to the
  system image viewer instead of draw.io. Diagrams now use the unique-name asset
  convention (so double extensions like `.drawio.svg` survive name collisions) and
  Tine auto-detects an installed draw.io the first time you edit.

- **Journal feed scrolls on first open** ([#39](https://github.com/martinkoutecky/tine/issues/39)).
  On macOS the journals view could open unscrollable until a window resize; Tine
  now forces the relayout itself once the feed loads.

## [0.4.6] - 2026-07-08

### Added

- **Search operators in Ctrl-K** ([#44](https://github.com/martinkoutecky/tine/issues/44)).
  The quick-search box now understands the mainstream full-text dialect: multiple
  words are an order-independent **AND** (all must match), `OR` (uppercase) is an
  alternation, `-word` **excludes**, `"a phrase"` matches contiguously, and
  `/regex/` runs a (case-sensitive) regular expression with an inline "invalid
  pattern" hint. A single bare word still ranks pages fuzzily as before; any
  second term or operator switches both the page list and block results to the
  operator grammar. Search is case-insensitive except inside `/regex/`.

- **Diagrams via your own drawio / Excalidraw** ([#38](https://github.com/martinkoutecky/tine/issues/38),
  proposed by @nataloko). Keep diagrams next to your notes as ordinary image
  assets and edit them in the diagram app you already have — Tine bundles no
  editor. A `/drawio` command creates a new editable `assets/…​.drawio.svg`,
  inserts it as an image, and opens it in drawio; hovering any `*.drawio.svg` (or
  `*.excalidraw.svg` / `.png`) shows an **Edit in …** button. When you switch back
  to Tine the rendered image refreshes. Because the file is a normal image
  reference, the same graph still renders in Logseq (round-trip intact). Configure
  the editor commands (with autodetect for drawio) under **Settings → Files →
  Diagram editors**; empty uses your system default opener. Desktop only.

- **Desktop voice memos** (`/record`). On desktop, `/record` starts a microphone
  recording in the app (via the WebView's recorder) and a second `/record` stops
  it and inserts the audio as an asset — no phone required. Previously mic capture
  existed only on Android.

- **Journals button in the toolbar.** A one-click "go to Journals" button now sits
  next to the date-jump control in the top bar, so you no longer need the sidebar
  to get back to today's journal.

- **Hover peek for page links** ([#40](https://github.com/martinkoutecky/tine/issues/40)).
  Dwelling on a `[[page]]` or `#tag` opens a small read-only preview card of that
  page's blocks — a quick look without navigating away, like Logseq. The fetch is
  lazy (only on hover, cached per open graph) and the preview is bounded, so it
  costs nothing until used.

- **Space after a completed reference** ([#35](https://github.com/martinkoutecky/tine/issues/35),
  contributed by @nataloko). Accepting a `[[page]]` or `((block))` autocompletion
  now inserts a trailing space after the closing brackets so you can keep typing
  without manually moving past them. On by default; toggle under Settings → Editor.

### Changed

- **Foldable blocks are now discoverable in the right sidebar**
  ([#41](https://github.com/martinkoutecky/tine/issues/41)). Blocks opened in the
  sidebar were already foldable (they're the same live blocks as the main pane),
  but the fold arrow only appeared on a pixel-precise hover and was easy to miss
  in the narrow pane. It now stays softly visible while the sidebar item is
  hovered, going full-strength on the block itself.

### Fixed

- **`{{query (property …)}}` with `:colon` keys and `[[page]]`/`#tag` values now
  matches.** A simple query like `(and (property :fach [[Course]]) (property :type
  "#assignment"))` returned "No results": the parser kept the leading `:` on the
  key (so `:fach` never matched the property `fach`) and dropped a `[[page]]` or
  `#tag` used as a property value. Both are now handled the way Logseq does (drop
  the `:`, map `_`→`-`, extract the page name / strip the `#`), for `property` and
  `page-property`, in both the query engine and the visual query builder.

- **Camera / voice-memo captures no longer overwrite each other's names.** Photos
  and voice memos were being named `photo.jpg` / `voice-memo.m4a` (colliding to
  `photo_1.jpg` / `voice-memo_1.m4a`), losing the timestamp naming that pasted
  images get. Captures now get the same unique `yyyymmdd-hhmmss-…` name as a paste,
  with their real extension.

- **Pasting a screenshot now works on Windows** ([#43](https://github.com/martinkoutecky/tine/issues/43),
  reported by @msjsc001). `Ctrl+V` of an image copied by a Windows screenshot
  tool (e.g. PixPin) did nothing; Tine now reads the image straight from the
  paste event on Windows and macOS (falling back to the OS clipboard on Linux),
  so the screenshot lands in `assets/` and inserts into the block directly.

- **The query builder's "⚙ advanced" pill no longer destroys the query.**
  Clicking it used to replace the simple query with a multi-line Datalog
  template that a `{{query}}` macro cannot even hold (macros are single-line
  and brace-free), so the block stopped rendering as a query and the original
  filters were lost. It now *converts* the current query clause-by-clause to
  an equivalent single-line `[:find …]` form, refuses (with a toast) when a
  clause has no Datalog equivalent, and undo restores the simple form.

- **Shift-clicking a link no longer selects text** ([#42](https://github.com/martinkoutecky/tine/issues/42)).
  Shift-clicking a `[[page]]`, `#tag`, or block reference opens it in the sidebar;
  the browser's native shift-range-selection is now suppressed so no stray text in
  the main editor gets selected as a side effect.
- **Org property drawers no longer show in the editor** ([#37](https://github.com/martinkoutecky/tine/issues/37)).
  In `.org` files a block's built-in `id` lives in a `:PROPERTIES:`/`:END:` drawer;
  when a block was zoomed/opened (which stamps an id for durable references) that
  drawer appeared as raw text on edit. It's now hidden from the editor — and the
  empty drawer wrapper removed — exactly like markdown `id::`, matching Logseq's
  `remove-built-in-properties`. The drawer is reattached at its canonical spot on
  save; a user property in the same drawer keeps it visible (only the built-in
  line is hidden).

- **Welcome screen can be closed on Linux** ([#36](https://github.com/martinkoutecky/tine/issues/36),
  contributed by @nataloko). Tine's frameless Linux window left the first-run
  Welcome overlay with no window controls, so it couldn't be dismissed. The
  overlay now draws its own close/window controls.

## [0.4.5] - 2026-07-07

### Changed

- **Reproducible Android builds.** The APK is now byte-for-byte reproducible from
  source (deterministic build timestamp, single codegen unit, canonicalized build
  paths), so F-Droid can verify its rebuild matches the signed release and ship the
  developer's own APK.
- **Developer tools now open as their own window** instead of docked into the app
  ([#31](https://github.com/martinkoutecky/tine/issues/31)). Docked, WebKitGTK put
  the window's resize grip at the top of the inspector pane and rendered the
  inspector at the wrong scale on HiDPI/fractional displays. A separate top-level
  window avoids both; WebKitGTK's inspector still has an attach button to dock it
  back. Linux only.

### Fixed

- **Crash (`SIGABRT`) when the sidebar, tabs, or switcher show a page whose name
  contains a color emoji** ([#29](https://github.com/martinkoutecky/tine/issues/29)).
  On Linux distros that harden libstdc++ (e.g. Fedora), WebKitGTK's Skia
  color-font (COLRv1) glyph path aborts while painting a raw emoji. Tine already
  renders emoji in block content as Twemoji SVG images to sidestep WebKitGTK's
  emoji handling; the sidebar (favorites, recent, all-pages), tab titles, quick
  switcher, and right-sidebar titles now go through that same path, so no color
  glyph is ever handed to the font renderer.

## [0.4.4] - 2026-07-07

### Added

- **About tab in Settings** ([#32](https://github.com/martinkoutecky/tine/issues/32)).
  Settings → About shows the version and build, links to the website, source, and
  support (Ko-fi), and credits the people and AI collaborators behind Tine.
- **Developer tools (WebKit Web Inspector), openable in release builds**
  ([#31](https://github.com/martinkoutecky/tine/issues/31)). Press **Ctrl+Shift+J**,
  run *Toggle developer tools* from the command palette, or right-click → *Inspect
  Element* to open the inspector for theme/CSS debugging — the shortcut toggles it
  closed too. Previously the inspector was only compiled into debug builds; it now
  ships in releases. (The usual Ctrl+Shift+I / F12 are reserved by WebKitGTK itself
  and never reach the app, so Tine uses Ctrl+Shift+J — Chrome's other devtools key —
  which is remappable under Settings → Keyboard shortcuts.)
- **Time entry in the SCHEDULED/DEADLINE date picker**
  ([#30](https://github.com/martinkoutecky/tine/issues/30)). The `/scheduled` and
  `/deadline` picker now has an **"Add time"** control: set an `HH:mm` clock time and
  it's written the way Logseq does — `SCHEDULED: <2026-07-07 Tue 14:30>` (time after
  the weekday, before any repeater). Tine already *rendered* a time on planning
  timestamps; now you can enter one. Re-picking the date (or changing the repeater)
  keeps an existing time instead of dropping it, and an `×` clears the time. Ranges
  aren't supported (neither is in Logseq's planning timestamps).

### Fixed

- **Clicking right of a bullet that ends in a link now puts the caret after the
  link, not before it** ([#34](https://github.com/martinkoutecky/tine/issues/34)).
  Clicking past the end of a line whose last element is a `[[page]]`/`#tag`/link
  used to drop the caret at the start of that element; it now lands at the end of
  the line as expected.
- **No more "Tine crashed" coredump when closing the app on Linux**
  ([#28](https://github.com/martinkoutecky/tine/issues/28)). The app already closed
  cleanly, but WebKitGTK's renderer subprocess ran the GPU driver's exit-time
  teardown on the way out, which double-frees on many Mesa/driver combos (SIGABRT →
  coredump notification), even on plain Intel graphics. Tine now terminates those
  WebKit helper processes directly at quit — after saving — so the buggy teardown
  never runs. GPU-accelerated rendering stays on for the whole session (the
  `TINE_GPU=0` software-rendering fallback remains available but is no longer needed
  for this). Linux only.

## [0.4.3] — 2026-07-07

### Fixed

- **Org files: block ids are written as a hidden `:PROPERTIES:` drawer, not a
  visible `id::` line** ([#25](https://github.com/martinkoutecky/tine/issues/25)).
  On an `.org` page, parking a block (zoom / open in sidebar / new tab) or making
  a block reference used to append a Markdown `id:: <uuid>` line, which org renders
  as visible body text *and* which Logseq doesn't read back as the block's id.
  Tine now writes the id the way Logseq does in org — a `:PROPERTIES:` / `:id:` /
  `:END:` drawer at the canonical spot (after the title and any
  SCHEDULED/DEADLINE lines), extending an existing drawer in place. It's hidden
  from the rendered view and read back correctly, so it also makes zoom/sidebar/tab
  spots actually survive a restart on org pages (they previously couldn't). Markdown
  pages are unchanged.

## [0.4.2] — 2026-07-06

### Fixed

- **Restore the macOS and Windows-arm64 release builds.** 0.4.1 shipped without them: a
  repo-wide `rust-toolchain.toml` (added while setting up F-Droid) pinned a Rust channel that
  didn't carry the cross-compile targets the release CI installs, so those two cross-builds
  failed (every other platform, including the Android APK, was unaffected). Removed the pin;
  the Android/F-Droid build installs its targets explicitly instead. No app-behavior change.

## [0.4.1] — 2026-07-06

### Added

- **Summarize query results — count, sum, average, group-by.** The visual query
  builder gains a **∑ summarize** control: with no code, count the matched blocks,
  sum or average a numeric property across them, and/or break the results down by
  page or by a property. Sum/average parse the property as a number and report how
  many rows were skipped (blank or non-numeric). The full result list still renders
  below the summary. (This goes beyond Logseq, which does aggregation only through
  Datalog `:result-transform`.)
- **Switch a query to advanced (Datalog).** The visual query builder gains a
  **⚙ advanced** button that drops a ready-to-edit `[:find … :where …]` template
  with a commented cheat-sheet of every supported clause. Writing Datalog flips the
  query to the advanced engine automatically, and the "ran / ignored" note keeps
  mistakes visible. (EDN `;` comments are now honored, so the cheat-sheet lines
  aren't parsed as filters.)
- **Wider coverage for advanced (Datalog) queries.** The `[:find … :where …]`
  mapper now also understands `(page …)`, `(namespace …)`, `(page-tags …)`,
  `(scheduled)`, `(deadline)`, `(journal)`, and a field-aware `(between …)` —
  matching what the everyday `{{query}}` DSL already supports. Clauses outside the
  supported set are still listed as *ignored* rather than guessed.

- **Camera and voice memo on Android.** The mobile editor toolbar gains a camera
  button (take a photo or pick an existing image — it goes straight into the
  graph's `assets/` and inserts the image) and a mic button that records a voice
  memo (`.m4a`) into `assets/` and drops in an audio player. The mic asks for
  microphone permission on first use and shows a red pulsing stop button while
  recording.

- **Paste a URL over selected text to link it** ([#23](https://github.com/martinkoutecky/tine/issues/23)).
  Select some text, paste a URL, and Tine wraps the selection as a link instead of
  replacing it — `[text](url)` on a Markdown page, `[[url][text]]` on an Org page.
  It's skipped inside code and when the selection is itself a URL (a normal paste
  happens then).
- **One-click copy for code and links** ([#24](https://github.com/martinkoutecky/tine/issues/24)).
  Hovering a fenced code block, an inline `` `code` `` span, or a link now shows a
  small copy button that puts the raw source on the clipboard — the ease-of-life
  the `logseq-copy-code`/`logseq-copy-url` plugins add to Logseq, built in.

## [0.4.0] — 2026-07-06

The headline of 0.4.0 is that **Tine now runs on Android** — a native build that
reads and writes your real Logseq graph on the phone, sharing the same Markdown
files with Logseq over Syncthing. This release also folds in the whole 0.3.x
series (PDF export, task checkboxes, in-page find, time tracking, the theme
gallery, and more).

> **Installing on Android:** the APK is sideloaded and signed with Tine's own key
> (not a Play Store key), so Google Play Protect will warn that it "doesn't
> recognize this developer" — expand the dialog and choose to install anyway
> (some devices ask you to confirm with your fingerprint). That's expected for
> any app from outside the Play Store. Also, if your graph doesn't open on the
> very first attempt, **restart the app and try again** — a known first-launch
> hiccup we're still chasing.

### Added

- **Tine runs on Android.** A native Android build (Tauri v2) opens and edits
  your real Logseq graph. On first run, grant Tine "All files access", then pick
  your graph folder (e.g. your Syncthing-synced notes) — Tine reads and writes
  the same Markdown files as Logseq, so the two coexist on one graph. The file
  watcher runs in poll mode, so external edits (Logseq mobile, Syncthing) appear
  live.
- **Above-keyboard editing toolbar (Android).** While a block is focused, a
  toolbar docks above the keyboard with the keyboard-only actions — outdent /
  indent, move block up / down, soft line break, TODO, date, `[[ ]]` / `(( ))`,
  the slash menu, and hide-keyboard.
- **Android quality-of-life.** A real Tine app icon, an edge-to-edge layout that
  keeps the toolbar clear of the status/navigation bars, a hardware Back button
  that navigates within Tine (exiting only at the root), and mobile-tuned journal
  headers and settings.
- **Signed Android releases, built in CI.** Each tagged release builds a
  release-signed `Tine_<version>_android-arm64.apk` on GitHub Actions (arm64
  devices); the signing key lives only in encrypted CI secrets.
- **Built-in theme gallery.** Settings → Appearance now has one-click Default,
  Nord, Solarized, and Gruvbox cards, each covering both light and dark mode. The
  selected gallery theme is saved through Tine's backend app settings
  (`theme.gallery`), not WebKit localStorage, and applies as a managed
  `#tine-theme` layer before the user's `logseq/custom.css`, so hand-written graph
  CSS still wins.
- **In-page find on normal pages.** `Mod+F` opens a browser-style find bar with
  next/previous navigation, match counts, and non-destructive highlights. Matches
  come from the loaded block model rather than the mounted DOM, so text under
  lazy-rendered or collapsed branches is counted and the target branch is expanded
  before the active hit is scrolled into view.
- **Logseq-compatible time tracking.** Moving tasks into `DOING`/`NOW` clocks in,
  and moving them back to `TODO`/`LATER` or into `DONE` clocks out by writing OG
  `:LOGBOOK:` `CLOCK:` rows. The writer uses Logseq's local timestamp shape,
  English weekday abbreviations, default seconds mode, and the exact `=>  ` span
  spacing; elapsed badges on `DONE`/`TODO`/`LATER` blocks show recent CLOCK rows in
  a tooltip. The feature is gated by `:feature/enable-timetracking?` (default on).
- **Rendered copy is more faithful.** Copy / export → **Rendered** now preserves
  `$…$` / `$$…$$` math delimiters, pre-warms off-screen block refs before copying,
  resolves `{{embed}}`, `{{query}}`, and media/widget macros to sensible text forms,
  and adds a **Resolve refs fully** toggle for multi-line block refs. Query exports
  are capped and visibly marked when truncated; full math-typeset-to-plain-text is
  still tracked separately.
- **Sub-directory scan Phase 2 polish** ([#21](https://github.com/martinkoutecky/tine/issues/21)).
  Sync-conflict and duplicate-day journal scanners now recurse under `pages/` and
  `journals/` through the same page-file walker as the main scan, so nested
  conflict copies are surfaced. The Pages list also disambiguates basename
  collisions only when needed (`foo — client-a/`) and opens file-backed entries by
  graph-relative path, so colliding nested pages save back to their own files
  without creating a flat twin.
- **Logseq `--ls-*` theme CSS mostly works in `custom.css`.** Tine now seeds the
  common OG color variables and routes its own theme tokens back through them, so
  Awesome-Styler-style themes can recolor backgrounds, text, links, borders, bullets,
  selection, marks, and inline code while Tine's default light/dark themes stay
  visually unchanged. This is CSS theme compatibility only, not Logseq plugin support.
- **Pages in sub-directories are now scanned** ([#21](https://github.com/martinkoutecky/tine/issues/21)).
  Like Logseq, Tine walks `pages/` (and `journals/`) **recursively**, so pages filed
  into real sub-folders — e.g. archiving `pages/client-a/…` — appear in the page list
  and are searchable and linkable instead of being invisible. A nested page is keyed by
  its **file name** (`pages/client-a/foo.md` → page `foo`), matching Logseq, and edits
  save back to that file in place. Namespaces (`parent/child`) remain the flat
  `parent___child.md` filename encoding, not real folders — also matching Logseq.
  The file watcher also descends sub-directories now, so a page added in a sub-folder
  (or delivered there by Syncthing) while Tine is open appears live, without a reopen.

## [0.3.5] — 2026-07-05

### Added

- **Export a page to PDF.** Right-click a page title → **Export to PDF…** (or run
  **Export current page to PDF…** from the command palette). A pre-export dialog offers
  **collapsed blocks: expand / keep folded**, **font size**, and **margins**. Tine
  renders the whole page — not just the blocks currently on screen — to a
  self-contained document (the same lsdoc renderer as the HTML export, with images
  inlined as data URIs) and opens your OS print dialog, so you can **Save as PDF**. The
  PDF always prints on a **light** background (whatever your theme), embeds the Inter
  font it uses (so italic/bold render correctly — no garbled synthesized glyphs) and
  turns off `->`/`--` ligatures. No new dependency: it reuses the HTML export plus the
  webview's own print engine. See ADR 0021.
- **Sync-conflict merge.** Syncthing/Dropbox `*.sync-conflict-*` (and Dropbox
  `(conflicted copy)`) files are now kept out of your page list and surfaced under
  Settings → *Backups & recovery* → **Sync conflict copies**. **Review & merge** shows a
  block-by-block diff against the current page — matched by `id::`, then content,
  then first-line similarity — with per-block **keep-current / keep-copy / keep-both**
  and a page-property merge; **Discard copy** trashes it. Merges write through the
  normal (base-revision-guarded, atomic) save path and move the copy to the
  recoverable trash — never auto-merged, never unlinked. See ADR 0020.
- **Page icons on inline references.** A page's `icon::` (emoji/character) now shows
  as a prefix on inline `[[references]]` and `#tags` to it — matching Logseq (Tine
  already showed it on the page title and in the namespace listing). Emoji render as
  Twemoji SVG for WebKitGTK. Icons are fetched batched + cached, so an icon-less graph
  costs one lookup and no re-render.
- **Raw HTML now renders (sanitized).** Inline and block HTML embedded in a note —
  `<ins>`, `<del>`, `<sup>`/`<sub>`, `<kbd>`, `<mark>`, `<abbr>`, `<a>`, a self-closed
  `<img/>`, and small containers — renders live the way Logseq shows it, in both the
  app and the HTML export. It's sanitized to a shared, contract-tested allowlist:
  scripts, event handlers (`onerror=`) and `style` are stripped. (A *bare* `<img>` is
  literal in Logseq too — only a self-closed `<img/>` is raw HTML; and the Markdown
  carets `^x^`/`~x~` aren't sub/superscript in either app.) See ADR 0019,
  [#16](https://github.com/martinkoutecky/tine/issues/16).
- **Load local-file images (opt-in).** A new **Settings → Editing → "Load local-file
  images"** toggle (off by default) lets a raw-HTML `<img>` load an image from an
  absolute path outside the graph — for imported notes that reference local files.
  Read over a gated, image-only IPC; the HTML export never serves local files.
- **HTML export now renders task facets, queries, and embeds.** The static export
  (`public:: true` pages) previously dropped task markers/checkboxes, priorities,
  `SCHEDULED`/`DEADLINE`, and block properties, and left `{{query}}`/`{{embed}}`/
  `{{namespace}}`/`{{video}}` blank. It now renders all of them — queries and embeds
  are resolved against your graph **at publish time** — so a published page matches
  what you see in the app. A new **Feature showcase** page in the demo site exercises
  every page-level feature.
- **Graph switcher in the sidebar.** The active graph's name now shows in the
  sidebar header (under "Tine") as a clickable control → **Open graph…** (native
  folder picker) / **New graph…**. Switching graphs was previously buried in
  Settings; this surfaces it. (You can also start Tine on a specific graph from
  the command line: `tine /path/to/graph`, or `TINE_GRAPH=/path`.) A saved
  recent-graphs list is still to come.
- **Windows ARM64 and Linux ARM64 builds.** Releases now include `aarch64`
  installers for Windows (Surface Pro X, Snapdragon X laptops) and Linux (Asahi,
  Raspberry Pi / SBC) alongside the existing x64 builds — pick the one matching
  your CPU. Linux ARM is built natively; Windows ARM is cross-compiled. (These
  build starting with the next tagged release.)
- **Task checkboxes.** A `TODO`/`DOING`/`NOW`/`LATER`/`WAITING`/… block now shows
  a clickable checkbox in front of it (like Logseq): click it to mark the task
  `DONE` (checked), click again to reopen it (`TODO`, or `LATER` under the "now"
  workflow). A repeating task (`SCHEDULED`/`DEADLINE` with a `+1w`-style repeater)
  rolls forward to its next occurrence instead of closing, matching OG. The marker
  word stays next to the box and still cycles on click. `DONE` shows a checked box;
  `CANCELED`/`CANCELLED` show none (OG parity). Checkboxes also render on tasks in
  Linked References, query results, and embeds.

### Fixed

- **Sidebar "+ New page" button now works.** It was wired to nothing (a dead
  button on every platform) — it now opens the quick switcher, where typing a name
  that doesn't exist offers "Create…". (GH #20.)
- **Deleting an auto-inserted `[[]]` no longer strands `]]`.** With general
  auto-pairing off, typing `[[` still auto-closed to `[[]]` (always-on page-ref
  pairing) but Backspace didn't clean the closer, leaving `]]`. Backspacing between
  the brackets now removes both, matching the always-on insertion. (GH #19.)

## [0.3.4] — 2026-07-04

### Added

- **Settings → Help improve Tine.** A panel that runs Tine's parser (lsdoc)
  against Logseq's own parser (mldoc) on your graph, entirely on your machine, and
  reports where they disagree plus a parse-speed comparison. Divergence snippets are
  **anonymized** (your words replaced, markup structure kept) and **re-verified** to
  still reproduce the divergence before they're shown — so they're safe to paste into
  a bug report. mldoc is loaded only when you press Run (no startup cost); nothing is
  ever uploaded.

### Fixed

- **Priority `[#A]` chip now shows on query and reference results.** A task
  surfaced by a query (or in Linked References / an embed) that was rendered in
  the read-only path dropped its `[#A]`/`[#B]`/`[#C]` priority marker — so a
  `(priority A)` query could list a block without visibly showing its priority,
  while the same block elsewhere showed it. The read-only renderer now draws the
  priority chip, matching the live editor.
- **Scheduled/deadline date picker no longer jumps when paging months.** The
  picker's header (`September 2026 · Scheduled`) was too wide for the popup and
  wrapped to a second line on the longest months, shoving the day grid down a row
  (and back up on shorter months). The popup is a little wider now and the header
  is kept to one line, so paging through months is stable.

## [0.3.3] — 2026-07-04

### Changed

- **Consecutive same-page query results share one heading.** When a query is
  sorted, several results from the same page that land next to each other in the
  order now render under a single page heading, instead of repeating the heading
  once per result. A page whose results fall at different positions in the sort
  (e.g. an A and a C task under a priority sort) still appears at each of those
  positions, and a page's blocks keep their document order under the heading.
- **A block that fails to parse no longer breaks rendering.** The parser is now
  guarded per block: if the WebAssembly parser ever traps on some block, Tine rebuilds
  a fresh parser instance and retries; if that block still traps, it's shown as raw
  text with a subtle marker while every other block renders normally — instead of the
  whole view going blank until restart. (Defense-in-depth: lsdoc v0.4.1 has no known
  trapping input; this guards the unknown.)
- **Parser updated to lsdoc v0.4.1.** Two threads since v0.3.0: (1) a batch of
  edge-case byte-exactness fixes that bring parsing closer to Logseq's own on
  uncommon constructs — Markdown table-separator rules, LaTeX-environment tails,
  definition lists, front matter, footnote definitions, `>>`/nested blockquotes,
  Markdown comments, and inline backslash/backtick residue (so a handful of unusual
  blocks now render exactly as Logseq renders them, where before they differed); and
  (2) more `O(n²)→O(n)` parse-path fixes (raw-HTML tag index, `>`-quote fallback
  reparse, and the Markdown link-label scan), so pathological blocks parse fast.

### Added

- **Sort query results with one click.** The visual query builder's **Sort**
  control now leads with preset buttons — *Newest first / Oldest first*,
  *Priority A→C*, *Page A→Z*, *Deadline*, *Scheduled* — so the common orderings
  need no typing (a free-text field remains for sorting by any other property).
  *Newest first* places results on one timeline: journal pages by the day they
  represent (stable — not the file's modified time), other pages by when the file
  was last modified, so journal-page and ordinary-page todos interleave
  chronologically. These extend Logseq's property-only `(sort-by …)`.
- **Copy/export "Rendered" mode resolves block refs and macros.** Copying or
  exporting in *Rendered* mode now flattens a `((block ref))` to the referenced
  block's text and a user `{{macro}}` to its expansion, instead of the bare uuid or
  the literal `{{…}}` — so the copied text matches what you see. Math stays as TeX
  (which is what selecting rendered KaTeX copies anyway).
- **User `:macros` can expand to real blocks (OG parity).** A `config.edn` macro
  whose template is block-level Markdown — a heading, a list, multiple paragraphs —
  now renders as real nested blocks instead of a flattened inline line. Single-
  paragraph/inline macros still render inline. Unfilled placeholders (`$5` with only
  two args) stay literal, and arguments now come straight from the parser, so a
  quoted argument containing a comma is no longer split in two — all matching Logseq.
- **Headings stay heading-sized while you edit them (OG parity).** Clicking into a
  single-line `#`/`##`/`###…` heading now keeps the editor text at its heading size
  and weight (the `#` markers stay visible at the same size), instead of shrinking to
  body size on focus and jumping back on blur. Multi-line heading blocks edit at body
  size (only the heading's own line is enlarged), matching Logseq's uniline rule.
- **Select text, then wrap it (OG parity).** With text selected in the editor,
  typing `[` twice wraps it as `[[selection]]` and opens the page search seeded
  with those words — so Enter links it to an existing page or creates it (#18);
  `(` twice does the same for a block ref `((selection))`. Emphasis marks wrap a
  selection too: `*`/`~`/`=`/`_` (and the Org markers `/`/`+`/`^`), so a second
  press gives `**bold**`, `~~strike~~`, `==highlight==`. This is always on and
  independent of the opt-in auto-pairing (which only affects the empty-caret case).

### Fixed

- **Clicking a query's collapse arrow toggles it, instead of editing the block.**
  The ▸/▾ arrow — and the other query controls (the title, result-page links,
  table headers) — now run their own action on click and no longer fall through
  into raw-text edit mode of the query block.
- **Collapsed query builders no longer flicker.** On WebKitGTK, moving the pointer
  off the page and back could flash a varying subset of collapsed `{{query}}`
  boxes; each now sits on a stable compositing layer, so the compositor reuses its
  texture instead of re-rasterizing it.
- **Deleting today's journal leaves an empty today.** Right-clicking today in the
  Journals feed and choosing *Delete journal* used to blank the top of the feed;
  it now restores the empty, writable today placeholder — the same one you get on
  reopening the journal — so you can start writing again straight away (#17).

## [0.3.2] — 2026-07-02

### Added

- **Portable Windows build.** Releases now include a `Tine_*_x64-portable.zip` alongside the
  installer — unzip and run `Tine.exe`, no install needed (requires the WebView2 runtime,
  preinstalled on Windows 10/11).

### Changed

- **Parser upgraded to lsdoc v0.3.0.** The parser's `O(n)` single-pass rewrite is
  now vendored in the frontend, with crash fixes for adversarial input,
  parser-owned table alignment in the app, and support for `data:` image links.
- **Click edits, drag selects.** A click on rendered block content opens the
  editor at the clicked character (the position is captured at mouse-down, so
  it stays correct even when the layout shifts as the previously-edited block
  collapses back to its rendered height). A drag selects instead of editing:
  within one block it is a normal text selection of the *rendered* text (copy
  gives the glyphs you see — `→`, `–`); the moment it crosses into another
  block it becomes Tine's block selection. Deterministic by design — the
  behavior depends only on where the pointer went, never on timing (unlike
  Logseq's mousedown-instant-edit). Links, chips, media, and checkboxes keep
  their click behavior.

- **Copy/Export modal: Rendered / Source content toggle** (Rendered is the
  default — plain select-mode copy stays source). Rendered emits the text as
  displayed — typographic glyphs, entity unicode, no markup markers — from the
  parser's AST, honoring the link/tag/property remove options; Source is the
  previous raw-text behavior.

### Fixed

- **Click-to-caret in marked-up blocks.** Clicking rendered Markdown/Org markup
  now maps through lsdoc inline byte spans, so the editor opens at the clicked
  source position instead of falling back to the end of the block. This includes
  text with rendered arrows/dashes (`->` → `→`, `--` → `–`).
- Clicking a block below a focused taller-in-edit block (e.g. one with a
  `DEADLINE:` line) no longer loses the caret entirely.

## [0.3.1] — 2026-07-01

### Added

- **Automatic updates (Windows & Linux).** Tine now checks for a newer version on launch
  and can download and install it in place (Tauri's signed updater); a one-time *“a newer
  Tine is available”* toast appears when an update is found. macOS stays a manual download
  for now (unsigned builds). This is the first release with the updater built in — update
  to 0.3.1 once by hand, and future versions can update themselves.

- **Tab conveniences.** **Reopen the last closed tab** with `Ctrl+Shift+T`, and **cycle
  tabs** with `Ctrl+PgUp` / `Ctrl+PgDn` (all remappable in Settings → Keymap). Reopening a
  page — or relaunching Tine — now **restores each tab's scroll position**.

- **Editor typing polish (opt-in).** Optional **auto-pairing** of brackets and quotes, and
  **“on-type” typographic replacement** (`->`→→, `--`→–, `---`→—) with an Off / on-render /
  on-type switch (Settings → Editor). Inter's `calt` ligatures are turned off so asterisks
  and arrows keep a consistent height while you edit.

### Fixed

- **Up/Down caret navigation.** Arrowing into a `SCHEDULED`/`DEADLINE` bullet that also
  shows up in the journal **agenda** no longer loses the caret: the agenda copy stays
  *rendered* (it no longer steals focus or flips into an editor) while you edit the real
  bullet. Up/Down now also **preserve the caret's column** across blocks, matching Logseq,
  instead of snapping to the start or end of the line.

- **Journal feed navigation.** Pressing Down past the last loaded day pulls in the next
  journal day, and returning to a page loads enough of the feed to **restore your saved
  scroll position**.

- **Clicking into an empty block** no longer nudges it down a couple of pixels.

## [0.3.0] — 2026-06-30

### Added

- **Hover an image → copy / trash** (matches Logseq). Hovering an embedded asset now shows
  a small action bar (top-right): **copy** the image to the clipboard, or **trash** it —
  which removes the `![](…)` reference from the block and moves the file to the recoverable
  trash (`logseq/.tine-trash`), after a confirm. Graph assets only.

- **Native window controls** — Tine's window now fits in on each OS. On **macOS** the
  window gets real rounded corners and traffic-light buttons (a transparent overlay title
  bar) while keeping Tine's compact, single-row layout — no wasted title-bar row. On
  **Linux/Windows** a new Settings → Appearance toggle, *“System title bar & window
  controls”*, switches between Tine's built-in compact controls (default) and your OS's
  native window frame.

- **Spell checking in the editor** (WebKitGTK's native checker). On by default, like
  Logseq: red squiggles while editing, with right-click suggestions and “add to
  dictionary”, using the system `hunspell` dictionaries. **Beyond Logseq:** check
  **multiple languages at once** — Settings → Editor *discovers the dictionaries installed
  on your machine* and offers them as a tick-list (with human-readable names; no locale
  codes to memorize), and every ticked dictionary is checked simultaneously, so a word
  valid in any of them isn’t flagged (bilingual editing). None ticked follows your OS
  locale. The toggle and selection apply **live, without a restart** (Logseq needs a
  relaunch). Install more dictionaries with your package manager (`hunspell-cs`, …) and hit
  Rescan.

- **Richer static HTML export — sidebar + fuzzy full-text search** (closer to Logseq's
  published graphs). Every exported page now carries a persistent **left sidebar** with
  **Favorites** (from `config.edn :favorites`), **Journals**, and **Pages** sections and
  an active-page highlight, plus a **search box** that does **fuzzy full-text** matching
  over block content (vendored Fuse.js, tuned to Logseq's published-search params). Results
  show a page title + snippet and **deep-link to the matching block** (`page.html#anchor`) —
  every exported block now gets a stable anchor for this. The search index and page list are
  embedded as `<script>` globals and read locally (never fetched), so the exported site —
  including search — works **offline / opened straight off disk** (`file://`). Not yet
  included: Logseq's interactive graph view (a separate follow-up).

- **Org-style callouts on Markdown pages.** `#+BEGIN_NOTE / TIP / WARNING / …`
  admonitions now render as colored callouts on `.md` pages, not only `.org` ones
  (on Markdown they were previously mis-read as a stray `#tag`). Both the
  Obsidian-style `> [!NOTE] …` and the org `#+BEGIN_… … #+END_…` forms now render
  as callouts in either file format.

### Changed

- **Block rendering now parses Markdown/Org in-browser via WebAssembly** (the same
  `lsdoc` parser the backend uses, compiled to wasm). Rendering is synchronous, so
  there's **no more first-paint flicker** on opening a page, and the hand-rolled
  TypeScript inline/markdown renderer (~1,300 lines) is gone — one parser now drives
  both the on-disk index and the on-screen render, so they can't drift. No change to
  how anything looks or round-trips.

- **The HTML export renders through the same parser, too.** The static-export
  renderer now consumes lsdoc's canonical HTML skeleton instead of a second,
  hand-rolled Markdown renderer in the exporter — so exported pages match the app:
  code blocks, tables (with column alignment), callouts, and in-block lists all
  render faithfully, kept in lock-step with the live renderer by an anti-drift test.

### Fixed

- **Headings render more like Logseq.** A `# heading` block's larger font now applies to
  the heading's *own* line only — a `> quote` (or table, list, …) continuation in the same
  block renders at normal size again. And the bullet no longer **jumps** when you start a
  heading: while editing, the bullet stays put (the editor is plain-height); it only shifts
  to align with the larger text once rendered.

- **Parser rebuilt and upgraded (now lsdoc v0.2.5).** The Markdown/Org parser was
  re-architected into a proper single-pass parser — an explicit container stack, no
  phase worse than `O(n log n)`, gated byte-exact against Logseq's mldoc — replacing
  the earlier "optimistic" scanner that was quadratic on some inputs. Along the way,
  closer Logseq parity and hardened against
  pathological input. Corrected: lone-`\r`/CRLF left in content (Windows or pasted
  text), blockquote-with-marker text loss, a stray leading `|` being mis-read as a
  table (and inventing phantom block-refs), an org tag backslash-unescape, and an org
  property value mistaken for a page reference. Also fixes multi-second hangs and a
  couple of crashes on adversarial block content (e.g. long `[`/`>` runs). New
  Clojure-hiccup `[:tag …]` nodes render as literal text for now (an edge construct,
  absent from real graphs).

## [0.2.3] — 2026-06-28

### Changed

- **Settings reorganized into clearer categories** (modeled on Logseq's own
  General / Editor / … grouping). New **Editor** tab (file format, link-autocomplete
  default, copy-sub-blocks, strip-collapsed, click-ref-to-zoom) and **Files** tab
  (asset-name format, watch-for-external-edits, orphaned-media cleanup); "Journals
  & tasks" → **Journals** (now also holds first-day-of-week and the duplicate-day
  reconciler); **Backups** is now just snapshots/restore. The asset-name format
  field moved out of "Backups" and its preset/preview layout is tidied.

### Added

- **Expanded audio player.** An ⤢ Expand button on an inline audio embed opens a
  wide, dimmed overlay player: a **waveform scrubber** (click/drag to seek) with
  ±5s / ±15s skip, play/pause, playback speed, and a time read-out. Esc or
  click-away closes. (Replaces the old inline “⇔ Widen” seek-bar toggle.)
- **Configurable asset filenames** (Settings → Backups → *Asset names*). A
  `%`-token template controls how pasted/dragged/imported media is named in
  `assets/`: `%assetname %ext %yyyymmdd %hhmmss` (plus granular `%yyyy %MM %dd
  %HH %mm %ss`). The default is now the **plain original filename** (closest to
  Logseq for dragged files; collisions still get a `_N` suffix); a one-click
  *Date + name* preset reproduces the previous timestamp-prefixed scheme. A
  clipboard paste (no filename) falls back to a timestamp.
- **Selection follows the viewport.** Holding Arrow / Shift+Arrow in multi-block
  selection now scrolls the active end into view as it crosses the top/bottom
  edge (it never recenters while the block is already visible).

### Fixed

- **External media player no longer “opens then closes immediately.”** When Tine
  hands a video/audio file to the OS default player (e.g. VLC) it now scrubs a
  broader set of its own render env vars (`LD_LIBRARY_PATH`, `GST_*`, `GTK_*`,
  `GIO_*`, …) and detaches the child into its own process group with null stdio —
  so the player no longer inherits a broken GL/video context from Tine.
- **Dim-inactive-blocks (`t b`) now actually dims.** The fade previously only
  applied while a block was being edited, so toggling dim — or entering focus
  mode (`t f`), which turns dim on — looked like it did nothing. Dim now applies
  whenever it's on (the surface sits in a calm wash; the line you're editing pops
  to full opacity), and it now also fades the page/journal titles and the
  Scheduled & Deadline agenda, not just block content lines.
- **Accented & non-Latin tags render correctly.** `#café`, `#škola/úkol`, `#中文`
  and the like now render and link with their full name, matching how they're
  indexed — previously the renderer truncated at the first non-ASCII character, so
  `#café` linked to `caf`.
- **Empty `[[]]` is no longer a page reference.** `[[]]` / `#[[]]` stay literal
  text (as in Logseq) instead of creating a blank-named page, so the brackets from
  `[[`-autocomplete don't momentarily add an empty page to the index.

## [0.2.2] — 2026-06-28

### Added

- **Scroll position restored on back/forward.** Navigating away from a long page
  and pressing back (Alt+←) now returns you to where you were scrolled, like a
  browser — and switching tabs restores each tab's scroll too. A new page still
  opens at the top.
- **First-run onboarding + "create a new graph".** Starting Tine with no graph
  configured now shows a **Welcome** screen instead of a blank window: *open an
  existing Logseq graph*, or *create a new graph* scaffolded with a small narrated
  demo — a "Welcome to Tine" tour plus `Features/…` and `Project/…` pages that
  exercise block references, embeds, namespaces and tasks, and walk a newcomer
  through quick-capture (with how to bind the hotkey), slash commands, the command
  palette, the sidebar, PDF annotation and tabs. The new graph is ordinary Logseq
  Markdown (triple-lowbar namespace filenames) — it opens in Logseq too.
- **Block-reference parity round 2.** Right-click an inline `((block ref))` for a
  context menu (open in sidebar / go to block / copy ref / copy embed). The
  per-block references panel now shows each referrer's **ancestor breadcrumb** (like
  OG). In the editor, **`Mod+C` with no text selected copies a reference** to the
  current block. Copying blocks now also puts a **`text/html`** flavor on the
  clipboard (best-effort) so a paste into a rich editor keeps the outline nesting. A
  block embedded via `{{embed ((self))}}` no longer shows its own ref-count badge,
  and a `((non-uuid))` in prose is no longer counted as a reference (both match OG).
  New option (Settings → Journals & tasks): *click a block reference to zoom in*
  (Logseq) vs scroll-to-it-in-place (Tine default).
- **More OG macros.** `{{twitter}}` (alias of `{{tweet}}`), `{{vimeo}}` and
  `{{bilibili}}` (iframe embeds, accept a bare id or a URL), `{{img url [w h]
  [left|right|center]}}` (sized/aligned image), and **user-defined `:macros`** from
  `config.edn` — `{{name a, b}}` substitutes the comma-separated args into the
  template's `$1..$N` placeholders and renders the result as markdown (so a macro can
  expand to `[[links]]`, **bold**, other macros…). `{{youtube-timestamp}}`,
  `{{cloze}}` (degrades to click-to-reveal) and `{{zotero-*}}` render in a degraded
  form and say so (no on-page-player seek / SRS engine / Zotero connector).
- **Video drag-resize + audio "⇔ Widen" toggle.** Video now has the same corner
  resize grip as images (persisted as a `{:width N%}` brace). Audio — which has no
  fullscreen — gets a toggle that stretches the seek bar to the full column for
  precise scrubbing.
- **Image lightbox closes on Esc** (previously click-away only).
- **Linked/Unlinked references in the right sidebar.** Opening a page in the sidebar
  now shows its Linked & Unlinked References sections too, like OG (not just the page
  body).
- **Configurable copy behavior** (Settings → Journals & tasks), with a new
  "Differs from Logseq" row style — an amber badge + a one-line "Logseq behavior"
  note + a "↩ Match Logseq" button — for options whose Tine default intentionally
  diverges from Logseq:
  - *Copy a parent block's sub-blocks* — **default OFF** (Tine copies only the
    blocks you actually selected; selecting just a parent no longer drags its whole
    tree into the clipboard). Turn ON for Logseq's "always copy the sub-tree".
  - *Strip `collapsed::` when copying* — **default ON** (Tine drops this view-state
    property from copied text; `id::` is always stripped too). Turn OFF to match
    Logseq, which keeps `collapsed::`.

### Changed

- **Asset filenames are now `yyyymmdd-hhmmss-name`** (timestamp first, human-readable),
  so a plain name-sort in `assets/` is also chronological. (Was `name_yyyymmddhhmmss`.)
- **Inline block refs are link-styled, not a grey chip.** They keep the full-strength
  text colour with a thin accent-coloured underline and a link-coloured hover (OG's
  `.block-ref`), instead of the previous grey-text-on-grey-fill that was easy to miss.

### Fixed

- **Copy/cut no longer leaks `id::` into pasted text.** A referenced block carries an
  `id::` property; OG strips it when copying to the clipboard and now Tine does too.
  (The `id::` stays in the file — opening a block in the sidebar/new tab/zoom still
  stamps one so those spots survive a restart — it's just removed from the clipboard
  copy, exactly like Logseq.) Quick-capture keeps `id::` (it writes to a file).
- **Left sidebar "All pages" works on large graphs.** The page-count and the
  expandable list keyed off a one-shot fetch that raced a slow-loading graph and never
  retried; it now refetches when the graph finishes loading.

- **Namespace pages match OG.** The `{{namespace}}` macro now renders the bold
  **"Namespace"** label + root link header (then the bulleted descendant tree), and
  every non-journal page that's part of a namespace gets OG's automatic
  **"Hierarchy"** section below its blocks — a bulleted list with **one breadcrumb
  row per namespace level** (`[[Formula1]] / [[2026]] / …`), each segment a link to
  its cumulative path. Intermediate levels are synthesized, so a namespace with no
  file of its own (e.g. `Formula1/2025` when only `Formula1/2025/…` exists) still
  gets its own row — like OG's recursive listing. Replaces the earlier non-OG
  "Namespace (direct children)" list.
- **Page `icon::` is hidden from the property list** (it's shown as the title icon),
  matching OG.

- **Per-block reference count + referrers panel.** A block that's referenced
  elsewhere now shows a small count badge to its right (matching Logseq): click it
  to expand the list of blocks that reference it (grouped by page, same-page
  referrers included), or shift-click to open the block in the right sidebar. The
  count covers bare `((id))`, labeled `[text](((id)))`, and `{{embed ((id))}}`
  references. (Like the page-level linked references, it refreshes when the graph
  changes, not on every keystroke.)

### Fixed

- **“Copy image” from the image viewer works now.** Click an image to open it,
  then right-click → **Copy image** (or the **Copy** button) to put it on the OS
  clipboard. WebKitGTK's *native* right-click "Copy Image" doesn't actually
  populate the clipboard (paste yielded nothing); Tine now encodes the image and
  writes it through the Rust clipboard path instead.

- **The pinned-tab pin is back (the red 📌).** Bundling a color-emoji *font* made
  WebKitGTK paint the `📌` as a blank glyph (an empty gap on pinned tabs); emoji
  now render as Twemoji SVG images, so the red pushpin shows everywhere again.
- **Labeled block references resolve.** The `[label](((block-id)))` form — a link
  whose target is a block — now renders as a clickable block reference showing
  *label* (and navigates to the block, with a hover preview), instead of a dead
  link that tried to open `((id))` as a URL. The bare `((id))` form already
  worked; this is the labeled variant Logseq writes for *"copy as link"*.
- **Clicking a block reference jumps to the block.** A block ref now scrolls to
  and briefly highlights the referenced block (even when it's on the *same* page,
  where it previously appeared to do nothing) instead of only opening the page.
  **Shift-click** opens the referenced block in the right sidebar.
- **Block references export correctly.** The static HTML export now resolves
  `((block ref))`s (bare and `[label](((id)))`) to a link to the target block's
  anchor on its exported page, with the block's text/label — instead of the old
  broken `publish/((5cfb…` link with a stray `))`. Unresolved refs render as plain
  text, never a broken link. (The export parser is now paren-balanced too.)
- **Inline link/image targets are paren-balanced.** The `[..](..)` / `![..](..)`
  parser now counts parentheses when reading the target, so a URL that itself
  contains parentheses is captured whole — fixing not just block-ref links but
  any link/image whose URL has a `(`, e.g. `…/wiki/Foo_(bar)` or `img_(1).png`.
- **Math renders in the HTML export.** Exported pages now load KaTeX (and mhchem
  for `\ce{…}`) and wrap `$…$` / `$$…$$` as `\(…\)` / `\[…\]`, so equations
  typeset client-side instead of showing raw TeX. (Typesetting fetches KaTeX from
  a CDN, so it needs a network connection when the page is viewed.)

### Added

- **`{{namespace X}}` macro.** Renders the full nested descendant tree of a
  namespace (like Logseq), each page showing its `icon::`. Previously it was
  printed as literal text.
- **Page icons.** A page's `icon::` property now renders as an icon next to the
  page title and beside each page in the `{{namespace}}` tree, matching Logseq.
- **Emoji render everywhere (Twemoji SVGs).** Emoji — page `icon::`s, emoji in
  notes — now render as bundled **Twemoji SVG images** instead of relying on an
  emoji *font*. WebKitGTK paints a color-emoji webfont as a blank glyph (page
  icons showed as empty gaps), but an `<img>` renders in every engine. The SVGs
  are bundled locally, so it works offline.

### Fixed

- **Dark theme: native form controls follow the theme** (`color-scheme`), so the
  number-input spinners (e.g. *Carry last N days*, the agenda window) are dark in
  dark mode instead of white.
- **“Open in external player” works for video, not just audio.** Tine launched
  the OS player inheriting the environment it sets for its *own* WebKitGTK
  rendering (`LD_PRELOAD`, `WEBKIT_DISABLE_*`, `GDK_BACKEND`); under those a
  player’s video output could fail — e.g. VLC opened and closed immediately —
  while audio (no video output) was unaffected. The external opener now runs
  with those variables scrubbed.

### Added

- **Configurable `[[`/`#` autocomplete default.** Settings → *Journals & tasks* →
  **Link autocomplete default**: ON makes Enter **link the first match**; OFF
  (default, matching Logseq) makes Enter **create a new page/tag** unless an exact
  match exists. The other options stay one arrow-key away either way.

## [0.2.1] — 2026-06-27

A maintenance release: **namespaces round-trip with Logseq's default filename
format**, **graph switching fully resets the workspace**, **images are
drag-resizable**, and a batch of editor/sidebar/quick-capture fixes.

### Added

- **Drag-to-resize images.** Hover an image and drag the corner grip to resize
  it. The width is stored as a **percentage of the column** (so it stays right
  when the window or sidebar width changes) using Logseq's own image-metadata
  brace — `![](img){:width "40%"}` — written as a quoted EDN string so the same
  file renders at that width in Logseq too. (Logseq's own resize writes raw
  pixels; both round-trip.)
- **Quick-capture: optional page title.** The capture window now has a page-title
  field at the top — fill it to file the capture as a **new page**, leave it empty
  to **append to today's journal**. The "…to submit" hint shows your actual
  configured shortcut.
- **Sidebars are remembered across launches.** The left/right sidebar open/closed
  state and the right sidebar's items now persist (in the session file, since
  WebKitGTK doesn't keep localStorage), so Tine reopens exactly as you left it.
- **`[[` auto-closes its brackets** (`[[` → `[[]]`, caret between) like Logseq,
  and typing the closing `]]` types through them so you never end up with `]]]]`.
- **Open media in the default player.** Inline video/audio now has an
  always-available "open externally" button (shown on hover) — for when WebKit
  renders the player but can't actually decode the file.
- **Startup debug mode.** Run `TINE_DEBUG=1 tine` (or `tine --debug`) to write a
  timestamped startup trace — environment, milestones, panics (with backtrace),
  and the frontend's own boot/errors — to a file (default `/tmp/tine-debug.log`).
  Makes diagnosing a "won't start" report a single round-trip. See the README.
- **Software-rendering warning.** If Tine detects it's painting on the CPU
  (GPU acceleration unavailable — most often an AppImage whose bundled graphics
  libraries don't match your system), it shows a banner explaining why scrolling
  may feel slow and how to get the fast path back. Speed is the whole point; a
  silent fallback shouldn't read as "Tine is slow."
- **Smooth scrolling (experimental, opt-in).** Settings → Appearance →
  *Smooth scrolling* animates the journal feed to smooth out WebKitGTK's stepped
  mouse-wheel jumps. Off by default; a feel experiment, easy to switch back off.

### Changed

- **`/priority` now leaves a trailing space** so the next word or `/command`
  flows without manually adding one. The convenience space is never saved
  (trailing whitespace is trimmed, matching Logseq).

### Fixed

- **Namespaces round-trip with Logseq's default filename format.** Tine now
  honors `:file/name-format`: a graph without that key (Logseq's `:legacy`
  default) encodes the namespace `/` as `%2F`, and `:triple-lowbar` graphs use
  `___`. Before, Tine always used `___` and never decoded `%2F`, so a namespace
  page created in Logseq on a legacy graph showed up as a literal `a%2Fb` page
  (and vice-versa). Both formats now read and write the way Logseq does.
- **Switching graphs fully resets the workspace.** Opening a different graph now
  closes the previous graph's tabs (back to a fresh Journals tab) and clears its
  recents and right-sidebar items, so stale pages from the old graph no longer
  linger in tabs or the quick switcher — matching Logseq, which keeps one graph
  open at a time.
- **Quick-capture window is no longer too tall.** Its auto-grow is now capped at
  half the screen height (was 80%); short captures still size to their content.
- **Backspace no longer eats the space before a word.** Deleting the last letter
  of a word kept removing the preceding space too (so you had to retype it);
  the editor now keeps the trailing space while you type and only trims it on
  save, matching Logseq.
- **Sidebar editing.** The caret no longer vanishes after pressing Enter in a
  right-sidebar block (it stays in the surface you're editing), and the
  `[[`/`#`/`/` autocomplete dropdown is no longer clipped by the sidebar — it now
  renders above everything.
- **Click anywhere on a block row** — including the empty space beside or below a
  short line — now reliably places the caret in that block.

## [0.2.0] — 2026-06-26

The big one: **Tine now opens, renders, and edits Org-mode graphs**, gets real
**in-block lists & checklists**, learns to **embed video/audio and manage media**,
and handles **custom journal date formats** — on top of a round of data-safety and
performance hardening. Everything still round-trips your plain files; Tine never
takes over your graph.

### Added

- **Org-mode support.** Open, render, and edit `.org` pages and journals:
  headlines as blocks; org inline syntax (`*bold*`, `/italic/`, `_underline_`,
  `~code~`, `[[target][desc]]`); TODO markers; `#+BEGIN_SRC`/`QUOTE` blocks; org
  tables; `#+` page directives; inline timestamps; and admonitions/callouts.
  Mixed `.md` + `.org` graphs work, and the **File format** setting
  (`:preferred-format`) chooses what new pages/journals are created in. An `.org`
  file is only ever rewritten when Tine can reproduce it **byte-for-byte** —
  anything it can't round-trip loads **read-only**, so it can never corrupt an
  org graph.
- **In-block Markdown lists & checklists.** OG-faithful `-`/`*`/`+` bullets and
  `1.` numbered lists *inside* a block, plus GFM `[ ]`/`[x]` checkboxes that are
  distinct from TODO tasks. Caret-context editing (Enter continues the list,
  re-indents, etc.), and numbered lists that number the block itself the way
  Logseq does — with the `logseq.order-list-type` property kept invisible.
- **Video & audio embeds.** Insert media as assets with an inline player that
  **falls back to a click-to-open chip** when the platform lacks the codec
  (common on Linux/WebKitGTK).
- **Drag-and-drop files.** Drop files from your OS file manager onto a block to
  insert them as assets.
- **Media management.** Instant feedback when pasting an image; an
  **orphaned-media** scanner (Settings → Backups) that finds `assets/` files no
  block references and moves them to a recoverable trash (clickable names, file
  dates, empty-trash button). New assets get human-readable, timestamped names.
- **Custom journal date formats.** Tine now reads `:journal/file-name-format` and
  `:journal/page-title-format`, so graphs that previously *"wouldn't load"* (e.g.
  `dd-MM-yyyy`, `yyyy-MM-dd`, `yyyyMMdd`) open correctly; the display-title format
  is pickable in Settings → *Journals & tasks*.
- **Duplicate-day reconcile.** If two files resolve to the same day (e.g. a
  `2026_06_26.org` plus a title-named `Friday, 26-06-2026.org` left over from a
  date-format change), Tine keeps **both** rather than silently dropping one, and
  Settings → Backups → **Duplicate journal days** lets you reach each file:
  **Open** it (editable, saves back to itself), **Merge** a stray into the
  canonical day, **Rename** it to a normal page, or **Trash** the redundant one.
- **Calculator block.** An OG-style live, in-place calc block.
- **Sticky, closable toasts.** Notifications that need attention stay until you
  dismiss them.

### Changed

- The **agenda** (Scheduled & Deadline in the journal) hides `DONE`/`CANCELED`
  items, matching Logseq.
- `SCHEDULED:`/`DEADLINE:` are now detected **anywhere in a block**, not only on
  the first line — so the badge renders and agenda queries match either way.

### Fixed

- **Rename is transactional and complete.** A page rename + every
  `[[ref]]`/`#tag`/`tags::`/namespace rewrite across the graph commits
  all-or-nothing (re-checking each file just before writing, rolling back on
  conflict), handles self-references, and **leaves refs inside code fences and
  bare URLs alone**. Org `[[file:…][desc]]` link targets are rewritten too.
- Context-menu **"Rename page"** now works (the WebKitGTK prompt was a silent
  no-op).
- **CRLF line endings round-trip** — editing a Windows-authored file no longer
  flips every line and churns Syncthing diffs.
- **Linux AppImage**: a Wayland EGL crash is auto-fixed at launch (no manual
  `LD_PRELOAD` needed).
- Several editor caret/selection fixes (multi-line Shift+Down block selection,
  click-to-caret position, within-block Shift+Right).
- Removed the per-file confirm when trashing media (it's recoverable and
  batch-friendly).

### Reliability & performance

- A data-safety audit pass closed concurrency and round-trip issues across the
  rename, derived-result cache, and org write paths; Tine **never silently
  overwrites a file that changed on disk** — it surfaces a conflict instead.
- Inline parsing rewritten to be linear (was O(n²) on big blocks); the page cache
  and derived results are now `Arc`-shared; query/backlink invalidation is scoped
  to the pages that actually changed; per-block search/reference projections are
  memoized; and the launch backup is staggered off first-paint I/O.

### Notes

- macOS and Windows installers are currently **unsigned** — on macOS right-click →
  Open; on Windows choose *More info → Run anyway*.

[Unreleased]: https://github.com/martinkoutecky/tine/compare/v0.6.4...HEAD
[0.6.0]: https://github.com/martinkoutecky/tine/compare/v0.5.10...v0.6.0
[0.5.10]: https://github.com/martinkoutecky/tine/compare/v0.5.9...v0.5.10
[0.5.9]: https://github.com/martinkoutecky/tine/compare/v0.5.8...v0.5.9
[0.5.8]: https://github.com/martinkoutecky/tine/compare/v0.5.7...v0.5.8
[0.5.7]: https://github.com/martinkoutecky/tine/compare/v0.5.6...v0.5.7
[0.5.6]: https://github.com/martinkoutecky/tine/compare/v0.5.5...v0.5.6
[0.5.5]: https://github.com/martinkoutecky/tine/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/martinkoutecky/tine/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/martinkoutecky/tine/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/martinkoutecky/tine/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/martinkoutecky/tine/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/martinkoutecky/tine/compare/v0.4.7...v0.5.0
[0.4.7]: https://github.com/martinkoutecky/tine/compare/v0.4.6...v0.4.7
[0.4.6]: https://github.com/martinkoutecky/tine/compare/v0.4.5...v0.4.6
[0.4.5]: https://github.com/martinkoutecky/tine/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/martinkoutecky/tine/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/martinkoutecky/tine/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/martinkoutecky/tine/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/martinkoutecky/tine/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/martinkoutecky/tine/compare/v0.3.5...v0.4.0
[0.3.5]: https://github.com/martinkoutecky/tine/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/martinkoutecky/tine/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/martinkoutecky/tine/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/martinkoutecky/tine/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/martinkoutecky/tine/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/martinkoutecky/tine/compare/v0.2.3...v0.3.0
[0.2.3]: https://github.com/martinkoutecky/tine/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/martinkoutecky/tine/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/martinkoutecky/tine/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/martinkoutecky/tine/releases/tag/v0.2.0
[0.1.0]: https://github.com/martinkoutecky/tine/releases/tag/v0.1.0
