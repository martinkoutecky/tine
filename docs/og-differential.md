# Differential bytes for G1

The oracle is master `ddf408c55`, the revision recorded in
`~/research/tine.build.json`. For every file-writing action, prepare one graph
fixture, copy it twice, run the same action in the master and og binaries, then
walk both result trees. Compare relative path, file type, and raw bytes for
every new or changed file. Record every difference, including renamed source
and destination paths. Ignore only explicitly listed volatile app-data files;
graph files, recovery files, backups and manifests remain in the comparison.
Use private copies of any user graph and keep its content out of the report.

The action driver should save a JSON report with: the input graph digest,
binary SHA-256 and revision, the exact user action, sorted changed-path lists,
SHA-256 and size for each output, and a bounded byte-difference excerpt for
synthetic fixtures only. A mismatch fails G1 until its behavior is argued in
the batch record. Rename actions compare *both* the renamed page and all
referrer files; an absent source path is part of the expected result.

## Worked example: ten page serializers

`scripts/fixtures/og-differential-pages.json` contains ten synthetic Markdown
and Org pages: properties, CJK, nested blocks, a fenced block, and whitespace.
Their expected raw UTF-8 output is in
`scripts/fixtures/og-master-pages-golden.json`. The golden was written by
master's Rust `tine_core::doc::serialize_with` and
`tine_core::org::serialize_org_detect`, through the scratch example
`scripts/og-master-page-oracle.rs`, copied into the scratch worktree as
`crates/tine-core/examples/og_diff_pages.rs`.
It is not a hand-transcribed expectation. The og integration test
`crates/tine-core/tests/og_differential_pages.rs` invokes og's corresponding
Rust serializers and compares byte slices, reporting the page path on failure.
Nine of ten match at this revision. The baseline difference is
`pages/Whitespace.md`: master preserves its one leading blank line; og
removes it. Both keep the two trailing line endings (og emitted three until
batch 13a fixed EOF blank-line doubling). The test pins both byte strings and
fails on any additional difference. A save never reaches this whole-page
serializer for an existing page whose layout it can map: batch 13a's
per-block retention keeps the file's own lines, so this difference shows
only on the fallback path.

The second golden, `scripts/fixtures/og-master-rename-golden.json`, comes from
master's `Graph::rename_page` path on the synthetic
`scripts/fixtures/og-rename-input.json`. The scratch oracle source is
`scripts/og-master-rename-oracle.rs`. It records the removed source path and
the raw bytes of the moved page, rewritten referrer, and rewritten journal.
`crates/tine-store/tests/og_differential_rename.rs` runs og's real store rename
on the same input and asserts the removed path list and every output byte.
This low-level rename matches. The native UI bench separately detects that
og's current save/rename workflow can refuse the action before reaching this
writer.

Reproduce the oracle from a clean scratch master worktree:

```sh
rtk git -C <master-checkout> worktree add --detach test-results/master-ddf408c ddf408c55
rtk cp scripts/og-master-page-oracle.rs test-results/master-ddf408c/crates/tine-core/examples/og_diff_pages.rs
rtk cargo run --quiet --release --manifest-path test-results/master-ddf408c/Cargo.toml -p tine-core --example og_diff_pages -- scripts/fixtures/og-differential-pages.json > scripts/fixtures/og-master-pages-golden.json
rtk cp scripts/og-master-rename-oracle.rs test-results/master-ddf408c/crates/tine-core/examples/og_rename_oracle.rs
rtk cargo run --quiet --release --manifest-path test-results/master-ddf408c/Cargo.toml -p tine-core --example og_rename_oracle -- scripts/fixtures/og-rename-input.json > scripts/fixtures/og-master-rename-golden.json
rtk cargo test -p tine-core --test og_differential_pages
rtk cargo test -p tine-store --test og_differential_rename
```

For other rename scenarios, use the same scratch worktree's real
`Graph::rename_page_guarded` path (`crates/tine-core/src/model/page_rename.rs:53`)
on a copied fixture graph. Save the before and after sorted file manifest and
the exact bytes of all changed paths, then compare og's store rename action
against it.
