# Changelog

All notable changes to Tine are documented here. Tine is a fast, local-first
outliner that reads and writes a real Logseq Markdown (and now Org) graph.

The format follows [Keep a Changelog](https://keepachangelog.com/); versions use
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **`{{query (all-page-tags)}}` works.** Logseq has this filter — every page
  that carries at least one `tags::` value — and Tine did not know it, so a
  graph written in Logseq that used it showed nothing. It now selects the same
  pages Logseq selects.

- **Ctrl+Y also redoes, on Windows and Linux.** Ctrl+Z undid and Ctrl+Y did
  nothing, because Logseq binds redo to Ctrl/Cmd+Shift+Z and leaves Ctrl+Y
  unbound — so the key most editors on those platforms use for redo reached no
  command at all. Ctrl+Shift+Z is still the binding the shortcuts list shows and
  remaps; Ctrl+Y rides along as a second default, keeps one Redo row, and is
  dropped if you bind Redo to something of your own. Not on macOS, where Ctrl+Y
  is the system's yank inside every text field (GH #491).

### Changed

- **`{{query (property …)}}` now matches the way you read the page, not the way
  the bytes happen to be written.** A property value matches whatever its
  letter case — `status:: Done` answers `(property status done)`; a value
  written as a list matches any of its items, so `topics:: rust, queries`
  answers `(property topics rust)`; and a key whose values are all numbers or
  all dates is compared as a number or a date instead of as text, so
  `rank:: 007` answers `(property rank 7)` and a date property compares in
  calendar order. Logseq compares the raw text, case-sensitively, and splits
  only a handful of built-in keys — so a property query that found nothing in
  Logseq may well find what you meant here. Page names (`[[…]]`, `tags::`,
  `alias::`) were already matched case-insensitively and still are.

- **The Concord base ledger now reclaims entries whose stored text has been
  removed from underneath it.** Its prune already dropped unreferenced blobs and
  unreadable records; an index or conflict pin naming a blob that an antivirus
  quarantine, a disk cleaner or a partial restore had deleted was kept forever
  as dead metadata. Conflict diffs were never affected — a missing blob has
  always degraded to the ordinary two-column diff — and the ledger still never
  warns about one.

### Fixed

- **A query with a filter Tine does not understand now says so instead of
  quietly answering a shorter question.** `{{query (and (task TODO)
  (frobnicate x))}}` used to drop everything from the unknown filter onwards
  and run as `(task TODO)` — a list that looked right and was not. The unknown
  filter is now reported, the rest of the query is still shown, and no results
  are returned until it is fixed.

- **A property line with no value no longer crashes the reference reader.** A
  block whose last line is a bare `size::` — a property you have written the
  name of but not the value — could panic while Tine gathered the evidence
  behind a linked reference.

- **On Android, dark mode no longer blanks the notification bar.** Since 0.6.981
  the strip behind the status bar has been painted by the app window rather than
  the page, and it followed the phone's light/dark setting while the clock and
  icons followed Tine's own — so running Tine in dark mode on a phone still in
  light mode put white icons on a white strip. One setting now decides both
  (GH #467).

- **A conflict Tine cannot read no longer freezes the page.** When the
  comparison failed, the panel stayed on "Reading both versions…" forever and
  the page body went blank, so the page could not be used at all. The failure is
  now shown, with the reason, and the rest of the page keeps working (GH #490).

- **A page with an unresolved save conflict can be opened on disk again.**
  "Open with default app" and "Show in folder" both refused while a conflict was
  pending, which left a page whose conflict view would not load with no way to
  read it at all. Both now open the file as it stands on disk and say so; your
  unsaved changes stay in Tine until you resolve the conflict (GH #490).

- **Clicking inside a code block now puts the cursor where you clicked.** In a
  code block of any size, clicking anywhere in it opened an editor that looked
  blank: the cursor jumped to the very end of the block, and the editor was
  scrolled to the far right of its longest line, so none of the code you clicked
  was on screen. Both halves are fixed — a click maps to the character under it,
  and the editor opens showing that character (GH #489).

- **Clicking a link to the PDF you are already reading keeps your place.**
  Opening a PDF link with no page or highlight attached — a plain `![](…​.pdf)`
  asset link to the document already open in that pane — jumped the reader back
  to page 1 and cleared the highlight you were looking at. Reopening the
  resource you are already on is now a no-op; a link that does name a page or a
  highlight still navigates.

- **Zooming a PDF no longer throws you back to page 1, and your place in a PDF
  survives quitting and reopening.** A zoom briefly collapses the reader's
  scroll position, and the reader was recording that momentary position as
  "where you are" — so zooming jumped to page 1, and a reopened PDF often
  started at the beginning instead of where you left off.

- **The PDF reader's Fit width, Fit height, Area highlight, Notes and Outline
  are reachable again in a narrow reader pane.** Below 520px — a split pane, a
  companion pane, a phone — the toolbar correctly moved those five tools into
  the More-settings menu, but a CSS ordering mistake left the menu copies hidden
  as well, so there was nowhere left to click them. They now appear in the menu
  exactly when the toolbar is too narrow to show them inline.

- **The whole left-sidebar row opens its page again, and the right sidebar's
  spare width is now the drag handle it was meant to be.** v0.6.981 narrowed the
  wrong pane: it made only the page title clickable in the left sidebar, which
  turned a page named `test` into a target a few characters wide, and it left
  untouched the right sidebar, where the reported problem actually was. Both are
  corrected. In the left sidebar every pixel of a row navigates, including the
  blank space beside a short name — a deliberate divergence from Logseq, since
  reordering a favourite is protected by the drag threshold, not by keeping part
  of the row inert. In the right sidebar the title anchor no longer stretches
  across the row, so the empty space beside it belongs to the reorder drag and
  the hand cursor stops following the pointer out over nothing (GH #468,
  GH #464).

## [0.6.982] - 2026-09-04

### Added

- **"Insert block above" in a bullet's right-click menu.** The keyboard route to
  a bullet above an existing one is Enter with the caret at its very start,
  which splits the bullet — but a code block keeps Enter for adding a line of
  code, so a page whose first block was a code block had no way to get anything
  above it. The menu item works on any bullet, at any depth (GH #480).

### Changed

- **Managed and Direct reference and query reads now share one evaluator per
  question, and a Managed navigation request loads its pending overlay at most
  once.** Backlinks, unlinked references, block referrers, simple and advanced
  queries and query export each ran through a separate Managed copy of the
  Direct evaluator; they now run the same code over the same projected page
  forest, and the Managed reference reads reuse the projection the query path
  already retains instead of rebuilding one per read. Results, ordering and
  bounds are unchanged.

- **Sorting a sheet now derives each row's sort key once.** The key was
  previously recomputed inside the comparator, so a 200-row sheet parsed,
  looked up or read one about 12.7 times per row per sort. Sort order and
  displayed values are unchanged.

- **Several query builders on one page now share a single facets request.**
  Every mounted builder previously asked the same whole-graph question
  independently on each data revision; they now share one request per graph and
  data revision, and all of them still show the current answer.

- **Direct and Managed reads now share template, backlink, and block-UUID
  ownership rules.** Duplicate UUIDs consistently resolve to Logseq's
  parser-order owner across projection, fallback, pending, and drained states,
  while Managed adapters reuse the shared template and backlink producers.

- **Phase-A native commands now keep typed errors to the Tauri boundary without
  changing rejection bytes.** `CommandError` retains tagged, coded, I/O,
  worker, Tauri, and core sources behind one legacy-string serializer; the
  remaining native command files are explicitly assigned to W4-E2b.

- **Every remaining native command and helper now uses `CommandError`.** The
  phase-B conversion drives command/helper `Result<_, String>` and temporary
  stringify bridges to zero, retains JSON, durability, worker, plugin,
  clipboard, platform, and graph-verification source families, keeps every
  legacy rejection string byte-identical, and makes the cross-target guard
  absolute.

- **Backend refusal handling now branches on typed error kinds instead of
  English wording.** Native control-flow failures use bounded JSON tags, one
  frontend funnel constructs the existing error family, panic records retain
  only fixed-shape metadata, and scenario I/O errors preserve `ErrorKind`.
  Plugin-visible errors keep their previous string shape.

- **Diagnostics now keep graph content out of automatic output.** Native
  detailed diagnostics are actually debug-gated, remaining core print sites and
  frontend console calls have equality censuses, save refusals report only a
  count, and lsdoc-diff failures cross worker/report boundaries only as fixed
  offsets, lengths, and hashes rather than raw parser text.

- **Managed shared joins now commit through one generation-bearing marker.**
  Verified baseline and operation directories are published under the next
  generation, and cold open follows the marker to one complete pair while
  reclaiming interrupted candidates.

- **Core duplicate paths now share their canonical converters.** Static query
  lists and query-backed sheets hydrate from one source-graph boundary, hot and
  borrowed-state page materialization share one allocation-neutral block
  collector, and graph-keyed conflict capsules use the session key helper.

- **Sheets, serialized writes, and local screenshot harnesses now share their
  canonical implementations.** Board and table field rendering use the same
  facet readers, query DTO facets are memoized by DTO identity, shared-key
  frontend writes use one promise-tail shape, and local preview scripts retain
  their individual retry budgets behind one readiness helper.

- **The pinned `tine-storage` release now certifies every shipped platform.**
  Version 0.12.2 preserves complete plugin packages that contain harmless extra
  files, strengthens interrupted-install recovery checks, and gives Linux and
  Windows directory publication one audited implementation each. Its immutable
  receipt names Linux, Windows, Android, macOS, iOS, and API-semver jobs.

- **Direct Files property facets and PageRef queries now use the same SQLite
  read families as Managed Storage.** One shared plan-to-SQL lowering narrows
  PageRef candidates before the existing parser evaluator runs, while stale or
  unavailable projections keep the exact full-walk fallback. No schema, index,
  or query-result cache was added.

- Added internal release-only benchmark drivers for query, facet, invalidation,
  and managed save-versus-drain attribution; production behavior is unchanged.

- **Repeated performance work is cut from four storage and query paths.**
  Terminal SQLite bootstrap no longer encodes every replay intermediate into a
  dead map; managed projection completion carries its already-authorized plan
  instead of parsing and planning again; routed page tag queries share one
  request per graph revision across the toggle, table, and split panes; and one
  Query Builder instance shares a graph-revision-keyed facets fetch across its
  controls. Product behavior and persisted formats are unchanged.

- **Conflict resolution now scales with unresolved races rather than retained
  history.** A disposable accepted-sequence index tracks concurrent block
  pairs, exact pure-create projection candidates, and descendant settlement;
  it rebuilds from immutable accepted batches on reopen and leaves conflict
  semantics and persisted formats unchanged.

- **Managed-storage startup diagnostics now say which stage did the work, and
  how much.** `TINE_DEBUG=1` already printed a per-stage timing line for a
  managed cold open, but a slow stage could not be attributed to a mechanism.
  The open now also emits one content-free work-counter record — batches
  replayed, receipt evidence names and content reads, full-catalog passes,
  summary and own-completion chain reads, archive inspections — and splits the
  old journal-drain boundary into own-endpoint retirement scan and
  absence-decision-map open. Diagnostics only: nothing in the open path reads a
  counter, and no user-visible behavior changes.

- **Direct Files indexed queries now avoid the whole-graph parser walk when
  their generation-bound projection is ready.** Candidate selection uses the
  shared Managed/Direct lowering, so a `{{query}}` block on a page property,
  block property, page, namespace, or boolean composition no longer re-reads
  every page after every keystroke. A query whose candidate set is not
  selective — more than one thirty-second of the graph — keeps the parser walk,
  which is faster for those shapes; stale or unavailable projections keep it
  too. Both routes return the same answer.

### Fixed

- **`:ref/linked-references-collapsed-threshold` in `config.edn` is honored.**
  Tine already collapsed a page's Linked References once the backlink count got
  large, matching Logseq's rule, but the number was hard-wired to Logseq's
  default of 100 and your setting was ignored. Setting it to `0` now means what
  it means in Logseq: every page opens with the panel folded (GH #479).

- **A query no longer returns the block it is written in.** `{{query "xyz"}}`
  matched its own text, so the results listed the page the query lives on —
  which renders the query again, which lists the page again. Matching Logseq,
  the query's own block is left out of its results; blocks under it are not
  (GH #469).

- **Tab inside a block embed no longer throws the caret out of the embed.**
  Indenting, outdenting or moving a bullet inside a `{{embed}}` did the right
  thing to the outline but reopened the editor on the block's other rendering
  further down the page, so the following keystrokes landed out of sight. These
  three operations now keep the caret on the surface you were typing on, as
  splitting and merging already did (GH #477).

- **Query Builder dropdowns close when you click somewhere else.** Its clause
  menu and add-filter picker stayed open while you clicked away and started
  editing a different block, even though the sort and summarize popovers beside
  them closed correctly. Closing on an outside press now has one implementation
  that every popover in Tine shares, so this cannot differ per menu again; the
  `+ sort` button also toggles shut when you press it a second time (GH #472).

- **A context submenu stays on screen.** Opened near the right edge of the
  window — over a wide table, say — the `Show children as →` submenu was drawn
  past the edge and its items could not be reached. It now mirrors to the left,
  or overlays its own menu when the window is too narrow for either side
  (GH #471).

- **A page opened only in the right sidebar can be typed into.** An empty page,
  or one whose only content is its `key:: value` header, offered nothing to
  click and no place for the caret unless you first opened it in the main pane
  (GH #483).

- **Both copy buttons in the references sections sit on the same edge.** The one
  in Linked References floated in the middle of its row because it and the
  filter button each claimed the row's free space (GH #475).

- **A `[[link]]` stops looking dead the moment its name becomes an alias.** After
  you added `alias:: page1` to another page, every existing `[[page1]]` kept the
  faded missing-page styling until you restarted Tine — even though clicking it
  went to the right page. Tine now notices that the set of names which resolve to
  a page has changed, not just the set of files on disk (GH #484).

- **Journal-day queries such as `(between …)` work in Managed Storage on a graph
  with a custom journal page-title format.** Tine's Managed reader worked out
  which day a journal page was from the DEFAULT title format rather than the one
  your graph configures, so on any graph that sets `:journal/page-title-format`
  every journal page looked like it had no date and every journal-day query came
  back empty — while the same query over the same files answered normally in
  Direct Files. Both now read the day from your configured format.

- **Failure messages no longer reach the logs with your notes inside them.**
  Twelve always-on diagnostic lines used to print whatever a failed operation
  said — and a failed save, print, or conflict capture says it about the page it
  was working on, by name or by path. Each failure is still reported, and still
  just as visible; what it prints is now the failure's kind and a content-free
  identity for it. Two of the twelve, in the Direct Files projection, keep their
  full text behind `TINE_DEBUG`/`--debug`, so you can still ask for it; the ten
  in the app itself do not print the text anywhere, which also means a debug log
  you send us will no longer contain it.

- **A Tine that cannot find anywhere to store its application data now says
  why.** The fatal startup message names the reason (for example
  `PermissionDenied`) rather than only that something went wrong — you cannot
  relaunch past that message to go looking for the cause.

- **The diagnostics flight recorder releases its single-writer lock when it
  shuts down**, instead of leaving release to whenever the last copy of the
  file descriptor happens to close. A process spawn duplicates that descriptor
  into the child until it execs, so the lock could outlive the recorder that
  held it.

- **Managed clean-open failures no longer collapse source types into prose.**
  Core maps the 16 reachable error classes to stable, content-free reason codes
  and carries them through the existing open-refusal boundary as tagged JSON, so
  a refusal you can read and copy no longer carries a note name or a path. The
  reason itself still reaches Settings and the startup recovery pane: the
  diagnostic sanitizer renders the typed envelope instead of discarding it.

- **Direct save errors can no longer become discard-capable conflicts because
  of a page title or block text.** The save producer carries a closed reason
  code and conflict epoch through tagged JSON, and the frontend no longer
  reconstructs either value from error wording.

- **Managed activation now recovers after a process abort before its authority
  marker is published.** The marker remains the sole commit point; the next
  activation retires a wholly recognized unmarked generation and disposable
  SQLite projection, then rebuilds from unchanged Direct Files.

- **Internal guards and Managed diagnostics now fail on the regressions they
  name.** Source ratchets detect helper-hidden error parsing and grouped
  filesystem imports, platform coverage is enumerated, projection/conflict
  counters cover their real drain/rebuild boundaries, generation refusals are
  scenario-pinned, and skipped checkpoint captures retain a bounded cause
  without being mislabeled as recovery.

- **Closing or switching graphs no longer leaves a conflicted page's
  crash-recovery draft up to half a second stale.** The close barrier now
  lands every pending conflict-capsule refresh before it resolves.
- **A refused sync join names the affected notes again.** The typed
  `shared-frontier-mismatch` refusal carries the bounded list of differing
  relative paths (at most 32, never note content) that the join panel shows
  and the storage/sync contract promises; the previous typed-error change had
  dropped it.

- **Managed recovery moves and Windows backup restore close their publication
  crash windows.** Private-root archive moves now synchronize both renamed
  parents, and Windows recovery publication cannot replace a same-named entry
  delivered concurrently by a sync service.

- **Static publishing no longer exposes a private page whose malformed
  `public::true` line is rejected by Logseq.** Publication now uses the shared
  lsdoc-transcribed property recognizer, including its exact whitespace rules.

- Managed inventory now determines Page-versus-Journal identity from the
  decoded filename exactly like Logseq, rather than from the containing
  configured directory.

- **Bulk insertions now keep one storage authority from admission through
  publication.** Clipboard, quick-capture, template, HTML, and dropped-file
  insertions route through the shared storage front door; managed limit tokens
  retain their binding-generation re-check so a late result cannot land after a
  backend switch.

- **Android: saving a page in Direct Files works again** (GH #466). 0.6.981
  failed every Direct Files save on Android with "unknown: Permission denied
  (os error 13)", because page publication had been moved onto a
  hard-link-based no-clobber move that Android's shared storage refuses. Direct
  Files pages are published again with the same no-clobber rename 0.6.98 used
  on every platform, keeping the exact-byte checks around it; a refused
  filesystem call now names itself in the error instead of showing a bare
  errno.

- **Conflicted pages no longer rewrite their restart-recovery capsule for every
  save attempt.** Identical retained drafts skip the atomic envelope write, and
  changed drafts use a capsule-specific debounce so continued editing preserves
  the latest recovery copy without coupling it to the ordinary page-save timer.

- **Concord now uses the full pane width while you review a conflict.** The
  ordinary reading-width limit left both versions and their decision controls
  cramped and wrapping in a spacious window unless you happened to discover
  wide mode. A mounted resolver now uses that pane's wide content width
  automatically, then restores the usual reading width when the conflict is
  resolved; narrow and split panes keep their responsive layout.

- **A failed PDF area-highlight save no longer leaves an invisible nested crop
  behind.** If the PNG lands but the highlight sidecar transaction is refused,
  Tine now moves that exact crop into recoverable asset trash before reverting
  the optimistic highlight.

- **Unrelated backend errors containing the word “conflict” no longer trigger
  the stale-file recovery path.** Direct save conflicts are classified once at
  the native call boundary from their exact wire tag; UI code consumes the
  typed result instead of searching arbitrary error prose.

- Retired plugin-registry settings-cache keys are no longer parsed or migrated.
  They are disposable cache data: Tine ignores that old shape and fetches a
  currently signed registry, while preserving all unrelated app settings.

- **Harvest D — asynchronous results stay with the graph and editor that
  started them.** Quick Switcher creation/search, pinned-tab confirmation,
  asset imports and recordings, plugin calls, focus rescans, and Settings
  operations now revalidate graph-binding or operation ownership after waits;
  display-only repaint changes no longer invalidate plugin work, and desktop
  media-editor settings remain hidden on mobile.

- Managed Storage clean reopen no longer semantically reapplies every accepted
  operation since activation. A crash-durable disposable engine checkpoint now
  restores the accepted frontier and replays only its unpublished archive tail;
  damaged checkpoint bytes always fall back to full replay, while missing
  authoritative manifests or required objects remain visible as archive damage.
  Per-save checkpoint capture now extends that cache from its durable frontier
  instead of recapturing accepted history. Pre-0.7 sharing descriptors no longer
  have a production decoder, and a failure after activation authority is retained
  resumes from the preserved marker, baseline, and archive on the next open.

- **Harvest F — hostile graph and clipboard content now reaches its existing
  consumption boundaries instead of bypassing them.** Macro links can no longer
  navigate the privileged WebView directly, mobile blocks unknown schemes,
  formula/query/peek/backlink recursion has explicit bounds, and pasted HTML
  links open only through the native scheme allowlist. (Packet F's
  re-enabled paste-time Markdown escaping was undone on review: literal
  brackets in pasted text stay literal, per `UI-PASTE-BRACKET-LITERAL-001`.)

- **Harvest H — pre-release Managed private formats have one decoder, and
  concurrent theme changes no longer overwrite one another.** Schema-4 lazy
  genesis and schema-1 forensic records now invalidate their containing store
  so the existing preserve-and-rebuild lifecycle handles them atomically;
  theme package installs and removals serialize their shared settings array.

- **A crash during plugin installation or removal no longer wedges that plugin
  version until its files are deleted by hand.** Plugin packages now publish as
  one durably staged, no-clobber directory on every shipped platform; removal
  retires the complete directory before reclaim, and the next plugin-store open
  cleans interrupted staging, retirement, or incomplete package residue
  (internal B5p, Harvest sweep).

- Media (audio/video) assets returned 403 and never loaded when a graph runs under Managed Storage; the native media protocol now serves assets under both storage authorities with unchanged path containment and binding checks (internal B026).

- Managed Storage stopped accepting work at 4,096 lifetime-distinct page
  names, and at 4,096 lifetime blocks a save could report success and then
  leave the store permanently unable to open. Four internal fixed-capacity
  limits on run-local identity indexes were removed; the indexes now simply
  grow with the graph's lifetime history (internal A4, Harvest sweep).

- **Open tabs and pane state now survive a crash immediately after an autosave.**
  Session and workspace files now use the same durable atomic replacement
  protocol as other app-private settings, including file and directory
  barriers; bursts of tab actions are serialized, and the one-time legacy
  session move receives the same directory durability check.

- **A draft that met an external edit no longer disappears when Tine is
  restarted.** Direct Files and Tine-managed storage now retain unresolved live
  drafts in one graph-keyed, app-private atomic capsule. Reopening restores the
  exact draft before the graph becomes interactive; Managed storage observes
  its current owner again instead of reviving stale overwrite authority, and a
  completed resolution durably retires the capsule before Tine reports success.

## [0.6.981] - 2026-09-01

### Added

- **The graph menu now has a right-click menu on every graph row.** Opening a
  graph in a second window was reachable only by Shift-clicking a row, which
  nothing announced. Right-clicking a row now offers **Open in a new window**
  and **Open here**, plus **Show in folder**, **Copy path**, and **Remove from
  this list**. The row for the graph this window already has open keeps both
  open actions visible but inert and says why, so the menu does not change
  shape from row to row; mobile, which has neither peer windows nor a file
  manager, shows the actions that apply to it.

- **PDFs are now ordinary pane tabs instead of a separate global side pane.**
  Opening a graph PDF on desktop preserves the source and uses one reusable
  companion pane; the PDF tab can be moved into any existing pane, split, or
  quadrant, and its Notes action opens the `hls__` page in the structural
  companion. Workspaces restore the layout and each PDF tab's page/zoom, while
  legacy dedicated-pane sessions migrate without losing their existing panes.
  Android keeps one route surface and Back returns through Notes, PDF, and the
  source page in order.

- **Alt+click opens an internal page or block link in the other pane.**
  After the unified click contract moved Ctrl/Cmd+click to background tabs,
  ordinary links had no mouse+modifier route to a pane; Alt+click is that
  route again, matching the long-standing Alt+click / Alt+Enter gesture on
  Search and Quick Switcher results. It splits right when there is only one
  pane, and Shift+click (right sidebar), Ctrl/Cmd+click and middle-click
  (background tab) keep their existing meanings everywhere. (GH #438)

- **Declarative themes can now shape reading presentation without running code.**
  Theme API 0.2 adds bounded Tine-owned presets for editorial serif typography,
  journal headers, and a Today task summary, while existing color-only 0.1
  themes remain compatible and `logseq/custom.css` still wins the cascade.

- **Managed-storage group deletions now have an explicit recovery surface.**
  Tier 2 and Tier 3 absence sweeps raise a warning and remain available in a
  Deleted pages dock with their member pages and durable action status. Restore,
  Re-apply, and Keep deletion map directly to the recorded backend actions;
  failed restores retain their cause and can be re-run, while dismissing the
  warning or closing the panel never records a deletion decision.

### Changed

- **Settings remembers whether you maximized it.** The maximize control added in v0.6.95 reset on every open, so anyone who prefers the wide dialog had to press it again each time. Settings now opens at the size you last left it, on this device, across restarts. If you have never pressed the control, nothing changes ([GH #427](https://github.com/martinkoutecky/tine/issues/427)).

- **PDF rendering now uses PDF.js's maintained page-view lifecycle under one
  window-wide admission and canvas-memory budget.** Visible/focused pages win
  scheduling, stale work is cancelled, evicted views synchronously release
  their backing stores, and zoom above 300% sharpens only visible clipped tiles
  while retaining an aligned lower-resolution fallback. Area highlights use a
  dedicated clipped capture, and a tab cannot multiply the old per-view
  resource ceilings. (GH #393)

### Fixed

- **LaTeX math now renders inside italic and other emphasized Markdown.** A
  whole agent response can be italic without turning valid `$…$` or `$$…$$`
  fragments back into literal dollar text. The shared parser now preserves
  top-level-valid math through ordinary italic, bold, strike, and highlight
  containers while leaving invalid math, code spans, Org, and unrelated link
  labels unchanged. ([GH #460](https://github.com/martinkoutecky/tine/issues/460))

- **Typing a PDF page number and pressing Enter now stays on that page.** The
  input could jump the reader and then immediately restore the old page number
  when it lost focus, so the requested position was not retained or restored
  with the PDF tab.

- **Annotating a PDF while its notes page was being saved could freeze Tine
  completely.** The two writers of an `hls__` page took the same two internal
  locks in opposite orders, so each could end up waiting for the other. Because
  one of those locks is shared by the whole graph, the freeze was not confined
  to that page: once it happened, no further edit anywhere in the graph could be
  saved until Tine was restarted. Six write paths took the locks in the wrong
  order — PDF opening and annotation, the three conflict-resolution paths, and
  one projection-recovery path — and all of them now take them in the order the
  ordinary page save has always used. A check that walks the source now fails the
  build if any future write path reintroduces the inversion. Only debug builds
  ever reported this as an error; release builds, which is what everyone runs,
  froze silently, which is why it survived so long.
- **Clicking in the empty space after a block that ends in formatting now puts the caret at the very end.** On a block like `*some text in italics.*`, clicking past the last letter landed the caret between the letter and the closing marker you cannot see, so pressing Enter split the italics instead of starting the next block. The click is now read as "the end of this block" wherever it lands past the final glyph, for emphasis, bold, inline code and their Org equivalents alike; a deliberate click inside formatted text still lands exactly where you aimed ([GH #465](https://github.com/martinkoutecky/tine/issues/465)).
- **Modifier-clicking a bullet now sends the block where the same modifier sends a link.** Ctrl/Cmd-click (or middle-click) opens the block zoomed in a background tab, Alt-click opens it in the other pane, and Shift-click still opens it in the right sidebar; before this, only Shift did anything and every other modifier just zoomed in place. Reference bullets in linked references, query results and embeds answer the same four gestures ([GH #456](https://github.com/martinkoutecky/tine/issues/456)).
- **Shortcuts whose only modifier is Alt now work while you are editing a block.** A binding such as Alt+S for the next tab fired everywhere in the app except inside a block, where you had to add a second modifier; it now fires there too. Alt combinations you have not bound to anything still reach the text, so accented characters typed with Alt/Option are unaffected ([GH #461](https://github.com/martinkoutecky/tine/issues/461)).
- **Ctrl/Cmd+Enter in search opens the highlighted result in a new tab**, matching what Ctrl/Cmd-click on the same result already did, and leaves the search open so you can fan several results out in a row ([GH #463](https://github.com/martinkoutecky/tine/issues/463)).
- Sidebar: only the page title in a sidebar row is a link now, not the whole width of the row. The blank space to the right of a short name showed the hand cursor and opened the page, which made reordering favourites a coin toss — grab a row to move it, and it navigated instead. That space belongs to the drag now, and favourites show a grab cursor to say so. Right-clicking anywhere on a row still opens that page's menu ([GH #464](https://github.com/martinkoutecky/tine/issues/464)).
- Block references: the small reference-count badge now sits on a block's **first** line instead of dropping to the bottom-right corner once the block wraps onto several lines. It is a right-floating chip, and a float rides whichever line the browser is on when it meets it — emitted after the text, that was the last line. It is now emitted before the text, so it looks the same on a long block as on a short one ([GH #454](https://github.com/martinkoutecky/tine/issues/454)).
- Outline: a block's bullet now sits on the middle of that block's first line of text instead of 2px above it. The bullet's column was sized to the line height but not to the small padding above the text, so every bullet in the graph was slightly high — and twice as high under the editorial-serif typography preset, which makes the line taller. The line's height is now defined in one place and the bullet column follows it, so a typography theme moves both together ([GH #459](https://github.com/martinkoutecky/tine/issues/459)).
- Settings: choosing **Graph**, **Journals** or **Backups & recovery** no longer makes the whole Settings window flash away and come back. Those three sections fetch something as they open, and that was enough to tear down the dialog around them; the section list, the search box and the window now stay put while a section loads ([GH #409](https://github.com/martinkoutecky/tine/issues/409)).
- Diagnostics: Tine no longer claims it "did not close cleanly last time" every launch on iOS, iPadOS and Android. A phone or tablet reclaims a backgrounded app as a matter of course, and Tine counted that as a crash because it had no other way to tell that a mobile session had ended; it now treats going to the background as the end of the session and coming back as the start of a new one, so a crash you actually see is still reported ([GH #426](https://github.com/martinkoutecky/tine/issues/426)). The desktop half of the same report, the warning after an ordinary quit on Linux and Windows, is fixed separately in this release (below). One last warning is expected the first time you open this version, from the marker the previous one left behind.
- Sidebar: the left sidebar's scrollbar can be grabbed with the mouse again. The drag-to-resize strip along the sidebar's edge was drawn on top of it, so on Windows the sidebar could be resized but never scrolled by dragging; the strip now sits beside the scrollbar rather than over it ([GH #435](https://github.com/martinkoutecky/tine/issues/435)).
- Outline: clicking a block's fold arrow no longer folds the entire subtree. The guide line beside the children (which folds every descendant) was drawn on top of the fold arrow's leftmost pixels, so an aim that landed slightly left of centre hit the wrong control. The guide now stops at the arrow's edge ([GH #423](https://github.com/martinkoutecky/tine/issues/423)).
- Theme packaging: `tine-theme.mjs check` no longer certifies a ported theme that Tine then refuses to install. It checked only the upstream source, revision and author list, so an unsupported `portedFrom.ecosystem`, a missing `relationship`, `name` or `license`, or an unknown provenance field passed the registry check and failed on install. The checker now holds `portedFrom` to the same vocabulary the app installs against ([GH #410](https://github.com/martinkoutecky/tine/issues/410)).

- **Links to local files and folders now open.** A link written the Logseq way
  (`[Test](file://D:\test.txt)`) or the Obsidian way
  (`[Test](<file:///D:\test.txt>)`) rendered as a live link but did nothing at
  all when clicked: the backend refused every scheme but http/https/mailto, and
  the renderer discarded the refusal so nothing was reported either. `file:`
  links now go to the OS default application for that file or folder, matching
  Logseq, and any link that cannot be opened — a bad URL, a path that is no
  longer there — says so instead of failing silently. Every other scheme stays
  refused. (GH #444)

- **Editing-toolbar buttons on mobile no longer depend on a WebView emitting a
  click.** On iOS 27 the toolbar appeared but nothing responded to taps, while
  the same build worked on iOS 18.5: the buttons acted only on the click a
  browser synthesizes after a pointer press, and that press is deliberately
  cancelled so the editor keeps focus. The tap itself now performs the action,
  with click kept as the keyboard and assistive-technology path. (GH #434)

- **Long-pressing a page link on iOS or Android no longer raises the system
  text-selection bar over Tine's menu.** The same hold that opens Tine's context
  menu is also the platform's own selection gesture, which no amount of event
  handling on our side can call off — so page links and tags simply decline to
  be selected on touch. Menus themselves are now unselectable everywhere. On
  desktop you can still drag a selection across a link's text. (GH #452)

- **Dragging a block by its bullet no longer paints a blue trail behind it.**
  On macOS the drag doubled as a text selection and highlighted every block it
  passed over, which made the landing spot hard to read. The same guard the
  sidebar already used now covers the outline. (GH #424)

- **A wide table no longer drags its "Add row" label across the screen.** The
  control spans the whole table, so its centred label sat at the midpoint of the
  full table width — off to one side and travelling as you scrolled sideways.
  The row keeps its full-width click target; only the label is now pinned to the
  visible edge, the way the first column already is. (GH #449)

- **iPad now behaves like a tablet instead of a Mac.** The on-screen editing
  toolbar (indent, move, insert) never appeared while editing on iPad, and
  long-press, text selection and window chrome all took their desktop branch.
  iPadOS 13+ serves a desktop-class `Macintosh; Intel Mac OS X` user agent from
  a stock WebView, and Tine was reading the platform out of that string; it now
  comes from the build itself. Split panes stay available on iPad — they follow
  the size of the screen, not the name of the operating system. (GH #446)

- **A reported switch between Direct Files and Managed Storage now survives a
  crash at the selector boundary.** App-private storage-mode bindings use the
  same certified durable create/replace/retire primitive as other authority
  names, including Windows write-through retirement and Android parent-entry
  flushing. Once activation reports Managed active, a stale Direct selector can
  no longer reappear after power loss and take precedence at the next startup.

- **Rapid Managed Storage moves no longer reuse a stale source page and then
  flood “missing or foreign root” errors.** Repeated cross-day commands now
  resolve one at a time against the preceding accepted move, while the actor's
  temporary response-replay files are retired after the committed page pair is
  installed. A native journey drives 120 uninterrupted move commands across
  four journal-day boundaries and compares that pressure with a durable
  20-block cross-page cut/paste.

- **Direct Files saves now use durable write-through name publication on
  Windows.** Creating, replacing, restoring, and retiring the sole-authority
  Markdown/Org name all cross the certified typed storage boundary, so a power
  loss cannot be acknowledged merely because a non-write-through rename was
  briefly visible.

- **A second device can join a synchronized graph that contains an honest
  duplicate-name backup file.** A fresh scan may choose the earlier-sorting
  backup even though the shared history already owns the canonical page path.
  Join now accepts that shape only when the provider-owned exact file is still
  present with exactly the shared semantics and both physical files decode to
  the same page identity; the extra file remains untouched. The Android
  app-UID journey now covers activation, share, a distinct-device join and
  reopen, and the real graceful Return-to-Direct-Files composition.

- **Managed Storage has one production actor and enrollment path.** Dead
  pre-clean-runtime mutation, provider, cursor-join, and handoff state has been
  removed from the retained actor, and a source guard prevents those fields or
  types from returning to the production prefix of `sync_runtime.rs`.

- **Managed Storage pages remain editable after heavy use evicts them from the
  in-memory document cache.** A cold point load now reconstructs the page from
  its accepted history instead of mistaking the cache miss for an untouched
  page. Retained-runtime recovery also reports content-free native sub-stages
  and has a tighter real-corpus performance gate.

- **A remote `http:`/`https:` link whose URL ends in `.pdf` now opens in the
  browser like every other external link, instead of being captured by Tine's
  PDF viewer.** Only graph/local asset PDF references and their highlight
  pages enter the viewer; an image-syntax remote PDF renders as an ordinary
  external link as well rather than a broken image frame. (GH #442)

- **The PDF reader's Close control is now the terminal toolbar action.** It
  stays at the conventional far-right edge with an explicit accessible label,
  instead of reading like another tool in the middle of the control cluster
  (GH #443).

- **Bounded managed block-referrer panels now show the same document-order
  prefix and exact result count as Direct Files.** The SQLite-backed route
  finishes generation-bound candidate discovery before applying the shared row
  and byte budget, instead of truncating an internal-ID-ordered subset.

- **Diagnostic and graph-verification report exports can no longer be left
  partially written by a crash.** User-selected JSON destinations now use the
  same temp-file, file-flush, atomic-replace, and directory-flush publication
  family as other small durable outputs.

- **Returning to Direct Files and restoring a backup now fail closed on real
  directory durability errors.** Graph-local managed state set-aside and
  rollback flush both changed parents in recovery-first order; the separate
  capability-bound backup stack now durably reserves recovery directories,
  retires live names, and publishes restored names before acknowledging them.

- **Managed storage no longer reparses every baseline page when rebuilding its
  disposable SQLite projection.** New baseline capsules carry a bounded,
  versioned semantic receipt that is verified and reused during a healthy
  rebuild. Existing receiptless baselines, oversized pages, parser upgrades,
  and invalid receipts retain the prior exact-source reparse-and-compare path.
  Foreground one-block saves also reuse exact bounded whole-document outline
  parses, keeping both the first and steady post-drain save within two such
  parser invocations; drains perform none.

- **Managed storage now opens before its search indexes finish building.**
  Both Unicode and CJK substring indexes are built in bounded background turns;
  searches remain complete through an explicit slower fallback and show
  “Search index building…” until readiness. Live edits are caught up before the
  marker flips, and later one-block saves update only that block's search rows
  instead of rewriting its whole page.

- **Managed storage no longer forces a durable SQLite cache sync for every
  accepted event or schema statement.** The disposable WAL projection now uses
  `synchronous=NORMAL`, creates its schema in one atomic transaction, and keeps
  durability at the existing explicit checkpoint and atomic file-set
  publication boundary. A lost or corrupt cache still rebuilds from the
  immutable baseline and accepted operation history.

- **A crash during managed archive publication can no longer strand an edit
  whose exact bytes are still durable in the local journal.** Cold open repairs
  only torn object names covered unambiguously by an undrained local record,
  under the workspace's sole-writer lease, and then performs the ordinary full
  archive validation. Uncovered corruption and torn manifests still refuse
  activation. Public archive publishers remain strict; the local drain installs
  recoverable object names before its batch-wide flush, then installs the
  already-durable manifest and only afterward may checkpoint the journal. The
  enforced 10/13 save and cross-page-move barrier totals do not change.
- **Android keeps the right-sidebar control one tap away at ordinary phone
  widths and keeps Tine outside the system bars.** Lower-priority topbar actions
  still move into `...` when space is constrained, while the right-sidebar
  button remains direct until the last-resort tier; native status, navigation,
  and display-cutout insets now bound the WebView itself. Android also treats
  that native viewport as the sole inset owner, so OEM WebViews that expose CSS
  safe-area values do not add a second wasteful band above the topbar (GH #205).

- **Android no longer treats a refused directory durability barrier as success
  for promoted managed-storage projection receipts.** Only the receipt store's
  pre-enrollment initialization remains reconstructible; bases, intents,
  attempts, mutation authority, completions, cleanup, and forensic records now
  keep strict private-authority barriers on every platform. Each process now
  verifies a promoted parent once before accepting an existing receipt name or
  operational namespace, so a crash cannot erase knowledge of a refused
  barrier; later same-process names and ordinary reads add no barrier. Android
  devices that cannot provide app-private directory durability may initialize
  the reconstructible empty store, but managed operations now refuse rather
  than claiming unsafe success.

- **Images changed by Syncthing, Dropbox, an external editor, or another Tine
  window now refresh in place without reopening the graph.** Tine observes the
  approved `assets/` directory separately from page reconciliation in Direct
  Files and managed storage, including an approved external-assets target.
  Asset bytes remain ordinary filesystem-synchronized files and never enter the
  managed oplog or `.tine-sync`; open PDF/audio/video sessions are not replaced
  mid-use and see new bytes when reopened.

- Windows updater failures now leave a privacy-safe stage and cause in
  Diagnostics, with a sanitized detailed chain available in debug mode. A
  32-bit Windows build also links to the manual package instead of offering an
  automatic install that its signed updater manifest does not publish (GH
  #241). The reporter's underlying native network failure remains under
  investigation.

- Block background colors no longer crowd their bullets. Regular dots and
  numbered-list ordinals now occupy the same 22px control track, while the
  rounded highlight keeps text aligned with an unhighlighted or edited block.

- **Same-structure external Markdown content edits no longer leave a managed
  page stale or raise `hot_source_join` merely because Tine reached it through
  a page-name route** (GH #397). Application-page routes now
  agree on current source content while managed reconciliation is still in
  progress. Structural edits such as adding, deleting, or moving bullets still
  wait for the managed watcher to reconcile them safely.

- Restoring deleted pages from the recovery panel no longer closes the panel
  by itself the moment the restore completes: the disposed sweep stays visible
  as actionless history until you close it.
- Quitting Tine normally no longer shows a false "Tine did not close cleanly
  last time" warning on the next launch. The clean-shutdown marker was cleared
  in code placed after the app's event loop, which never runs, so every quit
  since the flight recorder shipped was reported as a crash. This is the
  desktop half of [GH #426](https://github.com/martinkoutecky/tine/issues/426).

## [0.6.98] - 2026-08-27

### Changed

- **Managed-storage projection recovery is prepared for cheaper durable
  publication.** Every projection-only producer now records one durable,
  description-only turn before graph mutation, while foreground saves replay
  through the same turn-level executor. Existing projection receipts remain as
  redundant recovery evidence, startup drains semantic edits before projection
  turns, and a disposable per-page SQLite digest avoids rendering an unchanged
  page merely to prove it needs no terminal repair. Durability-barrier budgets
  remain 25 for a save and 74 for a cross-page move.

- **The shared parser is updated to lsdoc v0.5.6.** Raw HTML recognition now
  indexes every supported tag in one source pass, static HTML export renders
  safe Hiccup vectors as markup, and deeply nested projections serialize
  iteratively instead of consuming the native stack. Native and browser-WASM
  consumers use the same released parser and serialization contract.

- **Startup no longer loads the complete Settings implementation before the
  first page appears.** Settings and its plugin, theme, backup, and diagnostics
  controls now load only when opened; the small journal-conflict row shared with
  Concord remains available independently. Native startup is back within the
  immutable v0.4.7 performance budget.

- **Managed storage's private receipt-store format has been bumped**
  (development only). Projection records now carry an explicit target-kind
  discriminant, so a private store created by an earlier build is refused with a
  named notice telling you to re-activate managed storage for that graph. Your
  Markdown files are untouched by the refusal — the check runs before anything
  can modify the graph. Managed storage has not shipped, so no released build
  can have created such a store; this affects development machines only.

- **Saving a page performs fewer disk-durability round trips** (28 → 25
  core-initiated barriers per accepted single-block save; 77 → 74 for a
  cross-page move). Three barriers defended no failure Tine's threat model
  covers: a re-flush of a file that was already flushed before publication, a
  durable queue-state write on an empty cleanup queue, and a flush of a file
  about to be moved aside whose exact bytes are already recorded. Edits feel the
  difference most on slow, networked, or synced filesystems.

### Added

- **Keyboard Shortcuts can now be searched in place** (GH #380). The existing
  Settings search field filters command names, IDs, and bindings while that
  section is open, including built-in shortcuts and a clear no-results state.

### Fixed

- **Built-in themes now recolor the main page in dark mode** (GH #401). The
  no-flash startup frame now yields its high-specificity viewport colors to the
  active theme tokens after startup, so Nord, Solarized, and Gruvbox apply
  consistently to the page and sidebar while `custom.css` remains the final
  user override.

- **An exact `#` or `[[` page match no longer hides other matching pages**
  (GH #186). The exact page remains the first/default result and suppresses the
  redundant Create row, while prefix, substring, and fuzzy matches remain
  available in the existing bounded literal autocomplete pool.

- **A journal-feed read failure is no longer misreported as an empty graph**
  (GH #385). The initial Journals view now shows the actual bounded backend
  error, while a transient refresh failure still keeps an already visible feed
  intact and retries later. This makes cloud-filesystem and access failures
  diagnosable without mistaking present journal files for no journals.

- **Unlinked References no longer falls back to a whole-graph scan while an
  ordinary edit's one-page SQLite update is already in flight** (GH #400).
  Reference reads wait for that bounded background delta, then use the exact
  current candidate set; an unavailable or failed disposable projection still
  falls back to the parser. Exact Direct Files byte conflicts remain unchanged
  and continue to preserve both the disk version and the unsaved draft.

- **Long pages no longer grow and shrink when a block enters or leaves edit
  mode** (GH #390). Pane-relative end-of-page breathing room is now derived
  from whether the page content naturally overflows, rather than from the
  transient presence of a textarea, so its scroll geometry stays stable.

- **A held mouse click now shows the block caret on mouse-down, matching
  Logseq** (GH #368). Exact rendered-text caret placement still applies, and a
  drag can still select text in the editor or escalate across blocks.

- **Android and system-decorated windows keep frequent top-bar actions directly
  visible whenever they fit** (GH #205). The overflow threshold now accounts
  for whether Tine's three custom window controls actually occupy the row, so a
  390px phone no longer hides calendar, journals, theme, and right sidebar in
  the `...` menu while leaving usable space.

- **Copying the in-app Guide no longer stops after its first page** (GH #391).
  Guide pages now carry the same completed-write receipt as ordinary Direct
  Files edits, so Tine's native watcher recognizes its own multi-page copy
  instead of treating page one as an external change that blocks page two.

- **Creating pages on Windows no longer lets a filesystem callback interrupt
  the very Tine write that caused it** (GH #374). The earlier v0.6.97 repair
  proved exact file-event echoes, but an ambiguous Windows callback could still
  raise the graph-wide external-change frontier before waiting for the active
  writer. Callback admission now serializes behind graph-text publication;
  genuine ambiguous or external changes still go through the normal debounced
  reconciliation before later creations are admitted.

- **Rapid scrolling and zooming no longer make PDF pages compete for the UI
  thread or flash blank while sharpening.** The reader now has one
  viewport-prioritized full-page render scheduler, drops obsolete prefetch work,
  pauses speculative rendering during fast wheel bursts, and defers selectable
  text-layer construction until scrolling settles. A page's previous bitmap
  remains visible until its sharper zoom replacement is complete. The longer-term
  tile-aware viewer architecture is tracked in GH #393.

- **Long PDFs no longer skip pages or overload the viewer during ordinary
  scrolling.** Evicting an offscreen page's canvas could collapse its flex
  wrapper, shrinking the document while it was being scrolled and making many
  distant pages appear visible and render at once. Page placeholders now keep
  stable geometry, so scrolling stays sequential and rendering remains local to
  the viewport.

- **F-Droid can rebuild Tine's browser parser from source again** (GH #392).
  The logbook code shared with the small WASM wrapper had started reaching into
  a tine-core-only module, while Tine's own release path kept using the already
  generated WASM bundle. The property recognizer is now one dependency-free
  source module shared by both crates. Pull requests and full release CI now
  rebuild the WASM bundle from clean source. A daily monitor also files or
  updates an Inbox issue when F-Droid's public auto-update pipeline reports a
  new failure.

- **Unlinked References now opens when a matching source page contains 10,000
  or more blocks** (GH #388). Managed storage was applying the whole-page
  editor payload limit before constructing the already-bounded reference
  result. Large source pages now take one linear authenticated read while the
  panel keeps its existing row and byte limits.

- **PDFs no longer flash and disappear when two render triggers reach the same
  page together** (GH #275). Visibility and navigation could both begin before
  the first PDF page lookup finished, causing PDF.js to reject two simultaneous
  renders into one canvas. Each page now has one render owner from before its
  first asynchronous lookup through completion, zoom, eviction, or teardown.

- **The block you are typing in stays above the Android keyboard toolbar**
  (GH #384). Tine now accounts for its own toolbar when focus moves, the block
  grows, or the keyboard viewport changes, instead of letting the active text
  remain hidden behind the fixed controls.

- **Pressing Back beyond the first Android page now closes Tine instead of
  leaving a gray, unusable screen** (GH #386). Storage was already stopped
  safely, but the final handoff called a Tauri command that does not exist.
  Android now uses the installed process-exit API after the same guarded save
  and shutdown checks.

- **Enter at either side of an in-block line break now creates a clean new
  block** (GH #361). The first fix handled the caret at the start of line two,
  but pressing Enter at the end of line one still copied the newline into the
  new block as an empty first line. Both caret positions now consume the same
  structural boundary on desktop and mobile while preserving intentional blank
  lines.

- **Android page links now open their page menu consistently on long-press**
  (GH #207). Body wiki links, links and page headers in Linked References, and
  page results in search now distinguish Tine's deliberate hold from the
  browser's native text-selection gesture. Quick taps, scrolling, and desktop
  click gestures are unchanged.

- **The keyboard-shortcut recorder now captures Control on macOS** (GH #378).
  Physical Control is recorded as `ctrl`, while portable `mod` remains Command
  on macOS and Control on Windows/Linux.
- **Pages no longer become narrow or jump sideways when you start editing or
  expand references** (GH #382). The pane-relative end-of-page space added in
  v0.6.96 accidentally let the centered page column shrink to the width of its
  current contents. Standard pages now hold a stable reading width, and Wide
  mode fills the pane as intended. Settings → Appearance → Advanced also lets
  you tune the standard width or give Wide mode a custom maximum on this device.

- **Saving a page in a subfolder no longer costs extra waits for the disk.**
  Every time Tine wrote or renamed a file in your graph, it asked the operating
  system to confirm not just the folder it had actually changed, but every
  folder above it up to the graph root. A page in `pages/Work/Notes.md` therefore
  waited three times where a page in `pages/Notes.md` waited once, on roughly six
  file operations per save. Only the folder that actually changed is confirmed
  now; a folder Tine has just created is still confirmed at the moment it is
  created, so a crash cannot lose the path your page was written into. On
  Managed Storage an ordinary edit now waits on 28 disk flushes instead of 37,
  and a cross-page move on 77 instead of 93 -- measured both on a test fixture
  and on a 1,045-file copy of a real graph.

- **Managed Storage saves wait on fewer disk flushes.** Each accepted edit used
  to write four small bookkeeping files whose only job was to notice if
  something had swapped out a folder inside Tine's own private data directory --
  which nothing but a program already running as you could do, and which Tine
  does not defend against by design. Each of those files cost two waits for the
  disk to confirm, on every page an edit touches. They are gone, and Tine now
  simply recreates the folder if it is missing instead of refusing to save the
  page forever. An ordinary edit now waits on 37 disk flushes instead of 45, and
  a cross-page move on 93 instead of 109. On a slow or network-backed disk that
  wait is the bulk of the delay between finishing a keystroke and the file being
  safely on disk.

- **Opening and scrolling the Journals feed no longer walks the whole graph on
  Managed Storage.** The feed was assembled by enumerating every page in the
  graph and then loading the handful of days it actually shows -- on every
  open, and again for each three-day step as you scroll back. The runtime now
  keeps the graph's journal days indexed and rebuilds that index only when
  something is actually accepted, so a feed page costs a lookup plus its own
  page loads. Going back to Journals with nothing changed in between does no
  graph-sized work at all, and the whole request is now answered in one
  runtime turn instead of one per page.

- **Managed Storage saves stop paying a disk round trip per internal file.**
  Every accepted edit fanned its history out into several separately-flushed
  private files, and each flush is a real device round trip: one single-block
  save performed 55 of them and a cross-page move 130. On a local SSD that is
  tens of milliseconds; on a slow, network, or phone filesystem it is what makes
  an edit feel heavy. An accepted edit's history is now written and made durable
  as ONE unit -- 45 and 109 -- and three flushes that ran before *reading* a
  file, which could not affect what was read, are gone. Crash safety is
  unchanged and is now stated and tested: a save interrupted by a crash or power
  loss is either fully recorded or not recorded at all, never half-written, and
  the graph still reopens and completes the same edit.

## [0.6.97] - 2026-08-26

### Added

- Added a read-only synchronized-graph verifier that compares the actual Markdown and Org file bytes across devices, including nested and nonstandard layouts, without requiring shell or ADB access.

### Changed

- **A fault one core test injects can no longer fail an unrelated test beside
  it.** The shared-provider "this filesystem has no `renameat2` flags" fault was
  armed process-wide, so under a threaded `cargo test` the injected errno was
  visible to every other test renaming a provider file at that moment; the
  whole-suite failure set differed on every run. The fault is now scoped to the
  thread under test and handed explicitly to the sync actor for the one request
  that needs it, so the corpus reports the same result twice in a row (GH #350).

- **Queries and search are much faster on Managed Storage.** Every managed
  query used to re-read, re-parse, and structurally cross-check every candidate
  page, every time -- so a page full of `{{query}}` re-parsed the graph on each
  open, and search re-parsed it on each keystroke. Unchanged pages now reuse
  their complete parsed application view after their exact file bytes and
  stored page state are checked; an external edit or accepted actor change is
  re-read immediately. Searching for a literal phrase also consults the stored
  text index first instead of reading every page, and a result-limited search
  now does its work only for the results it keeps.

### Fixed

- **Task markers mean the same thing everywhere.** The editor, carry-forward,
  the sheet state/priority writers, and the rendered checkbox now share one
  leading-marker recognizer that is byte-faithful to the parser: a task written
  as `TODO<tab>x` no longer counts as open for carry-forward while rendering
  without a checkbox, cycling or setting state on an indented task replaces its
  marker instead of prepending a second one, and a bare `TODO` followed by
  continuation lines is no longer treated as a task the renderer cannot see.
- **`key:: value` property lines are recognized the same way by every tool.**
  Logbook insertion, managed template/content checks, rename, conflict handling
  and the read index now share the parser's exact property-line rule, so a
  dotted key like `logseq.order-list-type:: number` is treated as a block
  property everywhere (a logbook entry could previously be inserted above it),
  leading whitespace and Unicode keys behave identically, and `key::value`
  without a space is (as the parser already said) not a property.
- **Page-link gestures are the same everywhere.** Middle-click now opens a
  background tab from every page reference and page title — unlinked-reference
  page headers, namespace macro and hierarchy links, the zoom breadcrumb,
  right-sidebar item titles, query search-result rows, and the page title's
  Ctrl/Cmd+click all behaved differently (plain-click only, or autoscroll on
  Windows). Shift+click still opens the sidebar and plain click still navigates
  (GH #207).
- **A split pane's only tab keeps its close button.** The tab strip hid the ✕
  whenever a pane held a single tab, so after one split neither pane could be
  closed without first dragging a tab across. The ✕ now shows whenever closing
  actually works: any multi-tab strip, or a lone non-feed tab whose pane can
  close (GH #207).
- **Search and conflict handling now use one contract across runtimes.** Page
  and block search agree on Unicode whitespace, bare Unicode tags, and the
  common linear-time regex subset; query-workspace saves and Direct conflict
  resolution classify bounded failure codes instead of matching error prose.
  The browser mock now also announces the graph rebind caused by changing the
  default home page.
- **Managed Storage and Direct Files answer the same query the same way.** The
  two storage modes evaluated block queries through separate copies of the same
  logic, and the copies had drifted: a byte-budgeted block-referrers panel
  admitted a different number of rows, and kept a different set of them, on the
  two modes for identical content. Both now use one evaluator and one
  result-budget rule.
- **Journal recency respects the configured title format.** On graphs with a
  custom `:journal/page-title-format`, `(sort-by modified)` ranked every
  journal last on three of the four query paths (they parsed titles with the
  default format only). All producers now share one recency axis that honours
  the graph's configured formats.
- **Backlink filter truncation is reported consistently.** A page-property
  root entry that hit its text/facet budget marked the context truncated on
  Managed Storage but not on Direct Files; the two now agree.
- **Managed rename works for every page name.** The managed rename planner
  keyed its index lookups with a different Unicode fold than the index itself
  (the two disagree on Greek final sigma), so renaming such a page reported
  success while doing nothing. Rename lookups now use the index's own fold.
- **config.edn settings can no longer clobber a nested map.** Every setting
  writer now edits only the root map's direct entries; a same-named keyword
  nested inside another map (e.g. under `:default-templates`) survives
  byte-for-byte instead of being spliced over.
- **Write-durability errors are reported, not swallowed.** The workspaces
  registry, backup restore, and Linux window-identity writers follow the save
  path's directory-fsync policy (tolerate "unsupported here", report real
  errors), the window-identity writer uses unique create-only temp files, and
  the file watcher recognizes backup-restore temp files directly.

- **One spelling of a page is one favorite.** Starring a page under one
  spelling (different case, an alias, NFC/NFD accents, boundary slashes) now
  fills the star and toggles off under every other spelling instead of
  appending a duplicate; the sidebar arrangement, membership, page delete, and
  rename all use the same kind-scoped identity, so deleting a page no longer
  silently drops a journal favorite that merely shares its name. Backlink
  filter chips key by the same identity, so two spellings of one co-referenced
  page no longer produce two chips that miss each other's filters.

### Added

- **Batch copy/export for references** (GH #348). Linked References and
  Unlinked References each have an explicit export affordance: every entry is
  pre-selected for copy-all, uncheck for a subset, and the familiar Copy /
  export modal (Text, OPML, HTML, with the usual content/indent/depth/cleanup
  options) produces the selected blocks grouped by their source page. The two
  sections act independently, Linked honors its active filters, and normal page
  export is unchanged — references are never included unless you ask.
- **Favorites nest to any depth** (GH #102). A group can hold groups, and a
  favorite can hold favorites; drag a row to the right to nest it, to the left
  to lift it out. Depth is measured from where the drag started, so a plain
  vertical drag never nests by accident, and the insertion line is drawn at the
  depth the drop will land. Deleting a group still keeps everything it held —
  now one level up rather than at the top. The arrangement page round-trips
  every bullet at every depth, so hand-written nesting survives.

- **Editing the Favorites page updates the sidebar** (GH #102). The arrangement
  lives in an ordinary page, so editing it — in Tine or outside — is now
  reflected immediately instead of at the next graph open. A page edit is a
  membership statement: removing a `[[link]]` bullet unfavorites that page and
  adding one favorites it. This also gives favorites reordering a keyboard
  route, since the arrangement is a page like any other.

- **`logseq/config.edn` is re-read while Tine runs.** A settings change made in
  Logseq, in a text editor, or delivered by Syncthing is now picked up during
  the session rather than being ignored until the next graph open — and
  therefore no longer computed against a stale copy. Favorites, shortcuts,
  macros, the home page, journal formats, hidden properties and the rest all
  follow. Tine only pays for the reload when the file's bytes actually differ
  from those it is serving, so its own settings writes and repeated identical
  deliveries cost nothing.

- **Privacy-safe diagnostic reports** (GH #343). Every production build now
  retains a bounded current-and-previous-run flight recorder of fixed operation
  names, outcomes, timings and counts. Settings → Diagnostics previews the JSON
  before the user chooses to copy or save it; nothing is uploaded automatically,
  and graph content, paths, page titles, queries, URLs and credentials are
  excluded by the recorder API. Release CI also retains exact-commit native
  symbols and hidden frontend source maps outside the shipped packages.

- **Experimental 32-bit Windows packages** (GH #275). Releases now produce an
  `i686-pc-windows-msvc` installer and portable ZIP for older Windows 10 tablets.
  The pilot is deliberately excluded from automatic updates until it has been
  exercised on a reporter's real 32-bit device.

- **Favorites can be arranged into named groups.** One level, plain-text group
  names, collapsible, with drag between groups as well as within one. Deleting a
  group never unfavorites anything — its pages move back to the ungrouped list.
  The arrangement lives in an ordinary page in your graph, so it syncs and
  merges like everything else rather than as an opaque settings blob, and
  because its entries are real links a page rename follows them automatically.
  `config.edn :favorites` stays exactly what it was — the flat list Logseq
  reads — so Logseq keeps working. Nothing is created until you actually make a
  group: if you never group anything, nothing about Favorites changes (GH #102).

### Changed

- **Typing in the Linked References search box now also narrows the reference
  chips below it.** The reference list already shrank as you typed; the chips —
  the pages and tags you pick from — were computed over every backlink, so they
  and their counts never moved. They now follow what you type, which is what
  Logseq's equivalent field does and what the list is for. A chip you have
  already selected stays visible even when your text filters its last backlink
  away, at count zero, so a filter can never become unreachable (GH #173).

- **A journal day with more than one file is now resolved on the day itself.**
  These days — usually a leftover of changing the journal date format, which
  never overwrites the old file — used to reach you only as a sticky notice at
  startup pointing at Settings. They now join the conflict badge, the dock and
  the walk like every other conflict, and the day page offers the same
  side-by-side review: keep a line from either file, or keep both. Keeping both
  is what "Merge" did in Settings, so nothing was lost by moving it here; the
  other file goes to the recoverable trash once you apply. Opening, renaming and
  trashing an individual file are still offered, on the day and in Settings.
  Two files in different formats (a `.md` and an `.org` for one day) can't be
  folded together and say so instead of offering a choice that could not be
  applied. The startup notice is gone.


- **Copy / export names its two content choices after what you actually get.**
  The explicit *Content* option in "Copy / export as…" now reads **Plain text**
  (cleaned, as displayed) and **Markdown** — or **Org** on an Org page —
  (preserved source syntax: bold, highlighting, links, properties and
  structure). The preserve choice was always implemented but shipped under
  the internal label "Source"; users looking for the Markdown-preserving
  option could not find it (GH #352).

### Fixed

- **Future iOS/TestFlight builds now use Tine's application icon.** The generated
  Xcode project previously retained Tauri's template icon even though Tine's
  complete iOS icon set was tracked in the repository. Project preparation now
  installs that set before Xcode compiles the asset catalog, and the signed-IPA
  contract compares the packaged primary icon's pixels with Tine's source. The
  already-submitted `0.6.95.8` beta is unchanged.
- **Dragging a favourite in the sidebar no longer selects its title.** The
  reorder drag is a pointer gesture, not a text gesture, but nothing suppressed
  selection, so the label smeared under the cursor. Applies to both reorderable
  sidebar lists.
- **The Linked References filter no longer claims to have filtered a list it has
  not.** While the descendant index is still loading the panel deliberately
  shows every reference — the local text it has is only a subset, so hiding a
  reference could hide a real match — but the summary still read "N of N
  references". It now says indexing is in progress and that the filter applies
  when it finishes (GH #173).
- **Deleting an open page on Android no longer passes through an unexplained
  black frame.** Tine still waits for pending edits and the native trash
  operation to become durable before navigating anywhere, but it now retires
  the deleted page's pane routes in that same durable continuation, before the
  loaded page is purged. Debug mode also times confirmation, saving, native
  deletion, fallback loading and first paint separately so remaining
  device/storage latency can be diagnosed without logging note names or paths
  (GH #376).

- **Creating a journal no longer makes the next Direct Files page fail to save
  on Windows.** Windows may report several `Create`, `Modify`, and rename events
  for Tine's own atomic page publication. Those exact self echoes now validate
  the published path, bytes, and physical file identity under the same write
  lock instead of raising the external-change frontier while the watcher waits
  to reconcile them. A changed external file, an identical-byte replacement
  inode from a sync service or second Tine, ambiguous events, and portable
  case/NFC collisions remain fail-closed (GH #374, follow-up to GH #366).

- **Block zoom keeps pointing at the intended block after siblings are inserted
  or reordered.** Tine now keeps its deterministic, UUID-shaped runtime
  locators separate from authored `id::` / Org `:id:` identity, resolves the
  unique authored claimant first, and refuses ambiguous duplicate authored IDs
  instead of guessing. Existing graphs are not rewritten (GH #373).

- **Windows self-update now follows the system proxy route and shows one update
  offer at a time.** The native updater already used the Windows trust store,
  but its reduced feature set omitted Reqwest's separate Windows system-proxy
  integration, so it could fail to fetch `latest.json` even while WebView2 and
  the browser reached GitHub. Startup and manual checks also now replace the
  same release offer instead of stacking duplicate install prompts (GH #241).

- **Selecting text on Android no longer stacks Tine's formatting controls under
  the system Cut/Copy/Paste menu.** Tine's Bold, link, code, highlight and other
  formatting actions now dock above the existing mobile keyboard toolbar, while
  Android keeps the selection-adjacent region for its native actions and handles.
  The compact formatting overflow opens upward, away from the keyboard (GH #375).

- **Fitting split panes no longer keep a scrollbar merely for blank editing
  space.** The first GH #369 correction fixed short panes but still made an
  otherwise fitting page scroll whenever it occupied 60–100% of its pane. An
  idle pane now has zero blank scroll range at every fitting height; the 40%
  pane-relative breathing room returns while a block is actively edited, so a
  long page's tail remains comfortably reachable.

- **The full Rust core corpus now finishes and the release gate no longer hides
  passing tests.** Five obsolete shared-join test barriers now pause at current,
  finite clean-runtime concurrency cuts. An honest unfiltered run completed all
  2,116 selected-or-ignored tests without a hang or timeout, and 47 stale or
  passing names were removed from the release exclusion. The remaining 45
  known-red legacy-oracle tests are listed exactly and classified by behavior
  family, so a newly passing, renamed, or omitted test fails the contract
  instead of silently drifting outside the gate (GH #350).

- **Managed Storage can reopen retained edits and moves of blocks carrying a
  Logseq `id::` UUID.** Projection recovery now consults the same bounded
  baseline identity candidates as planning and commit validation, instead of
  refusing the accepted block as unauthorized when the UUID originated in the
  activation baseline (GH #370).

- **Android startup and page navigation no longer look like an unexplained black
  or blank screen while work is pending.** The native window now has light and
  night Tine backing colors before the WebView paints, startup immediately shows
  a matching readiness surface, and an in-app page load has a small visible
  indicator. Actual load failures still replace it with their error, and plugin
  revocation checks still finish before community code can activate (GH #299).

- **Clicking a block bullet no longer reports a false same-name file conflict.**
  A block zoom now keeps using the exact page already open in Direct Files
  instead of racing the durable block-ID save with a second read of the page's
  older bytes. Restored zoom tabs still load missing owners from disk, and
  genuinely different same-name physical files remain fail-closed (GH #354).

- **Page action control labels no longer join dragged text selections.** The
  journal carry-over buttons ("Carry unfinished tasks → today" and friends),
  the tag-table toggle, the guide copy button, and the ⋯ page-actions trigger
  are page chrome, not content: selecting across the page no longer copies
  their label text, while the buttons stay fully clickable and keyboard-
  focusable.

- **Joining a synced graph no longer gets stuck on an older local managed-storage
  marker.** If Direct Files is active but a previous activation left private
  managed state behind, Tine now archives that complete predecessor unchanged
  before bootstrapping the other device's shared identity, instead of trying to
  open the old marker as the new graph and reporting that it names a different
  catalog document.

- **A provider deletion can no longer erase a local edit that arrived in the
  same sync pass.** Managed sync now captures a pending Markdown or Org watcher
  epoch before applying provider projection, then drains the complete visible
  provider cut before classifying the resulting race. The local edit therefore
  enters immutable history and the normal edit-versus-delete rule settles it,
  even when the filesystem and oplog notifications arrive together.

- **Direct Files delete, rename, trash, and conflict-copy moves no longer read
  every document in the graph.** These exact no-clobber moves now validate the
  source and destination through retained metadata and portable path checks;
  document contents remain untouched and unrelated files are never opened.

- **A crash inside a Direct Files save can no longer leave a page invisible
  under hidden recovery names.** On the next checked open, one unambiguous
  stranded copy is restored without clobbering a live file. Ambiguous or
  superseded copies are retained or moved intact to recoverable conflict trash;
  lookalikes and files whose physical identity cannot be proved are untouched.

- **Changed Direct Files saves perform one fewer full-file read.** The atomic
  retirement already detaches and byte-checks the exact expected inode before
  publishing, so Tine no longer rereads the same live name immediately before
  that stronger proof. External edits still win or become an explicit conflict,
  and managed-storage projection behavior is unchanged.

- **Page lists stay fast after Direct Files file lifecycle changes.** With a
  warm graph, creates, deletes, watcher additions/removals, and renames now
  update the exact in-memory page inventory instead of making the next page
  lookup reopen and reparse the whole graph. Rename transactions reparse only
  documents whose retained final bytes they already changed; explicit titles,
  parse failures, physical paths, and watcher invalidation remain authoritative.

## [0.6.96] - 2026-08-24

### Added

- **Conflict review can now suggest a combined "Merged" version of a block both
  sides edited.** When two devices changed provably different parts of the same
  block's text (against the last version both sides agreed on), the in-page
  conflict review offers the combination as a fourth, pre-selected choice next
  to keep-mine/keep-theirs/keep-both — for sync-conflict copies and for
  `diff3`/Fossil-style merge markers alike. Where the edits overlap but the
  merge tool wrote its own `####### SUGGESTED CONFLICT RESOLUTION` sections
  (Fossil), that suggestion is offered in the same place instead, clearly
  labeled as the tool's proposal. Like every suggestion it is only
  pre-selected: nothing is applied until you confirm, the applied text is
  re-derived from the same three versions (never trusted from the UI), and
  edits that overlap — or whose combination would not survive as a single
  block — simply get no offer.

- **Conflict rows now preview the first line that actually differs**, not
  blindly the first line, with an `…` marker when earlier lines agree — and
  multi-line blocks gain a per-row expander showing the full text of each
  version with the differing lines highlighted.

- **A conflict can no longer scroll out of sight.** The in-page conflict
  review lives at the top of the page; once it scrolls out of view a slim
  one-line notice stays pinned to the top of the pane — especially on a
  phone, where the review was previously invisible until you scrolled up.
  Tapping the notice unrolls the same review in place (your reading position
  and any choices you already made are kept); tapping again, pressing
  Escape, or scrolling back to the top folds it away.

### Changed

- **The conflict review is now usable on a phone (and in a skinny split
  pane).** When the panel is narrow, the two versions stack full-width instead
  of being squeezed into unreadable columns, and the per-row choices become a
  compact strip — a color dot plus a short word, with the legend mapping each
  color to the real side name. Sync-copy names like
  `sync-conflict-20260705-141233-A2B2C3D` now read "Sync copy · Jul 5"
  everywhere (the exact file tag stays as a tooltip).

- **The startup conflict notices are gone.** The two toasts that appeared on
  every graph open while conflict copies or merge-marker files existed
  duplicated what the conflicts badge, the in-page review, and its pinned
  notice already say — and pointed at Settings instead of the in-page
  resolver. A toast still appears when a *new* conflict arrives mid-session,
  and for duplicate journal days (which have no other surface).

- **"Apply all suggested" no longer sweeps up a merge tool's own proposed
  text.** A Fossil `SUGGESTED CONFLICT RESOLUTION` body (labeled *Merged
  (tool)*) keeps whatever you set on that row; only your per-row choice (or
  the initial pre-selection you confirm) accepts text Tine did not compute
  itself.

### Fixed

- **A settings change can no longer overwrite a `config.edn` that arrived while
  it was being written.** Tine re-read the file before saving, but an external
  writer — Syncthing delivering a peer's copy, Logseq, an editor — could still
  land in the moment between that check and the write, and its changes vanished
  with no warning. Tine now publishes the file only if it still holds what it
  read, and otherwise re-applies your change on top of theirs. If a crash
  interrupts a write, the file is restored on next open rather than left
  missing.

- **Renaming a page no longer edits pages that are mid-merge.** A page whose
  file still contains unresolved merge-conflict markers is left exactly as it
  is instead of having its links rewritten inside both sides of the conflict.
  The rename still completes everywhere else, and a notice tells you how many
  conflicted pages still point at the old name.


- **Split panes whose content fits no longer show a useless vertical
  scrollbar** (GH #369 — the reporter's dashboard of short panes like "Lines"
  and "GRID" each carried a permanent bar). The end-of-page editing slack now
  measures against each pane's own height (flex spacer in the pane scroller)
  instead of the window (`40vh`): a pane up to ~60% full shows no bar at all,
  a near-full pane keeps at most 40%-of-pane of breathing-room range, and a
  long pane still scrolls independently and its tail can scroll ~40% of the
  pane up off the bottom edge while editing.

- **Advanced queries using `:inputs [:current-page]` now follow the focused
  pane** (GH #301). Tine binds Logseq's standard current-page page/ref
  relationship, reruns it on focused-page navigation, and keeps ordinary date
  inputs typed and unchanged.

- **A graph's Home page setting now follows the graph between devices** (GH
  #269). Tine reads and writes Logseq's `:default-home {:page "..."}` entry in
  `logseq/config.edn`, preserves other keys in that map, and safely migrates an
  older device-local setting after the graph opens.
- **Resolving merge markers inside Tine now stages the pre-resolution file in
  the recoverable trash** (Settings → Backups & recovery), so the sides you
  did not choose stay recoverable — the same guarantee resolving a sync
  conflict copy already had.
- **Adding a highlight no longer rewrites a PDF notes page that carries
  unresolved VCS merge markers.** The write is refused like any other save to
  a quarantined page; previously it silently lifted the quarantine while the
  merge was still unresolved in git/Fossil.
- **Conflict review no longer freezes on blocks with extremely long single
  lines.** Comparing candidate rows now caps at the first 512 characters per
  line (a 64 KB one-line block previously stalled the conflict panel for
  seconds to minutes), oversized flat pages avoid a slow exact-alignment
  cliff, and all conflict-review reads moved off the UI's request thread.

- **Managed-storage startup failures now keep their real recovery cause and stop looking active** (GH #370). A core reopen can return a typed retryable/refused result without a serving actor; startup previously wrapped that non-serving result in a managed graph slot and then replaced its useful cause with the generic “managed storage is not ready” page-readiness error. It now refuses before publication with the original redacted diagnostic, keeps the Direct Files escape available, and freezes the elapsed time when the operation reaches its terminal failure instead of counting upward indefinitely.

## [0.6.95] - 2026-08-23

### Added

- **The iOS beta can use a TineOutline-owned graph in iCloud Drive.** The native folder picker recommends `iCloud Drive → TineOutline` when iCloud is available and also accepts `On My iPhone/iPad → TineOutline`, while clearly refusing arbitrary third-party Files providers. Before Rust opens the selected graph, the iOS bridge requests any ubiquitous files that are not yet local. Signed TestFlight builds install and verify the exact iCloud Documents entitlements in both the provisioning profile and app signature.

- **macOS release builds are now signed and notarized.** The universal direct-download app and DMG use Tine's Developer ID identity, authenticate notarization with a short-lived App Store Connect token, and must pass independent signature, team, stapled-ticket, Gatekeeper, and DMG-integrity checks before the release lane can be staged. Windows signing remains separate future work.

- **Tine can now build a signed iOS beta for TestFlight.** A manual-only workflow creates and verifies a `TineOutline` IPA for the existing `page.tine.Tine` bundle ID, retains it as an immutable artifact by default, and only validates or uploads to TestFlight when that action is explicitly selected. It cannot submit or release a production App Store version. The iOS bundle includes its privacy manifest, export-compliance declaration, App Store provisioning proof, and a unique build number for each run.

- **Tine now publishes a plain-language privacy policy and support contact.** The in-app About tab links to both, the website links to the policy, and the policy distinguishes local graph data from the limited update, plugin-catalogue, documentation, and voluntary-support traffic Tine can make.

- **New editor command: Copy block embed** (GH #279) — `Mod+Shift+C` copies `{{embed ((block-id))}}` for the current block using its durable `id::`, the embed counterpart of the existing `Mod+C` block-reference copy. Remappable in Settings → Keybindings; with text selected it gets out of the way so normal copy works.

- **Logseq navigation hotstrings `gh`, `gn`, `gp`** (GH #276). `g h` opens the graph's configured home page (falling back to the journals landing when none is set — Logseq's default home), `g n` / `g p` open the next and previous journal day relative to the journal you're in (or relative to today anywhere else), in the graph's configured journal title format. All three are remappable.

- **Sheets now render in the HTML export and print output for any graph.** Until now, publishing a page holding a Tine Sheet emitted only the sheet's plain-bullet twin plus the visible `tine.*` view configuration, so the presentation — columns, grouping, computed cells — disappeared. Block-powered sheets now publish as meaningful read-only views: tables with their title/declared/formula/observed columns, typed checkbox cells, aggregate footers, and arithmetic formula columns; boards grouped by state, priority, tags, or fields, including query-backed boards; and grids with their positional cells, header rows, and nested grids. Numbered list blocks (`logseq.order-list-type:: number`) keep their ordinal markers, and blocks with LOGBOOK clock rows keep an elapsed-time badge while the drawer stays hidden. This applies to both whole-graph publishing and single-page print/PDF export.

### Changed

- **The managed-storage opt-in now says plainly that it is known to be buggy.** Settings and the built-in Guide no longer describe it merely as experimental or not mature: they state that it does not yet fully work in Tine's own testing and that work is ongoing, while retaining Direct Files as a fully supported peer rather than a migration step.

- **The first iOS beta keeps Wasm plugins disabled, as required by ADR 0052.** Installed guests are not loaded, the package/catalogue surface is absent from Settings, and plugin commands cannot register. Built-in and inert token themes remain available. Enabling iOS plugins later remains a separate product and App Review decision.

- **The iOS About tab omits the external Ko-fi support link.** Website, desktop, and Android support links are unchanged; iOS keeps only non-purchase project, privacy, and email-support links while the initial App Store review surface is deliberately minimal.

- **The public website "Demo" is now the Guide**, at `https://tine.page/guide/`: the read-only tour of workflows, reference pages, and rendered examples published by Tine's own HTML export, no longer described as a live or editable demo. Old `https://tine.page/demo/` links redirect there; the in-app Guide and the onboarding demo graph are unchanged.
### Fixed

- **Renaming a page no longer exhausts the Direct Files safety budget on very large graphs** (GH #364). A rename already makes one bounded, no-follow inventory and rechecks every file it will touch; its inner publication helpers nevertheless attempted a second whole-graph identity build, which could refuse the operation around 13,000 pages with `initial shadow peak build memory bound exceeded`. Rename now keeps the transaction's existing evidence and repeats only target-local collision and no-clobber checks while preserving external-edit detection, rollback, hard-link refusal, namespace cascades, and graph-wide reference rewriting.

- **Ctrl/Cmd+click on an internal link now opens a background tab everywhere** (GH #283). One standard pointer contract now covers page links, block refs, references, search results, Favorites, Recent, namespace crumbs and query group headers alike: plain click navigates, Shift+click opens the right sidebar, Ctrl(Win/Linux)/Cmd(macOS)+click opens a background tab — as does middle-click — and right-click keeps the explicit destination menu. (Behavior change note: Ctrl+click on a rendered `[[page]]` or `((block))` link previously opened the other pane; pane opening remains available through context menus and the window/pane gestures.)

- **Block embeds now track their source block's collapse state live and keep each occurrence's own fold** (GH #360). Before you change an embed locally it follows source folding immediately. Folding or unfolding that embedded root then stores an explicit `collapsed:: true` or `collapsed:: false` on the block containing the embed, so the choice survives reload independently of the source and other occurrences; the referenced source block is never changed.

- **PDF annotations are usable on mobile** (GH #191). Finishing a native touch text selection now opens the highlight color chooser, and long-pressing an existing text or area highlight opens the same recolor, remove, Copy ref, and Linked references actions as desktop. Touch-sized controls stay inside the phone viewport; desktop mouse and context-menu behavior is unchanged. The earlier mobile Close/Back escape fix remains intact.

- **Queries with `<% current page %>` rerun when you navigate** (GH #301). A query that explicitly carries the `<% current page %>` dyvar now binds it to the page in the focused pane — so a sidebar (or any open) query like "everything on #current page tagged #pin" updates as you move between pages. The authored query text is untouched, queries without the dyvar don't rerun on navigation, and a delayed result from the previous page can never leak into the new page's view. (`:query-page` remains bound to the block owning the query; the EDN `:inputs [:current-page]` advanced form is still outside the engine's entity binding and is documented in the issue thread.)

- **Mobile: long-pressing a page link now shows its context menu** (GH #231). Holding a `[[page link]]` (or anywhere on its angle bracket region) for ~½ second opens the same context menu desktop right-click gives, instead of silently selecting the link text. Quick taps still navigate, moving your finger cancels the hold (so scrolling is unaffected), and desktop + mouse behavior is unchanged.

- **Typing `(`, `{`, `[`, `"` or backtick now inserts its counterpart by default** (GH #291). Logseq has always paired these; Tine's pairing existed but was opt-in, so on a default install the two reporters naturally disagreed. It is now on by default — Settings stays available to turn it off (an explicit opt-out is remembered). Brackets doubling (`[[`, `((`, `{{`), selection wrapping, type-through, Backspace-on-empty-pair, IME composition, and the always-on `[[ ]]` page-ref pairing behave exactly as before.

- **Code blocks no longer jump layout when clicked** (GH #357). Clicking a fenced code block used to swap the mono, unwrapped, padded code card for a proportional 16px-textarea view that soft-wrapped long lines, visibly re-laying out every line. While a block is a code fence (Markdown ` ``` `/`~~~` or Org `#+BEGIN_SRC`/`#+BEGIN_EXAMPLE` — not `calc`, not mixed paragraph+fence content), the editor now presents as the same card: identical font, size, leading and padding, zero soft wrapping, horizontal scroll for long lines instead. The text stays the honest raw source, fences included, and it flips in and out live as you type.

- **Searching for a page alias opens the real page, not a phantom alias page** (GH #353). When an alias's text was also referenced anywhere (`[[Book]]`), search (Ctrl+K) and the `[[` / `#` autocomplete listed both the owning page and a separate alias-named virtual page — selecting it opened a standalone "Book" page instead of the original. The shared page-candidate search now treats an alias like the page name itself, so it can never be its own result; case variants and multiple aliases are covered, ordinary pages that merely overlap an alias still rank normally, and the owning row shows an "aka Book" hint so the match is understandable.

- **Files and folders linked inside `assets/` open in the system viewer / file manager** (GH #367). A labeled link such as `[image](./assets/quick-capture.png)` or `[path](./assets/)` (Markdown or Org) was handed to the external-URL opener, which rejected the relative path, so nothing opened. Such links now resolve extension-agnostically through the graph's asset opener: a file opens in the OS default app, a directory or the assets root opens in the file manager, and percent-encoded nested paths are decoded first.

- **Enter at an in-block line break no longer leaves an empty first line** (GH #361). Pressing Enter with the caret right after an in-block newline now turns that newline into the block boundary, matching Logseq: the previous block ends with its last text line and the new block starts with the next line's actual text — on desktop and mobile. Shift+Enter's in-block newline, mid-line splits, and Enter at the very start of a block are unchanged.

- **Blocks opened in the right sidebar always show their children** (GH #358). Shift-clicking the bullet of a collapsed block parked it in the sidebar but hid its child content until you first expanded it in the main page. The sidebar now treats the opened block as the root of its own view, exactly like zooming in the main page — children always render, and deeper collapsed blocks stay collapsed as expected.

- **Page-reference styling updates as soon as a referenced page gains content** (GH #355). A `[[Page]]` link could stay in the "missing page" style after its target was created later in the same session, refreshing only on restart. The existence answers now invalidate whenever the graph's page inventory changes, so links restyle immediately — both when a page appears and when one is deleted.

- **Block reference locations can be collapsed to a compact overview** (GH #344). The expanded view you get from a block's reference-count badge now offers Collapse all / Expand all: collapsed groups show only their source page title and disclosure button, which re-expands the breadcrumb and referenced block while the title itself keeps its normal navigation behavior. The state is local to that view — the page-level Linked References section keeps its own.

- **Arrow keys now move between bullets inside linked references, block references, query results, and embeds** (GH #341). Previously, pressing Up/Down while editing a block in one of those views moved the caret into the block's owning page outline — an editor that usually isn't visible there — so the cursor simply vanished. Navigation now steps through the blocks exactly as rendered in that view, while structural edits (merges, indent) keep operating on the real page outline.

- **Linked References: include filter chips now combine with OR** (GH #273). Selecting two pages/tags in the filter panel previously required every backlink to reference both, so the union you meant to keep collapsed to nothing. A backlink now survives when it carries any included facet; exclude chips still subtract cumulatively, and typed search still narrows the result further.

- **Search and Quick Switcher results support modified clicks** (GH #288). Middle-click and Ctrl/Cmd+click open a result in a background tab of the current pane while the switcher stays open, so you can fan several hits out at once; Alt+click opens in the other pane; Shift+click opens pages and blocks in the right sidebar. Ordinary clicks, Enter, and the existing Shift/Alt+Enter shortcuts are unchanged. This is the same action set the Enter-key modifiers already offered, now reachable by mouse.

- **New actions: keyboard pane resizing** (GH #286). Four remappable commands — Grow/Shrink active pane width and Grow/Shrink active pane height — nudge the nearest split of the matching axis by five points per step (the existing 15–85% limits apply). They ship unbound so you can assign your own chords in Settings → Keybindings, and are also available from the command palette.

- **New action: Toggle maximize active pane** (GH #285). One pane can temporarily borrow the whole pane area (sidebars untouched) — useful when limited space wraps long lines in a multi-pane split. Toggling again restores the exact split arrangement and sizes; the state never survives into sessions or workspaces. Default shortcut `Ctrl/Cmd+Alt+M`, remappable and available from the command palette.

- **Move Tab to pane {left,right,up,down} now creates the pane when none exists** (GH #282). Previously the command did nothing unless a pane already lay in that direction — so from a one-pane window (the most common starting point) it never did anything at all. With several tabs in the source pane, the active tab moves into a new pane on the requested side; with only one tab, the new pane opens as a mirror of the current tab and history and the original stays, since Tine panes are never empty. Moving into an already-existing pane behaves exactly as before.

- **Android: hiding the keyboard no longer taps the note behind the toolbar** (GH #336). The editor toolbar's hide-keyboard button blurred the editor on touch-down, which closed the keyboard and removed the toolbar mid-gesture, so the tap's release landed on whatever block happened to be underneath. The button now consumes the whole touch gesture.

- **Calculator blocks no longer render a phantom empty line** (GH #339). After exiting edit mode, a ```calc block showed one extra blank row below the last expression (the fenced block's trailing newline was rendered as a row). Blank lines between expressions still keep their positions.

- **The Settings dialog can be enlarged on desktop** (GH #287). A new maximize/restore button in the Settings header grows the dialog to nearly fill the window with a comfortable margin, preserving the selected page and scroll position; closing Settings always restores the default size. Narrow/mobile screens are unchanged — the sheet already fills the viewport there.

- **Tab close buttons always sit at the tab's right edge** (GH #340). With short titles the ✕ no longer drifts to just after the text — it stays pinned rightmost at any tab width — and it is now a real keyboard-focusable button with its own "Close tab" tooltip instead of showing the tab's "Double-click to pin" hint.

- **No more white frame around the page after clicking empty space and pressing a key** (GH #345). Clicking an empty spot of the main content focused the page scroller, and the next keypress flipped the browser's focus heuristic, painting its default white frame around the whole content area. Only that default frame is suppressed — the pane-select ring and the usual focus cues on buttons and other controls are unchanged.

- **Managed-storage block moves across journal-day boundaries now stay immediate and keep keyboard focus.** The source and destination publish as one durable foreground transaction, so archive/SQLite derivatives no longer dim or stall the interface before the block appears on the adjacent day. The foreground path is graph-size invariant at 100 and 10,000 pages, and rapid queued edits use exact pending-projection indexes instead of repeatedly scanning the journal prefix. Response-replay cleanup now runs on its own bounded retry lane, so a slow acknowledgement cannot stall the next move; actor cleanup remains crash-safe, bounded, and explicitly runnable. Multi-page foreground prefixes now also drain from accepted state instead of letting later queued catalog heads block an earlier projection, and a peer move no longer stalls merely because this device concurrently advanced an unrelated page in its catalog. Page and namespace renames likewise use exact SQLite name/range lookups instead of enumerating every unrelated graph page before the indexed reference rewrite. Accepted saves advance authenticated roots by path-copying only changed documents and the new batch, provider-head publication reads an incremental frontier-tip set, and clean projection attach decodes only current path heads; none of these routine boundaries now clone or replay the graph/session history.

## [0.6.94] - 2026-08-22

### Added

- **Live Direct Files save conflicts now use Concord's in-page, block-level
  resolver.** If another device changes a file underneath a retained Tine draft,
  both versions appear above that page with three-way suggestions and per-block
  keep-this / keep-that / keep-both choices. The draft is kept in app-private
  recovery state, survives a Tine restart, and is removed only after a
  revision-guarded resolution commits. The old global *Keep mine / Use current*
  bar is no longer used for Direct Files conflicts.

- **Conflicts are now one calm queue you resolve inside the page.** Everything
  that needs your judgement — a Syncthing/Dropbox/Seafile conflict copy, or a
  page carrying unresolved git/Fossil merge markers — appears in a single
  `N conflicts` badge at the bottom of the sidebar, and clicking it walks you to
  the next conflicted page. Nothing about a conflict is written into your graph:
  the queue is recomputed from what is on disk, so it costs no storage, cannot go
  stale, and survives restarts by construction. Opening a conflicted page shows
  the two (or three) versions block by block, above the outline, named by
  whatever produced them (a git ref, a device tag), with per-block keep-this /
  keep-that / **keep both**, `N conflicts ↑↓` to walk between them, and a
  suggested resolution pre-selected wherever a common ancestor answers the
  question. Keep-both writes the two versions as adjacent sibling blocks —
  ordinary outline Markdown. Nothing applies until you click *Apply resolution*,
  and leaving a page with work outstanding gets a one-line note, never a blocking
  dialog. (ADR 0057; Concord P4, part of GH #337; see docs/concord.md →
  "Resolving conflicts".)

- **git and Fossil merge conflicts can now be finished in Tine.** A page whose
  file carries `<<<<<<<` / `|||||||` / `=======` / `>>>>>>>` markers is still
  quarantined from ordinary saves — Tine never mangles a conflicted file — but
  its marker sections are now parsed into complete page versions and reviewed
  with the same block-level machinery as a sync conflict copy. `diff3`-style and
  Fossil markers carry their own common ancestor, so most blocks arrive already
  decided; applying writes the merged page with no markers at all, which lifts
  the save quarantine by itself. It is the one circumstance in which Tine writes
  a marker-bearing file, and only as the direct result of the resolution you
  just confirmed. (Concord L5 completion.)

- **Bulk external revisions — a `git checkout`, `fossil update`, branch switch, or big sync — are now handled as one calm epoch.** When one burst of external file changes touches more than 32 pages, the watcher reconciles it in a single pass against a consistent snapshot and tells the interface once (`graph-changed-bulk`) instead of once per page: one derived-view invalidation, visible pages refreshed through the existing safety checks (a page being edited defers its reload exactly like a single change), everything else reloaded lazily on navigation, and a single summary toast — "N pages updated externally", with a conflict count if any changed pages had genuinely diverged unsaved edits. Previously every page cost its own event, dataRev bump, and page fetch, and nothing summarized the revision. Small changes are untouched: at or below 32 pages, per-file behavior is byte-identical to before. (Concord P2, part of GH #337; see docs/concord.md → "External revisions".)

- **Sync-conflict merges now come with per-block suggestions.** Tine keeps a
  private per-page record of the last version it agreed on with the disk (the
  Concord base ledger — stored in app data, never inside your graph, always
  safe to delete). When you review a Syncthing/Dropbox/Seafile conflict copy,
  Tine compares both sides against that remembered version: blocks only one
  side changed arrive with the right choice pre-selected and labeled
  *suggested*, and only blocks both sides changed still need a real decision.
  Nothing is merged without your confirm, exactly as before; with no
  remembered version the diff simply looks the way it always did.
  (ADR 0056; part of Concord P3, GH #337.)

- **A path-free block-diff command pair (`text_block_diff` /
  `text_block_diff3`)** diffs two or three raw page texts with the same
  block-tree engine the conflict merge uses — the seam the upcoming in-page
  conflict review builds on.

- **Tine now refreshes when you return to its window.** Some setups deliver no filesystem event at all — a network mount, a sync client writing through a path the kernel doesn't report, an app the OS suspended while you were away — and a page could sit stale indefinitely with nothing apparently wrong. Coming back to the Tine window now replays any reload that was deferred while you were editing and asks the watcher for one full pass over the graph. Anything that changed is handled exactly as a live change: a page you are editing is still deferred, never yanked. Throttled, so alt-tabbing costs nothing. (Concord P5, part of GH #337.)

- **A new "Always ask before applying an external change" setting** (Settings → Backups & recovery, off by default). By default a page you have open with nothing unsaved updates silently when another editor or a sync tool changes its file, the same as a code editor. Turn this on and Tine holds the change instead: the page keeps showing what you were reading and offers *Reload from disk* / *Keep mine* in a small bar above the content — never a dialog, never blocking. Everything that already asked keeps asking; only the silent case changes. (Concord P5, part of GH #337.)

- **The file watcher now keeps always-on latency receipts for external-change batches.** Each batch that surfaces a change (or an error scheduling a retry) records how long it spent between the OS callback, the post-debounce reconcile, and the `graph-changed` events reaching the UI, logs one structured line, and lands in a small in-memory ring the new `watcher_latency_recent` debug command returns — so slow-external-change reports (GH #337's 5–20 s) can be diagnosed from the reporter's machine instead of guessed at. Works in both inotify and poll watch modes.

- **Managed-storage activation no longer rejects a complete graph because Direct Files or the startup path catalog retained a different one-page inventory.** Readiness is now proved inside the candidate managed generation by opening its transactionally complete, exact-frontier-stamped SQLite inventory and a real page; the candidate remains unpublished until that proof succeeds.

- **Files with unresolved git/Fossil merge-conflict markers are now quarantined instead of mangled.** A page whose file contains column-0 conflict markers (`<<<<<<<`, `|||||||`, `=======`, `>>>>>>>`, and Fossil's verbose variants) stays fully readable, but every save to it is refused with a message naming the markers — previously an edit re-indented or dropped the markers on re-save, which broke git's own conflict detection and could silently lose one side of the merge. Markers quoted inside code fences don't trigger the quarantine. Affected files are listed in Settings → Backups & recovery, and the page shows a banner explaining how to resolve.

### Changed

- **Conflicts are now resolved in one place: the page.** The block-by-block merge dialog inside Settings is gone. Settings → Backups & recovery keeps the *inventory* — which conflict copies and marker-bearing files exist, **Review in page…** to go to one, **Discard copy**, and the case of a copy whose original page no longer exists — while the review itself happens next to the blocks. The two surfaces had drifted into opening with different pre-selections for the same conflict, which is exactly what Concord exists to prevent; the in-page resolver gained the dialog's one exclusive capability (choosing what happens to the page's own properties when the two sides disagree) so nothing was lost. The VCS-merge-markers panel, which previously offered no action at all, now also links to the page. (Concord P5, part of GH #337.)

- **Opening a graph no longer renames journal files.** A journal file whose name is not its date (`Jun 18th, 2026.md` rather than `2026_06_18.md`) can't be matched to its day, so that day looks empty — and Tine used to fix this silently at every graph open, and after any settings change. It is a repair you didn't ask for, applied to files you own: in a graph kept in git it appeared as a batch of renames the moment Tine started. The files are now listed under Settings → Backups & recovery → **Journal files named by title** with one button to rename them, which takes a snapshot first and never overwrites a name that is already taken. (Concord P5, part of GH #337.)

- **Experimental managed-storage activation now reuses the parser facts it already produced during source capture.** Search text, tasks, properties, tags, headings, and collapse state travel through one bounded activation-only handoff and are accepted only when every terminal page and block still matches the authenticated engine state exactly; oversized or mismatched inputs fall back to the independent parser path. The 13,000-page fixture reused all 13,000 pages with no misses and cut SQLite lowering from about 8.1 s to 6.7 s, although total activation remained within run-to-run noise at 69.1 s. The same pass now reuses one authenticated catalog window while resolving block identities, avoiding a graph-sized catalog proof per UUID on pages that contain several `id::` values.

- **Experimental managed-storage activation no longer explodes a dense reference catalog into hundreds of thousands of tiny immutable files.** `tine-storage` 0.6.2 constructs oversized Patricia ranges in bounded canonical chunks and admits the representative 130,000-block catalog in seven packs with no capacity fallback. Tine also sizes that bounded construction from real available memory on Windows. On the 13,000-page release fixture this cuts activation from 86.1 s to 67.5 s and makes SQLite, rather than immutable-catalog fallback, the clear remaining bottleneck.

- **Experimental managed-storage activation now lowers each terminal page once for both SQLite and its exact-byte shadow proof.** SQLite consumes bounded terminal-page chunks while an activation-only sink derives compact, unpublished shadow evidence from the same parser-owned pages; only after SQLite has completed is that evidence bound into the atomic shadow proof. Cold recovery retains the independent builder, and differential plus crash-cut tests require both routes to publish identical durable bytes.

- **Experimental managed-storage activation no longer writes and rereads a graph-sized operation spool for ordinary graphs.** Canonical semantic operations stay in a measured, byte-bounded memory builder through partitioning and detached authoring; oversized imports retain the exact spill path, and both routes are differentially required to publish the same aggregate and commit. SQLite's uninterrupted-build shortcut now retains only the authenticated accepted events it actually consumes.

- **Direct Files task queries now use the same disposable SQLite fact layer as managed storage without making SQLite part of saving.** The already-parsed page cache feeds one background, coalescing worker; clean reopens reuse unchanged page facts, and task candidates are admitted only at the exact current cache generation before Tine's existing query parser evaluates them. A sidecar lease prevents concurrent graph instances from replacing one another's ready facts. Missing, stale, corrupt, incompatible, leased, or unwritable SQLite state falls back to the established Direct Files evaluator. The switched task family no longer uses its old whole-graph candidate scan as the ordinary route, while retaining the bounded final-result memo that keeps reactive re-renders cheap.

- **Block search, the `((` picker, and referenced-page autocomplete now share the same disposable SQLite fact layer in Direct Files and managed storage.** SQLite supplies only generation-coherent candidates and original-case reference spellings; Tine's existing parser still owns exact fuzzy matching, ordering, property-reference rules, and presentation. The ordinary ready path no longer scans every parsed block or maintains a second referenced-name semantic cache. Missing, stale, leased, or incompatible SQLite state still falls back to the already-parsed graph without blocking open, edit, save, or external-file observation.

- **Aliases, backlinks, unlinked references, and block-reference lookup/counting now use that same disposable SQLite projection in Direct Files.** SQLite narrows only an exact current cache generation and Tine still verifies every semantic result with the parser; unsafe tokenless names and non-UUID `id::` values use the parser fallback. The former in-memory alias, reference-candidate, block-identity, and block-reference-count indices and their foreground maintenance have been removed rather than retained beside SQLite.

- **Managed-storage path names now come from `tine-storage`'s certified format manifest.** Tine core retains only a definition-free compatibility import, so releases pin one complete, machine-readable layout vocabulary without changing any persisted path.

- **Ordinary saves under experimental managed storage do substantially less repeated work.** The hot path now reuses parser-owned commit evidence, the exact accepted editor post-state, and the preceding projection when their identities still match; ordinary projection edits are patched instead of rebuilding the page. The proofs fail closed to the complete path whenever any prerequisite is stale or absent.

- **Managed-storage saves now return from one bounded foreground commit instead of rebuilding or publishing graph-wide derived state.** The exact Markdown/Org update and one private journal append establish the accepted edit; the hot overlay serves it immediately while immutable archive, SQLite, provider, checkpoint, and journal-compaction work drain afterwards. Consecutive saves use the SQLite baseline plus the exact journal suffix to prove their predecessor without reconstructing the whole page. The release gate measures a 511-block page at under 50 ms p95, and fails if the old full-page predecessor reconstruction returns. Crash replay marks pending task-query facts incomplete and safely uses the complete evaluator until rebuilt; an append whose outcome is genuinely unknowable stops further edits until restart replay, rather than risking a duplicate operation.

### Fixed

- **Managed storage now activates and saves on Android shared storage without
  requiring hard-link or flagged-rename support.** Private segment, frontier,
  and selector publication use the app-private sole-writer atomic-rename
  fallback. The reconstructible Markdown/Org projection also falls back from
  Android shared storage's `EACCES` answer for flagged `renameat2` to the
  existing reserve-and-ordinary-rename protocol; a real permission denial still
  fails that ordinary rename, and shared/provider authority remains strict. The
  real app-UID activation, edit, crash-reopen, share-setup, shutdown, and reopen
  journey therefore no longer stalls after a durable edit with `Permission
  denied (os error 13)`.

- **A pending managed-storage edit no longer makes unrelated pages disappear
  until the derived projection drains.** Exact-path loads now combine the
  exact-frontier SQLite baseline with only that path's journal-durable overlay,
  instead of globally abandoning the baseline whenever any page has pending
  derived work. Untouched pages therefore remain readable after a crash reopen
  while another page's Markdown/SQLite publication catches up.

- **Direct Files no longer repeats whole-graph work on ordinary foreground
  paths.** Opening Journals before background warm now inventories filenames
  without reading and parsing every ordinary page. Creating one page uses the
  warm semantic-identity generation plus target-local no-replace publication,
  instead of hashing the graph twice, and advances the cached page inventory
  incrementally. Raw filesystem callbacks now publish an O(1) creation barrier;
  the debounced watcher owns the single final read/parse, so event bursts do not
  parse intermediate versions before coalescing.

- **Applying a sync conflict no longer leaves the pre-merge editor behind.**
  Concord now drains the page's pending save, blocks mutations for the guarded
  merge, installs the exact page returned by the native commit, and only then
  releases editing. This prevents the old open copy from immediately creating a
  second conflict or appearing to undo the chosen merge. Newly delivered copies
  also raise one actionable notice which retires with the resolved conflict;
  resolving today's journal updates it in place without dropping it from the
  live Journals feed. Deferred winner-file events are replayed as soon as the
  guarded mutation releases, and an older inventory scan cannot resurrect the
  resolved conflict when ordinary editing resumes. Same-content resolutions now
  keep the returned disk revision, an in-flight Journals refresh cannot publish
  a partial feed without today, and Apply refuses to discard edits made after a
  live-conflict comparison was shown. After restart, a recovered unsaved draft
  remains the reviewed side instead of being overwritten by the disk-loaded
  editor. Android now defaults to its native
  inotify-backed watcher; polling remains available and uses the same
  reconciliation semantics. (GH #337.)

- **Returning to Tine can no longer open an editor over a stale page.** A
  focus-driven disk scan now has an explicit native completion receipt, waits
  for frontend page application, and finally verifies only the visible/edited
  working set against the current backend cache before admitting input. Clean
  external changes appear first; dirty pages become Concord conflicts instead
  of being replaced. The check stays bounded by active pages, not graph size.

- Opening a long-lived managed graph got slower the more edit history it had,
  independent of graph size: the startup replay re-validated every remaining
  batch's projection objects once per admission round. Each batch's
  dependencies are now computed exactly once, which roughly halves a
  multi-hundred-save reopen; the remaining history-proportional cost is
  tracked for a deeper fix.

- The one-time app-data migration at startup no longer trusts a momentarily
  unreadable `backups/` folder: any read error now means "assume there is user
  data and leave everything alone", the existing folder is set aside instead of
  deleted until the migrated data is actually in place, and an interrupted
  fallback copy can no longer leave a half-populated app-data folder behind.

- **A formatting-only external rewrite no longer wedges every later managed-storage save of that page.** A Windows peer or external editor may legitimately change CRLF/LF line endings or trailing-newline spelling without changing the outline. Tine used to recognize that reconciliation was a semantic no-op but kept comparing later saves with the old activation bytes, so every save was refused forever. Local-save capture now proves the endpoint's exact live bytes against accepted semantics and uses those bytes as the guarded predecessor. Formatting remains local to that device and out of shared history; a real semantic change still reconciles normally, and a second external write is still protected by the exact-base guard. (GH #362.)

- **On Android, the Back gesture works again once you have navigated.** Tauri's own back handler was registering itself after Tine's — Android gives the gesture to whichever handler registered last — and it answered Back by stepping the WebView's history. With the mobile router pushing one history entry per page you open, that meant every Back after your first navigation quietly moved the page *behind* whatever was on screen: an open Settings modal never closed, and the drawer and the safe-close path never saw the gesture either. Tine now claims the gesture from its own Android plugin, which starts strictly later than Tauri's, so Back peels an in-progress shortcut recording, then a settings search, then the modal, then a drawer, exactly as it always intended to.

- **Saving PDF highlights no longer reformats the annotation page.** The `hls__…` page was written with default formatting rather than its own, so every highlight save re-indented the whole file — including notes you had typed under an annotation — and re-terminated it, even when the highlight set was unchanged. It now reproduces the file's own indentation, line endings and blank lines, and an unchanged highlight set writes nothing at all. (Concord P5 write-shyness, part of GH #337.)

- **A repository inside your graph folder no longer wakes the file watcher.** Events under `.git`, `.hg`, `.svn`, `.jj`, `.bzr`, Syncthing's `.stfolder`/`.stversions`, and `node_modules` are now discarded before they cost anything — a `git gc` or a rebase used to push thousands of events through the watcher's per-event work before each was discarded further down. Nothing that can contain notes is affected. (Concord P5, part of GH #337.)

- **Managed storage on Android turned itself on and then flooded the screen with the same red error, forever.** The graph had one page name written to disk twice — an ordinary thing to end up with: a backup copy, a graph synced between a Mac and a Linux box (which spell accented letters differently), or a title containing a `#` that one editor writes literally and Tine writes escaped. Turning managed storage on already handles that correctly: it keeps the first file as the page and leaves the other one exactly where it is. The trouble came immediately afterwards, because the part of Tine that watches for outside edits did not follow the same rule. It met the second file, decided it was a brand-new page, found its name already taken, and refused — not just that file, but the *entire* reconciliation, so no outside edit to any page in the graph could be imported ever again. And because that refusal repeated on every retry, and each retry passed through a step the app read as "recovered", the same message was raised as a new toast every few seconds with no way to dismiss it. Both halves are fixed. Reconciliation now makes exactly the same choice activation does: the established page keeps the name, the duplicate file is left untouched and simply carries no page, and everything else in the graph reconciles normally. Nothing is ever moved, rewritten or deleted to achieve that, and a page that already exists is never taken away from you. Separately, a condition like this now says its piece once and leaves the live detail to Storage & sync, instead of repeating itself until you close the app.

- **Managed storage on Android saved a page for the first time, and then could not set up sharing.** With the save fixed, the next step of the journey failed instead: turning the graph into a shared one refused immediately with a bare `Invalid argument`. The cause was the same missing rename: Tine's sharing area lives inside the graph, on Android's shared storage, and every file Tine publishes there ends by moving its own leftover temporary entry aside with the rename flag that storage does not implement — so not a single shared file could be written. Those leftovers are throwaway diagnostic copies, so they now move through the same claim-the-name-first publication the graph files use, and sharing completes. One of the sites needed a different answer: it swaps two names at once, and there is no non-atomic stand-in for a swap. Rather than fake it with a three-step shuffle, Tine uses the fact that the destination is already occupied by a zero-length marker it created itself, and does a single ordinary rename onto that marker — atomic everywhere, with no moment where the file being retired exists under neither name. What that gives up is stated plainly and handled: the old name is left free afterwards, so anything that reappears there is preserved as evidence and the operation refuses rather than guessing. Real disk errors still fail, as before, and every refusal in this area now names the exact operation and both filenames instead of a bare error number.

- **Managed storage on Android still could not save a page — the rename that publishes the file is unsupported there.** With the directory-flush refusal handled, the very next operation failed instead: to publish a page Tine moves the live file aside under a hidden name and then moves the new bytes into place, using a rename that the operating system guarantees will never overwrite an existing file. Android's shared storage does not implement that rename flag at all, so every save stalled again — the edit was durable inside Tine but never reached the graph. Tine now recognises exactly the three "this filesystem does not implement that" answers and publishes another way: it first claims the destination name with an exclusive create, which fails if anything is already there, and only then moves the file onto it. The guarantee that matters — never silently destroy a file that already exists at the destination — is unchanged, and a failed publication no longer leaves an empty file behind at a page name. Anything else the filesystem reports (a disk error, a full disk) still fails the write, and files Tine is the sole authority for keep the atomic rename on every platform. The same limitation exists on FAT/exFAT removable media and some network mounts, so the fix is not Android-only.

- **Managed storage on Android could still never save a page — this time the graph write itself.** With the previous refusal fixed, every save on Android reached the point where the Markdown/Org file is written into the graph and then failed there forever: 64 retries per save, all with the same `Invalid argument` from the operating system, so the edit was durable inside Tine but never reached the user's file. Android's shared storage does not always let an app force a directory's contents to disk, which is a barrier Tine uses to make a crash safe. Tine now distinguishes the two kinds of thing it writes: state Tine is the sole authority for keeps that barrier on every platform, while the Markdown/Org file — which Tine can always rebuild from its own already-durable record — accepts that Android cannot provide it, exactly as the app-private setup path already did. A barrier the platform refuses is not a crash-safety problem there; retrying it forever was. Two related improvements: a save that genuinely cannot be finished now gives up after it sees the same failure twice instead of burning the whole retry budget, and every filesystem operation on the graph-write path now reports *which* operation and *which* page failed instead of a bare error number.

- **Managed storage on Android could never save a page.** Activation worked, the page loaded, and then every single save was refused. The cause was one wrong branch: when a save's manifest commit succeeded but its disposable derived state (the SQLite cache and the Markdown projection) had to be retried, the retry was handed to the *retired* storage engine's publication machinery, which the current engine never populates — so it refused instead of retrying. Android takes that path on every save; desktop Linux never did, which is why it stayed invisible. The current engine now finishes its own retained work and reports the save, or defers it for a later retry, and never refuses. Should a retry not settle, the save now also reports *why* the retry was needed, so the underlying platform cause stays visible instead of hiding behind a successful-looking save.

- **Quitting managed storage after an external rescan is no longer slow on a real-sized graph.** Reading one page from the immutable managed baseline used to re-verify the *entire* sealed pack it lives in, so any pass over the graph — including the clean shutdown drain after a rescan — cost time proportional to pages × pack size. A 1,000-page graph spent about 18 seconds draining, and on a slower machine the drain hit its 30-second ceiling and gave up. Each sealed pack is now verified once per open instead of once per page: the same drain finishes in about 2 seconds. Every page's own bytes are still checked against their sealed digest on every single read, so damaged data is still refused.

- **A managed page save that fails for an internal reason now says which internal reason.** Refusals raised by the editor layer against a request the application layer built itself used to arrive as an unattributed "sync actor refused application page intent", so a failure reachable only on a device we cannot attach a debugger to (Android, or a user's machine) carried no evidence at all. Every such refusal on the managed load/save path now names its stage, and an error crossing between the editor and application surfaces keeps that stage instead of dropping it. No refusal decision changed; only what the refusal is able to tell you.

- **Managed-storage recovery and sharing no longer stall or refuse clean shutdown while reconciling an unchanged large graph.** The handoff scan now compares exact accepted bytes without rebuilding semantic mutation authority for every unchanged page; only paths whose bytes actually differ enter the full parser-owned reconciliation proof. The scan still yields between bounded slices, remains visibly pending, and cannot declare a safe shutdown until its exact epoch settles, while a clean drain is no longer capped by a retry count smaller than an ordinary graph.

- **Enabling or joining managed storage no longer takes the graph away while setup is still running.** Tine keeps the exact Direct Files generation serving while it builds and proves the private managed candidate, rejects the handoff if the source changed, and then publishes the ready candidate exactly once. A failure or cancellation therefore leaves Direct Files usable; a crash restarts in either Direct Files or the already-proven managed generation, never a half-switched mode. Settings trusts the native readiness receipt instead of racing it with a second page probe.

- **An external change to a page you are editing is no longer silently dropped.** The watcher correctly declines to yank the caret mid-edit, but the declined reload used to be forgotten: the page stayed stale until some unrelated event touched it again. Tine now records the skipped reload and replays it the moment the blocking state clears — editing ends, a block move settles, or a title rename/IME composition finishes — re-checking at that moment whether the page has meanwhile gained unsaved edits, in which case the normal conflict protocol takes over instead of a reload. (Part of GH #337.)

- **Conflict-copy detection now matches the real formats sync tools generate.** A page whose name merely contains `.sync-conflict-` (say `Foo.sync-conflict-notes`) is no longer silently hidden from the page list as a false-positive Syncthing conflict copy; Syncthing detection now requires the generated `.sync-conflict-YYYYMMDD-HHMMSS-DEVICEID` shape. Seafile conflict copies (`name (SFConflict … ).md`) are now recognized and surfaced in Settings → Backups & recovery instead of appearing as duplicate pages that could hijack page identity via `title::`.

- **The first cross-page subtree move after managed-storage activation no longer stalls forever in projection recovery.** Clean managed storage now reconstructs each accepted CRDT tail on its immutable activation baseline, rather than trying to import a baseline-dependent update into an empty document. The durable move therefore reaches SQLite and Markdown exactly once and remains recoverable after an immediate stop.

- **An external edit no longer becomes unrecoverable merely because an unrelated page was created after the last managed save.** Managed storage retains the latest accepted page projection when its local journal collapses, and validates that page's exact dependencies and current semantic rendering without treating an unrelated catalog advance as a change to the page itself.

- **Managed storage no longer reports a successful authority switch before the graph is usable.** Activation proves the candidate's complete page inventory and deterministic representative pages before its one native publication, then the frontend retires caches leased to the old generation as a reaction to that terminal receipt. The startup recovery button invokes the emergency Direct Files selector immediately—without a native confirmation dialog that can be delayed behind a stuck managed open—and stale managed workers remain unable to publish after that choice.

- **A failed or killed managed-storage recovery can no longer trap Tine's whole window, including unrelated Direct Files graphs.** One native supervisor now owns each graph transition, stale managed workers cannot publish after a newer graph selection, and the recovery screen's emergency return selects and opens the current Markdown tree without waiting for managed recovery. The frontend no longer invents a terminal `native.unavailable` failure from elapsed time. A healthy Settings return remains stricter: it drains managed storage and confirms its projection, while a failed drain offers the explicit emergency escape instead of silently force-stopping authority.

- **Experimental managed storage now preserves baseline `id::` block identity through editing, crash replay, and disposable-SQLite reconstruction.** Exact-frontier SQLite supplies bounded baseline UUID candidates while current CRDT documents remain semantic authority; committed-tail claims are combined with that baseline, and a missing or corrupt projection uses only a rebuild-scoped snapshot rather than restoring a resident Patricia or handwritten identity index. Ambiguous baseline UUID claims remain unresolved.

- Android managed-storage activation now uses the ordinary app-private
  `mkdirat`/`openat`/`renameat` boundary throughout reconstructible projection
  receipt initialization, rather than re-entering capability preflights after
  opening the receipt root. This fixes physical devices that allow normal
  private storage access but reject those Linux-oriented preflights with
  `Permission denied (os error 13)`.

- **Android managed-storage setup no longer requires hostile-replacement directory primitives inside Tine's private receipt store.** The app-private, single-writer tree is created and opened through Android's ordinary verified file API, including its root, while shared or externally writable namespaces retain strict no-follow capability opens. If an older or interrupted candidate left a receipt tree before managed authority was promoted, retry preserves one diagnostic copy and rebuilds that disposable state from the still-authoritative Markdown graph. A setup failure also keeps its exact inner operation when storage rediscovery runs, so a device-specific error can no longer be replaced by a generic retry message.

- **Enabling experimental managed storage no longer requires Android app-private storage to support Linux hostile-path or filesystem-wide primitives.** Tine uses ordinary app-private opens and validates the resulting file and directory handles, while permission or capability refusals from filesystem-wide flushes fall back to flushing each exact bootstrap tree. The same audit removed mandatory directory-fsync assumptions from graph-local shared-provider setup and publication while preserving file flushes, type and size checks, and strict durability for private authority. Ordinary graph and app-private access is sufficient; stronger platform-specific operations can no longer turn it into a misleading `Permission denied` setup failure.

- **Experimental managed-storage startup failures now say what safety condition actually stopped the open.** Durable refusals carry a stable contract scenario separately from their bounded reason/stage code all the way through the native API and Settings diagnostics; temporary I/O failures remain retryable instead of being mislabeled as corrupt data. The source guard requires every public refusal class and every durable blocked reason to stay mapped to the documented vocabulary.

- **Managed-storage activation failures now retain the exact activation and source-capture stage in logs and the visible diagnostic.** A platform filesystem error can no longer collapse to an unactionable raw `Permission denied`; Android device reports identify the operation that needs a compatibility repair.

- **A corrupt experimental managed-storage reconciliation cache no longer makes the graph permanently unopenable.** Tine preserves the exact disposable SQLite baseline files as diagnostics, rebuilds a fresh baseline, resumes safely if either the preservation or replacement was interrupted by a crash, and leaves authoritative oplog history and graph bytes unchanged. Unsupported filesystem entries are still refused without following them.

- **A paste can no longer delete text you typed while it was still in flight.** Pasting into an empty block on a page under experimental managed storage decided *before* its background work whether that block was empty enough to be replaced. If you kept typing while the paste ran, that decision was already stale and the block — with everything you had just typed in it — was removed. The decision is now made from the live block at the moment of insertion, on both storage paths. The same paste also re-checks the page's size limit against the page it is actually about to change, so an insertion the page has since outgrown is refused instead of stranded (GH #322).

- **Experimental managed storage now stops accepting large edits as soon as its writer starts failing.** If the background writer failed repeatedly, Tine kept reporting that the graph was writable — the failure notice is only sent once, and nothing else was sent afterwards — so pastes, drops and captures were accepted onto pages nothing could save, and were lost on reload. A writer failure now withdraws that permission immediately, and only a fresh confirmation from storage itself restores it (GH #324).

- **Pasting a large table into a Sheet under experimental managed storage no longer changes the page before the size is checked.** Sheet paste had its own clipboard path that skipped the size check entirely, so a big CSV or TSV filled in the cells first and the refusal only arrived at save time, leaving an edit that could not be written. Sheet paste now goes through the same prepare-then-apply route as the other Sheet commands: the cells are built off to the side, storage validates the whole resulting page, and the Sheet you are looking at changes only if that succeeds (GH #320).

- **Dropping files while you are switching a graph to experimental managed storage no longer inserts them behind the switch.** A drop decided which storage path to use the instant it started, then read the files — and enabling managed storage takes long enough (it asks you to confirm) that the drop could finish afterwards and insert under the old decision, skipping every size check the new storage requires. Both the drop and the clipboard paste now re-prove the storage route, alongside the graph and the target block, immediately before they change anything (GH #325).

- **Large pastes and captures under experimental managed storage are now measured against the whole page, not just the new text.** The size check that is supposed to refuse an insertion *before* the page changes only weighed the incoming text, so adding a little to an almost-full page was accepted and then could not be saved. It also let through text that landed exactly on the limit, which the writer counts as over. Both are fixed, and the estimate is deliberately conservative in the safe direction (GH #323).

- **Sheet structure commands now stay atomic when experimental managed storage must validate them first.** Row and column changes, rectangular clear/cut/paste/fill/move, edge growth, and field renaming are prepared as one detached page candidate. Direct Files still applies them synchronously; managed storage now refuses stale, oversized, unavailable, or overlapping commands before the live page, undo history, selection, or dirty state changes.

- **Experimental managed-storage cross-page move recovery now resumes the exact durable move after an immediate process loss.** The actor reconstructs only an episode-authenticated immutable local manifest, completes accepted SQLite/projection/provider work exactly once, and transfers a real external-edit conflict to the existing exact feed without overwriting the external bytes or reporting deleted affected pages as successful.

- **A managed-storage writer that has entered a terminal failure can no longer report a clean shutdown.** Tine keeps the graph protected and reports the unsaved state instead of letting close imply that durability was proven.

- **Parseable Markdown and Org that cannot be reproduced byte-for-byte can now be opened safely under experimental managed storage.** Tine preserves the original bytes and exposes the page read-only rather than rejecting activation or rewriting syntax it does not own.

- **Repeated moves under experimental managed storage keep the same live-resolution semantics as the editor.** The durable transaction follows the accepted current position instead of replaying an obsolete positional assumption.

- **Direct Files now rechecks page-creation authority at the final publication boundary.** A page or portable-equivalent path created externally after the initial check is detected before Tine publishes, closing the remaining race documented with GH #321 without overwriting external bytes.

- **Multiline paste inside `$$ … $$` display math stays in the same block**, matching Enter inside the same construct instead of splitting the expression into outline blocks (GH #278).

- **Task-marker clicks, full-screen images, and native text selection now target only what they should.** Clicking a task label uses Logseq's open-state toggle without removing `DONE`; generic `custom.css` image rules no longer constrain the lightbox; and selecting page text excludes Add block and reference-section interface labels (GH #259, GH #319, GH #328).

- **Large graphs finish their useful startup work sooner.** Optional home-page and related scans no longer delay opening the graph and are guarded against a late result replacing newer state (GH #266).

- **Experimental managed storage admits very large individual pages without false memory failures or thousands of tiny recovery resumptions.** A 20,000-block page is covered by the bounded import and recovery path (GH #311).

- **Moving blocks across structural boundaries now matches Logseq on Android and desktop.** Up and Down cross child-list edges correctly instead of getting stuck or landing at the wrong hierarchy level (GH #312).

- **Renaming a page onto an existing page now offers a complete Logseq-compatible merge.** Content, links, aliases, open panes, and storage transactions move together instead of leaving the collision unresolved (GH #327).

- **Property autocomplete can be completed entirely from the keyboard.** Suggestions open, filter, select, and accept consistently without requiring a pointer click (GH #306).

- **Dragging into a deeply nested target zone now makes the block a child of that target**, matching Logseq rather than flattening it beside the target (GH #326).

- **Middle-click opens a background tab in the pane that was already active.** Clicking a link rendered in another pane no longer steals pane ownership before opening the tab (GH #87).

- **Windows update checks now use the native trust store and ask separately before installing.** A successful check no longer turns immediately into an install, and certificate environments accepted by Windows no longer fail solely because the bundled TLS roots differ (GH #241).

- **Editable emoji use native color faces on healthy platforms again**, while the WebKitGTK COLRv1 crash-prone path remains avoided (GH #293).

- **Bundled Inter exposes its OpenType stylistic sets and character variants to `custom.css` again** (GH #298).

- **Concurrent block embeds from the same source page all hydrate.** One embed finishing no longer causes its sibling to miss the source group and remain empty (GH #315).

- **A fresh second device can join an experimental managed-sync graph, including from Android shared storage, or leave it for Direct Files before it has a local binding.** Provider bookkeeping files no longer obscure the canonical shared descriptor; Android no longer applies a desktop Unix owner-ID rule that its shared-storage layer does not promise; and local filesystem failures are no longer mislabeled as sync data that is still arriving. The two-device release journey proves setup, join, edits in both directions, convergence, restart, and the escape path, while the signed Android candidate compiles the platform-specific storage path.

## [0.6.93] - 2026-08-12

### Added

- **The built-in Guide is now a real manual rather than a feature tour.** Thirteen new pages cover the journeys people actually arrive with — bringing an existing Logseq graph in, where things are, capturing and planning a day, finding and revisiting, researching a document, keeping context visible, structuring repeated information, extending Tine, and what platforms exist — alongside precise reference pages for journals/tasks/scheduling, pages/links/references/search, files/external edits/backups, troubleshooting and recovery, and platforms and mobile. The index is grouped into Start, Workflows, Feature reference and Feature showcase, and every page is a read-only page in your own graph that you can copy into it. Every claim was reread against the shipped UI at the end, which is where several long-standing wrong labels and one round-trip overclaim were found and corrected.
- **Sheet table columns can now be resized and reset from their header edges.** Widths persist on the table's real block or tag/schema page by field identity, so they survive reloads and column reordering without moving to another field; wide columns scroll horizontally instead of crushing their text (GH #316).
- **Journal Settings now includes the `E, dd.MM.yyyy`, `EEE, dd.MM.yyyy`, and `EEEE, dd.MM.yyyy` title formats** already supported by Tine's date engine (GH #317).
- **Opening a graph now tells you what Tine is doing, and offers a way out when it cannot finish.** A progress card names the current phase and elapsed time once startup takes longer than a moment. If the open genuinely cannot complete, it becomes a recovery dialog with Retry lookup, Open another graph, Copy details, and — for a graph enrolled in experimental managed storage — Return this graph to Direct Files. It states plainly that no managed-storage data has been discarded. Previously a startup that stalled showed nothing and offered nothing.

### Changed

- **The retired v1 managed-storage prototype has been removed.** v1 was a pre-release format that never shipped enabled; its recovery paths, activation fence, lifecycle API and model path were already inert. Leftover v1 bytes in a graph directory are now simply ignored and can no longer cause a Direct Files open to be refused. Direct files, the default, is unaffected.

### Fixed

- **Direct Files can create a page or journal immediately after opening a graph.** A cold parsed-page cache was treated as evidence that the requested name already existed, so ordinary creation could be refused until background warming happened to finish. Creation now joins one bounded, generation-coherent identity build and proceeds only from its published evidence; malformed owners, generation drift, aliases, links, symlinks, and an external creator of the same file still fail closed without overwriting bytes (GH #318).
- **Experimental managed storage now refuses oversized bulk insertions before changing the page.** Multiline and structured paste, file drop, template expansion and quick capture are planned against the exact active runtime's page limits before anything is inserted; a stale, unavailable, or mismatched storage binding fails closed instead of allowing a partial edit that the native writer cannot save. Pasting a large CSV or structural selection directly into a Sheet's cells is not admitted this way yet (GH #320).
- **Heading commands and bullet dragging now act on the active block selection even when the pointer is on another block.** Multi-block heading changes and drags are atomic, undo together, preserve subtree order, and keep cross-page moves destination-first on disk (GH #240).
- **Indenting, outdenting and moving a multi-block selection no longer floods the undo history or the store.** Indent and outdent now apply as one shared-store command instead of one per block, and a run of Up/Down moves collapses into a single undo unit that closes on an idle pause, a three-second maximum, a change of selection, any other edit, or a reload — so one Undo reverses the move you just made rather than one block of it. A feed-spanning selection that includes a read-only day is still refused without dirtying either page.
- **Pasting a large selection that carries preserved block IDs is no longer quadratic.** Each pasted candidate used to scan the whole loaded document for an ID collision; 200+ candidates now share one pass, and the cost of the incoming check stops growing with the size of the page you paste into.
- **A managed-storage graph whose cold start failed can no longer trap you in it.** Malformed retained scratch left over from an interrupted run refused the open permanently instead of being recovered; the exact cold-start slot is now recovered, the recovered runtime is proven to accept saves again, and an escape back to Direct Files stays available and actionable throughout. Managed storage remains experimental and off by default.
- **Creating a page in Direct Files no longer stalls and then refuses the save on a large graph** (part of GH #249, GH #266 and GH #267). The creation path ran the managed-storage shadow-import capture twice, retaining and parsing every graph file before it could answer whether one filename was safe. On affected graphs that ended in `precheck.limit`, stranded the new edit, and made graph transitions or close wait behind repeated failed flushes. Creation now checks one non-retaining streaming census—without applying the retained-shadow cumulative-byte ceiling—against the already parsed, generation-current Direct Files index and passes that single proof to the no-replace publisher. Creation still refuses an existing target, portable case/NFC aliases of that target (including ancestor directories), semantic owners, hard links, symlinks, and an external creator of the same file without overwriting their bytes; existing exact-target saves retain their prior path-local traversal behavior. A conflicting *different* path created by another process inside the final publication window is still not detected, which is long-standing behaviour rather than new here (GH #321).

## [0.6.92] - 2026-08-11

### Added

- **Ctrl+O opens the page under the caret**, and Ctrl+Shift+O opens it in the sidebar — matching Logseq, so you can navigate `[[links]]` and `#tags` without the mouse (GH #274).
- **A link to a page that doesn't exist yet is now dimmed**, with a dotted underline, so you can see before clicking that it will open a blank page. The link still works and still creates the page. `#tags` are deliberately left alone, since a tag with no page file is perfectly normal. Logseq does not make this distinction; it was added after a graph whose links all pointed at names no page had, with nothing on screen to say so.

### Changed

- **Documenting a change that shipped undocumented in 0.6.90:** a page's name comes from its `title::` property when it has one, and only otherwise from its filename — which is what Logseq does (`title::` → filename → first block). Before 0.6.90 Tine used the filename and ignored `title::`. If a graph was built against the old behaviour, files carrying a `title::` that differs from their filename are now reachable under the *title*, and `[[filename]]` links to them will open blank pages. The new dimming above makes those visible; either drop the `title::` or link to its value.
- **Experimental Tine-managed storage moved a great deal this cycle** — journal
  format and rollover, enrollment-integrity migration, task and block queries
  served from the sparse index, crash-recovery and save/query latency ceilings.
  None of it changes anything for Direct files, which remains the default and
  what this release is really about. Managed storage is still marked testing
  only, still off unless you turn it on, and GH #292 (setup failing on some
  graphs) is still open.
- **Managed storage imports roughly twice as fast, and read about eight times
  fewer bytes.** Bringing a graph into managed storage was re-reading, re-hashing
  and re-decoding the *entire* batch of pending objects in order to fetch a
  single one — once per document, and again once per coordinator tick. On a real
  1,000-page graph that meant gigabytes of redundant work: importing 300 pages
  read 3.59 GB to move about 1.3 MB of notes. Objects are now fetched
  individually by the content digest the caller already holds, and the
  per-tick check reads only the batch manifest, which already records everything
  it was asking for. Measured on a real graph: 100/200/300/400/600 pages now take
  5.6/14.5/26.0/42.2/84.9 s, down from 10.8/29.1/50.2/80.0/157.0 s. No safety
  guarantee changes: objects remain content-addressed and are still verified
  individually when read.

### Fixed

- **Background page loads and conflict actions can no longer cross editor instances.** Every editable page now receives a core-issued identity when it is installed, and replacements use an exact two-phase handoff. A late sidebar, query, journal-feed, PDF-notes, watcher, save, or “Use disk version” response is refused if the graph or editor changed while it was in flight, so it cannot replace unsaved work, inherit another editor's conflict authority, or update the replacement's save baseline. “Use disk version” remains a read-only action in every outcome. (GH #254)
- **Android Back now stays under Tine's safety owner from startup through close.** Before the app interface is ready, Back no longer falls through to browser history or a raw activity exit. At the route root, Tine verifies every managed graph can stop before requesting activity exit: a zero-progress refusal leaves the graph editable and retries the complete close, while a partial or interrupted native check keeps the graph protected and retries only that native check. Only a verified safe result may request the final activity exit.
- **Managed-storage page and journal deletes now preserve the exact accepted file in typed trash before they accept the tombstone.** A failed trash write, a changed projection, or a concurrently accepted update leaves both the file and the open draft in place; successful deletes keep the source bytes under `logseq/.tine-trash/pages` or `journals`.
- **Your typing is now written to disk while you're still typing.** Tine waited for a pause before saving, and every keystroke restarted the wait — so during a long fluent passage nothing reached the file at all. Measured: twelve seconds of continuous typing, at a pace slower than most people type, produced no write. A save now happens at least every three seconds while you keep going. Found by the 2026-08-09 Direct Files data-safety audit.
- **"Use disk version" now reloads the file you were actually editing.** When two files share one page name — a duplicate day, or same-titled pages in different folders — Tine pins the page to its own file so saves go back to the right place. The conflict banner didn't honour that pin: it looked the name up instead and loaded the *other* file over your tab, discarding your unsaved work along the way. Found by the same audit.
- **Deleting a page with unsaved edits now warns that they are not recoverable.** The prompt said the file moves to `.tine-trash`, which is true — but the trash gets the *file*, so anything you typed since the last save (and everything on a page with an unresolved conflict, which by definition was never written) went with it and was not in the trash. The prompt now says so while you can still cancel.
- **Closing no longer offers to throw away a save that is still running.** If writing your changes took more than four seconds — a slow or network drive is enough — Tine reported it in the same words it uses for a save that can never succeed, and offered to close and lose the work. It now says it is still saving, waits, and only asks once the save has genuinely failed or stalled. Found by the 2026-08-09 Direct Files data-safety audit.
- **A conflict banner now goes away by itself once the file is back to what you loaded.** Some editors and sync clients briefly remove a file while replacing it; Tine treated the gap as a conflict and then had no way to un-conflict a page, so it stayed stuck behind a banner whose two buttons both destroy something. Tine now re-checks: if the file is back and matches what your editor started from, the banner clears and your edit saves normally. A file that comes back genuinely changed still raises the conflict. Found by the 2026-08-09 Direct Files data-safety audit.
- **The All Pages sidebar list renders faster on a large graph.** Every visible row was re-scanning the entire page list to work out whether its name needed disambiguating — about 21 ms at 5,000 pages and 39 ms at 20,000, repeated on every render. The same information is now worked out once per list. Names are labelled exactly as before. Found by the 2026-08-09 Direct Files performance audit.
- Deleting a page no longer freezes the app while it works. On a large graph the delete could block for the better part of a second (longer on very large graphs) because it ran on the same thread that dispatches every other command.
- Text typed while a synced file change was being loaded is no longer silently discarded. Tine decided it was safe to reload the page *before* fetching it, and never re-checked — so anything typed during the fetch was replaced by the disk version, along with its undo history.
- Fixed a state where Tine would insist it had unsaved changes forever — blocking every graph switch and offering to discard your work on every window close — for edits that were in fact already written to disk.
- **Edits are now saved when the app is backgrounded**, not only when a window is closed cleanly. On Android and iOS the system can reclaim a backgrounded app without warning, so anything typed since the last pause could be lost; desktop had narrower versions of the same hole (GH #255).
- **Editing one block no longer rewrites blocks you never touched.** On a real-shaped 1,045-file graph, editing a block and undoing it failed to restore 96 of 983 files: an unbulleted `## Heading` gained a `- ` prefix, and in the worst case the file's indentation was pushed *into* the note text and the block gained a nesting level. Found by the 2026-08-09 Direct Files data-safety audit; the same measurement now reports zero damaged files.
- **Typing no longer makes the next click slow.** Every save threw away Tine's index of which pages exist, and the only way to rebuild it was to re-read and re-parse the whole graph. So the first navigation or `[[` autocomplete after each typing pause stalled for about a quarter of a second on a 5,000-file graph, and longer on bigger ones. Found by the 2026-08-09 Direct Files performance audit.
- **Each save costs less on a big graph, and a pause in typing no longer ships the whole graph's link list across.** Two things ran on every save: a scan that rebuilt a normalized key for *every* filename in each directory along the path (4.87 ms per save with 4,000 pages in one folder, against 0.70 ms with the same pages spread over subfolders — unbounded growth in the one directory that only ever grows, inside the lock that guards the write), and a refresh of the full referenced-page-name set (15–23 ms and ~5,000 names at 5,225 files, parsed on the UI thread) that fired about 700 ms after every lull even though typing inside a block almost never changes which pages are linked. The scan now folds each path component once — which also fixes it being ~1.8× *slower* than before for any graph whose names carry diacritics — and the name set is only shipped when its digest actually changed. Found by the 2026-08-09 Direct Files performance audit.
- **A file changed outside Tine is noticed promptly again when you have more than one graph open.** If watching one graph's folder failed while another succeeded, the watcher could sit blocked on the healthy graph instead of retrying the failed one, so external edits to the failed graph went unseen for as long as the other stayed quiet. The wait is now bounded whenever any root is still unwatched.
- Linked and Unlinked References no longer collapse themselves while you scroll or expand a group on a large graph; the section now keeps the state you put it in (GH #272).
- **Unlinked References now finds mentions inside code blocks, inline code, math and other literal text**, as Logseq does — previously a page name written inside a fenced block, `$$…$$`, raw HTML, an example block or a hiccup vector was invisible to the panel (GH #270). Mentions inside `[[…]]` or after `#` still don't count, including inside code where there is no parsed link to recognise. The scan also got considerably faster: on a real 1,045-file graph an unlinked-reference lookup went from about 285 ms to 36 ms, despite now searching more of each file.
- Jumping to a block from the Quick Switcher (or any "go to block" action) now expands its collapsed parents, so the block is actually scrolled to and highlighted instead of the jump silently doing nothing (GH #258).
- Pressing Enter inside a multi-line `$$ … $$` math environment now inserts a new line and keeps the block together, instead of splitting it and breaking the environment (GH #278).
- **Undo can no longer silently overwrite a file that changed while you were away.** Tine keeps about eighty pages in memory and drops the oldest ones as you browse past that, deliberately keeping their undo history. But the kept history described the copy that was dropped: re-opening the page read the file fresh, and pressing undo then restored the *old* content and saved it — against the new file's revision, which the save guard accepts, because that baseline genuinely matched what was on disk. Nothing looked like a conflict. Undo entries now name the exact loaded copy they were recorded against; when that copy is gone, the entry is discarded and Tine says so instead of replaying it. Found during review of GH #254 (GH #305).
- **Tine no longer dies at launch when its data directory cannot be written.** On a machine where `~/.local/share` is owned by root — or `XDG_DATA_HOME` otherwise points somewhere unwritable — the app printed a Rust backtrace and quit, because Tauri creates the WebView's data directory during its own startup and panics on the error. Tine now checks that directory before anything resolves paths against it, moves the whole launch to the first writable fallback (`~/.tine-data`, then the runtime directory, then a temp directory), and says so in a sticky notice naming where it went. If nothing is writable it prints one actionable line instead of a backtrace (GH #303).
- **Images and other embedded media stored in a subfolder of `assets/` now display.** Every native read path applied the top-level-only rule that belongs to asset *creation*, so a file under, say, `assets/screenshots/` stayed blank even though the link was correct. Nested paths are now read, while absolute paths, `..` traversal and symlink escapes are still refused, and newly imported assets still land directly in `assets/` (GH #300).
- **A page changed by another program no longer becomes permanently unsaveable.** Most editors and sync clients (Syncthing, Dropbox) write a file by creating a new one and renaming it into place. Tine treated the result as a different file and refused every subsequent save with an internal message it then retried forever — even when the new content was byte-for-byte what Tine already had, and even when it was a genuine change you could have resolved. Tine now compares the content: identical content just saves, and a real difference raises the normal conflict banner. "Keep mine" is now bound to the exact disk version shown by that banner (including a deletion), so it cannot overwrite a newer unseen sync or editor change; a newer change is shown as a new conflict instead. (GH #254)

## [0.6.91] - 2026-08-07

### Added

- **Hierarchical Ctrl/Cmd+A and symmetric Shift+Up block selection.** In the
  editor, Ctrl/Cmd+A still selects the block's text first; a further press
  selects the block's subtree, then each ancestor's subtree, then the whole
  visible outline, where it stays. Shift+ArrowUp extended onto a parent no
  longer leaves the parent's later children behind: a selection that includes
  an ancestor of its anchor covers that ancestor's whole visible subtree,
  mirroring Shift+ArrowDown.
- **Block-boundary keyboard navigation and forward merge.** At the exact end
  of a block, Delete now merges the next visible block's text and children in
  (mirroring Backspace's merge into the previous block at offset 0), and
  ArrowRight moves into the start of the next block; ArrowLeft at the very
  start moves into the end of the previous block. Annotation and calc blocks
  are never merged in either direction, and a single undo restores both
  blocks after a forward merge.
- **Drag-reorder for sidebar lists.** Right-sidebar open items and
  left-sidebar favorites can be reordered by dragging the row (right sidebar:
  the header area). Sub-threshold presses stay ordinary clicks and the click
  that ends a drag is swallowed, so navigation, middle-click, context menus,
  close buttons and scrolling are unaffected. Order persists through the
  existing owners (right-sidebar session state; favorites in the graph's
  `config.edn`).
- **Configurable graph home page.** The Graph settings tab can now pick an
  existing page as the graph's home page; it opens in-place in the primary
  tab whenever that graph is opened. A deleted or renamed page is skipped
  silently at startup and surfaced in Settings with a Clear option. The
  setting is stored per graph via the existing app-settings owner.
- **"System" appearance option.** The Appearance settings now offer Light /
  System / Dark. System follows the OS/WebView `prefers-color-scheme` signal,
  applying it immediately and tracking live changes while the app runs; manual
  Light/Dark selections ignore system changes as before. Existing saved
  Light/Dark preferences are untouched.
- **"Open in new tab" in the block context menu.** Right-clicking a block now
  offers "Open in new tab", mirroring the existing page context-menu item and
  block-bullet middle-click. Works on both editable and read-only blocks.
- **One-click copy on calculator results.** Hovering a successful ```calc
  result shows a copy button that copies the displayed value to the clipboard
  via Tine's existing clipboard facade. Error and blank result lines do not
  offer the button. The button is available in both the rendered calculator
  and the live editor's result column; clicking it does not mutate the block
  or create an empty row.
- **`/That day` slash command.** Inserts a `[[date]]` reference to the
  containing journal page's date (not today's clock date), using the
  configured journal title format. On a non-journal page it does nothing and
  shows a toast.

### Fixed

- **Linked References, search, quick switch, queries and most of the rest of the
  app now work while Tine-managed storage is active.** Every page showed
  "Couldn't load references because the backend request failed", and the same
  refusal silently disabled search, quick switch, `{{query}}`, aliases,
  templates, page icons, block references, asset browsing and more: only page
  loading, saving and the journal feed had been given a managed path, and
  everything else was refused for not holding the older storage mode's
  authority. Reading and writing are now separate permissions, so whole-graph
  *reads* are answered from the managed graph's own files while writes stay with
  managed storage. Actions that change many pages at once — renaming, deleting
  and merging pages, and resolving sync conflicts — are still unavailable under
  managed storage and now say so plainly instead of reporting a failed request.
- **Settings changes save again while Tine-managed storage is active.** Every
  toggle in Settings was dead: favourites, task workflow, file format for new
  pages, journal title format, first day of the week, showing brackets, logical
  outdenting, Enter behaviour in document mode, time tracking and the default
  journal template all appeared to work and were forgotten on the next launch.
  Settings live in the graph's own `logseq/config.edn`, which managed storage
  does not own, so writing them never needed the permission that was being
  asked for. One exception remains: changing the journal title format no longer
  renames existing journal files under managed storage.
- **Cleaning up orphaned assets works again while Tine-managed storage is
  active.** Moving an unused image to the recoverable trash, and emptying that
  trash, were both refused — not because either touches your pages, but because
  the trash folder was covered by the same permission as page text. Deleting a
  page, journal or conflict copy is still unavailable.
- **Error messages from the references panels no longer hide what went wrong.**
  Both panels replaced every backend message with a generic "the backend request
  failed", including messages that explained the cause exactly. They now show
  what the backend actually reported.
- **Fewer false "changed on disk" warnings while editing a managed graph.** The
  background sync loop announced a change after every completed pass, including
  passes that committed nothing. Arriving while a page had unsaved edits, that
  contentless signal could be read as a conflict — which then blocked the very
  save that would have resolved it. Only a pass that actually committed
  something now reports a change.
- **A managed graph that was closed cleanly now reopens even if its internal
  cache database was deleted or damaged.** That cache is rebuildable from the
  graph's own history, and a graph closed by a crash already rebuilt it — but a
  graph closed *properly* refused to open at all, and the error offered a retry
  that could never succeed. Clearing a cache directory, a backup that skipped
  the file, or on-disk corruption could therefore make a graph permanently
  unopenable. Both cases now rebuild. A cache that is present and current still
  opens directly, so ordinary startup is unchanged.

- **"Keep mine" now works.** When Tine found that a page had changed on disk
  behind your back, it offered you two ways out — keep your edits, or take the
  disk version. Keeping your edits could never succeed for any page opened from
  a file: it failed every time with an internal message about a "captured exact
  owner", leaving discarding your own work as the only exit that did anything.
  It now writes your version, including after a genuine external change, which
  is the whole situation the prompt exists for.
- **Errors that are not conflicts no longer pretend to be.** Any unrecognised
  save failure was reported as a conflict, which put up that same two-button
  prompt — and neither button could resolve, say, a filename collision or an
  internal storage failure. It also replaced the real error text, so in the one
  case where Tine had safely set your unsaved bytes aside under a recovery file,
  the message telling you where they were is what got thrown away. Unrecognised
  failures now report what actually happened.
- **A "keep mine" during a cross-page move can no longer lose the moved block.**
  While a block moved between pages is being written to its destination, Tine
  holds back the source page so the block is never briefly in neither place.
  Resolving a conflict used to walk straight through that hold; it now waits and
  is applied the moment the destination is safely written.
- **The "couldn't save" message no longer promises a retry it wasn't making.**
  After three failed attempts Tine said it would retry and then scheduled
  nothing. The page was still saved on your next edit — which is what it now
  says.
- **External edits reach Tine-managed storage again.** Every reconcile of a
  change made outside Tine was refused with an internal mismatch and retried to
  the limit, so edits made by another program to a managed graph never came in.
  Two internal descriptions of the same unchanged bytes disagreed about a
  detail neither of them needed to agree on.
- **A refused save in Tine-managed storage now tells you.** A managed save that
  could not proceed retried quietly forever without ever surfacing; you could
  close the app believing the page had been written. It now reports the refusal
  and what resolves it.
- **Windows: a managed page rename no longer overwrites a file that appears
  underneath it.** The rename was documented and implemented as
  never-replace everywhere except Windows, where it would overwrite a file that
  arrived in the moment between the check and the rename.
- **A sync client working in the background no longer fails your save.** Before
  writing, Tine looks over the graph twice and requires the two looks to agree.
  Any file changing anywhere in the graph in between makes them disagree — which
  on a Syncthing, Dropbox or OneDrive folder is simply what a normal minute looks
  like — and a single disagreement failed the save outright. Disagreeing means
  something moved while Tine was looking, not that anything is wrong, so it now
  simply looks again.
- **A save that cannot succeed says so once, instead of trying twice more
  first.** When a save failed for a reason no retry could change, Tine retried it
  anyway, twice, each time redoing the whole (potentially slow) pre-save check
  before finally showing the error. That multiplier is a large part of why
  #267 reported "about a minute, then a red toast". Genuinely temporary failures
  still retry.
- **A page whose file was replaced behind Tine's back is no longer stranded.**
  Some tools replace a file wholesale even when its contents don't change —
  OneDrive filling in a file it had only been holding a placeholder for, a sync
  client landing the same bytes, a plain copy over the top. Tine checks both that
  the contents are what you started from *and* that it is the same file it read,
  so these left a page it refused to save: "existing page identity changed since
  load". Worse, the conflict prompt that followed had no working way out —
  "Keep mine" hit the same check and failed, and the only button that did
  anything threw your edit away. When the contents on disk are still exactly what
  you started from, Tine now accepts the replacement quietly. A file whose
  contents really did change is still a genuine conflict and still asks you.
- **One symlink no longer makes your whole graph read-only.** A symbolic link
  anywhere Tine looks — at the top of the graph, in `pages/`, in a folder of your
  own — made *every* save fail, including saves of completely unrelated pages.
  Tine never followed symlinks anyway; the pre-save check was the only place that
  treated finding one as fatal rather than as something to step over. It now
  steps over it. (Importing a graph into managed storage still stops, because
  quietly leaving a file out of an import is worse than refusing it.)
- **Errors that a conflict prompt can't fix no longer pretend to be conflicts.**
  Several unrelated save failures — two files whose names collide on
  case-insensitive filesystems, two paths pointing at the same physical file,
  another page already holding the title — all surfaced as "this page changed on
  disk", offering you a choice between two options that could not resolve any of
  them. Whichever you picked, the page was then marked as conflicted and silently
  stopped saving. Each now reports what actually happened.
- **Saving a page no longer re-reads and re-analyses your whole graph
  (GH #267).** Before writing, Tine checks that no other file in the graph is
  about to collide with the one you are saving. Whenever it had lost track of
  what was on disk — which, on Windows or a graph on a network drive, was
  essentially all the time — that check re-analysed every single document you
  own, on every save. On a large graph that is the difference between a save you
  don't notice and a save that takes long enough to give up and show an error.
  Losing track means Tine doesn't know what changed, not that everything did, so
  it now re-reads only the documents whose contents actually differ from what it
  last saw, and reuses what it already knew about the rest. Measured on a
  1,000-page graph: the documents examined per save dropped from all of them to
  three. Nothing about the safety check itself was relaxed.
- **Edits made outside Tine to pages that are not in `journals/` or `pages/`
  now show up while Tine is open (GH #268).** If you keep pages at the top level
  of your graph or in your own folders, Tine listed them correctly when it
  opened the graph, but then never noticed another editor or a sync client
  changing them — the page you were looking at simply stayed stale until you
  reopened the graph. Tine watched the whole graph folder all along; the part
  that decides which of those changes to act on had been left looking only at
  `journals/` and `pages/`. It now uses the same rule the rest of Tine uses to
  decide what counts as a page, so external edits, creations and deletions
  anywhere in your graph arrive the same way they already did under `pages/`.
  Folders Tine deliberately ignores (`assets/`, hidden folders, `logseq/bak/`)
  stay ignored, so dropping in an image still costs nothing.
- **Graphs on a network share no longer re-examine the whole graph on every
  save.** When Tine cannot use the operating system's file-change notifications
  — typically a graph on a network drive — it checks the folder every few
  seconds instead. Each of those checks was throwing away everything Tine knew
  about the graph's files, so the very next save had to walk and read the entire
  graph again before it could write. Saving got slower the bigger your graph
  was, permanently. The periodic check now records what it actually found, so an
  ordinary save no longer pays for it. If a check cannot read part of the graph,
  Tine still starts over from scratch rather than trusting an incomplete
  picture.
- **A folder Tine cannot watch is no longer a silent failure.** If the operating
  system refuses to install the folder watch — too many watches, a permission
  problem, an unavailable network share — Tine kept retrying quietly while every
  outside change went unnoticed. It now tells you once per distinct problem, and
  keeps retrying.
- **App now starts on macOS 12 with an older Safari/WebKit (GH #256).** A JS
  regex lookbehind shipped in the eagerly loaded bundle; pre-16.4 WebKit can't
  parse it and the app died with "SyntaxError: invalid group specifier name"
  and a white screen. The regex was rewritten lookbehind-free (byte-identical
  behavior) and a source guard test now rejects any lookbehind in `src/`.
- **Ctrl+F now scrolls to the exact occurrence, not just its block (GH #253).**
  In a block taller than the viewport, in-page find previously only centered
  the block, leaving off-screen matches invisible; the reveal now centers the
  active occurrence inside the pane (falling back to block-centering while the
  block is being edited).
- **Update failures are no longer silent (GH #241).** When the desktop
  self-update fails (signature, download, or install), the error is now written
  to the debug log and shown in a toast instead of indistinguishably opening
  the releases page; the releases page still opens as the safe fallback. The
  underlying Windows update failure reported in #241 is not yet diagnosed —
  the now-visible error message is exactly what a Windows repro needs.

- **Reopening a lived-in graph after a crash no longer rebuilds it.** Opening a
  managed graph that had been edited over many sessions could stall for over a
  minute after an unclean shutdown while it rebuilt its search/index database
  from scratch — and the delay grew faster than the graph did. It turned out the
  database had nothing wrong with it: it was already exactly up to date, but its
  identity included a note of where the previous run happened to keep some
  in-memory data, and reopening a graph moves that. Tine compared the two, saw a
  difference that meant nothing, and concluded the database was corrupt. That
  bookkeeping detail is no longer part of what identifies a graph's state, so an
  up-to-date database is now recognised and simply opened. Measured on a
  1,046-file graph: 72 s to 2 s, with the database step itself down to a few
  milliseconds and no longer growing with the size of the graph at all.
- **A page you are editing is no longer falsely reported as “changed on disk.”**
  Editing a block and immediately moving it could raise the “changed on disk
  (edited elsewhere or synced in)” banner with no external editor or sync
  involved — and because a page with an open conflict refuses to save, the
  banner's own warning that your unsaved changes weren't written became true
  only because the banner appeared. Tine now requires proof that the page
  actually diverged — its stored revision no longer matches the one the editor
  holds — before declaring a conflict, instead of treating any change
  notification arriving during an unsaved edit as one. A save already in flight
  is left to its own base-revision guard. Genuine external edits, and a file
  deleted out from under an unsaved edit, still raise the banner exactly as
  before.
- **Journal carry-over buttons now switch correctly at midnight.** The buttons
  under a journal title were chosen once at mount from a bare wall-clock
  comparison, so after the calendar rolled over, yesterday's journal kept
  today's pull-in buttons ("Carry from previous day", "Carry last N days")
  instead of switching to "Carry unfinished tasks → today". The choice now
  follows a reactive day tick (DST-safe, re-synced on focus/wake), so an
  already-open app swaps the button sets the moment the day changes.
- **Delete/Backspace over text selected in a rendered (not-editing) block now
  deletes that text.** Selecting a block's text with the mouse and pressing
  Delete or Backspace did nothing — the keypress reached no editor, the
  highlight stayed, and nothing changed (OG deletes it, since its rendered
  view is the editor). The key now maps the rendered selection back to source
  and deletes it through the normal store path (one undo step), then opens the
  block in the editor at the deletion point. Conservative no-ops remain for
  cross-block selections, selections wholly inside atomic constructs (links,
  chips), annotation/calc blocks, and read-only pages.
- **Managed Markdown saves now retain their authenticated projection predecessor
  across drain, compaction, and unsafe reopen.** The next save and delayed
  filesystem callback re-prove the exact completed-path receipt instead of
  regenerating its historical base through the canonical serializer, so
  empty-bullet and non-leading-heading layouts no longer strand the graph in a
  global “updating” state. Exact live bytes, path, page, frontier, claim,
  endpoint, and receipt bindings remain required; divergent external edits and
  Org layout differences still fail closed.

### Changed

- **Rebuilding a managed graph's internal cache is much faster, and now scales
  with the graph instead of against it.** On a 1,045-file graph the rebuild went
  from about 15 seconds to about 2.6 seconds, and the cost is now roughly
  proportional to graph size rather than growing faster than it — so the saving
  gets larger, not smaller, as a graph grows. This is the recovery a graph
  reaches after corruption or an internal format change, so it is rare, but when
  it happens it is the whole wait before the app is usable.
- **Managed graphs rebuild their internal cache once on first launch after this
  update.** A field that was recorded but never used has been removed from the
  cache's page-alias table, which changes its internal format. Nothing in your
  graph changes; the rebuild is the faster one described above, and it happens
  only once.

- **Managed-storage save refusals now report a bounded internal reason code.**
  This keeps graph content and filesystem details private while distinguishing
  preparation, commit, queue, decode, and authority failures that previously
  collapsed into one generic semantic-transaction message. Crash-reopen
  coverage now also exercises accepted undrained history both with and without
  the disposable resume accelerator.

## [0.6.90] - 2026-08-04

### Fixed

- **Post-activation managed-storage reads and writes no longer freeze the app
  while the required first reconciliation finishes.** Ordinary page, journal,
  query, editor, status, transition, tick, and shutdown commands run actor waits
  on the blocking pool and revalidate their exact graph generation before work,
  so queued commands cannot gain authority over a replacement graph.
- **Restarting after interrupted managed-storage activation no longer leaves the
  desktop window hidden behind synchronous native setup.** The themed window now
  paints before the unchanged fail-closed recovery path runs, while ordered
  recovery phases and periodic wait diagnostics make legitimate long recovery
  observable without admitting unvalidated state.
- **Sparse-v2 now detects sampled interior corruption in its disposable SQLite
  projection during reopen.** Versioned, bounded fingerprints cover up to 1 MiB
  across the interior of each database and WAL file in addition to their edges;
  a mismatch preserves forensic evidence and rebuilds from oplog authority.
- **Sparse-v2 activation now uses lsdoc as the single Markdown and Org outline
  authority.** Parser-owned source spans and topology replace Tine's duplicate
  structural scanners, so Logseq-compatible headings, multiline blocks, mixed
  indentation, and other unusual but parseable layouts import without changing
  their meaning. Admission remains fail-closed when a layout cannot be
  represented safely.
- **Reverse-delivered experimental shared-provider histories now schedule inbound
  batches by causal dependency instead of repeatedly scanning the pending
  queue.** Each ready manifest is registered once, already-active parents are
  satisfied immediately, and accepting a parent wakes only its deterministically
  indexed children while blocked work remains visible to status and Safe
  shutdown.
- **Experimental shared-provider sync no longer loses oversized or overlapping
  delivery callbacks or preempts an already-observed local file edit.** Rejected
  callbacks retain a bounded rescan before `Safe`, observations arriving during
  an active cursor force a subsequent scan, and watcher-captured bytes take
  priority over remote ingestion. Safe reopen also republishes a completely
  missing provider namespace, and exact deletion of an accepted manifest repairs
  from authenticated local archive bytes.
- **Managed-storage handoff reconciliation no longer monopolizes page reads or
  sync setup on a large graph.** The mandatory post-publication full scan now
  retains one exact cursor and compares a bounded path/time slice per actor
  turn. It remains pending for shutdown safety, but ordinary application and
  enrollment work runs between slices; the former 750 ms arrival-order delay is
  gone.
- **Eligible Markdown and Org files outside configured page roots stay
  discoverable and exact-path load/save never exposes a blank writable substitute**
  (GH #246). Lowercase `.markdown` files and external add/delete updates follow
  the same graph-wide text scope without overwriting the original bytes.
- **Page titles now map injectively to reversible, Windows-safe filenames**
  (GH #249). Existing graph filenames remain readable as stored, while unsafe
  create/rename/rescue identities and collisions are refused without overwriting
  another page's bytes.
- **A missing remembered graph no longer prevents Tine from starting** (GH #250).
  Startup reaches the visible graph chooser instead of panicking during native
  setup, without creating or changing the missing path.
- **Configured default journal templates materialize once across local midnight**
  (GH #260). Timer, focus, and visibility refreshes use one graph/day guard, so
  a new day is initialized without duplicate template blocks or stale-graph writes.

### Changed

- **Experimental sparse-v2 storage can explicitly enroll two honest devices
  through a shared filesystem provider.** A Safe, idle initiator publishes
  immutable archive objects and manifests before one descriptor; a second
  device joins only the exact shared lineage and projection base. Both roles
  reconcile exact paths or retained incremental cursors through the one actor,
  without whole-provider archive caps or historical-manifest replay. Durable
  pending-publication markers resume accepted local and external-import batches
  after a crash, poll mode observes provider delivery independently of graph
  polling, and `SharePrepared` resumes before descriptor publication. Exact
  byte-identical generated conflict copies are retired with no-follow proof;
  differing or ambiguous evidence remains visible and blocks without graph
  writes. The Settings actions remain explicit and experimental.
- **Experimental sparse-v2 storage now has an explicit per-graph application
  boundary.** Opt-in retires and drains the legacy graph authority before
  publishing the durable binding, then routes bounded queries, editor intents,
  whole-root external-file observations, recovery status, and clean shutdown
  through one actor. Startup never enables it implicitly or falls back to a
  legacy writer after opt-in.
- **The disconnected experimental v2 oplog engine now uses one leased,
  authenticated run-local scratch store.** Exact causal clocks, compact
  store-backed batch status/waits, shallow document checkpoints with
  content-addressed chunks, and paged conflict evidence replace archive-history
  scans and unbounded hot maps. Scratch performs no durability sync and cannot
  delete or rewrite authoritative archive bytes. This engine remains disconnected
  from graph startup, migration, and user-visible writes.

### Added

- **Experimental managed sync can make an existing Syncthing/Dropbox graph
  operation-backed without moving it.** Tine stores immutable per-session Loro
  updates in `.tine-sync/`, keeps Markdown/Org as an editable projection for Logseq
  and other tools, imports external file edits conservatively, and automatically
  removes only conflict copies proven to be generated projections. Activation adds
  durable Logseq-compatible block IDs after a complete local safety snapshot;
  ordinary backup restore is operation-first and crash-resumable. This first version
  manages page and journal text; assets, PDF sidecars, and configuration remain
  ordinary provider-synchronized files. (Experimental, opt-in.)
- **Sparse-oplog crash takeover can recover to a clean handoff.** A restarted
  experimental runtime that takes over an `Unsafe` predecessor now performs an
  authenticated full external-file reconciliation before admitting automatic imports
  or publishing `Safe`, preserving unimported projection bytes.

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

[Unreleased]: https://github.com/martinkoutecky/tine/compare/v0.6.98...HEAD
[0.6.98]: https://github.com/martinkoutecky/tine/compare/v0.6.97...v0.6.98
[0.6.90]: https://github.com/martinkoutecky/tine/compare/v0.6.5...v0.6.90
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
