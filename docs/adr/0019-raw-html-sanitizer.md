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
- The `file://` limitation is unchanged and orthogonal: Tauri's custom-protocol
  origin (WebView2 too) won't load arbitrary absolute local paths, so remote
  `https` images work but a bare `file://`/`F:\…` `src` still needs the file inside
  the graph or an explicit opt-in permission.
- Two allowlists must be kept in lockstep by hand; the contract fixtures are the
  guard. Adding a tag/attr means editing both modules and adding a fixture.
- The export now serves sanitized user HTML (by design); the sanitizer, not the
  escape pass, is the safety boundary for the published site.
