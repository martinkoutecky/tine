# OG-T1 receipt

Base: `ce411c6f717a75b29ad16cf188cd32860d5515a9`; branch `og-t1`.
Implementation commits: `7fcffa6e7`, `4dcd39035`, `9c7b88a98`.
The closing commit adds this receipt only.

## Scope (recorded before editing)

Investigate the three supplied native failures, find their introducing commits,
and preserve master’s final behavior. OG already has sheet scrolling,
marker-pair label toggles, and default bracket pairing. These are stale journeys;
no production change, feature port, store door, persisted format, or public
interface is needed. Invariants in play: I-4, I-12, I-20, I-21.

Master oracle: read-only `tine-master` at
`5dfc8450310af0912755f963aa6bc4eaf4116c40`.
Original Logseq oracle: `/aux/koutecky/logseq/og`, revision
`6e7afa8eb040686ff057156ee877193b581dd369`,
`src/main/frontend/handler/editor.cljs`: `autopair-map`, `autopair`,
and the keydown branches at lines 2889–2925. `[` inserts `]`, leaves the
insertion point between, and a second `[` invokes page search inside `[[]]`.

All raw evidence: `/tmp/og-t1-evidence/`. Binaries are copies under `bin/`;
artifact directories are separate from logs. Historical journeys ran from
isolated detached checkouts when their scripts differed. Never pushed, merged,
or edited another implementation lane, the master oracle, or real app data.

## 1. Board containment

**Introducing commit:** `86cf4c26811199dc334ab90ab9c686745935a4de`
(ports block-owned internal scrolling / disables default negative-margin
breakout). **Classification:** stale geometry journey, not escaped content.

- Archived `525194644`, its own journey: exit 0, all 72 checks pass
  (`525-sheets.log`). Archived `194f7fbbc`, its own journey: exit 1,
  exactly one failed check; board left 99, container left 434
  (`194-sheets-clean.log`).
- Narrowed with adjacent debug builds, custom-protocol, no release build:
  `a79969336` (parent of `86cf4c268`) passes 72/72, exit 0;
  `86cf4c268` fails only containment, exit 1
  (`viewport-before-sheets.log`, `viewport-after-sheets.log`). The latter
  has board left 99 versus container left 432. Build logs are adjacent files.
- WebDriver clicking the TODO card scrolls the internal viewport to reveal it.
  The screenshot shows content clipped inside that viewport; inspected the
  final native `/tmp/sheets-e2e-query-board.png`. Raw content rectangles move
  left while scrolled; the containment assertion was measuring that state.
- Master’s final journey already resets this scroller before measuring,
  introduced by `bafd98ebe` after `ee7730b48` (the original viewport change).
  Ported that normalization and retained all four containment bounds, the
  one-board assertion, heading/next-block non-overlap, selection, movement,
  and disk assertions. Added `priorScrollLeft` to failure diagnostics.
- Pass-after on the unchanged deployed base copy: 72/72, exit 0
  (`base-sheets-corrected.log`). Final-build repetitions: see below.

The historical builds need the old pinned Inter package, absent from today’s
shared node_modules. Only those detached Vite configs alias it to the already
archived `/aux/koutecky/logseq/og-perf-scratch/fontsource-inter/package`;
no dependency install or shared node_modules mutation. The historical build receipts already record a local Vite
configuration adjustment. The broad archived interval
was rebuilt only after it remained wider than five commits.

## 2. Marker pill

