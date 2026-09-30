# OG-B-STORE receipt

Date: 2026-09-30 UTC. Base: `og 0ce583ff5`; branch `og-b-store`. Invariants: I-4, I-12,
I-25 and the campaign E cost rules. No new persisted record or format.

## Step 0: scope and boundaries

This is checkpoint-4 defect repair, not a master feature port: master delta
and inventory feature-port accounting are not applicable. The current store,
EDN parser, parser-owned block regions and accepted ADR 0056 are the authorities.
Existing config setters already use the root-map selector. Reads must use that
same selector; every EDN string reader must use the EDN decoder. Sidecar rewrites
must retain numeric types. Journal proposals must share one path constructor.
Directory moves must flush each affected directory once.

All writes retain the existing guarded transaction/device atomic-write doors.
Trust boundary: malformed imported data, honest external-editor/sync races,
disk errors and crash/power loss; no arbitrary-account attacker hardening.
No X-class storage/index/managed machinery, dependencies, new edit kinds or
formats are introduced. No new user control/workflow: Guide exception is
defect repair and internal cost work.

Write-set exclusions: rollback and config-read functions in store model.rs,
and discovery-error surfacing in page_identity.rs belong to B-FAIL. Other
files outside the named write set are read-only; required changes there must
be recorded pending. No app-data or private brain access, pushing, integration,
release builds or deployment from this lane.

## Findings and proof

**Handoff status: review required, not ready to integrate.** Five repairs are
closed locally. The sixth (Float preservation) is implemented and passes its
new regression, but conflicts with two existing byte-exact PDF assertions.
Six findings need files outside this lane's write set. Three allegations are
already satisfied by the base. No existing assertion, fixture, allow-list or
ratchet was changed to obtain a pass.

The committed proof directory is [tests/og-b-store-evidence](tests/og-b-store-evidence).
It contains synthetic-fixture failure output, final focused results, paired
benchmark samples, ledger results and anonymized-graph gate summaries. The
temporary exploratory test was removed after collecting its observations.

