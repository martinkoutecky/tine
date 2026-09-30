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
The frontend initializes the parser before rendering or synchronous structural
calls. Its bounded AST cache retains regions from the same parse bundle. Parser
traps quarantine the block as literal; structural edits refuse quarantine.

Typography uses the editor's existing code-body/calc surface, adding no parse
to input handling. One memoized property split supplies editor value and commit.
The pre-existing facet renderer still makes one cold parse per changed raw:
With a 2,000-block page loaded and the edited Block mounted, 200 typed characters
produce 200 cold parses in prose and code, with zero additional cold parses from
this door. This probe does not mount the complete virtualized Page.

Unclosed/CommonMark code-card recognition and hidden-property split/reattach
recognition remain follow-up work; their existing implementations are retained.
The empty-card separator fix is included. Copy projection preserves a final raw
newline, including the seven manager-approved corpus exceptions.

Unit cost: no new persisted record, index or transport. Edits are bounded to one
block; cached optimized edits need zero ownership parses, raw entry points need
one, and changed debug edits add a preservation reparse. No page/graph parse.

Guards: `block_region_ratchet.rs` and `blockRegions.guard.test.ts` ratchet existing
structural recognizers outside the door. Native/wasm parity shares an 800-fixture
matrix; `block_regions.rs`, `region_edit_regressions.rs` and
`editor/regionUI.test.tsx` exercise edits, byte preservation and typing costs.
