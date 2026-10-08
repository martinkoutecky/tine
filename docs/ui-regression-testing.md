# UI regression testing

Tine's interaction tests should assert user-visible state, not merely that an
event handler ran. Caret and focus regressions are the clearest example: after
each gesture, record the active block, editing surface, selection range, visual
row and horizontal caret position. Preserve a screenshot, DOM snapshot, frontend
console and backend log on failure.

## Test layers

Use the cheapest layer that can actually observe the regression:

1. **Pure and jsdom tests** cover editor state, focus ownership, source offsets,
   parser output and deterministic layout algorithms with injected geometry.
   They are fast PR gates, but jsdom does not implement CSS layout.
2. **Browser-mode tests** run the frontend at a fixed viewport with mocked Tauri
   commands. They cover real CSS, edit/render transitions, clicks, collapse and
   screenshots cheaply. Chromium is useful here but does not replace WebKitGTK.
3. **Native Tauri WebDriver tests** launch a disposable graph and the production
   binary. They cover native key/pointer dispatch, WebKit layout and focus,
   routing, IPC and persistence. Linux uses WebKitWebDriver under Xvfb; Windows
   must also be tested natively for WebView2.

Visual appearance uses image snapshots with deterministic fonts, theme, zoom,
viewport, DPI and disabled animation. Interaction correctness should prefer
semantic assertions (active block, caret range/geometry, sidebar contents,
collapse state and persisted Markdown), because they explain failures better
than pixel diffs.

## Regression catalog

Consolidate the one-off `scripts/e2e-*.mjs` launch code into a shared runner,
then keep scenarios in a machine-readable catalog. Each entry should contain:

- stable regression ID and source GitHub issue or fixing commit;
- graph fixture and routed starting location;
- viewport, theme and platform requirements;
- gestures and observable invariants after every gesture;
- cheapest required test layer and native platforms;
- expected screenshots only when appearance is itself the invariant.

Initial interaction families:

- vertical/horizontal caret movement across short, explicit-multiline and
  visually wrapped blocks, including headings, properties and planning lines;
- entering, editing and leaving headings, emphasis, links, tags, numbers, emoji,
  code and formulas;
- collapse/expand by mouse and keyboard, including descendants and zoom;
- bullet click, Shift-click sidebar opening, references and duplicate split-pane
  surfaces;
- block selection, drag/drop, undo/redo, paste and persistence/reload.

Mine candidates from GitHub issues, `CHANGELOG.md`, commits whose messages include
fix/regression/caret/focus/collapse/sidebar, and existing regression test names.
Do not promote every old bug to a slow native test: place it at the lowest layer
that would have failed before the fix, and reserve native cases for layout,
platform, IPC and persistence boundaries.

## Execution policy

- Pull requests: unit/jsdom suite, browser-mode interaction suite, and a small
  Linux native smoke/caret matrix.
- Master/nightly: full Linux WebKitGTK catalog plus native Windows WebView2.
- Pre-release: the full catalog on every supported desktop platform, with
  screenshots reviewed when their baselines changed.
- Do not hide flakes with unconditional retries. On failure, retain artifacts;
  a diagnostic retry may classify a flake but the original failure remains.

## Container prerequisites

Linux native journeys need a private session bus as well as an X display.
The single-instance plugin forwards CLI launches over D-Bus; without a shared
bus, a second launch starts an independent process that WebDriver cannot see.
`scripts/run-e2e.mjs` starts each native scenario under Xvfb and a private bus
through `scripts/lib/e2e-session-bus.mjs`. Direct multigraph and Quick Capture
journeys call `ensurePrivateSessionBus()` before creating fixtures or starting
drivers, so `xvfb-run -a node scripts/e2e-multigraph.mjs` works too. The helper
reuses a runner-owned bus and preserves child output and exit status. Install
`dbus-run-session` (or set `DBUS_RUN_SESSION` to its path); missing it fails at
setup. Windows and macOS keep their native forwarding transports. The guard
`src/e2eSessionBus.guard.test.ts` requires this setup in every journey that
spawns a second app, with the multigraph journey as its exemplar (I-12/I-21).

For Debian/Ubuntu preload Rust stable, Node matching the lockfile, project npm
dependencies, `tauri-driver`, `libwebkit2gtk-4.1-dev`, `webkit2gtk-driver`,
`libayatana-appindicator3-dev`, `libgtk-3-dev`, `libsoup-3.0-dev`,
`libjavascriptcoregtk-4.1-dev`, `xvfb`, `xauth`, `dbus-x11`, `at-spi2-core`,
`x11-utils`,
Mesa software-rendering packages, and deterministic fonts. Also install
`ffmpeg` and ImageMagick for artifacts, plus `xclip`, `xsel` and `wl-clipboard`
for clipboard coverage. Playwright Chromium/WebKit are complementary browser
engines; they do not replace the native Tauri WebKitGTK run.

The container must allow child processes, localhost WebDriver ports and enough
`/dev/shm`. Linux cannot faithfully substitute for Windows WebView2 or macOS
WKWebView, so those need native CI runners. Tauri's current WebdriverIO service
and embedded test provider are worth evaluating when the shared runner is built,
especially for macOS coverage; keep any WebDriver plugins test-only so they are
absent from release binaries.
