# Block regions and structural edits

`crates/tine-outline/src/block_regions.rs` owns source structure for one raw
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
A bare marker title (`TODO` with nothing after the marker) is a task when
only property, planning or LOGBOOK rows follow it, or a LOGBOOK follows
anywhere: the shared parse boundary (`lsdoc_block_parse::restore_bare_task`)
decides it, so adding or removing those rows never changes the header. Blank
lines beside the rows do not count, and a CRLF title ends at its CR, so a CRLF
bare task reads as its LF twin. Body text after a bare title stays text, as in
Logseq (REG-BARE-TASK-HIDDEN-ROWS).

Raw identity, logbook and repeater APIs require a format. Published anchors,
block-reference targets and referrer links thread the document block's format.
Visible-body labels and export projections also require that source format;
unloaded pages use the existing inventory's physical owner and the same
`tine-outline::page_format::Format` rule as native code, without another read.
Markdown ATX label stripping applies only to Markdown. Rename planning and
validation classify file extensions through `Format::from_path`, including
mixed-case Org extensions. Clock edits take previous and next raw buffers at
the shared logbook boundary; previous marker recognition retries the title
line when a bare marker is followed by metadata (GH #741, L3-07).
Main initializes the frontend parser before rendering. Capture can paint its
seeded empty editor during initialization; Block defers durable identity and
reference-count reads until ready. Synchronous structural calls require init.
The bounded AST cache retains regions from the same parse bundle. Parser
traps quarantine the block as literal; structural edits refuse quarantine.

Backspace-at-start and Delete-at-end share `outline::merge` in Markdown and Org,
on routed pages and journals (F2, D-20261026-F2). The survivor keeps its hidden
properties and clocks; absorbed hidden properties and clocks are dropped.
An absorbed durable ID transfers with `IdTransferred` only when referenced and
the survivor has no ID. An unreferenced absorbed ID is dropped. The frontend
uses the existing graph reference-count index, including block refs and embeds;
an unavailable or cold count conservatively treats an absorbed ID as referenced.
When both blocks have IDs and the absorbed one is referenced, merge refuses
`referenced_identity` without mutation, keeping both targets resolvable. OG
`delete-block!` / `delete-concat` at `6e7afa8eb` instead replace the survivor UUID
and `update-block-refs` rewrites its graph-wide referrers; Tine's page-local merge
cannot perform that rewrite. Clock dropping follows OG's `move-to-prev-block`;
OG's unreferenced forward Delete can retain absorbed raw metadata and clocks,
an intentional difference to keep Tine's two entry methods consistent.

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

The classic block editor keeps code wrappers editable (GH #879). Block entry outside a
code payload, including sibling merge and arrow navigation, opens raw source.
Entering a payload opens its reversible body view; Escape, boundary arrows or
Backspace at its start return to raw source without rewriting the block. Empty
code payloads retain the existing Backspace deletion gesture. Raw whole-block
editing stays raw as the caret moves through the payload. Both modes use
visible-source caret coordinates for history and the existing guarded save path.
Markdown and Org, desktop and touch layouts share this boundary; calculator
blocks retain their specialized editor. OG reference: `og-1.0.0` at
`680b371f0f53968adf678759e9bfd5cb7ea5790c`, `block-content-or-editor`,
`move-to-prev-block` and `extensions/code.cljs` `render!` (Escape).
The experimental CodeMirror editor retains its existing standalone body-first
entry policy in this merge. Escape reveals its source; the other standalone
source-entry paths remain a follow-up.

Fence and property recognition are done: code containers come from `literal_blocks`/`open_fence`
(`fences.ts`), property lines and hidden-property split/reattach from `properties` (`splitProps`).
Both editors ask `properties.ts` for Markdown value positions through the existing
accepted-property-line parser; only then does autocomplete split comma-separated
query spans. Typing and pasting (including typing inside an existing value) share
this answer, with the existing parser-owned fence exclusion; moving the caret alone
never opens value suggestions, as in OG. A value session reads only that
key through `CompletionValues(key)`, without requiring a name session; pending
read subscribers refresh their current query when the answer arrives. Failed
reads cache nothing. Intentional difference from OG `6e7afa8eb`
`handler/editor.cljs:2211-2226`: OG starts values only after choosing a property
name. Org drawers retain OG's absence of a value trigger.
Retained outside this door, pending native doors: the Org drawer extents used when a page-property
removal empties a drawer (`editor/properties.ts` `pagePartsWithProperty`), the outline/bullet grammar of
pasted and copied outlines (`outline.ts`, `clipboard.ts` `outlineToHtml` cannot see a fence that
follows a bullet marker), and the leading-ATX/list-marker grammar of a bare list-prefix line
(`format.ts` `trimBlockTrailingSpace`). The empty-card separator fix is included. Copy projection
preserves a final raw newline, including the seven manager-approved corpus exceptions.

Display-math Enter state excludes parser-owned trailing whitespace when deciding
whether a closing delimiter is interior to a literal range (#862). Completed
math therefore permits the next outline block even when prose follows it.
The approved #278 difference from OG remains: Enter inside open display math
inserts a newline. Escape from a projected code body saves through the existing
editor commit, then reveals editable raw source at its end for the rest of that
editing session (#865, OG `extensions/code.cljs` `render!`). The next Escape
uses ordinary raw-editor block selection. Both Markdown and Org use this owner.

Unit cost: no new persisted record, index or transport. Edits are bounded to one
block; cached optimized edits need zero ownership parses, raw entry points need
one, and changed debug edits add a preservation reparse. No page/graph parse.
Publication's format-bearing raw identity lookups currently parse once per call;
they do not reuse the already-parsed document block's regions.

Guards: `block_region_ratchet.rs` and `blockRegions.guard.test.ts` ratchet existing
structural recognizers outside the door. Native/wasm parity shares an 800-fixture
matrix; `block_regions.rs`, `region_edit_regressions.rs` and
`editor/regionUI.test.tsx` exercise edits, byte preservation and typing costs.
