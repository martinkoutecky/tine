# Changelog

All notable changes to Tine are documented here. Tine is a fast, local-first
outliner that reads and writes a real Logseq Markdown (and now Org) graph.

The format follows [Keep a Changelog](https://keepachangelog.com/); versions use
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

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

### Fixed

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

[Unreleased]: https://github.com/martinkoutecky/tine/compare/v0.5.6...HEAD
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