| Row | Reproduced / disposition | Before → after, family sites and guard |
|---|---|---|
| L01:43 | Yes; repaired | Base `Config::parse` reports `Shadow` rather than root `Real`; real `Store::favorites` also fails. All ordinary string/keyword/bool/int/vector/set/shortcut/macro readers, the search-accent flag and direct nested settings now share `root_keyword` with the setters. Nested maps use the same direct-map selector. `og_b_store_integrity` guards against depth-blind reader calls; `og_b_store_config` saves favorites, reloads and checks the nested bytes survive. [before](tests/og-b-store-evidence/config-before.txt), [after](tests/og-b-store-evidence/final-focused.txt). |
| L01:44 | Yes; repaired | Base keeps `\\u0041` literally and selects an escaped directory literally. Every config string reader, including hidden paths, shortcuts, macros and nested strings, calls `read_string_at` → `edn::parse_strict`; the duplicate partial decoders were removed. The regression covers Unicode, surrogate pairs, octal and control escapes; the source guard prohibits the old decoders. [before](tests/og-b-store-evidence/integrity-before.txt), [after](tests/og-b-store-evidence/final-focused.txt). |
| L01:45 | Yes; pending outside write set | Saving a valid Org `#+BEGIN_SRC text` block containing marker examples returns `SaveOutcome::ReadOnly` for unresolved VCS markers. `concord_queue` is writable, but its callers do not carry the format needed to ask the parser correctly. The class spans store transaction preflight, conflict inventory/resolution and graph-feature conflict clients. Requires an explicit format-aware structural door and caller propagation outside this write set; no content-based format guess or new literal scanner was added. [observation](tests/og-b-store-evidence/probes.txt). |
| L01:46 | Yes; candidate implemented, integration question Q1 | Both real PDF rewrite doors (`pdf::write_highlights`, `pdf::write_pdf_view_state`) previously turn foreign `1.0`, `-0.0`, `1e3` into integers. Shared `edn::write_edn` now uses the shortest Float representation retaining its marker and negative zero. The regression checks reparsed foreign types/values through both doors. The full workspace gate finds two existing fixtures requiring integer spellings for owned bounding coordinates; they remain unchanged. [before](tests/og-b-store-evidence/integrity-before.txt), [focused pass](tests/og-b-store-evidence/final-focused.txt), [conflict](tests/og-b-store-evidence/workspace-pdf-conflict.txt). |
| L03:49 | No; already fixed at base | Ran the exact `* Example [[Old]]\n#+aaaaaé\n` rename input, preserving the directive and producing `[[New]]`. It passes in the base fail-before run while the three new integrity regressions fail. This is already covered by REG-OG-UNICODE-PREFIX-PANIC-001; the added exact-input recheck also passes. No duplicate repair. |
| L03:50 | Wrong export identity does not reproduce; structural cleanup pending | The base already routes `refs::block_id(raw, is_org)` to parser-owned `block_regions::parse(...).id` (D1). A real Store/publish regression checks Markdown metadata with a fenced fake id, an Org drawer id, corpus properties, exported anchors and cross-page fragment links; it passes on both restored base and candidate. [Base proof](tests/og-b-store-evidence/base-export.txt). `render.rs` still reparses raw text at four sites (392, 1676, 1683, 1812) rather than using cached `DocBlock::property("id")`. That cache/one-answerer cleanup belongs in the out-of-set renderer; no second id grammar was added. |
| L05:26 | Yes; pending outside write set | Exact plain evidence for NFD `Café` targets `cafe` as the prefix `Cafe` at bytes 0..4, while indexed unlinked lookup returns empty. The semantic matcher is `tine-core/src/reference_evidence.rs::visit_plain_matches`, outside the allowed core files. Fix its canonical grapheme/NFC boundary once and then verify all plain-reference clients, rather than compensating in signatures or `refs.rs`. [observation](tests/og-b-store-evidence/probes.txt). |
| L06:36 | Yes; pending outside write set | Markdown `#+ICON: 🏁\n- body` produces an icon through `model/page_icons.rs::pre_block_icon` even though it is not a Markdown page property. The canonical page-property grammar lives behind `tine-store/src/query/page_properties.rs` (`pub(super)` to query). Exposing a per-document property answer through `query.rs` and reusing it requires out-of-set files. No new scanner or duplicate property grammar. [observation](tests/og-b-store-evidence/probes.txt). |
| L06:40 | Yes; repaired | The actual incremental `model::preamble_read` path repeatedly parses a growing prefix for 1,000 leading Org directives. Its existing last-line inert filter now runs before `preamble_end(prefix, ...)`; only the settling line needs the full-prefix parse. All page identity discovery clients use this door. The test checks the real incremental calls and eventual title, plus an ordering guard naming the shared invariant. Base 1.54 s / candidate 0.01 s in the focused run; no timing threshold was introduced. [before](tests/og-b-store-evidence/preamble-before.txt). |
| L07:34 | Yes, duplicate construction; repaired | Valid behavior already agrees, but the base source guard finds zero shared-helper sites. `Store::journal_id` and `WholeGraph::resolve` now both use private `proposed_journal_id`; each keeps its live/captured JournalFormat and existing invalid-name fallback. Guard requires one definition and both clients; real Store tests cover configured directories, two filename formats and Markdown/Org. [before](tests/og-b-store-evidence/journal-before.txt), [after](tests/og-b-store-evidence/final-focused.txt). |
| L05:28 | Yes; pending outside write set | Every benchmark save with a held WholeGraph copies 2,002 or 10,002 page slots, before and after. `cache_upsert` uses `Arc::make_mut` on the full vector; observed-mtime and block-reference-count maps have sibling copy costs. A class repair requires persistent snapshot collections and changes to slice-based consumers in `tine-store/src/query/index.rs` and query interfaces, outside this write set. No partial vector-only patch or hidden cache was added. |
| L05:29 | Yes by source/caller trace; pending outside write set | New/remove/retitle publication scans all pages for each affected name winner (`pages.iter().filter(...).min_by(...)`); structural signature capture rebuilds path/position collections across the old/new sets. Complete repair needs a persistent winner/index representation shared with `query.rs::RealPageNames` and `query/index.rs`. No dynamic fail-before cost assertion is claimed for this pending item. |
| L05:30 | Yes by source/caller trace; pending outside write set | `prepare_page_content` parses old text for preamble protection, reclassification provenance, header promotion, layout retention and possible equivalence checks. Its shared serializer is `model/layout_retention.rs`, outside this write set. Reuse one parsed old document across the entire family rather than removing only some duplicates. Existing `parse_doc` counters omit these direct `doc::parse` calls, so the measured one publication parse is a lower bound, not the actual invocation count. No dynamic complete-parse count is claimed. |
| L08:52 | Yes; repaired | Real transaction asset moves count two directory syncs for a same-directory move at base. `sync_move_dirs` now deduplicates the two parent paths; all move/rollback callers share it. Added test-fault instrumentation counts actual calls. Same-parent move now performs one sync; different-parent move still performs two, preserving the bytes. [before](tests/og-b-store-evidence/move-before.txt), [after](tests/og-b-store-evidence/final-focused.txt). |
| L08:53 | Alleged duplicate-byte waste does not reproduce | Accepted ADR 0056 explicitly approves K=2 distinct texts, one blob plus index for a new distinct text, and no writes when bytes equal the newest entry. The 14 existing ledger tests pass, including repeat/new/retained-old cases, retention, crash/failure fallbacks and exact approved costs. No design or code change was needed. [proof](tests/og-b-store-evidence/ledger.txt). |

