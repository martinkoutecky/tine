# OG Logseq Feature Inventory for Tine Port

This document is a comprehensive categorized inventory of user-facing features in the original Logseq codebase (ClojureScript), organized to guide systematic porting to Tine (from-scratch reimplementation).

**Scope note**: Excludes flashcards/spaced-repetition (`:feature/enable-flashcards`), whiteboards/tldraw, and the plugin system/marketplace. Notes these are out-of-scope but lists what they do.

---

## 1. Block Editing & Outliner

### Core Editing Operations

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **New block (Enter)** | Press Enter to create new block at same level after cursor block | `handler/editor.cljs`, `handler/editor/keyboards.cljs`, `shortcut/config.cljs` (:editor/new-block) | Small | On-disk: new `block/uuid` assigned; block added to parent's children |
| **New line in block (Shift+Enter)** | Insert soft newline within a block without splitting | `handler/editor.cljs`, `shortcut/config.cljs` (:editor/new-line) | Small | Stays in same block; no new UUID |
| **Delete block** | Delete current block and its children | `handler/editor.cljs` (delete-block!, delete-blocks!) | Small | On-disk: block removed from parent's `:block/children`; if has refs, orphan them or convert to text |
| **Backspace** | Delete character before cursor; join with previous block if at start | `handler/editor.cljs` (editor-backspace) | Small | Triggers block merge logic if cursor at line start |
| **Delete key** | Delete character after cursor | `handler/editor.cljs` (editor-delete) | Small | Character deletion; no tree impact |
| **Indent block (Tab)** | Increase nesting level; move block as child of previous sibling | `handler/editor.cljs` (keydown-tab-handler :right), `handler/block.cljs` (indent-outdent-block!) | Medium | On-disk: parent ref changes; may reorder children lists |
| **Outdent block (Shift+Tab)** | Decrease nesting level; move block to parent's level | `handler/editor.cljs` (keydown-tab-handler :left), `handler/block.cljs` (indent-outdent-block!) | Medium | On-disk: updates parent refs and sibling order |
| **Move block up (Alt+Shift+Up / Cmd+Shift+Up)** | Reorder block above its sibling | `handler/editor.cljs` (move-up-down true) | Medium | On-disk: swap positions in parent's `:block/children` list |
| **Move block down (Alt+Shift+Down / Cmd+Shift+Down)** | Reorder block below its sibling | `handler/editor.cljs` (move-up-down false) | Medium | On-disk: swap positions in parent's `:block/children` list |
| **Expand block children (Cmd+Down)** | Open/unfold collapsed block's children | `handler/editor.cljs` (expand!) | Small | UI state; no on-disk change; stored in `:block/open?` |
| **Collapse block children (Cmd+Up)** | Close/fold block's children | `handler/editor.cljs` (collapse!) | Small | UI state; no on-disk change; stored in `:block/open?` |
| **Toggle collapse (Cmd+;)** | Toggle collapse/expand | `handler/editor.cljs` (toggle-collapse!) | Small | UI state toggle |
| **Zoom in on block (Cmd+. / Alt+Right)** | Focus a single block, hide siblings, go into that block's context | `handler/editor.cljs` (zoom-in!) | Medium | Navigation state; may render only one block as root |
| **Zoom out (Cmd+, / Alt+Left)** | Exit zoom; return to parent block context | `handler/editor.cljs` (zoom-out!) | Medium | Navigation state; restore full tree view |
| **Cut (Cmd+X)** | Cut selected blocks/text to clipboard | `handler/editor.cljs` (shortcut-cut) | Small | Clipboard + on-disk removal |
| **Copy (Cmd+C)** | Copy selected blocks to clipboard (with or without block ref) | `handler/editor.cljs` (shortcut-copy, shortcut-copy-text) | Medium | Copy block ref UUID on copy (:editor/copy embeds UUID), text-only variant (:editor/copy-text) |
| **Paste (Cmd+V)** | Paste from clipboard; auto-detect blocks vs text | `handler/paste.cljs` | Medium | Interprets clipboard format; may create new blocks; handles markdown parsing |
| **Paste text only (Cmd+Shift+V)** | Paste as plain text, no formatting | `handler/paste.cljs` (editor-on-paste-raw!) | Small | Strips formatting, inserts verbatim |
| **Undo (Cmd+Z)** | Undo last edit | `handler/history.cljs` (undo!), `modules/editor/undo_redo.cljs` | Medium | Stack-based; tracks edits, move, delete, property changes |
| **Redo (Cmd+Shift+Z / Cmd+Y)** | Redo last undone edit | `handler/history.cljs` (redo!) | Medium | Stack-based |
| **Select all blocks (Cmd+Shift+A)** | Select entire block tree on page | `handler/editor.cljs` (select-all-blocks!) | Small | Multi-select state |
| **Select parent (Cmd+A)** | Select current block's parent/root | `handler/editor.cljs` (select-parent) | Small | Multi-select state |
| **Select block up/down (Alt+Up/Down)** | Extend selection to adjacent blocks | `handler/editor.cljs` (on-select-block) | Small | Multi-select state |
| **Select text up/down (Shift+Up/Down)** | Extend text selection within block | `handler/editor.cljs` (shortcut-select-up-down) | Small | Text cursor selection |
| **Delete selection (Backspace/Delete on selection)** | Delete selected blocks or text | `handler/editor.cljs` (delete-selection) | Small | Multi-select or text selection |
| **Clear block (Ctrl+L / Alt+L)** | Clear all content in current block | `handler/editor.cljs` (clear-block-content!) | Small | Empties block text; keeps block structure |
| **Kill line before (Ctrl+U / Alt+U)** | Delete from start of line to cursor | `handler/editor.cljs` (kill-line-before!) | Small | Emacs-style |
| **Kill line after (Alt+K)** | Delete from cursor to end of line | `handler/editor.cljs` (kill-line-after!) | Small | Emacs-style |
| **Beginning of block (Alt+A)** | Jump cursor to start of block | `handler/editor.cljs` (beginning-of-block) | Small | Cursor navigation |
| **End of block (Alt+E)** | Jump cursor to end of block | `handler/editor.cljs` (end-of-block) | Small | Cursor navigation |
| **Forward word (Ctrl+Shift+F / Alt+F)** | Jump cursor forward one word | `handler/editor.cljs` (cursor-forward-word) | Small | Word navigation |
| **Backward word (Ctrl+Shift+B / Alt+B)** | Jump cursor backward one word | `handler/editor.cljs` (cursor-backward-word) | Small | Word navigation |
| **Forward kill word (Ctrl+W / Alt+D)** | Delete word after cursor | `handler/editor.cljs` (forward-kill-word) | Small | Emacs-style |
| **Backward kill word (Alt+W)** | Delete word before cursor | `handler/editor.cljs` (backward-kill-word) | Small | Emacs-style |
| **Block arrow navigation (↑↓←→)** | Move cursor between blocks; respects nesting | `handler/editor.cljs` (shortcut-up-down, shortcut-left-right) | Small | Tree-aware navigation |
| **Open block for editing** | Click or press Enter on block to edit | `handler/editor.cljs` (open-selected-block!) | Small | Enter edit mode; focus input |

