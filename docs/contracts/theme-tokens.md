# Contract — public theme tokens and `logseq/custom.css` (og)

What a user's `logseq/custom.css` may rely on across Tine updates, and how Tine
treats that file (GH #610). Kept true by same-commit updates and by the tests
listed at the end. The guard `src/themeTokens.guard.test.ts` reads THIS file:
the token tables below are the single source of truth for the shipped CSS and
for the Guide page "Customize Tine's look" (`crates/tine-core/src/templates/customize-look.md`).

## 1. Two public layers

- **Colors: Logseq's `--ls-*` variables, unchanged (compatibility).** A Logseq
  theme or snippet that sets `--ls-primary-background-color` and friends keeps
  working, and the built-in gallery palettes are written in them. Nothing in
  this contract changes their meaning.
- **Tine's own knobs: the `--tine-*` tokens in section 2.** Each one names a
  thing users have asked to change (or plausibly will) that has no `--ls-*`
  variable. A token is **unset** by default (`initial`), and every use is
  `var(--tine-x, <the value Tine drew before the token existed>)`, so an
  unset token reproduces today's rendering exactly and still follows whatever
  theme set the underlying variable. The declarations sit under `:where(:root)`
  (specificity zero), so a user rule on `:root`, `html`, `body` or a narrower
  scope all win. Set a token on `:root` or `html`: the two width tokens are read
  once at the root (they feed `--ls-main-content-max-width[-wide]`), and every
  other token is read on the element that draws it, so a narrower scope reaches
  that element and its descendants only.

## 2. Public tokens

Stability promise: removing or renaming a token in this table, or changing what
it controls, needs a CHANGELOG **Changed** entry. Adding one needs an **Added**
entry and a row here (the guard fails otherwise).

| Token | Controls | Default (what Tine draws when it is unset) |
|---|---|---|
| `--tine-danger-color` | Error and destructive-action text, error borders, and the asset trash button's hover fill | `var(--palette-danger)`, a red checked for text contrast on every bundled theme's light/dark surfaces |
| `--tine-embed-bg` | Background of an embedded block or page (`{{embed …}}`) | `var(--bg-secondary)`, the theme's secondary background |
| `--tine-embed-accent` | The accent line and bullet of an embedded block | `var(--block-embed-accent)`, 35% accent mixed into the bullet color |
| `--tine-bullet-color` | Color of the block bullet dot | `var(--bullet-color)`, i.e. `--ls-block-bullet-color` |
| `--tine-bullet-size` | Diameter of the block bullet dot | `var(--ls-block-bullet-size)`, 6px |
| `--tine-tag-color` | Color of `#tags` and tag chips | `var(--tag-color)`, i.e. `--ls-tag-text-color` |
| `--tine-highlight-bg` | Background of `^^highlighted^^` text | `var(--mark-bg)`, i.e. `--ls-page-mark-bg-color` |
| `--tine-missing-page-color` | Color of a `[[link]]` to a page that does not exist yet | A link-colored tint pulled toward `var(--text-secondary)` (`color-mix(in srgb, var(--link-color) 55%, var(--text-secondary))`) |
| `--tine-external-link-underline` | The line under a link to a web address or file (`border-bottom` shorthand; `none` removes it) | `1px solid`, in the text color (OG's `.external-link`) |
| `--tine-content-width` | Maximum width of the standard page column | 810px (what `--ls-main-content-max-width` is by default) |
| `--tine-content-width-wide` | Maximum width of the column in Wide mode | 100% (`--ls-main-content-max-width-wide`) |
| `--tine-page-title-size` | Font size of a page title | `var(--ls-page-title-size)`, 28px |
| `--tine-font-size` | Base font size of the page (`body`) | 16px |
| `--tine-content-font` | Font family of the interface and page text | `var(--ls-font-family)`, Inter then the system fonts |
| `--tine-mono-font` | Font family of code and other monospace text | `var(--ls-font-mono)`, MonoLisa then the system mono fonts |
| `--tine-editable-font` | Font family of text while you edit it (the block editor, inputs) | Inter, then the emoji face, then `var(--ls-font-family)` |

Notes.

- **Precedence.** The Settings overrides **Standard page width** and **Wide page
  width** (Settings > Appearance > Advanced) are applied inline to the document
  root and beat `--tine-content-width` and `--tine-content-width-wide`;
  `logseq/custom.css` loads after the theme gallery and wins over it, and the
  gallery wins over the stock palette.
- `--tine-content-width` and `--tine-content-width-wide` are inputs to
  `--ls-main-content-max-width` and `--ls-main-content-max-width-wide`; a
  snippet that sets those `--ls-*` variables directly still works and, being a
  direct declaration on the same element, outranks the token when it comes
  later in the cascade.
- **Presets and editors.** The editorial typography preset (Settings > Themes >
  Style) falls back to the public font tokens before its own serif face:
  `--tine-content-font` (page text and the editorial journal title),
  `--tine-editable-font` (via `--tine-editable-font-user`) and `--tine-font-size`
  win over it. The code and formula editors always edit in the monospace face
  (`--tine-mono-font`), so `--tine-editable-font` does not reach them by design.
- **Monospace.** `--tine-mono-font` covers every monospace surface Tine draws
  (code, query and diagnostic text, org timestamps, conflict and export
  previews, formula editors, the code editor). Where a surface had no
  `--ls-font-mono` link before (it drew `ui-monospace`), the token still wins and
  the default is unchanged.
- **Editor font.** When you set `--tine-editable-font`, keep the emoji face in
  the list: `--tine-editable-font: Georgia, var(--tine-editable-emoji-font), serif;`.
  Dropping it lets a generic family resolve to a system color-emoji font that
  has crashed WebKitGTK (see `src/editableEmojiFont.test.ts`).

## 3. Internal `--tine-*` variables (not public)

These exist in the shipped code but are implementation detail. They may change
without a CHANGELOG entry; a custom.css that sets them is unsupported.

| Variable | What it is |
|---|---|
| `--tine-main-content-max-width` | Set inline by Settings > Standard page width (`src/contentWidth.ts`); not a documented knob, use `--tine-content-width` |
| `--tine-wide-content-max-width` | Same, for Wide mode |
| `--tine-editable-font-user` | What you set `--tine-editable-font` to, copied at the root so the editorial typography preset can fall back to it before its own serif face (`src/styles/theme.css`, `src/styles/themePresentation.css`) |
| `--tine-editable-emoji-font` | The emoji face of the editable stack, chosen per platform in `src/styles/editableEmoji.css` (crash guard) |

## 4. `logseq/custom.css` itself

- **Where and layering.** `<graph folder>/logseq/custom.css`, one file per graph,
  loads after the Logseq shim, theme gallery and device `global.css`. The global
  file lives in Tine's per-user config directory and loads before the graph
  stylesheet, so graph rules can override device defaults.
- **Edit custom.css / Edit global CSS** (Settings > Appearance, all platforms,
  including Android and iOS). The quick switcher also finds both files by `css`.
  Both open in Tine's plain text editor. Missing files start empty: creation on save
  goes through the same guarded transaction as replacement. Every save checks the
  editor's original base revision and uses temp-file, fsync and rename; an external
  edit refuses the save and retains the buffer. Reload from disk explicitly
  discards the buffer after confirmation. Opening alone never creates a file.
- **Draft persistence.** Background visibilitychange, pagehide and freeze flush
  CSS alongside pages. Before saving, Tine keeps a recovery capsule in device app
  settings, scoped to the graph's root or the device global file, with the original
  revision. A refused save retains it; reopening the same CSS file recovers it
  without rebasing over external bytes. Successful saving clears it, unless newer
  typing remains. A background flush cannot keep a native WebView alive until IPC
  finishes; mobile termination during that IPC remains a runtime limitation.
- **Live reload (all platforms with a watcher).** The store watcher notes
  `logseq/custom.css` as its own event lane (like `config.edn`), compares a
  content revision, and publishes one external change when the bytes differ;
  the window answers `graph-custom-css-changed` by re-reading through
  `read_custom_css` and re-applying. No new timer or poll: the lane runs on
  events naming the file, on a full diff, and on the poll-mode cycle that
  already walks the graph. Saves paint immediately, then reconcile with disk:
  a newer external revision arriving before the save reply must win on screen.
  Reads from before a successful save are retired. Device CSS is also refreshed
  after saves, cross-window notifications and focus.
- **Disable custom CSS** (Settings > Appearance, every platform). A
  session-only safe mode held in memory: both CSS layers and Fine-tuning are blanked, the latest
  text is remembered, and it is never persisted, so restarting Tine always
  starts with custom CSS on and a broken stylesheet is always recoverable.
- **Developer tools** (Settings > Appearance, desktop; also Ctrl+Shift+J, Cmd+Shift+J on macOS: the binding is `mod+shift+j`). The
  inspector is compiled into release builds (tauri `devtools` feature, see
  `src-tauri/Cargo.toml`) so a user can see which rules style an element.

| Situation | Behaviour | In-scope scenario |
|---|---|---|
| `logseq/custom.css` is a directory, not a file | "Edit custom.css" refuses with an error toast; nothing is created or removed | Hand-made folder or a sync-delivery shape change; refusing is the only safe answer because there is nothing to open as text |
| The file is larger than the read limit | Applying and opening it fail with a read error; repair it in an external editor | Pasted-in or generated file; no partial buffer may replace the full file |
| The file vanishes or is half-written during a read | The lane re-looks once, then publishes what it sees (a removal, or a change with no revision). The window re-reads; if that read fails it keeps the last applied stylesheet (never blanks), retries once after a short pause, and reports only if the retry fails too. Only a graph-open read that fails applies nothing | External-editor save race (write-then-rename, truncate-then-write), Windows editor briefly holding the file |
| Two outside edits land close together | The newest re-read wins; an older read that finishes last is dropped | Editor autosave plus sync delivery |
| Another window or an editor writes the file | The window re-applies it without reopening the graph | Syncthing/Dropbox delivery, external editor |

## 5. Style knobs (Settings > Appearance > Fine-tuning)

Visual details that would otherwise each need a Settings toggle are rows of one
table, `KNOBS` in `src/styleKnobs.ts` (GH #610 item 3). Settings renders the
table; the table's rows are the contract, the Guide page names each one.

- **Same variables, not new ones.** A variable knob sets a public token of
  section 2 (`--tine-<id>`) and a class knob adds `tine-<id>-<option>` to
  `<html>`; the stylesheet default is the token's fallback, so an unset knob
  changes nothing (K3).
- **Order.** `#tine-knobs` sits after `#tine-theme` and before
  `#tine-custom-css`: a rule in `custom.css` outranks a knob (a class knob's
  rule, being a deliberate choice in Settings, outranks the matching token).
- **Only validated values reach CSS** (K2): the emitted text comes from the
  table (a declared option, an in-range number with the declared unit, a hex
  colour that also matches the theme manifest grammar), never from the stored
  string.
- **Device preference, no graph write** (K5): app setting `style.knobs.v1`,
  JSON `{knobId: value}`. A stored id this build does not declare is kept.
- **Safe mode** (K4): **Disable custom CSS** empties `#tine-knobs` and removes
  the knob classes for the session.

## Tests

- src/themeTokens.guard.test.ts (the token tables against the shipped CSS, the code and the Guide page)
- src/styleKnobs.test.ts and src/styleKnobs.render.test.tsx (the knob table, what reaches CSS, the order of #tine-knobs, safe mode, the Settings control)
- src/customCss.test.tsx (live reload, transient read failures keep the last sheet, latest read wins, safe mode, the palette command, the Settings affordances)
- src/styles/themes/soft.test.ts (the Soft palette's contrast on every real text/surface pair, including on-accent, links on the selection overlay and markers, GH #649)
- crates/tine-store/src/watch/custom_css.rs (the lane's own tests, including the metadata shortcut that skips hashing an unchanged file)
- crates/tine-graph-features/tests/custom_css_edit.rs (create-if-missing, oversized file, directory)
- src-tauri/src/watcher.rs::an_external_custom_css_edit_is_announced_and_an_own_write_is_not
- crates/tine-core/src/guide.rs::the_customize_page_is_reachable_and_teaches_the_whole_loop
- scripts/shot-theme-tokens.mjs (browser: defaults byte-identical, tokens reach the rendering, html-level overrides win)
- scripts/e2e-custom-css.mjs (native: edit the file on disk, the style is applied without a reopen)
