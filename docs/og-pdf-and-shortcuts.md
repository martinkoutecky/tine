# Logseq OG: PDF Highlighting & Keyboard Shortcuts Architecture

## Part A: PDF Highlighting System

### 1. Storage Format of Highlights

#### Highlight Pages (Index)
Highlights are indexed via **specially-named pages** in the graph:
- **Naming convention**: `hls__<sanitized-filename>` 
  - Example: `hls__2015_Book_Intertwingled_1659920114630_0` → display name "2015 Book Intertwingled"
  - Example: `hls__sicp__-1234567` → display name "sicp"
- These pages are stored in `pages/` like any other page (in the preferred format: markdown or org)
- The first block created in this page contains the file reference link
- **Property keys** on the page:
  - `:file` — markdown form: `[label](../assets/file.pdf)` or org form: `[[file-path][label]]`
  - `:file-path` — relative path to the PDF asset (e.g., `../assets/mybook.pdf`)

#### Highlight Blocks (Annotations)
Each highlight is stored as a **block** under the highlight page with these properties:
- `:ls-type` — always `"annotation"`
- `:hl-page` — the page number in the PDF (integer, 1-indexed)
- `:hl-color` — color of the highlight: `"yellow" | "red" | "green" | "blue" | "purple"`
- `:hl-type` — type of highlight: `"area"` (region/screenshot) or omitted (text selection)
- `:hl-stamp` — timestamp or image stamp:
  - For text highlights: `(js/Date.now)` (millisecond timestamp)
  - For area highlights: `(js/Date.now)` at creation time (identifies the stored PNG image)
- `:id` — the block UUID, which matches the highlight ID in the in-memory highlights array

**Example block content** (markdown):
```
some highlighted text here
hl-page:: 42
hl-color:: yellow
ls-type:: annotation
id:: 5e8f9c7b-1234-5678-abcd-ef1234567890
```

#### Highlight Data (.edn file)
Highlights are persisted in EDN format at:
- **Path**: `assets/<sanitized-filename>.edn`
- **Contents**: `{:highlights [<highlight-objects>] :extra {}}`

**Highlight object structure**:
```clojure
{
  :id       "UUID-string"                    ; Block UUID
  :page     42                               ; PDF page number (1-indexed)
  :position {
    :page    42
    :bounding {:top 100 :left 50 :width 400 :height 200}  ; Viewport coords at render time
    :rects [
      {:top 100 :left 50 :width 400 :height 20}          ; Each line's bounding box
      {:top 120 :left 50 :width 380 :height 20}
    ]
  }
  :content {
    :text "extracted text from selection"                  ; Only for text highlights
    :image timestamp-or-nil                               ; For area highlights: (js/Date.now)
  }
  :properties {
    :color "yellow"                         ; Highlight color
  }
}
```

### 2. Asset Storage & References

#### Asset Directory
- **Directory**: `assets/` in the graph root
- **Naming**: 
  - PDFs: original filename or sanitized name
  - Area images: `assets/<key>/<page>_<highlight-id>_<image-stamp>.png`
    - Example: `assets/my-book/42_5e8f9c7b-1234-5678-abcd-ef1234567890_1659920114630.png`

#### Markdown Link Forms
In highlight page blocks:
- **Markdown**: `[My PDF](../assets/my-book.pdf)` stored in `:file` property
- **Org**: `[[../assets/my-book.pdf][My PDF]]` stored in `:file` property

The link is navigable and clicking it opens the PDF viewer.

### 3. Highlight Model in Code

#### Core Files
1. **`src/main/frontend/extensions/pdf/core.cljs`** (45KB)
   - Component `pdf-highlights` — captures selection, manages state
   - Component `pdf-highlight-area-region` — renders area highlights with resize handles
   - Component `pdf-highlights-text-region` — renders text selection highlights
   - Functions: `add-hl!`, `upd-hl!`, `del-hl!` — manage in-memory state
   - Selection event listeners capture mouse and text range

2. **`src/main/frontend/extensions/pdf/assets.cljs`** (298 lines)
   - `inflate-asset` — parses asset paths and generates keys
   - `load-hls-data$` / `persist-hls-data$` — read/write EDN files
   - `ensure-ref-page!` — creates or gets the highlight page
   - `ensure-ref-block!` — creates a block for each highlight with all properties
   - `update-hl-block!` — updates properties on existing block
   - `persist-hl-area-image$` — writes PNG images via canvas
   - `area-highlight?` — checks if highlight has `:content.image`
   - `open-block-ref!` — navigates to block and opens PDF at that location