Config safety neighbor: strict root validation initially made a complete
`:pages-directory "archive"` disappear when a later form was torn. A new
regression failed on that intermediate candidate (`"pages"` versus
`"archive"`), then passed after readers retained complete values before the
torn suffix. Readers and writers still share root ownership; writers require a
balanced root before editing. Hidden-vector validation remains fail-closed.
Proof: [intermediate failure](tests/og-b-store-evidence/torn-before.txt).

## Unit cost and benchmarks

Benchmark fixture: 2,000/10,000 unrelated Markdown pages plus 1-block and
60-block target pages. Each side opens three times per execution, saves each
target five times per open and retains a WholeGraph throughout. Both binaries
were built in debug with `CARGO_INCREMENTAL=0`, then executed in order base,
candidate, candidate, base. Base was exactly `0ce583ff5` for the five changed
production files; all temporary replacements were restored with a `finally`
handler before building the candidate. The benchmark itself is identical.
There are six open samples and 30 save samples per metric per side.

| Metric, median ms | Base | Candidate | Change |
|---|---:|---:|---:|
| 2k open | 15.802 | 13.809 | -12.62% |
| 2k open through WholeGraph readiness (elapsed from open start) | 573.123 | 463.263 | -19.17% |
| 2k 1-block save | 2.166 | 2.092 | -3.39% |
| 2k 60-block save | 8.273 | 7.759 | -6.21% |
| 10k open | 86.919 | 62.336 | -28.28% |
| 10k open through WholeGraph readiness | 2,593.291 | 2,387.930 | -7.92% |
| 10k 1-block save | 5.190 | 4.969 | -4.27% |
| 10k 60-block save | 10.605 | 10.506 | -0.93% |

All paired medians meet the +10% relative limit. These are shared-host debug
observations, not evidence of a general speedup: individual candidate saves
range up to 523.928 ms, and an earlier concurrent-load run had worse open and
2k 60-block medians. Raw samples and ranges are committed; no sample was
discarded in the paired comparison. No frontend typing/paint, Windows, memory,
release-binary or master performance claim is made.