### Block Properties & Markers

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **TODO marker** | Add/cycle TODO task marker | `commands.cljs` (->marker "TODO"), `handler/editor.cljs` (cycle-todo!) | Small | On-disk: block has `:block/marker` = "TODO"; affects task filters |
| **Cycle TODO (Cmd+Enter)** | Cycle through NOW→DOING→DONE (or TODO→DOING→DONE) | `handler/editor.cljs` (cycle-todo!) | Small | Depends on `:preferred-workflow` config (`:now` vs `:todo`) |
| **Priority [#A] / [#B] / [#C]** | Set priority marker | `commands.cljs` (->priority), `handler/editor.cljs` (set-priority) | Small | On-disk: `:block/priority` = "A"/"B"/"C"; parsed before marker |
| **SCHEDULED date** | Schedule task for future date | `commands.cljs` (/Scheduled command), `components/datetime.cljs` | Medium | On-disk: `:block/scheduled` = journal day int; shows in scheduled query |
| **DEADLINE date** | Set deadline for task | `commands.cljs` (/Deadline command), `components/datetime.cljs` | Medium | On-disk: `:block/deadline` = journal day int; shows in deadline query |
| **Repeater (e.g., +1w)** | Set recurrence on scheduled/deadline | `handler/repeated.cljs`, `components/datetime.cljs` | Medium | On-disk: in scheduled/deadline AST; triggers logbook entry on completion |
| **Logbook / Clock** | Log time spent on task; auto-inserted on DONE | `handler/editor.cljs` (with-timetracking), `util/clock.cljs`, `components/block.cljs` | Medium | On-disk: `:block/logbook` drawer with CLOCK entries; `:feature/enable-block-timestamps?` enables timestamps |
| **Block properties (key: value)** | Custom properties on blocks | `handler/editor/property.cljs`, `util/property.cljs` | Medium | On-disk: properties block as first child with format `key:: value`; single or multi-line values |
| **Insert properties (Org)** | Create empty properties block | `commands.cljs` (->properties) | Small | Org-mode specific; empty `:PROPERTIES:` drawer |
| **Block title collapse** | Allow blocks with titles (no children) to collapse | `config.edn` `:outliner/block-title-collapse-enabled?` | Small | UI: shows collapse icon on multi-line blocks |

### Text Formatting (Inline)

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Bold (Cmd+B)** | Toggle **bold** formatting | `handler/editor.cljs` (bold-format!) | Small | Markdown: `**text**`; Org: `*text*` |
| **Italics (Cmd+I)** | Toggle _italic_ formatting | `handler/editor.cljs` (italics-format!) | Small | Markdown: `*text*`; Org: `/text/` |
| **Strikethrough (Cmd+Shift+S)** | Toggle ~~strikethrough~~ | `handler/editor.cljs` (strike-through-format!) | Small | Markdown: `~~text~~`; Org: `+text+` |
| **Highlight (Cmd+Shift+H)** | Highlight selected text (yellow) | `handler/editor.cljs` (highlight-format!) | Small | Markdown: `==text==` or `^^text^^`; rendered with bg color |
| **Underline** | Toggle <ins>underline</ins> (Markdown only) | `commands.cljs` (/Underline) | Small | Markdown: `<ins>text</ins>` only; no Org support |
| **Insert link (Cmd+L)** | Create `[label](url)` link | `handler/editor.cljs` (html-link-format!) | Small | Dialog prompts for URL + optional label |
| **Auto-pair brackets/quotes** | Auto-close `[]`, `()`, `{}`, `""`, `''` | `handler/editor.cljs` (editor-input) | Small | Configurable per editor; prevents accidental doubles |
| **Markdown shortcut** | Type `**`, `__`, `*`, `/`, `~~`, `==` auto-wraps | Built into editor input handler | Small | Real-time as you type |

### Slash Command Menu

The slash command menu (type `/` to open) includes:

#### Basic Commands
- **Page reference (`/Page reference`)**: Insert `[[page-name]]`
- **Page embed (`/Page embed`)**: Insert `{{embed [[page-name]]}}`
- **Block reference (`/Block reference`)**: Insert `((block-uuid))`
- **Block embed (`/Block embed`)**: Insert `{{embed ((block-uuid))}}`
- **Link (`/Link`)**: Insert `[label](url)` with dialog
- **Image link (`/Image link`)**: Insert image `[label](image-url)`
- **Underline (`/Underline`)**: Insert `<ins>text</ins>` (Markdown only)
- **Template (`/Template`)**: Search and insert named template
- **Upload asset (`/Upload an asset`)** (Desktop + local DB only): File upload UI

#### Headings (h1–h6)
- `/h1` through `/h6`: Insert heading levels; format depends on `:preferred-format`

#### Date/Time
- **Today** (`/Today`): Insert `[[<today-date>]]`
- **Tomorrow** (`/Tomorrow`): Insert `[[<tomorrow-date>]]`
- **Yesterday** (`/Yesterday`): Insert `[[<yesterday-date>]]`
- **Current time** (`/Current time`): Insert formatted time
- **Date picker** (`/Date picker`): Modal date picker

#### List Types
- **Number list** (`/Number list`): Toggle numbered list on current block
- **Number children** (`/Number children`): Number all children

#### Task Markers
Based on `:preferred-workflow` (`:now` or `:todo`):
- NOW, LATER, TODO, DOING, DONE, WAITING, CANCELED

#### Priorities
- `/A`, `/B`, `/C`: Set priority

#### Block Types
From `block-commands-map`:
- Quote, Src (code), Query, Latex export, Note, Tip, Important, Caution, Pinned, Warning, Example, Export, Verse, Ascii, Center, Comment

#### Advanced
- **Query** (`/Query`): Insert `{{query }}` with examples
- **Zotero** (`/Zotero`): Import from Zotero
- **Query function** (`/Query function`): Insert `{{function }}`
- **Calculator** (`/Calculator`): Insert `{{calc}}` block
- **Draw** (`/Draw`): Create Excalidraw drawing (calls `handler/draw.cljs`)
- **Embed HTML** (`/Embed HTML`): Insert HTML embed template
- **Embed Video URL** (`/Embed Video URL`): Insert `{{video }}`
- **Embed YouTube timestamp** (`/Embed Youtube timestamp`): Insert YouTube timestamp
- **Embed Twitter tweet** (`/Embed Twitter tweet`): Insert `{{tweet }}`
- **Code block** (`/Code block`): Insert ` ```code ``` ` with language selector

#### Extensible
- User-defined commands from `config.edn` `:commands`
- Plugin slash commands (via `register-slash-command`)

**Key file**: `frontend/commands.cljs`  
**Effort**: Medium (all commands; many are just text templates; some trigger modals)

### Autocomplete/Search

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **`[[` page autocomplete** | Type `[[` and start typing; fuzzy-search pages; navigate with arrow keys | `components/editor.cljs`, `handler/editor.cljs` | Medium | Renders live list; respects aliases; includes journal pages and regular pages |
| **`#` tag autocomplete** | Type `#` for tag suggestions | `components/editor.cljs` | Medium | Parses existing tags from graph; fuzzy match |
| **`((` block autocomplete** | Type `((` for block ref; search by content | `components/editor.cljs` | Medium | Full-text search on block content; includes page context |
| **`:` property autocomplete** | Type `:` in properties block to suggest property names | `handler/editor/property.cljs` | Medium | Suggests existing property keys from graph |
| **Custom autocomplete menu** | Navigate (↑↓), confirm (Enter), open link (Cmd+O) | `shortcut/config.cljs` (:auto-complete/*) | Small | Keyboard shortcuts for autocomplete interaction |

---

## 2. Blocks & Rendering

### Task & Status Markers

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Marker rendering** | Display task markers (NOW, TODO, DONE, etc.) as colored badges | `components/block.cljs`, format parsers | Small | On-disk: `:block/marker` field; CSS for styling |
| **Marker colors** | Color-coded: DOING=orange, DONE=green, NOW=red, etc. | `components/block.cljs` (CSS), format parser | Small | Visual; no semantic impact |

### Priorities

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Priority display [#A] [#B] [#C]** | Show priority badges; sort by priority in queries | `components/block.cljs`, `handler/export/common.cljs` | Small | On-disk: `:block/priority` field; parsed from block content |
| **Priority sorting** | Query results sortable by priority | `db/query_dsl.cljs` (sort-by priority) | Small | DSL filter |

### Scheduled & Deadlines

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **SCHEDULED date** | Display "📅 SCHEDULED <date>" inline; synced with calendar | `handler/block.cljs`, `components/block.cljs`, `components/datetime.cljs` | Medium | On-disk: `:block/scheduled` = journal day int |
| **DEADLINE date** | Display "🔴 DEADLINE <date>" inline | `handler/block.cljs`, `components/block.cljs`, `components/datetime.cljs` | Medium | On-disk: `:block/deadline` = journal day int |
| **Repeater syntax (+1w, +1d, etc.)** | Parse and apply repeater on scheduled/deadline | `handler/repeated.cljs`, AST parser | Medium | Creates new scheduled/deadline on next date; triggers on DONE |
| **Scheduled/deadline queries** | Built-in queries showing upcoming tasks | `config.edn` `:default-queries` | Medium | Journal page shows "NOW" and "NEXT" blocks with scheduled/deadline filters |
| **Calendar icon in datepicker** | Pick dates with calendar UI | `components/datetime.cljs` | Medium | Modal date picker; shortcuts for today, tomorrow, etc. |

### Logbook & Time Tracking

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Clock in/out** | Start/stop time tracking on a block | `util/clock.cljs`, `handler/editor.cljs` | Medium | On-disk: `:block/logbook` drawer; `CLOCK [start]--[end]` entries |
| **Auto logbook on DONE** | Auto-insert logbook entry when marker → DONE | `handler/editor.cljs` (with-timetracking) | Medium | Creates CLOCK entry with timestamp |
| **Logbook display** | Render logbook drawer as collapsible section | `components/block.cljs`, `util/drawer.cljs` | Small | UI: hidden by default; click to expand |
| **Time seconds support** | Toggle seconds in logbook timestamps | `config.edn` `:logbook/settings :with-second-support?` | Small | Default false (minutes only); config option |

### Block References & Embeds

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Block reference ((uuid))** | Inline link to block; click opens in sidebar/nav | `components/block.cljs`, `handler/editor.cljs` | Medium | On-disk: UUID is `:block/uuid`; bidirectional ref stored separately |
| **Block embed {{embed ((uuid))}}** | Display referenced block's full content inline | `components/block.cljs` (block-embed), `components/query.cljs` | Medium | Recursive rendering; respects max-depth; may cause infinite loops if circular |
| **Page embed {{embed [[page]]}}** | Embed entire page content inline | `components/block.cljs` (page-embed) | Medium | On-disk: page-ref parsed; recursively renders page's blocks |
| **Replace block ref at point** | Replace `((uuid))` with block content | `handler/editor.cljs` (replace-block-reference-with-content-at-point) | Small | Shortcut Cmd+Shift+R; expands inline |
| **Copy block embed** | Copy current block as embed `{{embed ((uuid))}}` | `handler/editor.cljs` (copy-current-block-embed), shortcut Cmd+E | Small | Clipboard operation |
| **Auto-expand block refs on zoom** | When zooming into a block, auto-expand its refs | `config.edn` `:ui/auto-expand-block-refs?` (default true) | Small | UI behavior |
| **Block ref default open level** | Configure expansion depth in linked references | `config.edn` `:ref/default-open-blocks-level` (default 2) | Small | Max depth to auto-expand |

### Block Rendering & Content Types

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Markdown support** | Parse and render Markdown blocks | `format/markdown.cljs`, `format/mldoc.cljs` | Medium | Full Markdown syntax support |
| **Org-mode support** | Parse and render Org-mode blocks | `format/org.cljs`, `format/mldoc.cljs` | Medium | `:preferred-format` config switches between Markdown and Org |
| **Headings (h1–h6)** | Render heading levels | `format/mldoc.cljs`, `handler/export/html.cljs` | Small | Markdown: `# Title`; Org: `* Title`; UI scaling based on level |
| **Inline code** | Render `code` with monospace font | Format parser | Small | Markdown: `` `code` ``; Org: `~code~` |
| **Code block with syntax highlighting** | Render fenced code blocks with language highlighting | `extensions/code.cljs`, CodeMirror | Medium | Language detection; line numbers optional; copy button |
| **LaTeX math** | Render `$$...$$` and `$...$` formulas | `extensions/latex.cljs` | Medium | Inline and display math; mathjax rendering |
| **Tables** | Render Markdown/Org tables | Format parser, CSS | Medium | Org: `|col|col|`, Markdown: `|col|col|`; editable cells |
| **Blockquote** | Render quoted text blocks | Format parser, CSS | Small | Markdown: `> quote`, Org: `#+BEGIN_QUOTE` |
| **Horizontal rule** | Render `---` or `***` as divider | Format parser | Small | Visual divider |
| **Admonitions / Callouts** | Render `#+BEGIN_NOTE`, `#+BEGIN_TIP`, etc. blocks | Format parser, CSS | Small | On-disk: `#+BEGIN_TYPE ... #+END_TYPE` syntax; styled with icons and colors |
| **Inline images** | Render images inline in block | `extensions/lightbox.cljs`, `components/block.cljs` | Medium | Lightbox on click; respects image size; drag-to-insert |
| **Image sizing** | Resize images with `{:width 200 :height 100}` | Format parser, image component | Medium | Org: `#+ATTR_ORG: :width 200px`; Markdown: `![alt](url){:width 200px}` |
| **Video/audio embeds** | Embed `<video>`, `<audio>` or platform embeds | `extensions/video/youtube.cljs`, `components/block.cljs` | Medium | YouTube, Vimeo, etc.; custom iframe embeds |
| **iFrame embeds** | Render raw `<iframe>` in block | HTML parser, sanitization | Medium | Content security policy applied |
| **Links (page refs)** | Click `[[page]]` to navigate; shows tooltip | `components/block.cljs`, `handler/editor.cljs` | Medium | On-disk: `[[page-name]]` parsed to page ref; bidirectional index |
| **Links (external)** | Click `[label](url)` to navigate externally | Format parser, `<a>` rendering | Small | No on-disk ref tracking |
| **Links (tag refs)** | Click `#tag-name` to filter by tag | Format parser, navigation | Medium | On-disk: tags indexed; similar to page refs but scalar value |
| **Aliases** | Page `Alias:: OtherName` allows refer by alternate name | `handler/page.cljs`, format parser | Medium | On-disk: `:page/alias` field (set); all aliases searchable |
| **Namespaces (a/b/c)** | Parent-child page hierarchy via `/` in title | Format parser, query | Small | No separate on-disk representation; purely in page name |
| **Footnotes** | Render `[^1]` and `[^1]: text` | Format parser, HTML export | Medium | On-disk: in AST; exported as footnote sections |

### Page & Block Properties

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Block-level properties** | Edit properties on individual blocks | `handler/editor/property.cljs`, `components/block.cljs` | Medium | First child block special syntax: `key:: value` across lines; custom drawer |
| **Page-level properties** | Edit properties on page (first block) | `handler/editor/property.cljs`, `handler/page.cljs` | Medium | Same as block properties but on page's first block |
| **Property datatype** | Support text, number, date, page-ref, URL, etc. | `util/property.cljs`, schema | Medium | Inferred from value; stored as field in graph |
| **Multi-value properties** | Properties with comma-separated values | `config.edn` `:property/separated-by-commas` | Medium | e.g., `tags:: tag1, tag2, tag3` |
| **Property UI inline editor** | Click property to edit inline | `components/block.cljs` | Medium | Datepicker for dates, autocomplete for page-refs, etc. |
| **Hidden properties** | Exclude certain properties from display | `config.edn` `:block-hidden-properties` | Small | e.g., `:block-hidden-properties #{:public :icon}` |
| **Property pages** | Create wiki-style page for each property | `config.edn` `:property-pages/enabled?` (default true) | Medium | Aggregates all blocks with that property; queryable |
| **Ignored property refs** | Properties not treated as page refs | `config.edn` `:ignored-page-references-keywords` | Small | e.g., `:author :website` won't create refs |

---

## 3. Pages & Journals

### Page Management

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Create page** | New page via command or autocomplete | `handler/page.cljs` (create!) | Small | On-disk: new `.md` or `.org` file in `pages/` dir |
| **Rename page** | Rename page; updates all references | `handler/page.cljs` (rename!) | Medium | On-disk: file renamed; all block refs + page refs updated in graph |
| **Delete page** | Delete page file; orphan references or drop them | `handler/page.cljs` (delete!) | Medium | On-disk: file deleted; optional: preserve backlinks as text or drop them |
| **Merge pages** | Merge one page into another | `handler/page.cljs` (merge-pages!) | Medium | On-disk: source page blocks moved to target; file deleted |
| **Page aliases** | Add `Alias:: OtherName` to allow refer by alternate name | Format parser, `handler/page.cljs` | Medium | On-disk: `:page/alias` field (set) |
| **Page properties** | Edit page-level metadata (author, date, tags, etc.) | `handler/editor/property.cljs`, format parser | Medium | On-disk: properties in page's first block |

### Journals

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Daily journal page** | Auto-create page for each day; click day to jump | `handler/journal.cljs`, `components/journal.cljs` | Small | On-disk: separate file per day in `journals/` dir; default format `yyyy_MM_dd.md` |
| **Journal page date format** | Customize display format (e.g., "Mon 19th, Jan 2038") | `config.edn` `:journal/page-title-format` | Small | Affects UI display only; file name separate from title |
| **Journal filename format** | Customize filename format (e.g., `yyyy-MM-dd` or `yyyy_MM_dd`) | `config.edn` `:journal/file-name-format` | Small | On-disk: filename only; **retroactive changes require manual rename** |
| **Default journal template** | Auto-populate new journals with template | `config.edn` `:default-templates :journals` | Small | Template name; blocks from template copied to new journal |
| **Journal infinite scroll** | Load older/newer journals on scroll | `components/journal.cljs`, `handler/journal.cljs` (load-more-journals!) | Medium | Infinite scroll pagination |
| **Go to date** | Jump to specific journal date | `handler/journal.cljs` (go-to-tomorrow!, go-to-next-journal!, etc.) | Small | Shortcuts: Gt (tomorrow), Gp (prev), Gn (next) |
| **Last modified timestamp** | Show when page was last edited | Block metadata, renderer | Small | On-disk: `:block/updated-at` timestamp |
| **Created timestamp** | Show when page was created | Block metadata, renderer | Small | On-disk: `:block/created-at` timestamp; can be disabled in config |
| **Block timestamps** | Enable/disable timestamps on all blocks | `config.edn` `:feature/enable-block-timestamps?` (default false) | Small | UI feature; affects rendering |

### Page Sidebar & Navigation

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Right sidebar (open in sidebar)** | Open page or block in right panel | `components/right_sidebar.cljs`, `handler/editor.cljs` (open-link-in-sidebar!) | Medium | Cmd+Shift+O opens link in sidebar; click "Open in sidebar" on ref |
| **Pin sidebar item** | Keep sidebar page pinned (don't auto-close) | `components/right_sidebar.cljs` | Small | UI state; toggles on/off |
| **Sidebar breadcrumb** | Show path to current block in sidebar | `components/right_sidebar.cljs` (block-with-breadcrumb) | Small | Navigational context |
| **Close sidebar** | Hide right sidebar | `handler/ui.cljs` (toggle-right-sidebar!) | Small | Toggle via shortcut Tr or modal |
| **Linked references** | Show all pages/blocks that reference this page | `components/reference.cljs`, database queries | Large | Query engine; may require full graph traversal |
| **Unlinked references** | Show pages/blocks containing page title as text but no explicit ref | `components/reference.cljs`, text search | Large | Fuzzy text search; can be expensive |
| **Reference collapse threshold** | Auto-collapse reference section if >N items | `config.edn` `:ref/linked-references-collapsed-threshold` (default 50) | Small | UI collapse logic |
| **Reference expansion level** | Expand references to N levels deep | `config.edn` `:ref/default-open-blocks-level` (default 2) | Small | UI expansion default |

### Page Favorites/Starred

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Favorite page** | Star page to quickly access; shows in sidebar | `handler/page.cljs` (toggle-favorite!) | Small | On-disk: stored in app state or global favorites list; shortcut Cmd+Shift+F |
| **Favorites in sidebar** | Display favorite pages at top of left sidebar | `config.edn` `:favorites` list | Small | Reorderable; shows as buttons/links |
| **Reorder favorites** | Drag or edit favorite order | `handler/page.cljs` (reorder-favorites!) | Small | On-disk: written to config.edn |

### Hierarchy & Namespaces

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Namespace hierarchy (a/b/c)** | Parent-child relationship via `/` in page title | Query, format parser | Small | e.g., "Project/Subproject/Task" = 3-level hierarchy; purely naming convention |
| **Hierarchy view** | Show pages organized by namespace tree | `components/hierarchy.cljs` | Medium | Tree UI; expandable/collapsible; links to pages |
| **All pages view** | List all pages; filterable, searchable | `components/page.cljs` | Small | Alphabetical or custom sort |

### Local Graph View

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Page local graph** | Visualize connections between page and neighbors (one hop) | `extensions/graph.cljs`, force-directed graph lib | **Huge** | Complex graph physics simulation; 3D canvas rendering; performant for large graphs is hard; requires pixi/threejs integration |
| **Graph settings** | Toggle orphan pages, built-in pages, excluded pages, journals | `config.edn` `:graph/settings` | Large | Requires graph recalc on toggle |
| **Graph force settings** | Tune physics (link distance, charge, range) | `config.edn` `:graph/forcesettings` | Small | Fine-tuning; no semantic impact |

---

## 4. Queries

### Simple Queries ({{query ...}})

Simple (DSL) queries support these filter operators:

| Operator | Example | On-disk implication | Effort |
|----------|---------|---------------------|--------|
| **and** | `{{query (and [[page1]] [[page2]])}}` | Intersection of conditions | Small |
| **or** | `{{query (or [[page1]] [[page2]])}}` | Union of conditions | Small |
| **not** | `{{query (not [[archive]])}}` | Negation of condition | Small |
| **\[\[page-ref\]\]** | `{{query [[project]]}}` | Blocks referencing this page | Small |
| **#tag** | `{{query #important}}` | Blocks with this tag | Small |
| **page-tags** | `{{query (page-tags #tag)}}` | Pages tagged with this tag | Small |
| **property** | `{{query (property author "John")}}` | Blocks with property key=value | Medium |
| **task / marker** | `{{query (task NOW LATER)}}` | Blocks with these markers | Small |
| **priority** | `{{query (priority A)}}` | Blocks with priority A/B/C | Small |
| **between** | `{{query (between -7d +7d)}}` | Scheduled/deadline in date range | Medium |
| **scheduled** | `{{query (scheduled between -7d +7d)}}` | Scheduled blocks in range | Medium |
| **deadline** | `{{query (deadline between -7d +7d)}}` | Deadline blocks in range | Medium |
| **sort-by** | `{{query (sort-by created-at asc)}}` | Sort results by field | Small |
| **sample** | `{{query (sample 5)}}` | Limit to N random results | Small |
| **full-text-search** | `{{query "search string"}}` | Full-text search blocks | Medium |

**Key file**: `db/query_dsl.cljs`  
**Effort**: Medium (all DSL filters)

### Advanced Datalog Queries

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Datalog query** | Write raw Datalog inside `[:find ... :where ...]` | `db/query_custom.cljs`, `db/query_react.cljs` | **Huge** | Full Datalog engine (datascript); rule definitions; complex query logic |
| **Query functions** | Define and call custom query functions | `db/query_custom.cljs` | **Huge** | Advanced feature; relies on Datalog knowledge |
| **Query caching & reactivity** | Recompute query on data changes; cache results | `db/query_react.cljs` | Large | Reactive subscriptions; performance-critical |

### Query Tables & Views

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Query result table** | Display query results as table (blocks, columns) | `components/query_table.cljs`, `components/query/result.cljs` | Large | Column selection, sorting, filtering; might use DataTable lib |
| **Query result transform** | Apply custom Clojure fn to transform results before display | `config.edn` `:query/result-transforms`, `db/query_react.cljs` | Medium | e.g., sort by priority, custom grouping |
| **Query views** | Define custom rendering function for results | `config.edn` `:query/views`, `db/query_react.cljs` | Medium | e.g., `(fn [r] [:div ...])` pprint or custom HTML |
| **Group by page** | Render query results grouped by source page | `components/query.cljs` | Small | UI grouping; no semantic change |
| **Collapse results** | Collapse query result section | `components/query.cljs` | Small | UI state |

### Query Builder UI

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Query builder modal** | Visual UI to construct simple queries without typing syntax | `components/query/builder.cljs`, `handler/query/builder.cljs` | **Large** | Form-based query construction; dropdowns for filters; previews |

---

## 5. Search & Navigation

### Global Search (Ctrl+K / Cmd+K)

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Global search** | Search blocks and pages across entire graph | `handler/search.cljs`, `components/search.cljs` | Medium | Full-text search; returns blocks + pages; fuzzy match |
| **Search by page name** | Filter to pages matching query | `handler/search.cljs` | Small | Subset of results |
| **Search by block content** | Full-text search block content | `handler/search.cljs` | Small | Indexed search; fast |
| **Search result preview** | Show context snippet for each result | `components/search.cljs` | Small | Excerpt with highlights |
| **Search result filtering** | Filter results by type (pages, blocks, templates) | `components/search.cljs` | Small | Toggles on result list |

### Search in Page (Cmd+Shift+K)

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Search in page** | Full-text search current page only | `handler/search.cljs` (route-handler/go-to-search! :current-page) | Small | Same engine, scoped to page blocks |

### Find in Page (Cmd+F / Electron only)

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Find in page (Electron)** | Native browser find dialog | `handler/search.cljs` (open-find-in-page!), `components/find_in_page.cljs` | Small | Electron IPC; not web-compatible |
| **Find prev/next** | Navigate find results | `handler/search.cljs` (loop-find-in-page!) | Small | Keyboard shortcuts Cmd+Shift+G (next), Cmd+G (prev) |

### Command Palette (Cmd+Shift+P)

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Command palette** | Search and execute commands | `handler/command_palette.cljs`, `components/cmdk.cljs` | Medium | Fuzzy search on all available commands; returns actions |
| **Cmd+Shift+1 (Run command)** | Execute shell/system command (Electron only) | `shortcut/config.cljs` (:command/run) | Small | Advanced; limited use case |

### Navigation

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Go home (Gh)** | Navigate to home page (config: `:default-home :page`) | `route-handler.cljs` (redirect-to-home!) | Small | Shortcut; sets app route |
| **Go journals (Gj)** | Navigate to journals list | `route-handler.cljs` (go-to-journals!) | Small | Shortcut Gj |
| **Go all pages (Ga)** | Navigate to all pages view | `route-handler.cljs` (redirect-to-all-pages!) | Small | Shortcut Ga |
| **Go graph (Gg)** | Navigate to graph view | `route-handler.cljs` (redirect-to-graph-view!) | Small | Shortcut Gg |
| **Go whiteboards (Gw)** | Navigate to whiteboards dashboard | `route-handler.cljs` (redirect-to-whiteboard-dashboard!) | Small | Shortcut Gw |
| **Go keyboard shortcuts (Gs)** | Open keyboard shortcut help | Shortcut Gs | Small | Modal help |
| **Go flashcards (Gf / Tc)** | Navigate to flashcard review (excluded from scope) | - | - | **Out of scope** |
| **Back/Forward browser history** | Navigate using browser back/forward | `route-handler.cljs` (go/backward, go/forward) | Small | Shortcuts Cmd+[ and Cmd+] |
| **Breadcrumb navigation** | Click breadcrumb to jump to parent | `components/breadcrumb.cljs` | Small | UI clickable trail |
| **Open in sidebar** | Open page/block in right sidebar instead of main view | `handler/editor.cljs` (open-link-in-sidebar!), shortcut Cmd+Shift+O | Small | Navigates via sidebar panel |

---

## 6. Properties & Config

### config.edn Settings

The main configuration file affects app behavior and rendering:

| Setting | Type | Default | Impact | Notes |
|---------|------|---------|--------|-------|
| `:meta/version` | int | 1 | Schema version | Future-proofing |
| `:preferred-format` | keyword | :markdown | Format for new blocks | `:markdown` or `:org` |
| `:preferred-workflow` | keyword | :now | Task marker set | `:now` (NOW/LATER) or `:todo` (TODO/DOING) |
| `:hidden` | vector | [] | Ignore dirs/files | e.g., `["/archived" "/test.md"]` |
| `:default-templates` | map | `{:journals ""}` | Template for new items | Maps type to template name |
| `:journal/page-title-format` | string | "MMM do, yyyy" | Journal display format | Java SimpleDateFormat |
| `:journal/file-name-format` | string | "yyyy_MM_dd" | Journal filename format | **Requires manual rename on change** |
| `:ui/enable-tooltip?` | bool | true | Show tooltips | Hover tooltips |
| `:ui/show-brackets?` | bool | true | Display `[[]]` around page refs | Visual; no semantic |
| `:ui/show-full-blocks?` | bool | false | Expand block refs fully on hover | Ref expansion |
| `:ui/auto-expand-block-refs?` | bool | true | Auto-expand refs on zoom | Zoom behavior |
| `:feature/enable-block-timestamps?` | bool | false | Show created/updated on all blocks | UI feature |
| `:feature/enable-search-remove-accents?` | bool | true | Ignore accents in search | e.g., "café" matches "cafe" |
| `:feature/enable-journals?` | bool | true | Enable journal feature | Disable if not used |
| `:feature/enable-flashcards?` | bool | true | Enable flashcards | **Out of scope** |
| `:feature/enable-whiteboards?` | bool | true | Enable whiteboards | **Out of scope** |
| `:feature/disable-scheduled-and-deadline-query?` | bool | false | Hide scheduled/deadline query on journal | Hides default query |
| `:scheduled/future-days` | int | 7 | Days ahead to show in scheduled query | Look-ahead window |
| `:start-of-week` | int | 6 | First day of week (0=Mon, 6=Sun) | Calendar start |
| `:custom-css-url` | string | nil | URL to custom CSS file | Injected into head |
| `:custom-js-url` | string | nil | URL to custom JS file | Injected into head |
| `:shortcuts` | map | {} | Override keyboard shortcuts | e.g., `{:editor/new-block "enter"}` |
| `:shortcut/doc-mode-enter-for-new-block?` | bool | false | Enter creates block in doc mode | Doc mode behavior |
| `:block/content-max-length` | int | 10000 | Max searchable/editable block size | Performance limit |
| `:ui/show-command-doc?` | bool | true | Show command docs on hover | Help text |
| `:ui/show-empty-bullets?` | bool | false | Display empty bullet points | Visual clutter |
| `:query/views` | map | `{:pprint ...}` | Custom query result renderers | Clojure functions |
| `:query/result-transforms` | map | `{:sort-by-priority ...}` | Transform query results | Clojure functions |
| `:default-queries` | map | (NOW/NEXT) | Queries shown on journal | Journal-specific; customizable |
| `:commands` | vector | [] | User-defined slash commands | Extends `/` menu |
| `:outliner/block-title-collapse-enabled?` | bool | false | Allow collapsing multi-line blocks | Collapse UI |
| `:macros` | map | {} | Text replacement macros | e.g., `{"poem" "Rose is $1, ..."}` |
| `:ref/default-open-blocks-level` | int | 2 | Default expansion depth for linked refs | Ref UI |
| `:ref/linked-references-collapsed-threshold` | int | 50 | Auto-collapse linked refs if >N | Ref UI |
| `:graph/settings` | map | (see notes) | Graph view filters | `:orphan-pages?`, `:builtin-pages?`, `:excluded-pages?`, `:journal?` |
| `:graph/forcesettings` | map | (see notes) | Graph physics tuning | `:link-dist`, `:charge-strength`, `:charge-range` |
| `:favorites` | vector | [] | Favorite pages for sidebar | Order matters |
| `:srs/learning-fraction` | float | 0.5 | Flashcard learning factor | **Out of scope** |
| `:srs/initial-interval` | int | 4 | Flashcard initial interval | **Out of scope** |
| `:block-hidden-properties` | set | nil | Properties not displayed | e.g., `#{:public :icon}` |
| `:property-pages/enabled?` | bool | true | Create wiki pages for properties | Property pages |
| `:property-pages/excludelist` | set | nil | Properties without wiki pages | Exclude from indexing |
| `:property/separated-by-commas` | set | nil | Comma-separated = multiple refs | e.g., `#{:alias :tags}` |
| `:ignored-page-references-keywords` | set | nil | Properties not treated as refs | e.g., `#{:author}` |
| `:logbook/settings` | map | (see notes) | Logbook behavior | `:with-second-support?`, `:enabled-in-all-blocks`, `:enabled-in-timestamped-blocks` |
| `:pages-directory` | string | "pages" | Directory for regular pages | On-disk location |
| `:journals-directory` | string | "journals" | Directory for journal pages | On-disk location |
| `:whiteboards-directory` | string | "whiteboards" | Directory for whiteboards | **Out of scope** |
| `:org-mode/insert-file-link?` | bool | false | Org: wrap refs as file links | Org-mode specific |
| `:publishing/all-pages-public?` | bool | false | Publish all pages as public | **Export/publish** |
| `:default-home` | map | nil | Home page + sidebar config | e.g., `{:page "Changelog", :sidebar ["Contents"]}` |
| `:quick-capture-templates` | map | nil | Quick capture text/media templates | Mobile feature |
| `:quick-capture-options` | map | nil | Quick capture behavior | Mobile feature |
| `:file-sync/ignore-files` | vector | [] | Ignore files when syncing | Regexp patterns |
| `:dwim/settings` | map | (see notes) | DWIM (Do What I Mean) behavior | `:admonition&src?`, `:markup?`, `:block-ref?`, `:page-ref?`, `:properties?`, `:list?` |
| `:file/name-format` | keyword | :triple-lowbar | Filename escaping for titles | `:triple-lowbar` (default) or others |
| `:mobile/photo` | map | nil | Mobile photo upload config | `:allow-editing?`, `:quality` |
| `:mobile/gestures` | map | nil | Gesture settings | `:disabled-in-block-with-tags` |

**Key file**: `src/resources/templates/config.edn`  
**Effort**: Small (all individual settings are flags/strings)

### Custom CSS/JS

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Custom CSS file** | `logseq/custom.css` in graph root | File system, CSS injector | Small | Or via `:custom-css-url` config |
| **Custom JS file** | `logseq/custom.js` in graph root | File system, JS injector | Small | Or via `:custom-js-url` config; sandboxed |
| **Theme switching** | Toggle light/dark theme | `state.cljs` (toggle-theme!), CSS | Small | CSS vars; two built-in themes |
| **Accent colors** | Customize UI accent color | `handler/ui.cljs`, CSS | Medium | Color picker; stores in state |

---

## 7. Assets & PDF

### Asset Handling

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Asset upload** | Upload files (images, PDFs, etc.) via `/Upload` or drag | `handler/assets.cljs`, `components/assets.cljs` | Medium | File picker; multipart upload; stores in `assets/` dir |
| **Asset paste** | Paste images from clipboard; auto-upload | `handler/paste.cljs`, `handler/assets.cljs` | Medium | Clipboard handler; creates file; inserts ref |
| **Asset drag-and-drop** | Drag files into editor; auto-upload | `handler/dnd.cljs`, `handler/assets.cljs` | Medium | Drag event handler; upload + insert |
| **Asset directory** | Configure where assets are stored | `config.edn` (implicit: `assets/`) | Small | Fixed dir; all assets in one place |
| **Asset linking** | Reference assets in blocks (images inline) | Format parser, lightbox | Small | Markdown: `![alt](path)`, Org: `[[file:path]]` |

### PDF Viewer (Already Partially Ported)

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **PDF viewer** | Display PDF inline or in modal | `extensions/pdf/` | **Large** (core done, extras needed) | PDF.js library; page by page rendering |
| **PDF navigation** | Previous/next page (Alt+P / Alt+N) | `extensions/pdf/utils.cljs` | Small | Keyboard shortcuts |
| **PDF zoom** | Fit page, fit width, zoom controls | `extensions/pdf/utils.cljs` | Small | UI buttons; zoom in/out |
| **PDF text search** | Find text in PDF (Alt+F) | `extensions/pdf/utils.cljs` | Medium | Search within PDF; highlights matches |
| **PDF area highlights** | Draw box/region highlights on PDF | `extensions/pdf/assets.cljs` (likely) | Large | Capture coordinates; store as page + block ref or separate data structure |
| **PDF highlight colors** | Color-coded highlights (yellow, green, pink, etc.) | `extensions/pdf/assets.cljs` | Medium | Metadata per highlight |
| **PDF highlight export** | Export highlights as blocks/markdown | `extensions/pdf/` | Medium | Generates block refs + snapshot on a page |
| **PDF toolbar** | Controls for page nav, zoom, search | `extensions/pdf/` | Small | UI buttons |

---

## 8. UI/UX Chrome

### Left Sidebar

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Left sidebar** | Main navigation panel; pages, journals, searches | `components/sidebar.cljs`, `modules/layout/core.cljs` | Small | Collapsible; favorites at top |
| **Sidebar sections** | Favorites, Journals, Pages, Graphs, etc. | `components/sidebar.cljs` | Small | Expandable sections |
| **Toggle sidebar** | Hide/show left sidebar (Tl) | `state.cljs` (toggle-left-sidebar!), shortcut Tl | Small | UI state |
| **Sidebar search** | Quick search on sidebar items | `components/sidebar.cljs` | Small | Filter pages/journals as you type |

### Right Sidebar

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Right sidebar** | Secondary navigation; open pages/blocks | `components/right_sidebar.cljs` | Medium | Pinnable; stacked items |
| **Open in sidebar** | Open page/block in right panel instead of main | `handler/editor.cljs` (open-link-in-sidebar!) | Small | Cmd+Shift+O |
| **Pin sidebar item** | Keep item pinned; prevent auto-close | `components/right_sidebar.cljs` | Small | Icon toggle |
| **Toggle sidebar** | Hide/show right sidebar (Tr) | `handler/ui.cljs` (toggle-right-sidebar!), shortcut Tr | Small | UI state |
| **Close sidebar** | Remove item from sidebar (Ct) | `shortcut/config.cljs` (:sidebar/close-top) | Small | Shortcut Ct |
| **Clear sidebar** | Remove all sidebar items (Cmd+C Cmd+C) | `shortcut/config.cljs` (:sidebar/clear) | Small | Keyboard shortcut |

### Theme & Appearance

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Theme toggle (light/dark)** | Switch theme (Tt) | `state.cljs` (toggle-theme!) | Small | CSS vars; UI state |
| **Theme color picker** | Choose accent color (Ti) | `handler/ui.cljs` (show-themes-modal!) | Medium | Modal color picker; saves to state |
| **Theme reset accent** | Reset to default accent color (Co) | `state.cljs` (unset-color-accent!) | Small | Clears color override |
| **Wide mode** | Full-width layout (Tw) | `handler/ui.cljs` (toggle-wide-mode!) | Small | CSS class toggle |
| **Zoom / Font controls** | Increase/decrease UI font size | Settings modal or shortcuts | Small | CSS zoom or font-size var |
| **Document mode** | Single-column, distraction-free mode (Td) | `state.cljs` (toggle-document-mode!) | Small | Layout toggle; hides tree structure |

### Settings UI

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Settings modal** | All configuration in one modal (Ts on Mac, Ts or Cmd+, on others) | `components/settings.cljs`, `handler/ui.cljs` | Large | Many tabs: graph, editor, display, shortcuts, about, etc. |
| **Settings persistence** | Save settings to config.edn | `handler/config.cljs` | Small | Write config back to file |
| **Editor settings** | CodeMirror options, logical outdenting, etc. | `config.edn`, `components/settings.cljs` | Medium | Fine-tune editor behavior |
| **Search settings** | Search accent removal, rebuild index | `components/settings.cljs`, `handler/search.cljs` | Small | Triggerable actions |
| **Graph settings** | Filter orphans, built-ins, journals | `components/settings.cljs`, `config.edn` | Small | Graph view filters |

### Breadcrumbs

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Breadcrumb trail** | Show path: Page > Parent > Current Block | `components/breadcrumb.cljs` (likely) | Small | Clickable; navigates on click |

### Context Menu (Right-Click)

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Block context menu** | Right-click on block; options: edit, copy, delete, etc. | `components/block.cljs`, event handler | Medium | Contextual actions; varies by block type |
| **Context menu items (typical):** | | | |
| - Edit | Open block for editing | - | Small |
| - Copy | Copy block to clipboard | - | Small |
| - Delete | Delete block | - | Small |
| - Duplicate | Create copy of block | - | Small |
| - Open in sidebar | Open block in right panel | - | Small |
| - Copy block ref | Copy `((uuid))` | - | Small |
| - Make template | Convert to reusable template | - | Small |
| - Move to / Indent / Outdent | Tree operations | - | Small |

### Block Context Menu (via right-click or slash menu)

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Page context menu** | Right-click on page; options: rename, delete, move, etc. | `components/page_menu.cljs` | Medium | Page-specific actions |

### Keyboard Shortcut Customization UI

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Keyboard shortcut help** | View all shortcuts (Gs or Shift+?) | `components/shortcut_help.cljs` | Small | Modal listing all shortcuts by category |
| **Shortcut customization** | Edit shortcuts in Settings → Shortcuts | `components/shortcut.cljs`, `handler/config.cljs` | Medium | UI editor; validates; persists to config.edn |
| **Shortcut categories** | Organize shortcuts by function | `modules/shortcut/config.cljs` (*category) | Small | Display grouping |

### Zoom / Font Size

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **UI zoom (CSS zoom or font-size)** | Adjust overall UI scaling | Settings or browser zoom | Small | Applies to all UI |
| **Font size controls** | Adjust editor/content font size | Settings modal | Small | CSS var |

### Notifications

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Toast notifications** | Show success, error, or info messages | `handler/notification.cljs`, UI components | Small | Auto-dismiss or manual close |
| **Clear all notifications** | Dismiss all active toasts (Cmd+C Cmd+A via ui/clear-all-notifications) | `handler/notification.cljs` | Small | Bulk dismiss |

---

## 9. Import/Export/Publish

### Export Formats

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Export as Markdown** | Download blocks as `.md` file(s) | `handler/export/text.cljs` | Medium | Flat tree; may lose hierarchy in some cases |
| **Export as OPML** | Download outline as OPML (Outline Processor Markup Language) | `handler/export/opml.cljs` | Medium | XML outline format; preserves tree structure |
| **Export as HTML** | Download as static HTML for publishing | `handler/export/html.cljs` | **Large** | Complex HTML generation; CSS styling; handles all block types |
| **Export as EDN** | Raw data export (developer-focused) | `handler/export/common.cljs` | Small | Machine-readable; full fidelity |
| **Copy as...** | Copy current page/block in various formats | `handler/export/common.cljs`, context menu | Medium | Markdown, HTML, Org, plain text |

### Publishing

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Publish page** | Mark page as public; generate static URL | `handler/page.cljs` (update-public-attribute!), publishing handler | **Huge** | Requires backend/hosting; custom domain support; SSO possible; complex feature |
| **Publish all pages** | Publish entire graph as static site | `config.edn` `:publishing/all-pages-public?`, publishing handler | **Huge** | Sitemap, indexing, SEO; same backend complexity |
| **Export graph as HTML** | Download entire graph as static HTML archive | `handler/export/html.cljs` (download-repo-as-html!), shortcut Cmd+Shift+E | Large | Creates zip/archive; may be gigabytes |

---

## 10. Sync / Collaboration / Git

### File Watching & Auto-Save

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **File watching** | Monitor file system for external changes; reload on change | `handler/file.cljs`, `fs.cljs` | **Large** | Cross-platform FS watching; merge conflicts; transaction handling |
| **Auto-save** | Periodically save changes to disk | `handler/file.cljs` | Small | Debounced writes; prevents data loss |

### Git Integration

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Git auto-commit** | Automatically commit changes to git repo | `handler/git.cljs`, `components/git.cljs` | **Large** | Requires git binary; commit messages; history; conflict resolution |
| **Commit modal** | Prompt user for commit message | `components/commit.cljs`, shortcut Cmd+G Cmd+C | Medium | Interactive commit UI |
| **Git history** | View commit history of page/block | Git integration | **Huge** | Requires git binary; history traversal; blame tracking |

### Real-Time Collaboration

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Logseq File Sync service** | Cloud sync of graphs across devices | `handler/file_sync.cljs`, `components/file_sync.cljs` | **Huge** | Proprietary cloud service; real-time sync; conflict resolution; requires auth + server |
| **Multi-user collaboration** | Multiple users edit same graph concurrently | File sync + conflict resolution | **Huge** | CRDT or OT algorithms; UI for conflicts; presence indicators |

### Mobile Sync

| Feature | Description | Key Files | Effort | Notes |
|---------|-------------|-----------|--------|-------|
| **Mobile app** | Native iOS/Android app | `src/electron/`, `src/main/frontend/mobile/` | **Huge** | Separate codebase; platform-specific; sync with cloud |
| **Mobile quick capture** | Add content from phone to graph | Mobile app + sync | **Huge** | Template-based capture; background sync |

---

## PORT PRIORITY: Top ~20 Features

Based on daily-driver researcher/PKM user needs:

1. **Block editing core** (new block, delete, indent/outdent, move up/down) — *Daily essential*
2. **Undo/redo** — *Daily essential*
3. **Page creation & navigation** — *Daily essential*
4. **Markdown & Org parsing** — *Daily essential*
5. **Bold/italic/strikethrough formatting** — *Daily essential*
6. **`[[page-ref]]` and `((block-ref))` parsing & links** — *Daily essential*
7. **Slash command menu** (at least `/page`, `/block`, `/link`, `/heading`) — *Daily essential*
8. **Simple queries** (`{{query [[page]] (task TODO)}}`) — *Daily essential for PKM*
9. **Journal pages** (auto-daily, date navigation) — *Very common*
10. **Task markers** (TODO, DONE, cycling) — *Very common*
11. **Priority [#A/B/C]** — *Very common*
12. **SCHEDULED & DEADLINE dates** — *Common for GTD-style users*
13. **Linked references** (backlinks, unlinked mentions) — *Very common for graph building*
14. **Left & right sidebar** (navigation, open-in-sidebar) — *Very common*
15. **Local graph view** (if doable without 3D; 2D force-directed) — *Common for visual thinkers*
16. **Block properties** (key:: value syntax) — *Common for power users*
17. **Favorites & sidebar sections** — *Common for organization*
18. **Settings modal** (basic appearance, shortcut customization) — *Expected UX*
19. **Search** (global + page-scoped) — *Daily-driver essential*
20. **Theme switching** (light/dark) — *Expected UX*

---

## PROBABLY UNREASONABLE / HUGE

| Feature | Why | Estimated Effort |
|---------|-----|-------------------|
| **Datalog queries + query builder UI** | Full-featured datalog engine requires datascript-equivalent; complex query builder UI | **Huge** |
| **Full graph view (3D/pixi)** | Force-directed graph physics on large graphs (1000s nodes) is complex; requires performance tuning | **Huge** |
| **PDF area highlights** | Capturing screen coordinates, storing relationships, exporting highlights = complex subsystem | **Huge** |
| **Publishing (any form)** | Requires backend server, hosting, auth, custom domains, SEO, etc. | **Huge** |
| **Real-time collaboration / File Sync** | Requires cloud infrastructure, CRDT/OT, conflict resolution, auth | **Huge** |
| **Git integration (full)** | Requires git binary, history navigation, blame, conflict resolution | **Huge** |
| **File watching + external changes** | Cross-platform FS watcher; merge logic on external changes; transaction handling | **Large** |
| **Mobile app** | Separate platform codebase; OS-specific APIs; cloud sync | **Huge** |
| **Plugins/extensions** | Plugin sandbox, API, marketplace, auto-updates | **Huge** (out of scope anyway) |
| **Flashcards (SRS)** | Full spaced-repetition algorithm (SM-2, etc.); review scheduling | **Large** (out of scope) |
| **Whiteboards (tldraw integration)** | Complex drawing library; shape manipulation; collaborative editing | **Huge** (out of scope) |
| **Zotero integration** | External API; citation parsing; library sync | **Large** |
| **Advanced keyboard shortcuts** | Emacs/Vim modes; complex chord bindings | **Medium** (but lower priority) |

---

## On-Disk Format Gotchas

1. **Block UUIDs are immutable**: Once assigned, never change. References depend on them.
2. **File name format**: Default is `yyyy_MM_dd.md` for journals, `page_title.md` for pages. Changing `:journal/file-name-format` requires manual file rename on disk.
3. **Page/block nesting**: Stored as tree in block children; indentation = nesting level. Moving/indenting updates parent refs and sibling order.
4. **Properties format**: Lives in first block as `key:: value` (optional multiline). Parser must handle this syntax precisely.
5. **Scheduled/deadline AST**: Embedded in block content as `SCHEDULED: <date>` tokens; parser must extract and index by date.
6. **Markers**: Parsed at start of block content (before priority). Must parse in correct order to avoid conflicts.
7. **Logbook drawer**: `LOGBOOK` is special drawer; entries are `CLOCK [start]--[end]` format with timestamps. Must preserve on edit.
8. **Aliases & namespaces**: Purely naming conventions in page names; no separate on-disk representation. `a/b/c` is one page with `/` in title.
9. **Block refs**: References stored as bidirectional index in DB; resolving refs requires full graph traversal.
10. **Forward/backward compatibility**: Ensure new features don't break on old graphs; schema versioning matters.

---

## Summary for Porting

The OG Logseq is a feature-rich PKM system built on:
- **Core**: Tree-based outliner (blocks + nesting), bidirectional references, full-text search, task markers.
- **Key missing for Tine v0.1**: Datalog queries, graph view, full export/publishing, real-time sync, plugins.
- **High-value for daily driver**: Edit operations, slash commands, simple queries, journals, tasks, links, sidebar.
- **Gotchas**: UUID stability, file name format changes, property parsing, logbook drawer format.

Start with block editing + markdown/org parsing, add references + journals, then simple queries. Defer graph view, publishing, and sync for later phases.

