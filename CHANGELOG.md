# Changelog

All notable changes to Tine are documented here. Tine is a fast, local-first
outliner that reads and writes a real Logseq Markdown (and now Org) graph.

The format follows [Keep a Changelog](https://keepachangelog.com/); versions use
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

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

[Unreleased]: https://github.com/martinkoutecky/tine/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/martinkoutecky/tine/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/martinkoutecky/tine/releases/tag/v0.2.0
[0.1.0]: https://github.com/martinkoutecky/tine/releases/tag/v0.1.0