3. **`src/main/frontend/extensions/pdf/utils.cljs`** (203 lines)
   - Coordinate transforms: `viewport-to-scaled`, `scaled-to-viewport`
   - `vw-to-scaled-pos`, `scaled-to-vw-pos` — position conversions
   - `get-range-rects<-page-cnt` — extracts bounding boxes from DOM Range
   - `optimize-client-rects` — merges overlapping rects (JS bridge)
   - `get-bounding-rect` — union of all rects
   - `hls-file?` — checks if filename is a highlight index page

#### Highlight Lifecycle
1. **Capture**: Text selection via DOM Range or area selection via Shift+Click (or Alt+Click on Mac)
   - `pdf-highlights` component listens to `mouseup` + `selectionchange` events
   - `pdf-highlight-area-selection` component listens to `mousedown`/`mousemove`/`mouseup`
2. **Create**: User chooses color from context menu
   - Color action triggers `add-hl!` with new UUID
   - `copy-hl-ref!` creates the corresponding block
3. **Persist**: 
   - In-memory: `set-highlights!` updates React state
   - Block: `ensure-ref-block!` creates a block with properties
   - EDN file: `persist-hls-data$` writes highlights array
   - Image: `persist-hl-area-image$` writes PNG if area highlight
4. **Display**: 
   - `pdf-highlights-region-container` mounts highlight overlay components for each page
   - Text highlights: `pdf-highlights-text-region` renders colored divs over text rects
   - Area highlights: `pdf-highlight-area-region` renders a resizable div

### 4. Rendering Library

