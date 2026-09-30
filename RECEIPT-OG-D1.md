# RECEIPT-OG-D1 — structural edit door

**Status: incomplete / blocked; not ready to integrate.** The manager's “partial fix is a failed fix” rule applies. The shared door and several fixes are committed, but #7, the typing-cost gate, minimum sibling migrations, and full green gates are outstanding. Existing tests were not deleted, skipped, weakened or made exclusive. No push, merge, deployment, app-data access, brain access, or edits to another worktree occurred.

Base: `1cdff07c4633ca002ded1ef7ecf0a9b96e0830c0`; branch/worktree: `og-d1`, `/aux/koutecky/logseq/tine-og-d1`. Implementation commits: `12f120589` (door/projection foundation), `2c3764ab5` (migrations, regressions, parity, ratchet). Final commit adds this receipt, the read-only corpus diagnostic, changelog and final guard refinement.

## Scope and evidence base

Read the canonical external working agreement, then `12-common.md`, `22-common.md`, `og-k-common.md`, `og-d1-task.md`, and PARITY-CAMPAIGN sections 1–3; consulted the door inventory. This is the manager's redesign, not a master feature transplant. Before implementation, og already shared native/wasm single-block parsing and logbook source, but property, literal, planning, ID and copy ownership had independent recognizers. No X-class persistence/index/readiness/durability machinery was introduced. No public ledger family is settled by this incomplete lane; a master LOC delta is not applicable to this new manager-specified interface.

Placement oracle: original Logseq 0.10.9 `src/main/frontend/util/property.cljs:226–312` (`insert-property`) in the local read-only OG source. Org insertion follows title → contiguous planning → own Properties drawer → body; Markdown insertion follows the title/planning head and preserves existing head-key order. Legacy trailing user properties relocate to the head; builtin ID/collapse/list-order metadata retains trailer placement. Protecting parser-owned literals is the named divergence from OG's raw planning/property scans.

## Door and class sites

`crates/tine-core/src/block_regions.rs` is compiled natively and included directly by `crates/lsdoc-wasm/src/lib.rs`. It derives raw UTF-8 coordinates from the shared single-block AST, undoing trim and synthetic bullet preparation. It recursively visits lists, quotes/customs, tables and inline children; merges every required parser-owned literal; supplies header, accepted property entries, standalone planning, drawer/CLOCK rows and primary-format ID. Property sub-token splitting handles folded Markdown/drawer/directive Properties nodes and protects unrelated folded directive entries.

Operations: set/remove property, batch annotation values, ID through property operations, set/remove planning, strip-copy metadata, visible-body projection, normalize planning and drawer-row insertion. Native debug/tests reparse changed output and compare literal slices and unrelated property/planning/drawer contents. Offsets may move; transport newline adjacent to removed metadata may disappear. Header-shape and complete edit-intent validation remain less comprehensive than the task's full requested shape contract. Clock-out rewrites a door-selected CLOCK row in shared logbook code, rather than being a dedicated door operation. These are limitations, not a claim that the full operation contract is complete.

Moved: R02 drawer/row/insertion ownership and summaries; R03 annotation block refresh (page preamble reader remains); R04/R05 literal masks; R06 Markdown ID lookup only; R07 keep-both copies; R14 template DTO copies; T05 block ID/property writes; block-property portion of T04; T07 schedule read/write; T08 planning normalization; T09 repeater ownership; T13 literal lines. T15 fixes only the empty-body join separator. T36 is not fixed. Remaining siblings are explicitly listed below.

Native annotation refresh, keep-both and templates reuse the original DocBlock projection. The frontend AST LRU retains the regions from the same parse bundle in a WeakMap, using existing byte→UTF-16 helpers for line masks. Warm region lookup performs no new wasm parse. Native parse traps quarantine the whole raw source; wasm traps use the existing reinstantiation/quarantine path. Edits refuse quarantine and pre-init access; schedule/property UI callers show the refusal before undo/dirty mutation. Other identity/repeater consumers and their old test harnesses still need initialization/refusal integration.

## Eight acceptance cases

Fail-before logs are under `/tmp/og-d1-*`; summarized here so the receipt does not depend on those temporary files. Initial Rust evidence used the old behavior sites with the door scaffolding present. Later historical-source checks replace only the named behavior site with base source, retain the new test, and restore current bytes in `finally`. They are behavior-before checks, not a clean full-base checkout/build.

