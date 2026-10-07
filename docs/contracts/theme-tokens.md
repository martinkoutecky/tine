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
| `--tine-embed-bg` | Background of an embedded block or page (`{{embed …}}`) | `var(--bg-secondary)`, the theme's secondary background |
| `--tine-embed-accent` | The accent line and bullet of an embedded block | `var(--block-embed-accent)`, 35% accent mixed into the bullet color |
| `--tine-bullet-color` | Color of the block bullet dot | `var(--bullet-color)`, i.e. `--ls-block-bullet-color` |
| `--tine-bullet-size` | Diameter of the block bullet dot | `var(--ls-block-bullet-size)`, 6px |
| `--tine-tag-color` | Color of `#tags` and tag chips | `var(--tag-color)`, i.e. `--ls-tag-text-color` |
| `--tine-highlight-bg` | Background of `^^highlighted^^` text | `var(--mark-bg)`, i.e. `--ls-page-mark-bg-color` |
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

- **Where.** `<graph folder>/logseq/custom.css`, one file per graph, loaded after
  the Logseq shim and the theme gallery. It is the user's file: Tine never
  edits an existing one.
- **Edit custom.css** (Settings > Appearance, desktop). Creates the file with
  a short commented starter when it is missing, through the store's guarded
  create (`tine_graph_features::custom_css::ensure_custom_css`, a `Transaction::create`
  that fails if the file appeared meanwhile), then hands it to the operating
  system's default app for `.css` files. That app is not guaranteed to be a text
  editor (a browser or a viewer on some systems), so the button's hint and the
  Guide say so and tell the user to pick an editor. On a synced graph, creating
  the starter before the real file has arrived can make the sync tool keep the
  starter and move the real file to a conflict copy; the Guide says to wait for
  the file first. An existing file is never read, rewritten or size-checked by
  this action, so an oversized or broken file can still be opened and repaired.
  Android and iOS have no editor hand-off, so the control is replaced by a hint.
- **Live reload (all platforms with a watcher).** The store watcher notes
  `logseq/custom.css` as its own event lane (like `config.edn`), compares a
  content revision, and publishes one external change when the bytes differ;
  the window answers `graph-custom-css-changed` by re-reading through
  `read_custom_css` and re-applying. No new timer or poll: the lane runs on
  events naming the file, on a full diff, and on the poll-mode cycle that
  already walks the graph. Tine's own creation of the file does not echo.
- **Disable custom CSS** (Settings > Appearance, every platform). A
  session-only safe mode held in memory: the stylesheet is blanked, the latest
  text is remembered, and it is never persisted, so restarting Tine always
  starts with custom CSS on and a broken stylesheet is always recoverable.
- **Developer tools** (Settings > Appearance, desktop; also Ctrl+Shift+J, Cmd+Shift+J on macOS: the binding is `mod+shift+j`). The
  inspector is compiled into release builds (tauri `devtools` feature, see
  `src-tauri/Cargo.toml`) so a user can see which rules style an element.

| Situation | Behaviour | In-scope scenario |
|---|---|---|
| `logseq/custom.css` is a directory, not a file | "Edit custom.css" refuses with an error toast; nothing is created or removed | Hand-made folder or a sync-delivery shape change; refusing is the only safe answer because there is nothing to open as text |
| The file is larger than the read limit | Applying it fails with the existing "custom.css could not be read" toast; "Edit custom.css" still opens it so it can be fixed | Pasted-in or generated file; recovery over refusal |
| The file vanishes or is half-written during a read | The lane re-looks once, then publishes what it sees (a removal, or a change with no revision). The window re-reads; if that read fails it keeps the last applied stylesheet (never blanks), retries once after a short pause, and reports only if the retry fails too. Only a graph-open read that fails applies nothing | External-editor save race (write-then-rename, truncate-then-write), Windows editor briefly holding the file |
| Two outside edits land close together | The newest re-read wins; an older read that finishes last is dropped | Editor autosave plus sync delivery |
| Another window or an editor writes the file | The window re-applies it without reopening the graph | Syncthing/Dropbox delivery, external editor |

## Tests

- src/themeTokens.guard.test.ts (the token tables against the shipped CSS, the code and the Guide page)
- src/customCss.test.tsx (live reload, transient read failures keep the last sheet, latest read wins, safe mode, the palette command, the Settings affordances)
- src/styles/themes/soft.test.ts (the Soft palette's contrast on every real text/surface pair, including on-accent, links on the selection overlay and markers, GH #649)
- crates/tine-store/src/watch/custom_css.rs (the lane's own tests, including the metadata shortcut that skips hashing an unchanged file)
- crates/tine-graph-features/tests/custom_css_edit.rs (create-if-missing, oversized file, directory)
- src-tauri/src/watcher.rs::an_external_custom_css_edit_is_announced_and_an_own_write_is_not
- crates/tine-core/src/guide.rs::the_customize_page_is_reachable_and_teaches_the_whole_loop
- scripts/shot-theme-tokens.mjs (browser: defaults byte-identical, tokens reach the rendering, html-level overrides win)
- scripts/e2e-custom-css.mjs (native: edit the file on disk, the style is applied without a reopen)
