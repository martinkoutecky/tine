# Screenshots — keeping them current

The README's images live in `docs/img/`. They are **not** auto-generated on
build, so they drift silently when the UI changes. **When you ship a feature or
change an existing feature's design, check the table below and regenerate any
shot it touches** (and update the README prose in the same commit).

## Verify UI work visually before handing it off (project process)

The same harness is how Claude **self-verifies any visual/UI change** — not just
README shots. For a new or changed UI feature: build, drive the relevant state in
the mock, screenshot it, look at the image, and iterate until it matches the
intended (and OG) look *before* asking Martin to test. Martin has OK'd the extra
token cost — it saves his time, and he shouldn't be the one eyeballing something
that can be screenshotted here. (No Xvfb needed; it's headless Chromium against
the real frontend. Xvfb + the real WebKitGTK app is reserved for WebKitGTK-only
rendering quirks — fonts/emoji — not layout.)

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
node scripts/shot-improve.mjs# → screenshots/improve-{empty,report,findings}.png (Help improve Tine diff panel; uses __tineDiffFixture)
```

Scripts write to the gitignored `screenshots/` dir; the README set is then
**curated by hand** — copy the chosen file to its `docs/img/` name (below).

## README image inventory

| `docs/img/` file     | Shows                                              | Generator → source file                          | Regenerate when… |
|----------------------|----------------------------------------------------|--------------------------------------------------|------------------|
| `hero.png`           | Journals feed (blocks, tasks, code, table, sidebar)| `screenshot.mjs` → `journals-light.png`          | journal/outline rendering, block markers, **sidebar sections (e.g. Namespaces)** change |
| `tabs.png`           | Built-in tabs (3 tabs, one pinned)                 | `shot-tabs-pdf.mjs` → `feat-tabs.png`            | tab bar, pinning, or topbar layout changes |
| `focus-dim.png`      | Focus mode + dim-inactive-blocks                   | `shot-readme.mjs` → `rm-focus-dim.png`           | focus/dim behavior or chrome changes |
| `dim.png`            | Dim-inactive-blocks (one block spotlit)            | `shot-features.mjs` → `feat-dim.png`             | dim behavior changes |
| `carry.png`          | Carry-unfinished-tasks buttons on a journal        | `shot-features.mjs` → `feat-carry.png` (clipped) | carry UI/buttons change |
| `query.png`          | Query results + visual query-builder chip bar      | `shot-features.mjs` → `feat-query.png`           | query rendering or the builder bar changes |
| `sheets.png`         | Sheets grid/table/board composite                  | `shot-sheets.mjs` → `shot-sheets.png`            | sheet schema table, formula columns/filter chip, tag board, grid/table/board rendering, or controls change |
| _(probe only)_       | Grid hover-`+` edge affordances + board Group-by toolbar | `shot-chunk2.mjs` → `/tmp/shot-chunk2-{grid,board}.png` | grid edge-grow affordances or board group-by picker change (verification probe, not a curated README image) |
| `quick-capture.png`  | Quick-capture mini-window with slash menu open     | `shot-capture.mjs` → `rm-quick-capture.png`      | capture window, slash menu, or editor-parity changes |
| `pdf.png`            | PDF pane + text highlight + area (image) highlight | `shot-tabs-pdf.mjs` → `feat-pdf.png`             | PDF viewer, highlight rendering (text/area), or pane layout changes |
| `settings.png`       | Settings modal (shortcuts shown)                   | `shot-settings.mjs` → `settings.png`             | Settings modal gains/loses controls (**watch mode, first-day-of-week**, themes, snapshots) |
| `calc.png`           | Live `/calc` block (inputs, results, a variable)   | `shot-stills.mjs` → `feat-calc.png`              | calc rendering (line numbers, result column) changes |
| `callouts.png`       | Colored note / warning / tip callouts              | `shot-stills.mjs` → `feat-callouts.png`          | callout colors/title/body styling changes |
| `waveform.png`       | Audio waveform overlay player (decoded waveform)   | `shot-media.mjs` → `audio-overlay.png`           | audio overlay / waveform rendering changes (shot synthesizes a real WAV so the waveform draws) |

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

These shipped without a screenshot — add one if the site would benefit: the
sidebar namespace tree, advanced (datalog) query results. (Tabs, dim, carry,
queries+builder, text+area PDF highlights, `/calc` blocks, and callouts are now
illustrated — calc + callouts via `scripts/shot-stills.mjs`.)
