# Tine scope ledger (2026-06-16)

Decision-oriented consolidation of the OG feature map (full per-feature detail in
`subagent-tasks/notes/featmap-1..6-*.md`). Goal: agree scope, then implement every
IN gap. Status verified against Tine source (some agent notes mis-flagged shipped
features — corrected here).

Owner-confirmed exclusions: whiteboards, flashcards/SRS, file-sync, git, full datalog.
Owner-confirmed IN: query-engine OG-parity extension (the "query follow-up").

---

## A. OUT of scope (confirm nothing important is misclassified)

Subsystems — confirmed OUT:
- Whiteboards/tldraw · Flashcards/SRS · File-sync · Git · Real-time collaboration ·
  Encryption · Plugins/marketplace/plugin-API · Mobile/Capacitor · Zotero ·
  Slides/reveal.js · Excalidraw/draw · Server/HTTP API · Onboarding/handbooks ·
  global force-directed graph view *(see fork G2)*.

Feature-level OUT (code-execution / datalog-adjacent or non-applicable):
- Full datalog `[:find :where]`, `:result-transform`, `:inputs`, `{{function}}`,
  `:query/views`, `:query/result-transforms`, SCI evaluation.
- `{{cards}}`, custom user macros via plugins, Hiccup, custom KaTeX-macro plugin hooks.
- CodeMirror-specific options, `:custom-js-url` (arbitrary JS injection), Arweave.
- Clock/time-tracking query columns, NLP-date query-table sort.
- `icon::` property, quick-capture (mobile/share-sheet).

---

## B. BORDERLINE forks — RESOLVED by owner (2026-06-16)

| # | Fork | Decision |
|---|------|----------|
| G1 | Org-mode format | **OUT** — Markdown-only. Callouts via Markdown `> [!NOTE]` only; org `#+BEGIN_*`/delimiters/drawers/footnotes NOT parsed. |
| G2 | Local + global graph view | **OUT** |
| G3 | Repeaters (`+1w`/`.+1d`/`++1d`, roll forward on DONE) | **IN** |
| G4 | Logbook / time-tracking (CLOCK drawers, auto-log on DONE, summaries) | **IN** |
| G5 | Block created-at / modified-at timestamps | **OUT** — so `(between created-at …)`, timestamp sort, query-table date cols stay OUT. |
| G6 | Import (Roam/OPML/Markdown/merge) | **OUT** |
| G7 | Extra export formats (OPML, JSON/EDN) | **OUT** — Markdown export + copy-as-Markdown/HTML still IN. |
| G8 | Property pages | **IN** |
| G9 | Custom CSS (`custom.css`) | **IN** |
| G10 | Accent color + extra built-in themes | **IN** |
| G11 | mhchem chemistry in KaTeX (`\ce{…}`) | **IN** |
| G12 | Editable tables (inline cell edit) | **OUT** — edit via raw block text. |

Everything not in A is **IN** and appears in C.

---

## C. IN scope and currently MISSING — the build backlog

Grouped by area, with effort (S/M/L). Already-done items omitted. This is what I'll
implement once scope is confirmed.

### C1. Editor — inline formatting & cursor ops (mostly S, high daily value)
- **Inline format toggles**: Bold ⌘B, Italic ⌘I, Strikethrough, Highlight, Underline (`<ins>`), Insert-link ⌘L. (S each)
- **Markdown autoformat** (type `**`/`*`/`~~`/`==` around selection) + **auto-pair** brackets/quotes. (S)
- **Cut / Copy / Copy-as-ref / Copy-block-embed / Paste-plain-text (⌘⇧V)**. (S–M)
- **Emacs motions**: kill-line before/after, word fwd/back, kill-word fwd/back, beginning/end of block, select-text up/down. (S each)
- **Clear block (Ctrl+L)**, **Select-all (⌘⇧A)**, **Select-parent (⌘A)**. (S)
- **Priority setter**: `/A /B /C` + keybinding (currently render-only). (S–M)
- **Marker aliases** WAIT, IN-PROGRESS in marker set + slash menu. (S)
- **Slash commands** missing: Block-ref `((`, Block-embed, Page-embed, Image-link, Underline, Number-children, video/youtube/tweet embeds, export/verse/center/comment/ascii blocks, calc, user-defined `:commands`. (S–M)
- **`((` block autocomplete**, **`::` property-key autocomplete**. (M)

### C2. Rendering — block & inline syntax (mix; some L)
- **Admonitions/callouts** via `> [!NOTE/TIP/IMPORTANT/WARNING/CAUTION]` (+ icons/colors). (L)
- **Blockquote** `> text`, **horizontal rule** `---`, **table alignment** `:--:`, **hard line breaks**. (S each)
- **Image sizing** (`{:width}`/`{:height}`), **image lightbox** (click-zoom). (M)
- **Video/audio** local embeds + **YouTube/Vimeo** iframe (phased), **`{{video}}`/`{{tweet}}`/`{{namespace}}`** macros. (M)
- **Raw HTML / iframe** with sanitization. (M, borderline-risky → behind a setting)
- **Footnotes** `[^1]`. (M) · **Emoji shortcodes** `:smile:`. (S) · **Copy-code button**, **code line numbers**. (S)
- **Closed/`:LOGBOOK:` drawer lines hidden** from body (regardless of G4). (S)
- **Multi-line property values** continuation. (S)