| # | Reproduced / fail-before | After / evidence limit |
|---|---|---|
| 1 CLOCK/planning in literal | Yes; `clock_out_keeps_literal_drawer` failed (101): CLOCK inside fenced LOGBOOK acquired an end timestamp/duration. `/tmp/og-d1-before-core.log`. | Public core `clock_out_at`/`clock_in_at` regression passes; clock-in also preserves fenced planning. Actual Tauri invocation was not exercised. |
| 2 annotation refresh | Yes; public `merge_hls_page_for_format` changed fenced color/page lines (101), same before-core log. | Public annotation merge regression passes; accepted color changes to blue and literal color/page remain byte-identical. Tauri/PDF UI invocation not exercised. |
| 3 keep-both copy | Yes; old `without_id_line` removed literal ID and kept real ID (101), `/tmp/og-d1-before-copy.log`; public `merge_blocks` fail-before also exits 101 (`/tmp/og-d1-before-keep-command.log`): the copied real ID remains and the literal ID disappears. | Public `merge_blocks(... both ...)` passes and resulting copy has no canonical ID; literal ID survives. No live conflict dialog test. |
| 4 Org inline refs | Yes; `rename_refs_multi` changed both `~[[Old]]~` and `=[[Old]]=` (101), before-core log. | Public rename regression passes; only ordinary `[[Old]]` changes. Existing Markdown/Org-source expectation conflicts and remains red. |
| 5 template DTO | Yes; old template DTO stripped literal ID/template (101), `/tmp/og-d1-before-template.log`; real `Store::open`/template-query before check also fails (101, `/tmp/og-d1-before-template-query.log`): copied block loses the literal metadata. | Two tests pass: helper covers MD literals/Org metadata; actual `Store::open` + whole-graph template query preserves literal metadata. No frontend template insertion test. |
| 6 calendar planning | Yes; old reader selected a literal stamp and writer inserted into a leading fence (1), `/tmp/og-d1-before-ts.log`; real Block/DatePicker before check also fails (1, `/tmp/og-d1-before-calendar-ui.log`): Today changes the fenced date. | Actual Block + DatePicker Today UI and store-entry regression pass. Glued non-standalone planning expectation remains a contract question. |
| 7 code typography | Yes; actual Block editor with typography mode `type`, typing `>` after `echo -`, stores `echo →` instead of `echo ->`. Baseline UI exit 1. | **Still fails**, same result. The editing-surface fact is at Block.tsx, outside this lane's write set; no per-keystroke parse was added to typography. |
| 8 empty code card | Yes; helper and actual Block typing failed before (1): empty JavaScript fenced card lost the separator on committing `x`. `/tmp/og-d1-before-ts.log`, `/tmp/og-d1-ui-before.log`. | Helper and actual Block typing pass, storing opener/newline/`x`/newline/closer. Card projection's existing scanners remain pending. |

