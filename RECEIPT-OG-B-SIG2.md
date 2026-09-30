# OG-B-SIG2 receipt

Base: a87fa4f0e; branch og-b-sig2.
Invariants in play: I-25, I-12, I-13/I-15, E cost invariants.
Unit cost: no persisted record, format, edit kind or file change. The existing save/event transport gains a bounded derived-answer signal; ordinary successful save bytes increase from 27 to 94 while two graph-wide follow-up reads disappear.

Step 0: checkpoint-4 defect repair, not a master feature port (master delta accounting not applicable). Existing answers are Store snapshot publication, RenderGraph query execution, publish_query current-page substitution, and doc::parse_with_opts. No X-class machinery is needed. The prior lane installed the parser API but none of the three findings is closed on this base. Scope: bounded native save/watcher signals consumed by the two frontend answer caches; owning-page execution/cache identity for static queries and sheets; one old-source/layout parse at the three authorized model sites. No new user controls or workflow: Guide exception for cost optimization and correction of existing query semantics.

Trust boundary: honest external changes, sync delivery, malformed imported content, disk errors and crashes; existing guarded transactions remain the sole write path.

## Scope clarifications and necessity evidence

The contemplated public Change field/SURFACE addition was replaced before implementation by serializing private deltas through the existing Change type. SURFACE.txt, SHALLOW.txt and numeric surface budgets are unchanged. No approval is needed for that resolved question.

Write-set support: store/snapshot.rs and store/answer_changes.rs produce the result/notification delta at the existing native publication point. model.rs exposes the existing referenced-name contribution helper crate-internally (visibility only), avoiding a second name answerer. backendTypes.ts carries the external notification wire. No B-FAIL2 discovery or non-save command implementation was changed.

Prior regression patches are installed as real tests: pageIndex economy assertions now expect no text-revision refresh; current-page regression lives in query_owner_context.rs and compares static and baked rows across different owners; saveAnswerCost.test.ts runs the actual setRaw/flushPage path. The IPC probe awaits the real debounced dataRev bump; a zero-delay-only probe proved insufficient and was corrected before recording necessity.

The prior query fixture's one-row/unwanted-row assertion assumed a block does not reference its owning page. The existing core includes that owning-page relationship; both static and baked Second therefore include its own TODO plus Third's TODO. The new test asserts identical baked/static membership, excludes Third on Public, and requires Third on Second. TODO rows avoid recursively rendering query hosts. This retains the owner/cache contract without changing core query semantics. Advanced inline macro syntax is parser-limited at a closing curly brace, so the advanced list fixture remains the prior accepted syntax; sheet coverage uses the parser-recognized current-page template macro. No new content scanner was introduced.

Necessity logs in /tmp/og-b-sig2-evidence: inventory-before.log exit 1 (extra inventory call); ipc-before.log exit 1 (2 extra calls, 326694/1646694 bytes at 2k/10k); query-before.log exit 101 (no owning-page results); context-necessity.log exit 101 (all three final owner/list/sheet/template regressions fail against the base query files); parse-necessity.log exit 101 (ratchet 5 sees 6). Candidate parse measurement is 4, so final ratchet is 4.


## Results and family coverage

| Finding | Reproduced / pass-after | Shared answerer and sibling sites | Prevention |
| --- | --- | --- | --- |
| L10:59 / L15:80 | Before: ordinary `setRaw`/`flushPage` rereads both graph-wide answers. After: zero extra answer IPC on 2k and 10k pages. Native save-wire necessity against base fails because the signal is absent. | `Snapshot::answer_changes` compares changed parsed-page contributions through the existing native block-reference/name helpers. Save transaction results, single-save adapter, create/save/alias-save/group-save consumers and watcher single/bulk/signal-only notifications all carry/apply the same publication signal. | `saveAnswerCost.test.ts`, `graphAnswers.test.ts`, native `answer_changes.rs`, save-wire cost test, watcher transport test, and the I-12/I-25 source guard in `ogBSig2.guard.test.ts`. |
| L04:38 | Before: all three final owner-context tests fail against base query production. After: static list results and laid-out query sheets agree with baked publication rows for distinct owners; template substitution also agrees. | `RenderGraph::query_parsed`/`query_bounded`, all three static cache dialect keys, `render_sheets::query_rows`/sheet inputs/emit, page print/site contexts and `publish_query` use current-page context and the same substitution helper in `render_query_cache.rs`. | `query_owner_context.rs` and I-12 substitution/cache source guard. |
| L05:30 | Before: six parses per ordinary save; ratchet five fails. After: four on 1/60-block pages at 20/2000 pages; final ratchet four. | Three authorized model sites use `parse_doc_with_opts`, delegating Markdown document/layout discovery to existing `doc::parse_with_opts`. Org remains on the existing Org parser/default options path. Redundant `detect_serialize_opts` is retired. | Existing `i13_edit_cost` counter gate tightened from six to four, plus the I-15 model source guard. |

