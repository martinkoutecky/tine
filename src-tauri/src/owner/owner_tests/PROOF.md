The CLI write proof runs with `cargo test -p tine --lib owner::owner_tests::proof`.

`write-transcripts.json` is the reviewed corpus: synthetic input bytes, CLI argv
(`$GRAPH` is the disposable fixture), owner request, text output, JSON output,
exit status and resulting page bytes. Successful verbs and semantic refusals run
against a real store and page host. `injected_host_outcome` rows exercise the
production host-result mapping and terminal rendering directly; they pin the
volatile I/O error vocabulary and admission flags, not host fault recovery.
The host's own binding tests cover those faults. The CLI argv is documentation;
this Rust corpus runs the request and renderer, not a subprocess or Clap.

A mismatch fails without modifying the corpus. To propose a deliberate output
change, run with `TINE_REGENERATE_WRITE_TRANSCRIPTS=1`, then review the git diff.
Never regenerate as part of CI or ordinary verification.

The five `generated_*` tests each run 40 seeded pages per format. They vary
root count, target, nested depth, properties, permanent ids, task markers and
multiline text. An independently assembled expected page is parsed and compared
for every raw block and its tree position. Every untouched subtree must also
remain an exact physical byte substring, including the changed root's untouched
descendants for edit and prop. Prop alternates set and remove.

Native journeys: `scripts/e2e-cli-writes.mjs`, with three contracts in
`tests/ui-regressions/e2e-contracts.json`. It uses fresh synthetic graphs and
private app data, cache, config and D-Bus. Use a fresh `E2E_TMP_DIR` each run,
a UTF-8 locale and `xvfb-run -s "-screen 0 1920x1080x24"` on Linux. The window
journey records its failure and proceeds to an independent typing page so a
notification defect does not hide the other proof. A failed journey exits 1.

Acceptance and timings: `scripts/cli-write-acceptance.mjs APP SOURCE OUTPUT`.
SOURCE is copied read-only; OUTPUT must be new. Ten fixed add/edit/prop/move/delete
cycles make exactly 50 writes on pages and journals (Org too when available),
including a page create. Each cycle removes its own synthetic subtree. Every
pre-existing file is compared byte for byte, only the intentional page may be
new, and doctor runs before and after. Another warm-up and 30 cycles measure
fresh CLI processes against the same owner. The report contains counts and
timings only. Never commit the disposable graph or print its content.