**Introducing commit:** `733199612262f5d182d6c8b1673a02b2eca6c3eb`
(port of master `c94917d67`: GH #259). **Classification:** stale workflow
expectation. WAIT is no longer a clickable open-pair label.

- Before: archived `194f7fbbc` passes the old WAIT→LATER check at the real
  sheet entry (`194-sheets-clean.log`). After: manager’s two unchanged-base
  native logs under `og-e2e-ce411c6f7/sheets{,-rerun}/stdout.log` show the
  selected cell, no editor, and WAIT unchanged on disk. That matches the
  exact changed caller in `733199612`: `cycleField(state)` became
  `toggleStateMarkerLabel`, whose existing answerer is `toggleMarkerLabel`.
- Master’s current journey seeds TODO and requires DOING. Adopted both that
  fixture and expectation. No keyboard-cycle or product change. The other
  WAIT query fixtures stay intact. Pass-after on the identical base binary:
  72/72, exit 0 (`base-sheets-corrected.log`).
- Existing tests at the shared and rendered boundaries cover both open pairs,
  inert terminal markers, selection, and no editor entry:
  `repeat.test.ts`, `Block.markerClick.test.tsx`, `SheetTable.test.tsx`.
  All pass in the full suites. No new answerer or write path.

## 3. Capture page-ref typing

**Introducing commit:** `97604065eef1a584653cce0cbb0ec27443d57dd2`
(port of master `46cd3997c`: GH #291). **Classification:** stale default and
input-owner expectations.

- Native unchanged-base capture journey on archived `194f7fbbc`: exit 1,
  expected `[` / actual `[]` (`194-capture.log`).
- Native real editor typing probe (`pair-probe.mjs`): archived `525194644`
  yields first `[` at caret 1; archived `194f7fbbc` yields first `[]` at
  caret 1. Both yield `[[]]` at caret 2 after the second key
  (`pair-525.log`, `pair-194.log`, both exit 0). The source history narrows
  this default change to `97604065e`: `loadStr(...) === "1"` becomes
  `loadStr(...) !== "0"`; auto-pair implementation is unchanged.
- Master `5dfc84503` requires exactly `[]`/caret 1 then `[[]]`/caret 2.
  Ported those observations through the existing single helper, covering first
  show, hidden-window reopen, and cold restart. This strengthens the insertion
  point check instead of accepting several transient values.
- The first correction then exposed master’s second fix in `041d04d29`:
  Ctrl+A on the already-empty filed scratch block exits editing. Native
  fail-before has a null input owner at the reopened first bracket
  (`base-capture-corrected.log`, exit 1). Replaced clearing with an assertion
  that the reset editor is empty and focused, exactly as master does.
  Pass-after on unchanged base: `base-capture-complete.log`, exit 0.
- The fresh debug build exposed a further fixture race at cold restart:
  `[[Fz]]` was correctly entered, but no candidates appeared (two repeatable
  failures: `final-capture-1.log`, `final-capture-rerun.log`). Both master
  `src-tauri/src/lib.rs:801` and OG `:729` defer startup graph loading to the
  visible WebView. Native window existence is not graph-fixture readiness.
  Wait for the seeded outline in the restarted main window before invoking
  the still-cold Capture WebView. All native focus, first-show typing,
  existing-first/typed policy, save, and restart assertions are retained.
  A detached diagnostic probe passes on the same binary with only this fixture
  precondition (`capture-ready-probe.log`, exit 0); then committed the step as
  `9c7b88a98`. No production fix for capturing during startup is claimed.

## Master agreement and relaxation ledger

The archived master application is `ddf408c55` (receipt in og-bench/binaries).
Its relevant SheetContainer, autopair, SheetTable, and journey implementations
match master `5dfc84503`; the only relevant SheetBoard difference is `appNow()`
in place of `new Date()`, immaterial to state grouping here. With master’s
current scripts, `master-sheets.log` passes all 76 checks, exit 0, and
`master-capture-wm.log` passes, exit 0. Thus the corrections follow final master
behavior, not intermediate port commits or invented compatibility choices.

| Journey | Changed observation/setup | Authority and retained outcome |
| --- | --- | --- |
| e2e-sheets | Reset horizontal scroll before measuring board bounds | Master `bafd98ebe`; retains containment and no vertical bleed, without forbidding user scrolling. |
| e2e-sheets | TODO→DOING replaces WAIT→LATER click expectation | Master `c94917d67`, current master journey; proves label toggle, selection, no editor, and durable marker. |
| e2e-capture | Require paired brackets and their caret | Master `041d04d29`, default `46cd3997c`, cited OG source; proves literal input ordering and autocomplete policy. |
| e2e-capture | Assert empty focused scratch editor instead of Ctrl+A/Backspace | Master `041d04d29`; preserves the input owner after filing. |
| e2e-capture | Wait for restarted graph fixture before page-candidate proof | Both oracles defer graph loading to the visible WebView; capture remains cold, native focus and policy outcomes remain asserted. |

## Shape, scope, and cost

Production delta: zero. Harness delta: sheets +8 net lines, capture +29 net
lines. No changed public interface (Rule 2 review not applicable), no new
answerer, regex/content-structure scanner, format, artifact per edit, dependency,
allow-list, ratchet, or baseline. Class sites: sheet bounds after native scrolling;
the sheet label/keyboard marker split; all three capture page-query lifecycles
through one helper. Existing native assertions are the recurrence guards.

Master ledger rows `c94917d67` and `46cd3997c` are already ported; this lane
repairs their native evidence, so no product status or family row is changed.
G4/G5: no production size/cost/performance delta; a debug binary is not a release
size benchmark. G6/G6b and byte differential: not applicable because no
save/serialize/write implementation changed. Guide/contract exception:
verification-only; no user-visible capability or workflow changes. Catalog:
no new product regression accepted, so the native journeys remain their
existing catalog evidence, not new bug rows. No push, integration, or deployment
into the manager’s app destination; the final native application is the copy
listed below.

## Final build and gates

Final app: `/tmp/og-t1-evidence/bin/final2`, freshly built from
`9c7b88a98b66c394851952c5a76c5cd7ab832209` with rebuilt frontend,
`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, and custom-protocol.
SHA-256: `e9917f505a5864eb3be86bb9777affc522c0d85dc58f5ddb042bb5e8e3cb7380`.
Build exit 0; provenance `/tmp/og-t1-evidence/final2-build.json` (the sole dirty
input is this then-untracked receipt; all app and journey code is committed).
Closing receipt commit does not change application/journey code.

| Gate | Result / log |
| --- | --- |
| `timeout 120 npx tsc --noEmit` | 0; `tsc-final.log` |
| `timeout 1500 npm test -- --maxWorkers=4` | Initial run 1: five source-guard 5s timeouts during historical compilation; rerun 0, no Errors line: node 2288 passed/2 skipped, render 1960 passed/3 skipped. `npm-test-rerun.log`; final repetition also exit 0 with the same counts and no Errors line (`npm-test-final.log`). Existing skips unchanged. |
| `timeout 180 cargo fmt --all -- --check` after sourcing env.sh | 0; `fmt.log`; npm rerun was after this gate. No formatting mutation. |
| `CARGO_INCREMENTAL=0 timeout 1500 cargo test --workspace --tests --no-fail-fast -j 4` | 0; 108 suites, 1839 passed, 6 existing ignored, 0 failures; `cargo-test.log`. Uses external target dir and debug/test symbols disabled. |
| `timeout 120 npx vitest run src/ogEnforcement.test.ts` | 0; 5 tests; `og-enforcement.log` |
| `timeout 120 npm run check:ui-catalog` | 0; 262 entries; `ui-catalog-final.log` |
| `timeout 30 node --check` on both touched journeys; `git diff --check` | 0 |
| Final unchanged-binary sheets / capture repetitions | **All 0: sheets 72/72 each, capture full journey passing each. All three originally red checks pass 3/3 on the identical final binary.** `final2-sheets-{1,2,3}.log`, `final2-capture-{1,2,3}.log`, corresponding `.exit` files. |

All gates and native/build runs were timeout-bounded. Disk free-space observations stayed above 150 GB
through this lane (initial 173 GB, low-water observation 157 GB). No release builds. Removed the lane-owned external Cargo build cache after
verification; the copied binaries, build receipts, raw logs, and images remain.

## Invalid/preliminary evidence and questions

Initial overlapping sheets probes shared `/tmp/sheets-e2e`; they were stopped
and are excluded (`f546-sheets.log`, `194-sheets.log`). All valid sheets probes
run serially. Capture needs an EWMH window manager: runs without Openbox failed
before typing. Valid final commands run `xvfb-run -a dbus-run-session` with
Openbox in that same isolated session. Native artifact paths are separate from
logs. Preliminary capture retries also encountered one native-focus timeout
and one WebDriver session timeout; those are preserved, not counted as passes.
Normal capture scripts log a retried Settings click rejection on Linux even on
passing master/base runs; the final policy/save outcomes are explicitly tested.

Open questions: none within this lane. Capturing before the initial graph has
opened is outside this persisted-policy journey’s established graph fixture;
this lane does not claim new product behavior at that startup boundary.
