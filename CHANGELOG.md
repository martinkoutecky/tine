# Changelog

All notable changes to Tine are documented here. Tine is a fast, local-first
outliner that reads and writes a real Logseq Markdown (and now Org) graph.

The format follows [Keep a Changelog](https://keepachangelog.com/); versions use
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

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

[Unreleased]: https://github.com/martinkoutecky/tine/compare/v0.3.5...HEAD
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