| Ordinary save cost, both graph sizes | Base | Candidate |
|---|---|---|
| 1-block target | 12 payload bytes; 1 file; 1 instrumented publication parse | same |
| 60-block target | 543 payload bytes; 1 file; 1 instrumented publication parse | same |
| Page-vector slots copied with held view | 2,002 / 10,002 | same; L05:28 pending |
| Distinct page sources involved in parsing | 1 target page | same; total repeated parser invocations not fully instrumented (L05:30) |

The synchronous payload counters cover the real Store save. The approved
asynchronous ledger is a separate device-data cost: 44-byte/2,631-byte sample
texts write respectively 220/2,808 bytes in two files and remove one evicted
blob when full. Retained footprints are 263/5,438 bytes in three files. An
identical newest record writes zero; returning to a retained older text
rewrites only the index. These match ADR 0056 before and after; no ledger cost
or persisted schema was changed. Store save and ledger fixture sizes differ
because the ledger's approved fixture includes additional metadata.

The only changed namespace-write path is the shared move sync helper. A
same-parent move still writes no payload bytes and now syncs that directory
once; a different-parent move syncs each directory once. There is no new
write operation, edit kind, durable record, multi-step protocol or transport
artifact.

Reproduce the benchmark with:

```sh
rtk proxy bash -c 'source scripts/env.sh; export CARGO_INCREMENTAL=0; timeout 600 cargo test -p tine-store --test og_b_store_cost -- --ignored --nocapture --test-threads=1'
```

Its `#[ignore]` marks this new, explicit workload benchmark only; no existing
test was skipped or disabled.

## Gates

All Rust commands use the pinned environment and `CARGO_INCREMENTAL=0`.
Every gate was time-bounded; no release build or dependency install was used.

| Command / gate | Exit | Result |
|---|---:|---|
| `timeout 180 npx tsc --noEmit` | 0 | Typecheck passed. |
| `timeout 1500 npm test -- --maxWorkers=4`, after `cargo fmt` | 0 on retry | 292 Node test files / 2,297 tests passed, plus 256 render files / 1,966 tests passed. Existing skips untouched; no `Errors` line. First run exited 1 with six 5-second scanner-test timeouts during heavy native compilation; the identical retry passed. |
| `timeout 120 npx vitest run src/ogEnforcement.test.ts` | 0 | Five shape/size tests passed; no ratchet change. |
| `timeout 120 cargo fmt --all -- --check` | 0 | Final formatting passed. |
| `timeout 1800 cargo test --workspace --tests --no-fail-fast` | 101 | Final-source run: 1,869 passed, two PDF byte-fixture failures in `tine-graph-features --test client`; all other targets passed. The earlier run has the same conflict. See Q1 and committed failure output. |
| Focused core/store regressions | 0 | Six integrity tests, one preamble test, and config/export/journal/move tests all passed. |
| `timeout 600 cargo test -p tine-core --tests`, after restoring candidate | 0 | All 639 core tests passed across 16 targets. The base export and exact Unicode rechecks also each passed (exit 0). |
| `timeout 900 cargo test -p tine --lib concord_ledger -- --nocapture` | 0 | All 14 ledger tests passed. |
| G6 `og-g6-untouched.sh`, debug adaptation | 0 | 1,075 pages read, zero unreadable, zero byte diffs; 1,075 structural round trips, zero structural bugs. |
| G6b `og-g6b-one-block-edit.sh`, debug adaptation | 0 | Frontend projection passed; 1,004 editable pages passed, 71 had no leaf block, zero editable-page failures. |
| `timeout 120 node scripts/check-regression-catalog.mjs` | 0 | Regression inventories and existing UI catalog passed. |
| Paired base/candidate explicit benchmark | 0 each | Four workloads passed; medians within relative budget, primitive costs unchanged. |

G6/G6b adaptations were temporary copies of the existing scripts with only
` --release` removed and ROOT set to this worktree (their script directory
otherwise points into `/tmp`). G6b used this worktree's existing target dir
and a `/tmp` report. The fixture source was read-only; gates operated on
temporary copies. Neither checked-in script changed. Final G6/G6b runs are on
the final production source.