### C3. References & embeds (mostly M)
- **Block-ref hover preview** popover (you requested this earlier). (M)
- **Linked-refs include/exclude filter dialog** + co-ref counts + filter-icon state, persisted to page `filters::`. (M–L)
- **Collapse linked-refs section** (+ auto-collapse past threshold). (S–M)
- **Editable backlinks in place** (like query results) + **parent/breadcrumb context** per hit. (M)
- **Editable embeds in place** + embed source-page header link. (S–M)

### C4. Query engine — OG parity (owner-approved "follow-up"; mostly the Rust `query.rs`)
- **Relative-date `between`**: `-7d/+7d/today/yesterday/tomorrow`, `Nw/Nm/Ny`. *(biggest gap)* (M)
- **`(page name)`**, **`(namespace ns)`**, **`(sample N)`**, **bare `"full-text"` string**. (S each)
- **`(page-property k [v])`**, **`(page-tags …)`**, **`(all-page-tags)`**. (M)
- **Multi-value / page-ref property-value matching** in `(property k v)`. (M)
- **Honor `(sort-by field asc/desc)`** in DSL (+ surface in builder). (M)
- **Query front-matter** `:title`, `:collapsed?`, `:table-view?` defaults. (S each)
- **Surface new filters in the builder** once the engine runs them. (S each)

### C5. Tasks / agenda / properties / config
- **Default journal agenda**: built-in NOW/NEXT query + "Scheduled & Deadline" foldable on today's journal (engine already supports it). (M)
- **Date+time in SCHEDULED/DEADLINE** timestamps (datepicker time field). (M)
- **config.edn coverage** (currently 5 of ~50 keys): `:journal/page-title-format` & `:file-name-format`, `:start-of-week` (datepicker hardcodes Sunday), `:hidden` (skip paths), `:favorites` (read/write on-disk, not just localStorage — **portability gap**), `:block-hidden-properties`, vector & `false` shortcut bindings, `:default-home`, `:scheduled/future-days`. (S–M each)
- **Expand hidden-property set** to OG's (heading, title, filters, query-*, created/updated-at). (S)

### C6. Pages / journals / search / navigation
- **Page aliases** (`alias::` resolution in links, backlinks, search, rename). (M)
- **Namespace hierarchy UI** (breadcrumb + children list on `a/b/c` pages). (M)
- **Block-path breadcrumbs** (page view header + sidebar items). (M)
- **"Today" / go-home / go-to-all-pages** nav + an **all-pages route/view** with sort/filter. (S–M)
- **Calendar widget** to jump to any journal date. (M)
- **Journal templates** (`:default-templates :journals`) on new-journal creation. (M)
- **Command palette** (⌘⇧P) over all commands; **shortcut-help modal** (Shift+?). (M)
- **Search**: result type filters, **search-in-page (⌘⇧K)**, snippet/highlight. (S–M)
- **Sidebar search/filter**; **clear-all right-sidebar**. (S)

### C7. UI chrome / assets / export
- **Wide mode** + **document mode**; **font-size / UI zoom** controls; **About** + **editor/display settings tabs**. (S–M)
- **Toasts/notifications**; **hover tooltips/previews** for page & block refs (overlaps C3). (S–M)
- **Text-selection floating menu** (bold/italic/highlight/link). (M)
- **Context-menu completeness**: page rename/delete/move; block move/indent/outdent. (M)
- **Asset drag-and-drop** upload; **PDF text-search** polish; **PDF highlights → hls__ page** export. (M)
- **Export Markdown** + **copy-as-Markdown/HTML**; publish HTML styling polish. (M)

---

## D. Proposed implementation order (after scope sign-off)

1. **Editor essentials** (C1) — formatting toggles, copy/cut, autoformat, priority setter, marker aliases. *Highest daily value, low effort.*
2. **Query engine parity** (C4) — your approved follow-up; mostly Rust.
3. **Rendering gaps** (C2) — callouts, blockquote/hr, image sizing, drawer hiding, footnotes.
4. **References UX** (C3) — hover preview, ref filtering, editable backlinks/embeds.
5. **Tasks/agenda + config** (C5) — journal agenda, config.edn coverage, favorites-on-disk.
6. **Pages/nav/search** (C6) — aliases, namespaces, breadcrumbs, command palette, calendar.
7. **UI chrome/export** (C7) — modes, tooltips, selection menu, context-menu completeness, export.

Delivered + committed in batches; tests + build green per batch.