Native public regressions: `crates/tine-core/tests/region_edit_regressions.rs` (4). Additional door regressions (5) include multibyte round-trip, unrelated folded directive retention, adjacent duplicate deletion with LF/CRLF and legacy trailing placement. Query regression module: 2. UI acceptance/cost tests: `src/editor/regionUI.test.tsx` (5: #6/#8 pass; #7/two cost assertions fail).

## Change size

Base-relative tracked changes plus authored new code/tests/fixtures, excluding this receipt: **+5,227 / −809 lines**. Most added lines are the 800-fixture matrix; wasm base64 is a generated single line. This is not a feature-port comparison to master.

## Retired recognizers

Numbers below count deleted/replaced **old source lines within each named implementation**, using a line diff against the base; retained thin wrappers and value/date/duration sub-token parsing are not counted as retired ownership scanners. They are not an additive total because diff alignment can overlap nearby implementations.

| File:function | Deleted old lines |
|---|---:|
| `crates/tine-core/src/logbook.rs:insert_logbook_line` | 65 |
| `crates/tine-core/src/logbook.rs:leading_properties_end` | 25 |
| `crates/tine-core/src/logbook.rs:logbook_bounds` | 16 |
| `crates/tine-core/src/logbook.rs:is_drawer_start` | 7 |
| `crates/tine-core/src/logbook.rs:is_drawer_end` | 2 |
| `crates/tine-core/src/pdf.rs:block_property_key` | 14 |
| `crates/tine-core/src/pdf.rs:refresh_annotation` | 57 |
| `crates/tine-core/src/refs.rs:strip_list_bullet` | 8 |
| `crates/tine-core/src/refs.rs:code_ranges` | 44 |
| `crates/tine-core/src/refs.rs:inline_code_spans` | 33 |
| `crates/tine-core/src/refs.rs:org_block_ranges` | 35 |
| `crates/tine-core/src/refs.rs:code_ranges_for` | 19 |
| `crates/tine-core/src/refs.rs:block_id` | 4 |
| `crates/tine-core/src/sync_diff.rs:without_id_line` | 43 |
| `crates/tine-store/src/query.rs:template_dto` | 20 |
| `src/document/edits/identity.ts:existingBlockId` | 18 |
| `src/document/edits/identity.ts:rawWithBlockId` | 12 |
| `src/document/edits/identity.ts:orgRawWithProperty` | 31 |
| `src/document/edits/properties.ts:markdownRawWithProperty` | 28 |
| `src/document/edits/properties.ts:readSchedule` | 12 |
| `src/document/edits/properties.ts:setSchedule` | 34 |
| `src/editor/literalLines.ts:isLiteralBlock` | 2 |
| `src/editor/literalLines.ts:closedFences` | 10 |
| `src/editor/literalLines.ts:closedBeginBlocks` | 11 |
| `src/editor/planning.ts:normalizePlanning` | 41 |
| `src/editor/repeat.ts:hasRepeater` | 6 |
| `src/editor/repeat.ts:rollRepeat` | 9 |

Retired T13 fence tracker/BEGIN fallback constants as well. Test-only Markdown property-grammar helper remains for unchanged existing tests; it is not a production fallback. Code-card wrapper and hidden-property split scanners were **not** retired. Source may still contain value/token parsing that is confined to parser-accepted regions, such as logbook clock timestamps/durations and repeater date arithmetic.

## Door backlog / pending minimum siblings

Inventory IDs retain their original questions. Minimum migration gaps: R06 Org published ID lacks a format-bearing caller; R02 legacy `clock_out_at`/clock-summary APIs default to Markdown (format-bearing marker transition uses the correct cached regions); T02/T03/T25 hidden-property split/reattach and copy siblings remain; T15 card recognition/closer ownership remains; T36 code typography remains. Repeater public helpers also retain their default-MD interface. No scanner fallback was added to bridge these gaps.

| Inventory | Remaining question |
|---|---|
| R01 | Which property grammar admits block/page metadata? |
| R03 remainder | Which page-preamble fields belong to PDF metadata? |
| R05 remainder / R24 | Which exact ref/tag/property-value tokens represent a target or evidence occurrence? |
| R08 | Which conflict markers are authored outside literals? |
| R09 / R10 | Which page preamble/title/directive/property entries define page identity/metadata? |
| R11 / R12 / R13 | Which page icon, property backlink or alias entries belong to the page? |
| R15 / R16 | Which preamble fields survive page/journal/conflict merge? |
| R17 | Which page metadata authorizes publication? |
| R18 / R19 | Which first-root properties promote to page header and satisfy admission? |
| R20 | Which journal blocks are visible content? |
| R21 / R22 | Which nesting/preparation facts belong to outline admission and inline protection? |
| R23 | Which macro/query extent belongs to a block? |
| T01 / T02 / T03 | Which editable/page fields and hidden own properties split, reattach or mutate? |
| T04 remainder / T12 / T19 | Which page parts, list order, heading/checkbox and DTO header forms are edited/promoted? |
| T06 | Which ID-looking values must conservatively reserve collisions (broader than canonical identity)? |
| T10 / T11 | Which marker/priority prefix is edited? |
| T14 / T15 / T16 | Which unclosed fence/math/calc/card wrapper owns the editor payload and closer? |
| T17 / T18 | Which in-block list/checkbox/pasted outline form is being changed? |
| T20 | Which journal raw is authored content? |
| T21 / T22 / T23 | Which inline wrapper/autopair context/nearest link surrounds the caret? |
| T24 | Which canonical frontend ref target permits graph rename? |
| T25 / T26 / T27 | Which metadata/IDs survive clipboard/split/copy/fill/sheet restructure and field rename? |
| T28 / T29 | Which template variables or clipboard HTML/newline forms are authored? |
| T30 / T31 / T32 | Which visible body and labels export/display? |
| T33 / T34 / T35 | Which query/macro extent/options are owned? |
| T36 | Which editing surface allows smart typography? |
| T37 / T38 | Which media/image source span is edited? |
| T39 | Which published identity/permalink is canonical? |
| T40 / T41 | Which syntax is anonymized/minimized or recognized in the mock runtime? |
| T42 / T43 | Which favorites page DTO/header and list-prefix spaces survive round-trip? |

## Costs and guards

- Same single-block boundary; native cached edits do zero new ownership parses in optimized builds. Debug changed edits perform the required preservation reparse. Raw convenience operations cost O(one block bytes), with one initial ownership parse. No page/graph parse in the door. Native cached-operation counts were reasoned from call sites, not independently instrumented. Adversarial coordinate/sub-token lookup complexity was not profiled; the cost claim is bounded to this block, not a demonstrated strict linear-time bound.
- 200 input events with a 2,000-block graph loaded, rendering the target Block (not a full virtualized Page mount): **before prose** 200 calls/0 hits/200 misses; **before code** 1,000 calls/800 hits/200 misses; **after both** 1,000 calls/800 hits/200 misses. Misses count actual cold wasm parses. Both zero-additional-parse requirements fail. Literal-line helpers add warm cache reads on prose; they do not improve the existing one cold parse per character. The necessary editing/facet lifecycle is in Block.tsx/facets.ts, outside the write set.
- Wasm: base **415,740 bytes**, final **454,255 bytes**, delta **+38,515 bytes** (under +40,000/+40KiB). Vendored optimized artifact regenerated by the canonical script. Native gates used debug only; the canonical wasm script internally builds its optimized wasm release profile.
- Native desktop binary size delta: **not measured**; no comparable base/candidate desktop binaries built. This required acceptance metric remains open. No desktop deployment or native/WebKit E2E evidence is claimed.
- Native/wasm parity: the same **800** fixtures produce identical BlockRegions. Matrix includes both formats, metadata shapes in first/last/alone positions, all required parser literal variants, nested quote/custom, non-ASCII and multibyte spans, CRLF, empty and unclosed bodies. Parity tests alone do not establish a corrected CommonMark editor contract.
- Rust/TS source ratchet shares an after-lane baseline of **33 recognizer lines in 19 production files**; each file may only fall. Excludes tests/examples/generated wasm and the door. Failure states “I-12: ask lsdoc through the block-region door; no regex over content structure” and names the door. Mutation checks detect direct ID/BEGIN tests and a planning regex. This is a lexical line recognizer, not a complete semantic scanner: indirect constants and multi-line patterns can escape it.

## Anonymized graph

Only the approved `/home/koutecky/research/logseq-anonymized` graph was read, through temporary copies. Corpus diagnostic outputs ordinals/counts/reasons, never graph text. 1,075 files / 14,694 blocks; zero panics; zero differences from the historical canonical visible projection. Copy comparison uses the old template raw-line filter as its oracle because DocBlock has no canonical strip-copy projection. Seven copy differences are listed below after final diagnostic classification. Their old/accepted removed-metadata counts are all zero; they are transport-line differences, not evidence of a literal-metadata fix. Accepting them requires a named preservation exception to the task's literal-fix-only comparison rule.

All seven are exactly the final transport newline retained by the door but dropped by the old line iterator. File/block ordinals (sorted corpus traversal) are: **38/4, 160/10, 203/13, 203/14, 203/15, 232/1, 1026/17**. None removed metadata under either implementation. These differences are byte preservation, but the prescribed comparison permits literal fixes only, so this acceptance question remains open.

## Gates

All gates have timeouts; native commands source `scripts/env.sh` and set `CARGO_INCREMENTAL=0`. G6/G6b run their exact scripts on copies with an exported cargo shim that strips `--release`, preserving the debug-only native build rule; scripts were not modified. G6b uses this worktree's existing target directory. Free disk remained >=150GB (192GB at start; 157GB at final checks).

| Gate | Exit / result |
|---|---|
| `cargo fmt --all -- --check` + wasm manifest fmt check | 0 |
| `npx tsc --noEmit` | 0 |
| `npm run build:wasm` | 0, final vendored artifact |
| `npm run build` (pin/oracle checks + Vite) | 0 |
| `npm run check:ui-catalog` | 0, 262 entries / 157 issues |
| Focused native door/public edit tests | 0: 5 door, 4 public edit tests |
| Native template-query module | 0: 2 tests |
| Rust ratchet | 0: 2 tests |
| TS ratchet/parity/edit + ogEnforcement | 0: 11 tests |
| `cargo test --workspace --tests --no-fail-fast` | **101**, one failing target/test: `tine-core --lib`, `refs::tests::rename_skips_refs_inside_org_begin_blocks`; 579 core lib tests pass, other targets pass |
| `npm test -- --maxWorkers=4` after fmt | **1**, 11 files/41 tests fail; 278 files/2,243 tests pass; 2 pre-existing skips; **4 unhandled errors** |
| Separate `npm run test:render -- --maxWorkers=4` | **1**, 6 files/9 tests fail; 249 files/1,956 tests pass; 3 pre-existing skips; **1 unhandled error** |
| G6 `og-g6-untouched.sh` | 0: 1,075 pages, zero unreadable/open byte diffs; zero structural bugs; 999 canonicalized / 76 byte-identical |
| G6b `og-g6b-one-block-edit.sh` | 0: 1,004 pass / 71 no-leaf / zero editable-page failures; report `/tmp/og-d1-g6b-final.tsv` |
| Read-only region corpus | 0 execution, zero visible diffs; seven copy-line differences still require acceptance decision |
| Desktop binary delta / deploy / three consecutive native E2E passes | Not performed; incomplete lane, no deployment claimed |

Final full-gate logs: `/tmp/og-d1-workspace-final.log`, `/tmp/og-d1-npm-post-fmt.log`, `/tmp/og-d1-render-final.log`, `/tmp/og-d1-tsc.log`, `/tmp/og-d1-build-final.log`, `/tmp/og-d1-guard-final.log`, `/tmp/og-d1-ratchet-final.log`, `/tmp/og-d1-g6-final.log`, `/tmp/og-d1-g6b-final.log`, `/tmp/og-d1-corpus-final.log`.

## Blocking questions / requested next ownership

1. Which parser contract wins? lsdoc accepts Org BEGIN blocks even on the Markdown path, and shorter/mismatched Markdown fence closers; existing Rust rename and TS normalization/caret tests require other behavior. Changing lsdoc or retaining structural scanner fallbacks is forbidden. No existing test was weakened. An asynchronous parser-contract clarification was sent; no answer has been received.
2. The task only owns standalone accepted planning lines, while `store.test.ts` requires detaching a glued planning timestamp/body suffix. Is that accepted timestamp sub-token editing in scope, or a revised parser contract?
3. Permit the owner of `src/components/Block.tsx` and `src/render/facets.ts` to finish code-surface typography, compute structural facts once per edit session, and remove the existing per-character facet parses. Typography.ts alone cannot receive the missing surface fact from the current caller.
4. Establish parser initialization for the synchronous identity/repeater/hidden-property test harnesses and render suites. Production bootstrap already awaits init; old unit harnesses do not. The parser crash mock targets old `parse_block_json`, while the render cache now consumes `parse_block_bundle_json`; updating that existing out-of-set mock is pending. Refusing pre-init access is not replaced by a scanner fallback or sync wasm compilation (engine synchronous size limit).
5. How should unclosed/CommonMark code-card editing facts be exposed by lsdoc? T15 cannot retire its wrapper scanners using the current accepted spans while retaining existing editor expectations. T02/T03/T25 migration needs the editing-session lifecycle above; those implementations remain at base.
6. Add format-bearing calls for R06 published identity and the legacy logbook/repeater APIs; their callers extend beyond this write set.
7. Approve or reject the seven terminal-line copy differences explicitly; they preserve existing raw bytes instead of the old `.lines().join` normalization and are not literal fixes.
8. Complete native binary measurement, complete shape-intent verification/door clock-out operation, command/UI coverage through the actual desktop entry points, and final green/native E2E gates before treating OG-D1 as done.
