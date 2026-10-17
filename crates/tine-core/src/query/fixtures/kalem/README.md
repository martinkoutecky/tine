# Kalem's captured Logseq answers

Unmodified fixtures from [getkalem/plugins](https://github.com/getkalem/plugins),
commit `46ffb4d`, `plugins/graph/corpus/{logseq-md,logseq-org}` and
`plugins/graph/tests/logseq/answers{,-org}-0.10.9.json`.
Copyright and attribution: Kalem contributors; original copyright notice is in
LICENSE-MIT. Distributed under the upstream Apache-2.0 OR MIT licence; both
licence texts are included. The author agreed to reuse with attribution on
2026-10-12. No upstream NOTICE file accompanies these fixtures.

The JSON `about` fields describe the capture: Logseq 0.10.9's database/API
answers on 2026-10-11, with today fixed to that day. Compare sets, ignoring
ordering and duplicates. Block identity is the lower-cased page title and first
source line without its Markdown bullet or Org headline stars; page identity is
just the lower-cased title. Backlinks compare referring page names. Graph edges
are undirected, with each pair sorted. See upstream `tests/logseq_parity.rs`.

Tine's harness is `tine-store/src/kalem_parity_tests.rs`. Unlike Kalem's own
consumer it does not silently omit absent pages or known Logseq graph artifacts.
The corpus includes hidden files, backups, assets and a whiteboard intentionally;
do not trim them. Tests copy the graph before opening it so Store never writes
into these source fixtures. Today's transient Org journal is absent from the
capture by design. Graph view is currently unimplemented on Tine's OG line
(docs/BACKLOG.md, Graph view); the captured requests remain available for that
implementation's future conformance gate.