G2 local review: no new public signature, type, operation or vocabulary. The
existing journal proposal docs retain live/captured configuration and cost
facts. G3 existing invariant guards pass except the stated golden conflict;
no new structural-content scanner, error-string branch or storage bypass.
Manager review, invariant-sweep review, stable UI journeys, hosted platforms,
integration and deployment are not claimed by this implementation lane.

## Size and contract delta

Production LOC uses the repository's `productionSource`/`lines` census,
excluding inline test modules, against `0ce583ff5`:

| Module | Base | Final | Net |
|---|---:|---:|---:|
| core config.rs | 965 | 923 | -42 |
| core edn.rs | 478 | 480 | +2 |
| core model.rs | 1,102 | 1,101 | -1 |
| store store.rs | 3,431 | 3,431 | 0 |
| store transaction/io_helpers.rs | 103 | 106 | +3 |
| Total | 6,079 | 6,041 | -38 |

The oversized store module does not grow. There are 120 added production diff
lines and 349 added Rust test/benchmark lines (within 3× added production
lines); four catalog records and proof data are metadata, not production.
No dependency, API/wire schema, graph format, edit kind, allow-list or pinned
format-count delta. Float output spelling changes intentionally to preserve
numeric type, which is precisely the byte-contract conflict in Q1. Guide
templates are unchanged because there is no new workflow or capability.
This defect batch has no master feature-port Δ or parity-ledger feature row
to close. Public catalog rows cover the four user-visible repaired families;
the Float row explicitly notes the outstanding integration conflict.

## Questions and required follow-up

1. **Q1 — L01:46 cannot integrate with the current frozen byte oracle.** The
   existing tests `legacy_pdf_artifacts_stay_on_open_and_match_after_write_migration`
   (`crates/tine-graph-features/tests/client.rs:1091`) and
   `old_vs_new_matrix_on_identical_fixtures` (`:851`, `assets/paper.edn`)
   require bounding values spelled `0`/`1`; type-preserving EDN spells
   `0.0`/`1.0`. Batch rule: “If an existing test conflicts with a fix, stop and
   record it as a question.” Which contract should the integration lane adopt:
   foreign-field type preservation with explicit owned-coordinate integer
   construction, or an approved change to the legacy byte oracle? No existing
   tests or golden fixtures were edited here; the shared writer candidate is
   available in its own first commit for review.
2. **Write-set follow-up:** assign the six reproduced pending defects to a
   lane allowed to repair their complete shared doors: Org marker format
   propagation; plain-reference matcher; canonical page-property export;
   persistent snapshot/query collections and name winners; parsed-old-source
   reuse in layout retention. Export's four raw-id callers are an additional
   cleanup, with no current wrong-ID reproduction. No pending item is marked
   fixed by a narrower local workaround.
3. **Cost evidence limitation:** accurate total parser-invocation accounting
   is needed alongside L05:30. The existing publication counter's `1` must
   not be interpreted as one actual parse. The whole-vector copy cost remains
   measured and unresolved. No private app-data record was introduced to
   address it.
4. **Authority reconciliation:** E-invariants' older I-25 “one file/no private
   record” wording must be read with the accepted ADR 0056 K=2 exception.
   The approved ledger is fed off the save thread and already deduplicates
   equal newest text. Do not remove/reduce it based on L08:53's allegation.

## Commits and handoff

- `e09edc451` — shared config ownership/decoder and type-preserving Float writer.
- `1cbf29030` — bounded Org discovery, shared journal proposal, distinct-parent move sync.
- `c1ed843d6` — torn-suffix safety neighbor, export proof and explicit cost benchmark.
- Final receipt/evidence commit — this document, durable proof and catalog attribution.

Only authored files in the lane write set were staged. B-FAIL's rollback,
config-read and discovery-error functions were not edited. Per the user's
explicit final-commit instruction, this receipt is committed even though the
batch common defaults receipts to untracked. No push, merge, integration or
deployment was performed. Disk stayed above 150 GB free.
