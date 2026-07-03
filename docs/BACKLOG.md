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
| **Live editing, cheap tier: facet chips** (heading-sized editor DONE) | *(Promoted from P2 at Martin's request.)* **Heading-sized editor text SHIPPED (Jul 3 2026)** — editing a single-line heading now keeps heading size/weight (OG's `.uniline-block` rule), verified vs OG source. Remaining: the **beyond-OG** half — render the TODO/priority/SCHEDULED/DEADLINE chips live beside the textarea from the facets Tine already parses per keystroke (~40 µs), read-only, must not alter source. OG shows zero live preview while editing (confirmed from source), so this is a Tine addition — keep it unobtrusive; flag the design to Martin. Full live preview (Obsidian-style in-text): the CodeMirror 6 route, still Deferred. |
| **Parser crash-hardening: guarded parseBlock + fresh-instance recovery** | The vendored-bump half is DONE: **lsdoc v0.4.0 is vendored** (Jul 3 2026) — a perf release that makes the raw-HTML tag index and the `>`-quote fallback reparse linear (the `O(n²)` families found in the Jul 2 whole-codebase audit), on top of the earlier hand-verified v0.2.5 crash/quadratic families (resync stack-overflow at 4KB, `>`-spine OOB at 2KB, permanent instance poisoning after any trap) which parse in single-digit ms, ~linear scaling to 32KB, no traps. Remaining, pending Martin's go: (1) the defense-in-depth Tine-side guard — `parseBlock` (`src/render/parse.ts`) still has no per-call try/catch, so an *unknown* future trap would still poison the instance and kill all rendering until restart; catch per-call traps, re-init a fresh wasm instance and retry once, quarantine the offending block (render raw + subtle marker) if it traps again; (2) keep the crash repro as a test. |

---

## P2 — backlog (real, lower urgency)

| Item | Notes |
|---|---|
| **Plugin CSS-variable alias shim** | Alias OG `--ls-*` CSS variables so the Awesome-Styler theme family "mostly works". A themes-compat slice, **not** full plugin support (that's WONTFIX). ~1–2 days. |
| **Datalog query coverage expansion** | A scoped subset of advanced Datalog queries works today (unsupported clauses are flagged, not silently dropped); expand coverage on demand. |
| **Quick-capture browser extension (Firefox)** | Snap a page/selection/link into today's journal or an inbox page via a capture template. Prefer a watched drop-file (no open port; rides the existing file-watch + journal-append). |
| **Performance: remaining levers** | The July 2026 whole-codebase audit (perf × data-safety × architecture, two independent auditors per dimension) closed the found eager/superlinear paths (graph-open foreground scans, bulk icon lookup, batch block-ref resolution, advanced-query memoization). Remaining, measurement-first: block **DOM windowing** on very large pages (unmount off-screen blocks — gated on measuring a big page first), and flipping the CI bench job from advisory to gating once shared-runner noise is calibrated. |
| **Rendered-copy fidelity: resolve refs / macros / math** | *Deepen the Copy/Export "Rendered" mode + drag-select-of-rendered-text* (`renderedText.ts`), which today flattens the parsed block but can't resolve **graph state**, so three constructs fall back to their source form: **block refs** `((uuid))` → the bare uuid (should be the referenced block's rendered text), **user/provider macros** `{{…}}` → the literal macro call (should be the expansion you see), **math** `$…$`/`$$…$$` → the TeX source. Block refs + macros are the real ask: thread the SAME resolution the on-screen renderer already uses (`BlockRefView` / `UserMacroView`) into `renderedText.ts` — one resolver, no second parser. **Math is genuinely lossy as plain text** (a typeset fraction/integral has no faithful text form): decide per-construct between TeX source (today), KaTeX text-content, or a Unicode approximation — likely keep TeX for display math and only "render" trivial inline math. Playground: open **`Rendered text demo`** in the tine-test graph, compare on-screen vs *Copy / export as… → Rendered*. |
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
