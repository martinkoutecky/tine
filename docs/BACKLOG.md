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

_Empty — the Jul 3 2026 batch cleared it (heading-sized editor `7090da1`, block-level
`:macros` `48e602b`, parser crash-guard `af09a14`). The "live facet chip" idea resolved to
the CodeMirror 6 live-preview route — see Deferred; Martin chose to keep it deferred._

---

## P2 — backlog (real, lower urgency)

| Item | Notes |
|---|---|
| **Rendered-copy fidelity — still open (math + residual gaps)** | The Jul 3 2026 pass (`f1c5721`) made *Copy / export → Rendered* resolve block refs + user macros. **Still open:** (1) **Math renders as TeX, not typeset/text** — `$…$`/`$$…$$` copy the TeX source. A faithful plain-text form is genuinely lossy (a typeset fraction/integral has none), so this is a per-construct call: keep TeX for display math, but maybe a Unicode approximation or KaTeX text-content for trivial inline math (`E=mc²`). Martin wants this tracked as open, not closed. (2) **Best-effort ref cache** — a `((uuid))` only resolves if it was already rendered on screen in this graph epoch (`resolvedBlockRefSync`); exporting an off-screen subtree, or a ref never rendered, falls back to the bare uuid. Making the export path *await* the async resolve (instead of a sync cache read) would close it. (3) **Built-in/provider macros stay literal** in copy (`{{video}}`/`{{embed}}`/`{{query}}` …) since they're interactive widgets on screen; some have a sensible text form (`{{video url}}` → the url) if wanted. (4) A block ref copies only the referenced block's **first visible line** (matches on-screen `BlockRefView`); resolving a multi-line/block-level target more fully is open. |
| **Plugin CSS-variable alias shim** | Alias OG `--ls-*` CSS variables so the Awesome-Styler theme family "mostly works". A themes-compat slice, **not** full plugin support (that's WONTFIX). ~1–2 days. |
| **Datalog query coverage expansion** | A scoped subset of advanced Datalog queries works today (unsupported clauses are flagged, not silently dropped); expand coverage on demand. |
| **Quick-capture browser extension (Firefox)** | Snap a page/selection/link into today's journal or an inbox page via a capture template. Prefer a watched drop-file (no open port; rides the existing file-watch + journal-append). |
| **Performance: remaining levers** | The July 2026 whole-codebase audit (perf × data-safety × architecture, two independent auditors per dimension) closed the found eager/superlinear paths (graph-open foreground scans, bulk icon lookup, batch block-ref resolution, advanced-query memoization). Remaining, measurement-first: block **DOM windowing** on very large pages (unmount off-screen blocks — gated on measuring a big page first), and flipping the CI bench job from advisory to gating once shared-runner noise is calibrated. |
| **Theming: semantic-token sweep → prebuilt themes** | Tine is already on ~30 semantic CSS tokens with working custom.css injection; a ½–1 d sweep of the ~140 remaining hardcoded colors (plus defining the referenced-but-never-set `--accent`, and moving theme persistence off localStorage, which doesn't survive restarts in WebKitGTK) makes prebuilt themes nearly free (~1 d for 2–3, incl. an OG-Classic port). Feeds directly into the existing `--ls-*` alias-shim item below. |
| **Slash-menu autocomplete UX** ([#15](https://github.com/martinkoutecky/tine/issues/15)) | Two OG-parity/discoverability gaps in the `/` autocomplete: (1) a **visible scroll affordance** — WebKitGTK hides scrollbars, so overflow items below the 280px fold look absent (affects every menu: slash, block-ref, page); (2) an **OG-style `/template` picker/submenu** — reporter expects Logseq's dedicated template chooser. Backend discovery + listing are verified working; this is the UX layer. Blocked on a repro for the "sees only one template" part. |
| **Raw-HTML rendering via a sanitizer (`<img>` etc.)** ([#16](https://github.com/martinkoutecky/tine/issues/16)) | Markdown should support inline/block raw HTML (OG does), but Tine deliberately renders only a sandboxed-https `<iframe>` today and shows all other raw HTML as text (`renderRawHtml`/`InlineHtmlInline` in `inline.tsx`; lsdoc already *parses* it as `inline_html`/`raw_html`, so parsing is NOT the blocker). **The blocker is safety, and it applies even to a local app:** synced/imported/pasted notes aren't self-authored, and in Tauri an injected `onerror=`/`<script>` can call Tine's IPC to read/write the whole graph or exfiltrate via `fetch`; a remote `<img src>` is a tracking pixel; and the static-HTML **export re-publishes** raw HTML as served content. So the real work is a **sanitizer** (vendor DOMPurify): allowlist tags + per-tag attributes, restrict `src`/`href` schemes to `https:` + Tine's asset protocol, strip event handlers — and it MUST cover the export path too. Bounded slice: enable `<img>` first (`alt`/`width`/`height`/`title`, safe `src` only). **`file://` caveat (surfaces every time #16 comes up):** it's not a WebKitGTK quirk — Tauri serves the app from a locked-down origin (custom protocol + CSP), inherited by **WebView2 on Windows too**, so a bare `<img src="F:\…">`/`file://` won't load; OG dodges this only because it's Electron (loads from `file://`, relaxed `webSecurity`). Tine already shows local images that live **inside the graph** (routed through its asset protocol), so after `<img>` lands: **https images work; arbitrary absolute local paths (like the reporter's `F:\…Joplin…`) still won't** unless the file is moved into the graph or we add an explicit "allow notes to load any local file" opt-in (a real permission decision, not a silent default). Net: the sanitizer is worth doing (md-should-support-html); the specific reporter's Joplin-resource path is only partially helpable. |

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
| **Live-preview editor (CodeMirror 6)** | The only honest route to Obsidian-style in-text liveness (widget decorations, marker hiding, real bold while typing): ONE CM6 instance for the single edited block, decorations fed from lsdoc's AST + inline spans — no second parser, ~76–116 KB gz. **This is what Martin's "live facet chip" idea actually needs** (the marker shows as raw text while the caret is in/adjacent to it, and becomes a clickable chip once the caret leaves = CM6 widget-decoration + reveal-source-under-cursor). **The old blocker is cleared:** lsdoc now ships inline spans (v0.4.0). Remaining cost: multi-week, and it reopens the just-stabilized caret/focus cluster (ADR 0013, editorController, the e2e caret harness is textarea-hard-wired). A cheaper narrowing — a scoped CM6 spike on JUST the task marker — still pays most of the CM6 *integration* cost (the expensive part is adopting CM6 + caret parity, not the per-construct decoration). Decision pending Martin. The transparent-textarea overlay is NOT an alternative for chips (see WONTFIX — glyph metrics must stay byte-identical). |
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
