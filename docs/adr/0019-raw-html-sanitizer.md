# 0019. Raw HTML renders live, through a sanitizer, in both render surfaces

- **Status:** Accepted
- **Date:** 2026-07-05

## Context

Logseq renders raw inline/block HTML embedded in a Markdown/Org block live —
`<ins>`, `<sup>`/`<sub>`, `<kbd>`, `<mark>`, `<div class="note">…`, a self-closed
`<img/>`, and so on. lsdoc already parses these faithfully into `inline_html` /
`raw_html` nodes carrying the source bytes verbatim (byte-parity with mldoc — see
[0005](0005-lsdoc-separate-parser-crate.md)/[0015](0015-lsdoc-wire-contract.md)).
Until now Tine rendered only a sandboxed-`https` `<iframe>` subset and showed every
other raw-HTML node as escaped text, in both the app and the static-HTML export.
That's the GH #16 gap ("`<img>` displays in Logseq but not Tine").

The blocker was never parsing — it's **safety**. Notes are not self-authored:
they arrive via Syncthing sync, import (the #16 reporter imported from Joplin),
paste, and shared/downloaded graphs and templates. In Tauri an injected
`onerror=`/`<script>` can call Tine's IPC to read or write the whole graph, or
exfiltrate via `fetch`; and the export **re-publishes** raw HTML as served content
on tine.page. So rendering raw HTML live requires sanitizing it first.

Two facts shaped the design:

- **Sanitizing is a render-layer policy, not a parse.** mldoc doesn't sanitize
  (it emits `Raw_Html` verbatim), and lsdoc must not either, or it breaks oracle
  parity. Different render targets could also want different policies. So the gate
  belongs at each render boundary, not in the parser — i.e. in Tine, not lsdoc.
- **Tine has two render surfaces in two languages:** the SolidJS app (`renderRawHtml`
  in `src/render/inline.tsx`) and the Rust static-HTML export (`publish.rs`). Each
  needs the sanitizer in its own language, and the two must not drift — otherwise
  the published page and the app disagree about what's safe (the same "two
  renderers silently diverge" trap as the block-facet renderers).

## Decision

We will render raw HTML **live in both surfaces, gated by a single shared
allowlist**, using the standard sanitizer for each language:

- **App:** DOMPurify (`src/render/htmlSanitize.ts`), rendered via `innerHTML`.
- **Export:** `ammonia` (`crates/tine-core/src/html_sanitize.rs`), emitted verbatim
  (already safe).

The allowlist (tags + per-tag attributes) is **mirrored by hand** in the two
modules and **contract-tested** by a shared fixture set,
`fixtures/html-sanitize-cases.json`, run by both `htmlSanitize.test.tsx`
(DOMPurify) and `html_sanitize.rs`'s `contract_fixtures` (ammonia), asserting the
two enforce the same policy. All event handlers, `style`, `<script>`, `<iframe>`,
forms, and `javascript:` URIs are stripped; `href`/`src` are limited to safe
schemes. The app keeps its pre-existing sandboxed-`https` `<iframe>` fast-path
*above* the allowlist (a deliberate embed feature); the export does not render raw
`<iframe>` (it has `{{video}}` for that) — the one documented, narrow asymmetry.

lsdoc is untouched.

## Consequences

- The GH #16 family lands as one allowlist, not just `<img>`: `<ins>`, `<sup>`,
  `<sub>`, `<kbd>`, `<mark>`, `<u>`, `<abbr>`, `<a>`, `<img>`, small containers, etc.
- **Parity caveat surfaced while grounding this against the oracle:** mldoc only
  classifies a **self-closed** `<img/>` (and paired tags) as raw HTML; a **bare
  `<img src="…">`** is `Plain` in mldoc — so **Logseq renders it literally too**.
  That's parity, not a Tine gap, and it means the #16 reporter's bare-`<img>` Joplin
  export is likely literal in Logseq as well.
- **Second parity fact from the oracle:** mldoc only starts inline HTML at a
  **word boundary** — `<sub>2</sub>` alone or ` <sub>2</sub> ` renders, but a tag
  glued to a letter (`H<sub>2</sub>O`, `mc<sup>2</sup>`) is `Plain`, i.e. literal in
  Logseq too. This is an lsdoc/mldoc parse fact, upstream of the sanitizer, but it
  governs *what* reaches it, so it's noted here.
- The `file://`/absolute-local-path limitation is handled by an **opt-in** rather
  than the asset protocol: Tauri's custom-protocol origin won't load an arbitrary
  `file://`/`F:\…` `src`, so instead the app reads those bytes over a **gated
  `read_local_image` IPC** (image extensions + size cap, OFF by default — Settings →
  "Load local-file images") and swaps in a blob URL, reusing the same
  blob mechanism as in-graph images. This is **app-only**; the HTML **export never
  serves local files** (ammonia strips the `file:`/absolute `src`), since a published
  page can't reach the author's disk anyway. The opt-in is a real permission —
  synced/imported notes aren't self-authored — so it defaults off.
- Two allowlists must be kept in lockstep by hand; the contract fixtures are the
  guard. Adding a tag/attr means editing both modules and adding a fixture.
- The export now serves sanitized user HTML (by design); the sanitizer, not the
  escape pass, is the safety boundary for the published site.

## Amendment (2026-11-24): Hiccup and colour styles

- **Hiccup goes through the same sanitizer in the export.** lsdoc transcribes a
  Hiccup vector (`[:div {:style …} …]`) to HTML; the export
  (`body_html` in `crates/tine-graph-features/src/render.rs`) turns that HTML into
  a raw-HTML node before rendering, so published pages, print and PDF pass it
  through `html_sanitize::sanitize` like typed HTML. lsdoc's own Hiccup checks
  still decide which vectors transcribe at all, but they are no longer the
  export's safety boundary. A vector lsdoc refuses stays escaped text.
- **`style` keeps colour declarations, in both surfaces, by one rule.**
  `color_only_style` in `crates/tine-core/src/html_sanitize.rs` decides which
  declarations survive: the property must be in the policy file's
  `browser.styleProperties`, the value one closed colour literal. ammonia applies
  it in export; the app calls the same function through the wasm parser module,
  once per sanitized fragment for all its `style` values (`color_only_styles`;
  `src/render/htmlSanitize.ts` has no rule of its own). It gives the answers
  the app's former JavaScript rule gave: the same splitting, ECMAScript trimming
  (U+FEFF is whitespace, U+0085 is not), `toLowerCase` on the property, and
  ASCII-only case folding in the colour patterns. A differential test in
  `src/render/htmlSanitize.test.tsx` runs a copy of the old rule against the wasm
  rule over generated declarations. Everything else in `style`
  (positioning, sizing, `url()`, `var()`, `!important`) is dropped, so a note
  cannot overlay or hide a published page.
- **One link rule in export.** `safe_export_url` (in `html_sanitize`) decides
  which destinations a note-authored link may navigate to: Markdown links in
  `decorate`, and every `href` that `sanitize` keeps on a raw-HTML or Hiccup
  anchor. A link that fails it keeps its text and loses its destination, so a
  `//host`, backslash, blank or control-character destination is inert however it
  was written (raw-HTML `//host` links kept their destination before this
  amendment). lsdoc's Hiccup attribute checks only decide whether a vector
  transcribes at all. `img` and media `src` keep the sanitizer's scheme policy.
- So an export shows what the app shows: a Hiccup or raw-HTML colour survives,
  and positioning, forms, `id`s and event handlers do not.

## Amendment (2026-11-26): Logseq's highlight colours

- **`background-color` also keeps one variable form:**
  `var(--ls-highlight-color-<name>, #<hex>)`, where `<name>` is one of Logseq's
  seven built-in colours (`browser.highlightColors` in the policy file, OG
  `frontend/ui.cljs:59-66`) and the fallback is a hex colour literal. This is the
  markup the selection toolbar's colour row writes; Logseq keeps the `style`
  (DOMPurify 2.4.0, `frontend/security.cljs:5-11`) and resolves the variable
  (`packages/ui/src/vars-classic.css:19-25`). The same `color_only_style` decides
  it in both surfaces. Any other variable, a variable on another property, a
  variable without a hex fallback, and a nested function are dropped, as before.
  Tine defines the seven variables for light and dark in `src/styles/ls-shim.css`;
  the export, which has no such variables, draws the fallback.