- **PDF.js** (Mozilla's browser PDF renderer) — via `pdfjsViewer` global
- **Version**: Not pinned in source; included as JS library
- **Components used**:
  - `pdfjsViewer.PDFViewer` — main viewer instance
  - `pdfjsViewer.PDFLinkService` — handles link navigation
  - `pdfjsViewer.PDFFindController` — text search
  - `pdfjsViewer.EventBus` — event dispatch
  - `textLayerMode: 2` — enables text layer for selection
  - `annotationMode: 2` — enables annotations
  - `removePageBorders: true` — seamless page layout

**Highlight rendering**:
- Created div overlays with CSS classes (`.extensions__pdf-hls-text-region`, `.extensions__pdf-hls-area-region`)
- Styled with `data-color` attribute (CSS applies background colors)
- Positioned absolutely over the PDF canvas pages
- Uses `interact.js` for area highlight resizing

### 5. Block ↔ Highlight Link

**Forward (Highlight → Block)**:
- When highlight is created, `ensure-ref-block!` is called
- Creates a block with `:id` property = highlight UUID
- Block is stored under the highlight page (`hls__<pdf-name>`)

**Reverse (Block → PDF)**:
- When user clicks a highlight block, `open-block-ref!` is called
- Extracts `:hl-page` from block properties
- Looks up the original highlight object in the EDN file by matching UUID
- Sets state: `:pdf/ref-highlight` = the highlight object (with `:page` and `:id`)
- Opens the PDF viewer
- `pdf-highlight-finder` component scrolls to that highlight

**Property Keys for Navigation**:
- Block `:id` — the UUID of the highlight
- Block `:hl-page` — the page number (fallback if highlight is deleted from EDN)
- Highlight `:id` — matches block UUID

---

## Part B: Keyboard Shortcuts System

### 1. Default Shortcuts Definition

**File**: `src/main/frontend/modules/shortcut/config.cljs` (967 lines)

**Map**: `all-built-in-keyboard-shortcuts` — a large EDN map with 130+ entries

**Entry structure**:
```clojure
:command-id {
  :binding "key-combo"           ; or ["key1" "key2+alt"] for alternatives
  :fn      handler-function      ; or qualified keyword :namespace/fn for lazy eval
  :inactive condition-boolean    ; optional, disables shortcut if true
}
```

### 2. Representative Default Keybindings

#### Block Editing (in edit mode)
| Command | Binding | Function |
|---------|---------|----------|
| `:editor/new-block` | `enter` | Create new block below |
| `:editor/new-line` | `shift+enter` | Soft line break |
| `:editor/indent` | `tab` | Increase nesting |
| `:editor/outdent` | `shift+tab` | Decrease nesting |
| `:editor/move-block-up` | `mod+shift+up` (Mac: meta) / `alt+shift+up` (Linux) | Move block up |
| `:editor/move-block-down` | `mod+shift+down` | Move block down |
| `:editor/bold` | `mod+b` | **Bold** |
| `:editor/italics` | `mod+i` | *Italic* |
| `:editor/highlight` | `mod+shift+h` | Highlight text |
| `:editor/strike-through` | `mod+shift+s` | ~~Strikethrough~~ |
| `:editor/backspace` | `backspace` | Delete backward |
| `:editor/delete` | `delete` | Delete forward |
| `:editor/undo` | `mod+z` | Undo |
| `:editor/redo` | `mod+shift+z` or `mod+y` | Redo |

#### Navigation (global)
| Command | Binding | Function |
|---------|---------|----------|
| `:go/search` | `mod+k` | Global search |
| `:command-palette/toggle` | `mod+shift+p` | Command palette |
| `:go/search-in-page` | `mod+shift+k` | Page search |
| `:go/journals` | `g j` | Go to journals |
| `:go/home` | `g h` | Go to home page |
| `:go/all-pages` | `g a` | All pages |
| `:go/graph-view` | `g g` | Graph view |
| `:go/backward` | `mod+[` | Browser back |
| `:go/forward` | `mod+]` | Browser forward |

#### UI Toggles
| Command | Binding | Function |
|---------|---------|----------|
| `:ui/toggle-left-sidebar` | `t l` | Toggle sidebar |
| `:ui/toggle-right-sidebar` | `t r` | Toggle right sidebar |
| `:ui/toggle-theme` | `t t` | Dark/light theme |
| `:ui/toggle-document-mode` | `t d` | Doc vs outline mode |
| `:ui/toggle-settings` | `t s` (or `mod+,` on Mac) | Settings |
| `:ui/toggle-help` | `shift+/` | Help |

#### PDF (when PDF viewer open)
| Command | Binding | Function |
|---------|---------|----------|
| `:pdf/previous-page` | `alt+p` | Previous page |
| `:pdf/next-page` | `alt+n` | Next page |
| `:pdf/close` | `alt+x` | Close PDF |
| `:pdf/find` | `alt+f` | Open find dialog |

### 3. User Customization via config.edn

**Location**: User stores custom shortcuts in:
- **Graph-level**: Graph's `config.edn` (`:shortcuts {...}`)
- **Global**: Global `config.edn` (`:shortcuts {...}`) — persisted by Logseq

**Config.edn format**:
```clojure
{
  :shortcuts {
    :editor/bold         "cmd+2"           ; Override binding
    :editor/italics      false             ; Disable shortcut
    :go/search           "ctrl+,"          ; Custom binding
    :custom/my-command   ["mod+alt+x"]     ; New command (if registered via plugin)
  }
}
```

**Merging rules** (in order):
1. Default from `all-built-in-keyboard-shortcuts`
2. Merged with global config `:shortcuts`
3. Merged with graph config `:shortcuts`
4. User binding overrides default

**Persistence** (in `core.cljs`):
```clojure
(defn persist-user-shortcut!
  [id binding]
  (let [graph-shortcuts (or (:shortcuts (state/get-graph-config)) {})
        global-shortcuts (or (:shortcuts (state/get-global-config)) {})]
    ;; if binding is nil, removes from shortcuts
    ;; if binding is string/vector/false, adds/updates in global config
    (global-config-handler/set-global-config-kv! :shortcuts (into-shortcuts global-shortcuts))))
```

### 4. Handler Architecture & Dispatch

#### Handler Groups
Shortcuts are organized into handler groups for context-aware enablement:

**File**: `src/main/frontend/modules/shortcut/config.cljs` (lines 599-749)

| Group | Context | Examples |
|-------|---------|----------|
| `:shortcut.handler/block-editing-only` | When editing a block | Backspace, Delete, Format |
| `:shortcut.handler/editor-global` | When NOT editing | Block movement, selection |
| `:shortcut.handler/global-prevent-default` | Always on | Undo, Search, Save |
| `:shortcut.handler/global-non-editing-only` | Navigation, not editing | Home, Go to journals, Toggles |
| `:shortcut.handler/pdf` | PDF viewer open | PDF page nav (with `:before` guard) |
| `:shortcut.handler/whiteboard` | Whiteboard editing | Canvas tools |
| `:shortcut.handler/auto-complete` | Autocomplete menu open | Up, Down, Enter, Escape |
| `:shortcut.handler/date-picker` | Date picker open | Arrows, Enter |
| `:shortcut.handler/cards` | Flashcard mode | Answer, Next card, etc. |
| `:shortcut.handler/misc` | Always active | Copy fallback |

#### Core Registration & Dispatch System

**File**: `src/main/frontend/modules/shortcut/core.cljs`

1. **Installation** (`install-shortcut-handler!`):
   - Creates a `goog.ui.KeyboardShortcutHandler` instance
   - Registers all shortcuts in a handler group
   - Attaches event listener for `SHORTCUT_TRIGGERED`

2. **Registration** (`register-shortcut!`):
   - Takes handler-id, command id, and optional override
   - Looks up binding via `dh/shortcut-binding`
   - Calls `.registerShortcut(handler, id, binding)` on the Closure handler
   - Binding may be a single key or array of alternatives

3. **Dispatch** (event listener `fn [e]`):
   - On trigger: extracts shortcut ID from event
   - Looks up handler function from shortcut map
   - Calls `plugin-handler/hook-lifecycle-fn! id dispatch-fn e`
   - Function receives `[state event]` or `[event]` depending on context

4. **Guard/Before Functions**:
   - Handlers can have a `:before` function in metadata (line 606, 610, etc.)
   - Example: `(with-meta handler-map {:before m/enable-when-not-editing-mode!})`
   - Before function wraps the dispatch function to add preconditions

#### Key Functions

**`frontend.modules.shortcut.data_helper`**:
- `shortcut-binding [id]` — gets user override, then default binding
- `mod-key [shortcut]` — converts "mod" to "ctrl" (Linux) or "meta" (Mac)
- `binding-for-display [k binding]` — formats binding for UI display
- `flatten-bindings-by-id` — caches binding lookups by command ID
- `flatten-bindings-by-key` — caches binding lookups by key combo (detects conflicts)

**`frontend.modules.shortcut.utils`**:
- `undecorate-binding` — removes display formatting
- `safe-parse-string-binding` — parses "mod+a" to JS key objects
- `decorate-binding` — adds display formatting (for UI)

#### Configuration Format in Code

From `config.cljs` **lines 44-49**:
```clojure
;; A shortcut is a map with the following keys:
;;  * :binding - A string representing a keybinding. Avoid using single letter
;;    shortcuts to allow chords that start with those characters
;;  * :fn - Fn or a qualified keyword that represents a fn
;;  * :inactive - Optional boolean to disable a shortcut for certain conditions
;;    e.g. a given platform or feature condition
```

From **line 278-294** (persistence):
```clojure
(defn persist-user-shortcut!
  [id binding]
  (let [graph-shortcuts (or (:shortcuts (state/get-graph-config)) {})
        global-shortcuts (or (:shortcuts (state/get-global-config)) {})
        global? true]
    (letfn [(into-shortcuts [shortcuts]
              (cond-> shortcuts
                      (nil? binding) (dissoc id)
                      (and global? (or (string? binding) (vector? binding) (boolean? binding)))
                      (assoc id binding)))]
      ;; Persists to global config
      (global-config-handler/set-global-config-kv! :shortcuts (into-shortcuts global-shortcuts)))))
```

---

## Implementation Notes for Clone

### PDF Highlights
- **Must preserve**: Exact EDN structure in `assets/<key>.edn`; block properties (`:hl-page`, `:hl-color`, `:ls-type`); PNG paths; UUID linking
- **PDF rendering**: Can substitute PDF.js with compatible library (pdfium, etc.) as long as coordinate system and zoom scale are preserved
- **Rects/Bounding**: Store in scaled coordinates (document space), not viewport space

### Keyboard Shortcuts
- **Must preserve**: Config.edn `:shortcuts` key format; binding string syntax (`"mod+k"`, `["alt+p" "alt+n"]`); command ID namespace (`:editor/`, `:go/`, etc.)
- **Merging order**: Default → Global config → Graph config → ensures compatibility
- **Handler architecture**: Can use different dispatch library but must support context-aware enable/disable (editing mode, PDF mode, etc.)

---

## Key File Paths (OG)

### PDF
- `/aux/koutecky/logseq/og/src/main/frontend/extensions/pdf/core.cljs` (500+ lines, 45KB)
- `/aux/koutecky/logseq/og/src/main/frontend/extensions/pdf/assets.cljs` (298 lines)
- `/aux/koutecky/logseq/og/src/main/frontend/extensions/pdf/utils.cljs` (203 lines)
- `/aux/koutecky/logseq/og/src/main/frontend/extensions/pdf/toolbar.cljs` (500+ lines)
- `/aux/koutecky/logseq/og/src/test/frontend/extensions/pdf/assets_test.cljs`

### Shortcuts
- `/aux/koutecky/logseq/og/src/main/frontend/modules/shortcut/config.cljs` (967 lines, all defaults + categories)
- `/aux/koutecky/logseq/og/src/main/frontend/modules/shortcut/core.cljs` (295 lines, dispatch/installation)
- `/aux/koutecky/logseq/og/src/main/frontend/modules/shortcut/data_helper.cljs` (253 lines, config merging/lookup)
- `/aux/koutecky/logseq/og/src/main/frontend/modules/shortcut/utils.cljs` (decorator/parser utilities)

### State/Config
- `/aux/koutecky/logseq/og/src/main/frontend/state.cljs` (contains `shortcuts()`, `get-config()`, `merge-configs()`)
- `/aux/koutecky/logseq/og/src/main/frontend/handler/config.cljs` (config read/write handlers)

### C++ Prior Art (not used in OG, but reference for PDF extraction)
- `/aux/koutecky/logseq/logseq-native/src/pdf_worker/main.cpp` (uses PyMuPDF for metadata extraction)