Catalog rows `REG-OG-SIG2-SAVE-ANSWERS`, `REG-OG-SIG2-QUERY-OWNER` and `REG-OG-SIG2-SAVE-PARSE` are covered. This closes the assigned checkpoint findings; this repair does not settle additional master-port ledger families. Three additive CHANGELOG entries describe the behavior. No Guide controls/workflow changed.

Create, rename and delete use real Store save/move/trash transactions in `answer_changes.rs`: create emits target count 1 plus inventory invalidation, rename retains the total and invalidates names, delete emits count 0 plus inventory invalidation. External single alias/reference changes and a 40-page bulk create/reference-removal scan produce final native target counts. Watcher tests prove one signal per publication, including own delete/rename publications with no page events; frontend tests prove those single, bulk and empty-bulk signals reach both caches. Per-target revisions reject older updates independently, graph binding rejects stale events/save callbacks, and a late initial count read preserves already received deltas. B-FAIL2 inventory unreadable reporting remains intact.

Frontend reference counts are fetched once per graph and patched in place; applying a delta does not clone the graph-sized map. Name invalidation compares source contributions conservatively: a contribution change may request an inventory refresh even if another page retains the same referenced name. Ordinary text edits still emit false/empty. The notification channel has one explicitly bounded app-lifetime count-cache subscriber, avoiding a document/cache import cycle and an unowned listener Set.

Existing tests keep their user-visible assertions. Page-index economy assertions adopt the requested signal contract. The reference-name inventory test now proves a generic content revision causes no read and the native inventory signal changes the answer. The reference-count refresh test now proves native save deltas update the badge with no second read; durable-ID resolution uses an initial graph-epoch read. The failed count-read test still requires a sticky failure report and preserved native last-good counts, with failure injected into the initial read instead of an obsolete text-edit refetch. No test was deleted, skipped or restricted with `.only`; pre-existing suite skips remain.

## Unit cost and measurement limits

| Pages | Before: save IPC / bytes | Before: extra answer IPC / bytes | Before: total IPC / bytes | After: save IPC / bytes | After: extra answer IPC / bytes | After: total IPC / bytes |
| --- | --- | --- | --- | --- | --- | --- |
| 2,000 | 1 / 27 | 2 / 326,694 | 3 / 326,721 | 1 / 94 | 0 / 0 | 1 / 94 |
| 10,000 | 1 / 27 | 2 / 1,646,694 | 3 / 1,646,721 | 1 / 94 | 0 / 0 | 1 / 94 |

The single save response is measured by the real native `save_wire` boundary against synthetic 2k/10k Store graphs (`native-signal-necessity.log`, `native-cost.log`). Its signal has graph revision `2`, false inventory invalidation, and no count targets. The two eliminated responses are byte counts of the installed prior-lane frontend fixture payloads, reached through the real `setRaw`/`flushPage` entry point with backend mocks (`ipc-before.log`, `signals-focused-final.log`, `npm-final-4.log`). Totals combine those independently measured layers; this is not a live Tauri end-to-end transport trace. Revision-string length can change absolute response bytes, but unrelated graph size cannot.

Native save counter comparison: parses 6 -> 4; old-source parses remain 1; full reads remain 4; fsyncs remain 2; files written remain 1; bytes written remain 8 for one block and 539 for 60 blocks; readdir, corpus materialization and full snapshot rebuilds remain 0. No persisted-record growth or new dependency. No global wall-time/RSS/Windows/release-size comparison is claimed by this lane; deterministic operation counts and existing full-suite budgets are the evidence.

## Scope and size

Support within the authorized family: store publication modules produce the save/notification delta at the existing snapshot answer point; no discovery/error-path implementation or non-save command implementation is modified. `backendTypes.ts` expresses the external wire. `src/graphAnswers.ts` provides the shared bounded frontend consumer channel. Renderer test contexts now explicitly use `None` where no owner exists. Existing transaction/save tests only adapt destructuring to the richer result, retaining their assertions.

Raw line accounting against `a87fa4f0e` at implementation commit `9879938`: production-file additions/deletions 469/202 (net +267); test-file additions/deletions 481/29 (net +452); changelog/catalog 8/1 (net +7). These are git numstat file categories; inline Rust tests remain counted in their production files. Test net growth is 1.69x production net growth, below 3x. Master delta is not applicable to this assigned og defect repair.

