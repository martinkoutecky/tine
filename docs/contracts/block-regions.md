# Block regions and structural edits

`crates/tine-core/src/block_regions.rs` owns source structure for one raw
Markdown or Org block. Native code and `crates/lsdoc-wasm` compile the same
implementation. lsdoc supplies ownership; it follows mldoc's grammar, including
Org source blocks on the Markdown path and mldoc's fence-closing behavior.

`parse(raw, is_org)` derives `BlockRegions` from the shared single-block parser.
`from_blocks` reuses its AST. Coordinates are half-open raw UTF-8 byte ranges;
the frontend uses `render/spans.ts` for UTF-16 conversion. Literal ownership
includes nested containers and inline code/verbatim. Property entries, planning
timestamps, drawer rows and identity come only from accepted parser regions.
Sub-token splitting is confined to those regions. A planning timestamp at the
start of a line may have a glued body suffix; edits retain that suffix, detaching
it onto a body line when replacing the timestamp. Mid-prose timestamps remain
body content.

`BlockRegions::apply` sets/removes properties and planning, strips copy metadata,
projects visible body, normalizes planning, inserts drawer rows and closes a
clock row. It uses regions for exactly the supplied raw and format. Clock value
interpretation lives in `logbook.rs`; the actual replacement goes through the
region door. New Org drawers follow title, accepted planning, drawer, body;
creating a drawer hoists accepted planning even when authored below body text.
Literal planning lookalikes remain in place. Markdown built-in identity/collapse
metadata keeps trailer placement. Debug builds reparse edits and compare the
header, unrelated metadata/drawers/planning and literal source slices.

Raw identity, logbook and repeater APIs require a format. Published anchors,
block-reference targets and referrer links thread the document block's format.
Main initializes the frontend parser before rendering. Capture can paint its
seeded empty editor during initialization; Block defers durable identity and
reference-count reads until ready. Synchronous structural calls require init.
The bounded AST cache retains regions from the same parse bundle. Parser
traps quarantine the block as literal; structural edits refuse quarantine.

Editor literal decisions are the parser's, asked through this door and never recomputed from
content (I-12). Typography adds no parse to ordinary input handling: a parse (`inlineLiteral.ts`,
cached per text) runs only once a replacement trigger such as `->` has completed, to ask whether the
range is literal, and counts a span still being typed (an unclosed backtick or Org `~`/`=`) as literal
by asking lsdoc about the text with the closer spliced in. The same answer gates list continuation
(`blockParts.listLineAt` via `literalBlockOfLine`), the checkbox toggle, pasted-text block
classification (`pastedPlainBlocks`) and live reference counting at paste; `$$` display-math state
(`fences.ts`) skips literal `$$`, and the block-reference and tab labels read property lines and
inline markup through `splitProps` and the inline AST. One memoized property split supplies editor
value and commit. The pre-existing facet renderer still makes one cold parse per changed raw: with
a 2,000-block page loaded and the edited Block mounted, 200 typed characters produce 200 cold parses
in prose and code, with zero additional cold parses from typing itself. This probe does not mount the
complete virtualized Page.

Fence and property recognition are done: code containers come from `literal_blocks`/`open_fence`
(`fences.ts`), property lines and hidden-property split/reattach from `properties` (`splitProps`).
Retained outside this door, pending native doors: the Org drawer extents used when a page-property
removal empties a drawer (`editor/properties.ts` `pagePartsWithProperty`), the outline/bullet grammar of
pasted and copied outlines (`outline.ts`, `clipboard.ts` `outlineToHtml` cannot see a fence that
follows a bullet marker), and the leading-ATX/list-marker grammar of a bare list-prefix line
(`format.ts` `trimBlockTrailingSpace`). The empty-card separator fix is included. Copy projection
preserves a final raw newline, including the seven manager-approved corpus exceptions.

Unit cost: no new persisted record, index or transport. Edits are bounded to one
block; cached optimized edits need zero ownership parses, raw entry points need
one, and changed debug edits add a preservation reparse. No page/graph parse.
Publication's format-bearing raw identity lookups currently parse once per call;
they do not reuse the already-parsed document block's regions.

Guards: `block_region_ratchet.rs` and `blockRegions.guard.test.ts` ratchet existing
structural recognizers outside the door. Native/wasm parity shares an 800-fixture
matrix; `block_regions.rs`, `region_edit_regressions.rs` and
`editor/regionUI.test.tsx` exercise edits, byte preservation and typing costs.
