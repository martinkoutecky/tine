# Tine — Backlog & Roadmap

The public front-door for not-yet-done Tine work: what's next, what's deferred, and what's
explicitly out of scope. (Detailed internal engineering/data-safety debt is tracked separately
and privately; **lsdoc**, the parser, has its own backlog in its own repo.)

**Categories are deliberate and distinct:**
- **In flight** — being built right now.
- **P1 / P2** — will do; P1 next, P2 when P1 clears.
- **Deferred** — genuinely might do later, but no slot yet. *Deferred is NOT WONTFIX.*
- **WONTFIX** — decided we are not doing it, with a reason.

Kept current in the same chunk of work: add items when they surface, remove them when done,
move them between categories in place.

---

## In flight (being built now)

| Item | Where |
|---|---|
| **[#14](https://github.com/martinkoutecky/tine/issues/14) — Backspace can't delete an empty top node.** Fallback in the `start===0` Backspace branch: visually-empty + childless + a next block exists → delete it, caret to next. | `src/components/Block.tsx` |
| **[#13](https://github.com/martinkoutecky/tine/issues/13) — paste multiline into a code block makes bullets.** Guard the `onPaste` multiline→outline branch: inside a fenced/calc block, insert raw text at the caret. | `src/components/Block.tsx` |
| **[#15](https://github.com/martinkoutecky/tine/issues/15) — templates: finds 1 of 4, can't pick.** `templates()` only sees cached pages → whole-graph discovery; confirm slash-list is scrollable. | `crates/tine-core/src/query.rs` |

---

## P1 — do next (high value, bounded scope)

| Item | Notes |
|---|---|
| **Table column-alignment render** | The parser already carries column alignment; the renderer + HTML export don't emit it yet. Small, pure OG-parity gap. |
| **User-defined `:macros` text substitution** | Honor the graph's `:macros` config map at render time — highest user-facing payoff in the macro cluster. Medium effort. |
| **Easy embed macros** — twitter, vimeo, bilibili, `img` | Quick OG-parity wins, self-contained. |

---

## P2 — backlog (real, lower urgency)

| Item | Notes |
|---|---|
| **Click→caret placement in marked-up blocks** | Clicking a block that renders markup currently drops the caret at end-of-block; needs the renderer to map a click back to the source offset (parser support for this now exists). Daily-visible papercut — bump to P1 if it nags. |
| **Plugin CSS-variable alias shim** | Alias OG `--ls-*` CSS variables so the Awesome-Styler theme family "mostly works". A themes-compat slice, **not** full plugin support (that's WONTFIX). ~1–2 days. |
| **Datalog query coverage expansion** | A scoped subset of advanced Datalog queries works today (unsupported clauses are flagged, not silently dropped); expand coverage on demand. |
| **Quick-capture browser extension (Firefox)** | Snap a page/selection/link into today's journal or an inbox page via a capture template. Prefer a watched drop-file (no open port; rides the existing file-watch + journal-append). |
| **Performance audit / refactor** | A focused pass on the remaining perf levers, measurement-first: block **DOM windowing** on very large pages (unmount off-screen blocks — gated on measuring a big page first) plus internal cache refactors. Perf is why Tine exists, so this gets its own deliberate slot rather than piecemeal fixes. |

---

## Deferred — genuinely later, no slot yet (NOT WONTFIX)

| Item | Notes |
|---|---|
| **Android app (Tauri v2 mobile)** | **Under evaluation — promising.** A focused ~1–2 week project, not "free" but not blocked: Logseq's own Android app proves the "operate directly on a synced folder of files" model works (via all-files storage access + real filesystem paths), which is exactly Tine's file model — so the Rust core ports almost unchanged, and Android's Chromium WebView is an *upgrade* over Linux's WebKitGTK. Main costs: a small Android folder-picker plugin, a mobile UI/UX polish pass, and gating the desktop-only bits. Cheapest first step: `tauri android init` + run the existing UI in the emulator. |
| **Graph view** | The visual graph of page links. Deferred (later). |
| **Interactive graph view in HTML export** | The static-site export has sidebar + search but no interactive graph yet. |
| **macOS notarization** | The macOS build is unsigned, so Mac users hit the "unidentified developer" wall (right-click→Open works around it). Signing needs an Apple Developer ID ($99/yr) — exploring a shared/borrowed ID or revisiting later. **Deferred, explicitly not WONTFIX.** |
| **Verso/Servo engine swap** | A longer-term answer to WebKitGTK's scroll/render gaps. Servo's web-compat for a dense editor isn't ready yet; revisit ~early 2027. |
| **Nested-grid ("breadth") views** | A TreeSheets-inspired grid over child blocks — a post-1.0 exploration. |
| **Donations / sponsor link** | A single low-friction Ko-fi/Liberapay link + `FUNDING.yml`. Parked — low priority. |
| **Feature video clips for the website** | Looping clips of the motion features (PDF, audio waveform, video, tabs). Parked — headless capture is too heavy for now. |

---

## WONTFIX — not doing (with reason)

### By-design non-goals
| Item | Reason |
|---|---|
| **Whiteboards** | A separate application domain; Tine is a fast local-first outliner. |
| **Flashcards / SRS** | Needs a dedicated spaced-repetition review engine; out of scope for an outliner. |
| **Full plugin system (`@logseq/libs`)** | Months of work (a datascript engine + Logseq's render model); Tine coexists with Logseq instead. |
| **Built-in git** | Delegate to your own sync tool (Syncthing, etc.). |

### Subsystem / feature calls
| Item | Reason |
|---|---|
| **`function` macro** | Needs a Clojure (SCI) evaluator plus query-result plumbing. Large, rarely used. |
| **`cloze` / `cards` macros** | Only meaningful inside the SRS review loop → same reason as flashcards. |
| **Zotero connector macros** | Niche; needs Zotero data-dir + item-metadata resolution Tine doesn't have. |
| **CEF / Chromium engine swap (desktop)** | Evaluated and ruled out — too heavy (a multi-process Chromium bundle per OS + a JS-compat shim). Staying on WebKitGTK; Verso is the live long-term bet (Deferred). |
| **YouTube-timestamp seeking** | OG-faithful seeking needs the YouTube player API; not worth it. (A plain clickable timestamp is the most that would be considered.) |