| Production file | Net raw lines |
| --- | ---: |
| graph-features/publish_query.rs | -24 |
| graph-features/render.rs | -45 |
| graph-features/render_query_cache.rs | +101 |
| graph-features/render_sheets.rs | +1 |
| store/model.rs | 0 |
| store/store.rs | -24 |
| store/store/answer_changes.rs (new) | +89 |
| store/store/save_failure.rs | +4 |
| store/store/snapshot.rs | +62 |
| store/transaction.rs | +5 |
| tauri/commands/save_wire.rs | +6 |
| tauri/watcher.rs | +35 |
| src/backend.ts | +11 |
| src/backendTypes.ts | +1 |
| src/blockRefCounts.ts | +19 |
| src/document/external.ts | +4 |
| src/document/save/engine.ts | +2 |
| src/graphAnswers.ts (new) | +22 |
| src/pageIndex.ts | -2 |

Oversized production files shrink or stay flat; store publication helpers and query runners move to their existing focused modules. No SURFACE/SHALLOW/size baseline/allow-list ratchet was raised. Existing public native type `Change` serializes private signal data; no new native public operation/question/type is introduced. New files are below 1,500 lines. No content-structure regex, line scanner or frontend metadata parser was added.

## Gates and retained evidence

Linux synthetic fixtures plus the permitted anonymized corpus; all shell commands use RTK. Logs are retained privately under `/tmp/og-b-sig2-evidence/`; no corpus bytes are committed.

| Gate | Exit | Evidence |
| --- | ---: | --- |
| Focused native save/transaction/signal/surface tests | 0 | `store-focused.log` |
| Focused query publication/sheets/owner tests | 0 | `query-focused.log` |
| Focused frontend signals/failure/inventory/cost tests | 0 | `signals-focused-final.log` |
| Native save response cost | 0 | `native-cost.log`, actual 94-byte responses at 2k/10k |
| Parse cost / tightened ratchet | 0 | `parse-final.log` |
| Watcher transport tests | 0 | `watcher-focused.log` |
| `timeout 300 npx tsc --noEmit` | 0 | `tsc-final-3.log` |
| `timeout 1500 npm test -- --maxWorkers=4` after formatting | 0 | `npm-final-4.log`: 308 files/2,401 tests passed (2 existing skips), then render config 264 files/2,007 tests passed (3 existing skips); no Errors line |
| `timeout 120 cargo fmt --all -- --check` | 0 | `fmt-final.log` |
| `CARGO_INCREMENTAL=0 timeout 3000 cargo test --workspace --tests --no-fail-fast` | 0 | `cargo-final-2.log`: 131 suites, 1,930 passed, 0 failed, 8 existing ignored |
| Regression catalog index/UI catalog check | 0 | `catalog-final.log` |
| G6 untouched open and roundtrip | 0 | `g6.log`: 1,075 pages, 0 unreadable, 0 untouched-byte diffs, 0 structural bugs; 999 acceptable canonicalizations / 76 byte-identical roundtrips |
| G6b one-block edit | 0 | `g6b.log`: frontend projection passes, 1,004 saved pages pass, 71 without a leaf block, 0 editable-page failures |
| `git diff --check` / staged diff check | 0 | implementation and receipt checked before commits |

G6/G6b run the repository scripts against `/home/koutecky/research/logseq-anonymized` with timeouts. Those scripts hardcode `--release`; to obey the lane's explicit no-release rule, a private cargo adapter strips only `--release` and invokes the real cargo with `CARGO_INCREMENTAL=0` and this worktree's debug target directory. Script bodies and corpus comparisons are unchanged. Adapter retained at `/tmp/og-b-sig2-evidence/debug-bin/cargo`. No release build, deploy, push, integration, hosted-platform gate or manager shape/invariant sweep is claimed. Last disk check: 214 GB free, above the 150 GB limit.

Failed intermediate gates remain in evidence: first full Rust compile needed three owner-less renderer test fixtures initialized; first full frontend run exposed the document/count-cache import cycle and obsolete content-refetch triggers; next run exposed the unowned observer Set; browser run exposed the remaining legacy count-refresh trigger. These were corrected and both final complete suites pass. Necessity runs restored only the selected base production files under try/finally and returned to the candidate; the committed branch contains no base swap.

## Commits and handoff

- `70ca14a71`: owning-page query execution/cache/sheets/shared substitution and combined old-source/layout parse, installed owner-context tests and tightened parse ratchet.
- `9879938`: native publication answer signals, save and watcher transport, shared frontend consumption, cost/path tests, family guard, covered catalog rows and signal changelog.
- Final commit: this receipt, force-added at the root as explicitly requested despite the shared batch default of an untracked receipt.

All assigned findings are implemented. No pending write-set repair, product question or approval remains. Working tree is expected clean after the receipt commit. Manager integration and release/runtime/platform gates remain manager work.
