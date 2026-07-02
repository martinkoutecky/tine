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
| **1A — Table alignment: one parser (lsdoc owns it)** | Column alignment is currently parsed *outside* lsdoc — the app re-derives it by re-scanning raw text (`body.tsx` `tableAligns`/`parseAligns`/`isTableSep`/`splitRow`) and the export emits `data-align` from a separate source. Two/three parsers of one grammar → violates "one parser per parsed thing". Fix: **lsdoc carries per-column alignment in the Table AST**; Tine reads it and deletes the re-scan. Cross-repo: **lsdoc side DONE (Jul 2 2026, lsdoc `7ab1649`)** — `Block::Table.aligns` is always-serialized `("left"\|"center"\|"right"\|null)[]`, one entry per dropped md separator cell; `[]` = no dropped separator (incl. org tables); `render_html` emits `data-align` only for center/right; contract documented in lsdoc `AST.md`. Remaining = the Tine integration (bump lsdoc tag past `7ab1649`, rebuild wasm, add `aligns` to the TS mirror, delete the re-derivation, screenshot-verify app + export). |
| **1B — User `:macros` OG-parity gaps** | `{{name a, b}}` → `$1..$N` substitution works (`inline.tsx` `UserMacroView`; recursion-guarded; re-renders as markdown). Two known gaps to verify against OG's *actual* `:macros` semantics and fill: (i) block-level template output currently degrades to inline (documented limitation, `inline.tsx:726`); (ii) whether OG supports more than positional `$N` (e.g. `$@`/all-args, named args). **Study OG source first** — do not infer semantics from Tine's code. |
| **Parser crash-hardening: guarded parseBlock + fresh-instance recovery + vendored lsdoc bump** | Verified by hand (Jul 2 2026): the vendored lsdoc v0.2.5 wasm is quadratic on several adversarial content families (10ms at 249 *bytes*, 34ms at 1KB on the `resync` family) and hard-crashes on others (`>`-spine OOB at 2KB, stack exhaustion at 4KB) — and after the FIRST trap the wasm instance is permanently poisoned: every subsequent parse of any content throws, killing all block rendering until app restart. `parseBlock` (`src/render/parse.ts`) has no try/catch — only *init* failure is handled. All families are fixed at lsdoc HEAD but no tagged/vendored release contains the fixes. Fix, in order: (1) Tine-side guard now — catch per-call traps, re-init a fresh wasm instance and retry once, quarantine the offending block (render raw + subtle marker) if it traps again, keep the app alive; (2) vendor a fixed lsdoc build as soon as the lsdoc rewrite cuts a tag (coordinate — this also gates any live-editing feature); (3) keep the crash repro in a test. |
| **Right-click Delete for journals + sidebar rows** ([#17](https://github.com/martinkoutecky/tine/issues/17)) | OG deletes journal pages exactly like normal pages (no special-casing; right-click title → page menu; trash-backed via `.recycle`). Tine already has the universal right-click PageMenu and the trash-backed delete flow — Delete/Rename are just gated to `pageKind === "page"` (`ContextMenu.tsx`), and Sidebar rows have no context menu at all. Scope: un-gate Delete for journals (OG parity), add the context menu to sidebar rows (beyond-OG, explicitly requested), unfavorite/un-recent on delete (entries currently dangle), and fix the confirm wording ("cannot be undone" is false — it goes to `.tine-trash`). ~1–2 h, no backend work. |

---

## P2 — backlog (real, lower urgency)

| Item | Notes |
|---|---|
| **Click→caret placement in marked-up blocks** | Clicking a block that renders markup currently drops the caret at end-of-block; needs the renderer to map a click back to the source offset (parser support for this now exists). Daily-visible papercut — bump to P1 if it nags. |
| **Plugin CSS-variable alias shim** | Alias OG `--ls-*` CSS variables so the Awesome-Styler theme family "mostly works". A themes-compat slice, **not** full plugin support (that's WONTFIX). ~1–2 days. |
| **Datalog query coverage expansion** | A scoped subset of advanced Datalog queries works today (unsupported clauses are flagged, not silently dropped); expand coverage on demand. |
| **Quick-capture browser extension (Firefox)** | Snap a page/selection/link into today's journal or an inbox page via a capture template. Prefer a watched drop-file (no open port; rides the existing file-watch + journal-append). |
| **Performance: remaining levers** | The July 2026 whole-codebase audit (perf × data-safety × architecture, two independent auditors per dimension) closed the found eager/superlinear paths (graph-open foreground scans, bulk icon lookup, batch block-ref resolution, advanced-query memoization). Remaining, measurement-first: block **DOM windowing** on very large pages (unmount off-screen blocks — gated on measuring a big page first), and flipping the CI bench job from advisory to gating once shared-runner noise is calibrated. |
| **Live editing, cheap tier: facet chips + heading-sized editor** | OG itself edits in a plain textarea with zero live preview (confirmed from source), so in-editor liveness is beyond-OG. Tine already parses each block per keystroke via lsdoc-WASM for facets (~40 µs typical), so no new parser is needed. Cheap tier: keep the TODO/priority/SCHEDULED chips rendered live beside the textarea from those facets, and adopt OG's heading-sized editor text (an actual OG-parity gap — headings shrink to body size while editing in Tine). Days of work, zero caret risk. Full live preview: see Deferred (CodeMirror 6). |
| **Theming: semantic-token sweep → prebuilt themes** | Tine is already on ~30 semantic CSS tokens with working custom.css injection; a ½–1 d sweep of the ~140 remaining hardcoded colors (plus defining the referenced-but-never-set `--accent`, and moving theme persistence off localStorage, which doesn't survive restarts in WebKitGTK) makes prebuilt themes nearly free (~1 d for 2–3, incl. an OG-Classic port). Feeds directly into the existing `--ls-*` alias-shim item below. |
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
| **Live-preview editor (CodeMirror 6)** | The only honest route to Obsidian-style in-text liveness (widget decorations, marker hiding, real bold while typing): ONE CM6 instance for the single edited block, decorations fed from lsdoc's AST + inline spans — no second parser, ~76–116 KB gz. Multi-week; reopens the just-stabilized caret/focus cluster (ADR 0013, editorController, the e2e caret harness is textarea-hard-wired); blocked on a spans-bearing lsdoc release regardless. Revisit after the lsdoc restructure + spans land, and only if the cheap tier (P2) still leaves in-text liveness wanted. |
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
| **Overlay/backdrop "live highlight" editor mirror** | The transparent-textarea-over-rendered-mirror trick can only honestly deliver metric-safe color changes — no chips, no marker hiding, no bold (glyph metrics must stay byte-identical to the textarea) — in exchange for a permanent font-metric-sync liability on WebKitGTK. The CodeMirror 6 route (Deferred) strictly dominates it. |
