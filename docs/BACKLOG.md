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

| Item | Notes |
|---|---|
| **1A — Table alignment: one parser (lsdoc owns it)** | Column alignment is currently parsed *outside* lsdoc — the app re-derives it by re-scanning raw text (`body.tsx` `tableAligns`/`parseAligns`/`isTableSep`/`splitRow`) and the export emits `data-align` from a separate source. Two/three parsers of one grammar → violates "one parser per parsed thing". Fix: **lsdoc carries per-column alignment in the Table AST**; Tine reads it and deletes the re-scan. Cross-repo: lsdoc spec at `subagent-tasks/lsdoc-table-alignment-spec.md` (check-first, then add the field), then Tine integration (bump lsdoc tag, rebuild wasm, add `aligns` to the TS mirror, delete the re-derivation, screenshot-verify). Sequenced after the in-flight lsdoc rewrite. |
| **1B — User `:macros` OG-parity gaps** | `{{name a, b}}` → `$1..$N` substitution works (`inline.tsx` `UserMacroView`; recursion-guarded; re-renders as markdown). Two known gaps to verify against OG's *actual* `:macros` semantics and fill: (i) block-level template output currently degrades to inline (documented limitation, `inline.tsx:726`); (ii) whether OG supports more than positional `$N` (e.g. `$@`/all-args, named args). **Study OG source first** — do not infer semantics from Tine's code. |

---

## P2 — backlog (real, lower urgency)

| Item | Notes |
|---|---|
| **Click→caret placement in marked-up blocks** | Clicking a block that renders markup currently drops the caret at end-of-block; needs the renderer to map a click back to the source offset (parser support for this now exists). Daily-visible papercut — bump to P1 if it nags. |
| **Plugin CSS-variable alias shim** | Alias OG `--ls-*` CSS variables so the Awesome-Styler theme family "mostly works". A themes-compat slice, **not** full plugin support (that's WONTFIX). ~1–2 days. |
| **Datalog query coverage expansion** | A scoped subset of advanced Datalog queries works today (unsupported clauses are flagged, not silently dropped); expand coverage on demand. |
| **Quick-capture browser extension (Firefox)** | Snap a page/selection/link into today's journal or an inbox page via a capture template. Prefer a watched drop-file (no open port; rides the existing file-watch + journal-append). |
| **Performance: remaining levers** | The July 2026 whole-codebase audit (perf × data-safety × architecture, two independent auditors per dimension) closed the found eager/superlinear paths (graph-open foreground scans, bulk icon lookup, batch block-ref resolution, advanced-query memoization). Remaining, measurement-first: block **DOM windowing** on very large pages (unmount off-screen blocks — gated on measuring a big page first), and flipping the CI bench job from advisory to gating once shared-runner noise is calibrated. |
| **Editor focus/caret structural hardening** | The one place the July 2026 audit found a real regression cluster: caret/focus across duplicate rendered instances of one block. The ownership rules are now written down (ADR 0013); the remaining structural step is extracting an `editorController` as the sole writer of the caret/focus signals, plus keeping the duplicate-instance invariant scenario in the e2e harness green. |
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
| **`src-tauri/main.rs` module split** | The least-cohesive large file (watcher, backup/restore, settings, spellcheck, IPC plumbing in one 2k-line file). Split into modules *after* the write/watch protocol ADR (0012) so the split preserves the protocol instead of hiding it. |
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
