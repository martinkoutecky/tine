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

_Nothing actively in flight._

---

## P1 — do next (high value, bounded scope)

_Nothing currently queued — the previous three entries (table column-alignment,
user-defined `:macros`, easy embed macros) were already implemented and shipped;
removed as stale. Next candidates are in P2._

---

## P2 — backlog (real, lower urgency)

| Item | Notes |
|---|---|
| **Click→caret placement in marked-up blocks** | Clicking a block that renders markup currently drops the caret at end-of-block; needs the renderer to map a click back to the source offset (parser support for this now exists). Daily-visible papercut — bump to P1 if it nags. |
| **Plugin CSS-variable alias shim** | Alias OG `--ls-*` CSS variables so the Awesome-Styler theme family "mostly works". A themes-compat slice, **not** full plugin support (that's WONTFIX). ~1–2 days. |
| **Datalog query coverage expansion** | A scoped subset of advanced Datalog queries works today (unsupported clauses are flagged, not silently dropped); expand coverage on demand. |
| **Quick-capture browser extension (Firefox)** | Snap a page/selection/link into today's journal or an inbox page via a capture template. Prefer a watched drop-file (no open port; rides the existing file-watch + journal-append). |
| **Performance audit / refactor** | A focused pass on the remaining perf levers, measurement-first: block **DOM windowing** on very large pages (unmount off-screen blocks — gated on measuring a big page first) plus internal cache refactors. Perf is why Tine exists, so this gets its own deliberate slot rather than piecemeal fixes. |
| **Slash-menu autocomplete UX** ([#15](https://github.com/martinkoutecky/tine/issues/15)) | Two OG-parity/discoverability gaps in the `/` autocomplete: (1) a **visible scroll affordance** — WebKitGTK hides scrollbars, so overflow items below the 280px fold look absent (affects every menu: slash, block-ref, page); (2) an **OG-style `/template` picker/submenu** — reporter expects Logseq's dedicated template chooser. Backend discovery + listing are verified working; this is the UX layer. Blocked on a repro for the "sees only one template" part. |

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
