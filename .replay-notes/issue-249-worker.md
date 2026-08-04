# GH #249 worker report

## Outcome and root cause

Implementation commit: `ca06810f1b7787490245a841abeb5b60852404ad`.

Tine's shared `encode_page_name` encoded only namespace `/` and triple-lowbar
ambiguity. A new logical title such as `2026-07-23_18:01:20` therefore selected
`pages/2026-07-23_18:01:20.md`; Windows rejects the colons, the audited save
returned an error on every dirty retry, and the unsaved page never acquired a
durable path. Existing Logseq-created files worked because their `%3A` path was
decoded on load and every later save remained pinned to that exact path.

The repaired codec is single-pass and injective for supported nonempty titles:
literal `%` is escaped before generated escapes; Windows-reserved/control
characters, legacy dots, leading dots, trailing dot/space, and case-insensitive
DOS devices are percent-encoded; configured legacy `%2F` versus triple-lowbar
`___` namespace spelling is retained. Decoding remains backward-compatible.
Existing loaded/imported/projection files are not renamed or rewritten merely
because their historical path differs from the new canonical spelling.

## OG provenance and path inventory

Inspected current local Logseq OG revision
`6e7afa8eb040686ff057156ee877193b581dd369` (`version/file`, 2026-05-28):

- `src/main/frontend/util/fs.cljs`: `file-name-sanity`,
  `legacy-url-file-name-sanity`, `tri-lb-file-name-sanity`, percent pre-escape,
  reserved-character/device handling, and triple-lowbar disambiguation.
- `deps/graph-parser/src/logseq/graph_parser/util.cljs`: `title-parsing`, legacy
  dot/percent decoding, and triple-lowbar namespace-before-percent decoding.
- `src/main/frontend/handler/page.cljs`: page create and rename callers use the
  same sanitizer; conversion uses it too.

Material semantics transcribed: safe reversible logical title, configured
namespace compatibility, percent decoding, and no destructive target collision.
Incidental/incomplete OG spelling was not copied where it violated the frozen
contract: device matching is case-insensitive, trailing dot/space is explicit,
and every literal percent is unambiguous.

Tine inventory:

- Direct/managed new save: `save_target -> managed_path_for -> encode_page_name`.
- Direct lookup/onboarding and copy: `path_for` and
  `create_markdown_page_if_absent` use the same codec.
- Transactional namespace/page rename and rescue rename use the codec; Org
  `file:` link rewrites now receive the graph filename format and use the same
  decode/encode pair as the physical move.
- Sparse managed editor creation in `sync_runtime` uses
  `new_sparse_page_path`, which uses the same codec before occupancy checks.
- Load, watcher, external import, bootstrap/reconciliation, and projection
  recovery consume exact existing `ManagedPath` values and decode identity;
  they do not derive replacement paths, so exact historical paths/bytes remain.
- Journals retain their separate configured date-stem mapping
  (`journal_format.file_stem`); page-title sanitation is not applied to journal
  date identities. PDF/HLS pages retain their separate key-derived identity.

## Changed files

- `crates/tine-core/src/model.rs`: portable reversible codec, shared rename
  call, memory-bound adjustment, and create/edit/reopen/rename/rescue/sparse/
  collision/existing-path tests.
- `crates/tine-core/src/refs.rs`: config-aware Org file-link rewrite and tests.
- `tests/regressions/non-ui.json`: `REG-PAGE-FILENAME-IDENTITY-249` covered row.

## Fail-before and pass-after evidence

Fail-before on base `1eec95c1` after adding tests only:

- `rtk cargo test -p tine-core page_name_encoding_is_injective_reversible_and_windows_safe -- --nocapture`
  failed: emitted `2026-07-23_18:01:20` unchanged and failed the Windows-safe
  assertion.
- `rtk cargo test -p tine-core unsafe_page_titles_ -- --nocapture`: 2 failed.
- `rtk cargo test -p tine-core safe_filename_collisions_refuse_without_overwriting_existing_bytes -- --nocapture`:
  failed because unsafe `A:B.md` was created beside occupied `A%3AB.md`.

Pass-after on implementation commit:

- Filename/create/rename/collision/legacy/Org-link focused filters: 9 tests
  passed across the named filters.
- `rtk cargo test -p tine-core --test graph`: 64 passed.
- Sync filename-policy/new-page/final-name collision filters: 3 passed.
- Namespace/active rename precommit budget filters: 4 passed.
- `rtk npm run check:regressions`: passed (234 UI entries, 144 issues; both
  regression inventories valid).
- `rtk proxy npx vitest run src/persistence.test.ts src/pages.test.ts`: 4 passed.
- `rtk proxy npx vitest run --config vitest.render.config.ts src/graph.test.tsx`:
  12 passed.
- `rtk proxy npx tsc --noEmit`: passed with zero errors.
- `rtk git diff --check`: passed before commit.

No formatter was run; the controlled-worker contract did not authorize a root
format pass.

## Confidence limits

The literal Windows packaged-app create/edit/close/reopen journey and hosted
Windows gate were not available in this worker. The proof reaches the shared
Rust path boundary, asserts Windows filename rules directly, and exercises real
disk/restart behavior on Linux, so platform confidence is strong structurally
but remains below same-platform native confirmation. No native E2E harness or
bulk migration was added. Existing graph paths remain authoritative and are
covered by exact-path/no-byte-change tests plus the existing graph suite.
