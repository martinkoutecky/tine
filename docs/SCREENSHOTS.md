# Screenshots — keeping them current

The README's images live in `docs/img/`. They are **not** auto-generated on
build, so they drift silently when the UI changes. **When you ship a feature or
change an existing feature's design, check the table below and regenerate any
shot it touches** (and update the README prose in the same commit).

## How they're made

All shots are **headless Chromium renders of the built frontend against the mock
backend** (`src/mock.ts` + `src/fixtures/kitchen-sink.md`) — *not* the real
WebKitGTK app. So they reflect the real layout/CSS and current components, but
with fake data and Chromium font rendering. No display/Xvfb is needed.

```bash
source scripts/env.sh        # sets PLAYWRIGHT_BROWSERS_PATH + the nspr/nss libs
npm run build                # refresh dist/ so the harness serves current code
node scripts/shot-readme.mjs # → screenshots/rm-tabs.png, rm-focus-dim.png, rm-quick-capture.png
node scripts/shot-capture.mjs# → screenshots/rm-quick-capture.png (better: slash menu + window frame)
node scripts/screenshot.mjs  # → screenshots/journals-light.png, pdf-notes-light.png, … (the review set)
```

Scripts write to the gitignored `screenshots/` dir; the README set is then
**curated by hand** — copy the chosen file to its `docs/img/` name (below).

## README image inventory

| `docs/img/` file     | Shows                                              | Generator → source file                          | Regenerate when… |
|----------------------|----------------------------------------------------|--------------------------------------------------|------------------|
| `hero.png`           | Journals feed (blocks, tasks, code, table, sidebar)| `screenshot.mjs` → `journals-light.png`          | journal/outline rendering, block markers, **sidebar sections (e.g. Namespaces)** change |
| `tabs.png`           | Built-in tabs (background + pinned)                | `shot-readme.mjs` → `rm-tabs.png`                | tab bar, pinning, or topbar layout changes |
| `focus-dim.png`      | Focus mode + dim-inactive-blocks                   | `shot-readme.mjs` → `rm-focus-dim.png`           | focus/dim behavior or chrome changes |
| `quick-capture.png`  | Quick-capture mini-window with slash menu open     | `shot-capture.mjs` → `rm-quick-capture.png`      | capture window, slash menu, or editor-parity changes |
| `pdf.png`            | PDF pane + highlights + notes page                 | `screenshot.mjs` → `pdf-notes-light.png`         | PDF viewer, highlight rendering (incl. **area highlights**), or notes layout changes |
| `settings.png`       | Settings modal (shortcuts shown)                   | settings shot (curated)                          | Settings modal gains/loses controls (**watch mode, first-day-of-week**, themes, snapshots) |

## Known limitations / honest caveats

- **Quick-capture is a frameless OS window.** The mock can only render its web
  content (a bare editor on white), so `shot-capture.mjs` adds the drop-shadow +
  rounded corners the window manager draws and opens the slash menu, so it reads
  as a floating window doing real work. For the *most authentic* shot — the real
  `tine --capture` window floating over another app — capture it on a real Linux
  desktop (build the release binary, bind/run `tine --capture`, screenshot the
  window). Swap that into `docs/img/quick-capture.png` if you want the real thing.
- Mock data is generic ("kitchen-sink"). If a feature needs specific content to
  be visible (e.g. a `/calc` result, a callout, a datalog query), add it to
  `src/fixtures/kitchen-sink.md` or a dedicated shot script.

## Currently unillustrated (candidates for new README shots)

These shipped without a screenshot — add one if the README would benefit:
`/calc` blocks, callouts (org `#+BEGIN_…` / `> [!note]`), the visual query
builder with a sort clause, area (image) PDF highlights, the sidebar namespace
tree, advanced (datalog) query results.
