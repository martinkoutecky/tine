# GH #137 reference-matching evidence — 2026-07-16

## Verdict and boundary

The reporter's continuous-text observation was a real v0.5.9 parity defect. For
the demonstrated **alphanumeric-ended canonical titles and aliases**, Logseq OG
1.0.0 accepts a match beside Chinese, Japanese, Thai, `_`, accented Latin, and a
single bracket, but rejects it beside an ASCII letter or digit. Current Tine has
matched that bounded behavior since `dd77a629d7398868919e2cba1ec9bb585f0c17a9`.

This is evidence only. It does not resolve the reporter's private aggregate
count discrepancy, prove global equality for every page-name endpoint/context,
authorize another semantics change, or justify closing #137/removing
`needs-info`/adding whole-issue `fixed-on-master`.

## Provenance and source result

- Full #137 body, all eight comments, and the label/project timeline were read
  through the latest event, the reporter's 2026-07-15 08:00:33 UTC continuous-
  text comment. The issue remains open with `bug` and `needs-info`.
- OG source was read at the required revision
  `6e7afa8eb040686ff057156ee877193b581dd369`. In
  `src/main/frontend/db/model.cljs:1320-1350`, Unlinked References constructs a
  case-insensitive regex for the canonical page and every alias:

  ```clojure
  (?i)(^|[^\[#0-9a-zA-Z]|((^|[^\[])\[))<escaped-name>($|[^0-9a-zA-Z])
  ```

  This is substring-like matching with **ASCII-only** edge exclusions, not
  whitespace tokenization and not unrestricted substring matching. The special
  left branch admits one `[` but not `[[`; it also suppresses `#<name>`.
  Linked References is separate: `model.cljs:1227-1283` reads parsed
  `:block/path-refs` for the canonical page/alias entity set.
- Current Tine HEAD was
  `f6f6de13e1180f61ce8d54267d164ffe994eeae7` (tree
  `98e2b6f23f59d50b26d8e8a47225865d533c56bc`). Its reference/parser paths are
  unchanged from the exact native-UI candidate `15bbddc0c5596c3fa72e84c4f3ad90c722db81a0`
  and have no working-tree edits.
- Tine pins lsdoc `v0.5.3` at
  `96e909634ce247bd261851c04a837096b4ccef7f`. lsdoc emits source spans for plain
  text, `[[page]]` links, and `#tags`; Tine's
  `reference_evidence.rs:184-367` turns those parser nodes into eligible plain
  ranges and explicit occurrences. Code/verbatim and other opaque nodes are not
  plain ranges.
- `reference_evidence.rs:376-431` now rejects only adjacent ASCII
  alphanumerics when a needle endpoint is alphanumeric. The v0.5.9 source at
  `660879a3aee9ae9f00eccfee2bc6996886479c41` instead rejected every adjacent
  Unicode alphanumeric plus `_`, which caused the continuous-CJK/Thai omission.
  Canonical titles and aliases share one normalized name set in
  `query.rs:388-467`; explicit and plain panels then use that set separately.

Documentation caution: ADR 0043's older phrase “Unicode-alphanumeric
boundaries” no longer names the exact predicate. Current code and diagnostics
use OG ASCII-edge semantics and rule `plain_og_boundary`.

## Exact witness

One canonical six-file synthetic fixture (manifest SHA-256
`eee0d3d423215a4e09bd53a10c7aa060c7544f414a6b40afb8bfdc4d4d0f4e9a`) was
observed through the official OG 1.0.0 Linux app and an exact clean Tine native
UI receipt. Membership was compared by stable source labels with multiplicity;
panel totals alone were not used.

| Target / alias | Linked rows | Unlinked rows | Excluded from both |
| --- | --- | --- | --- |
| `Target` / `Latin Alias` | `#Target`, `[[Target]]`, `[[Latin Alias]]` | `北京Target北京`, `東京Target東京`, `ไทยTargetไทย`, `前Latin Alias後`, `_Target_`, `éTargeté`, `[Target]` | `aTargetz`, `1Target2` |
| `目标` / `中文别名` | `[[中文别名]]` | `北京目标北京`, `前中文别名後` | — |
| `ターゲット` / `日本別名` | `[[日本別名]]` | `前ターゲット後`, `前日本別名後` | — |
| `เป้าหมาย` / `นามแฝง` | `[[นามแฝง]]` | `ก่อนเป้าหมายหลัง`, `ก่อนนามแฝงหลัง` | — |

Thus OG does support continuous Chinese/Japanese/Thai neighbors for this family.
Latin boundaries still exist, but they are specifically ASCII letter/digit
boundaries. Explicit links/tags remain Linked, while a plain canonical name or
alias follows the same Unlinked edge rule.

The parity rule deliberately carries false-positive risk: OG accepts identifier-
like `_Target_`, accented/non-ASCII adjacency, and `[Target]`. Replacing it with
unrestricted raw substring matching would be broader still and would bypass
Tine's parser-owned code/syntax exclusions. A different Unicode segmentation or
minimal-exclusion policy is a later product decision requiring exact positive
and false-positive siblings; this witness does not choose it.

## Focused checks run

- Fresh isolated OG UI run: PASS. Official package SHA-256
  `eef58b152b48fbf12630c67c53b0f9083d67b406c2a9005eac5455f9539e1e79`,
  released source `680b371f0f53968adf678759e9bfd5cb7ea5790c`, exact row matrix above. The only
  console errors were the harness-classified ResizeObserver/global-handler/
  Sentry echoes; no unclassified error occurred.
- Paired comparator: PASS. It matched that fresh OG receipt to retained Tine
  exact-clean candidate `15bbddc0` / tree `135ec0f6664e35fa37507bb2eb981cd11f16d5b1`
  / binary SHA-256
  `739453c03c358f97d12c87cc1cded91f489f3a7632aad510c540ada6a22c912a`.
  Fresh OG receipt SHA-256:
  `8de64d74fb8eaa341a1e3d08bea96c033384125720d850d45e5ddf92032c7af0`;
  retained Tine receipt SHA-256:
  `d799272b8dfc323252bd53eb008f0fe8a8c2fa19d9feef52a52f50002a440727`;
  clean-build receipt SHA-256:
  `fbafe9ab6d2c7f7deafd8678a1352147ec8fd63c17c14f24495c44b54bca482f`.
- Harness self-test: PASS; extra row, duplicate label/distinct identity,
  duplicate row identity, and duplicate source group were all rejected.
- Cleanup self-test: PASS; all five injected OG/Tine failure points exited
  nonzero with zero temporary-runtime residue.
- Current HEAD focused Rust tests: PASS, 5/5
  `reference_evidence::tests::*`, including continuous Unicode/underscore versus
  ASCII-neighbor boundaries, explicit/plain separation, parser ranges, code,
  and bounded construction. PASS, 1/1
  `query::tests::canonical_reference_evidence_keeps_mixed_alias_occurrences_and_properties`.

No app build, release, deploy, GitHub mutation, or private-graph read occurred.
The literal Windows UI and a final v0.5.10 artifact were not observed; confidence
for the reporter's Windows context is therefore moderate. Punctuation-leading/
trailing names, normalization/case-folding edges, and contexts outside the
fixture remain later-batch questions.

## Proposed public comment

Thank you — your continuous-text example exposed one concrete parity gap in the
v0.5.9 source. Tine was treating every Unicode alphanumeric character and `_`
as a word neighbor, while Logseq OG 1.0.0 excludes adjacent ASCII letters and
digits for the alphanumeric-ended titles and aliases demonstrated here. That
could omit a page title or alias inside continuous Chinese/Japanese/Thai text.
This bounded family was corrected on master in `dd77a62` and is expected in
v0.5.10.

I ran one byte-identical synthetic input fixture through the official OG 1.0.0
Linux app and an exact clean Tine build. I asserted actual rendered row
identities, not only panel totals. Both classified `Target`/`Latin Alias` as
Linked = explicit alias, explicit title, and hashtag; and Unlinked = Latin title
inside Chinese, Japanese, and Thai text, the plain alias inside continuous text,
`[Target]`, `_Target_`, and `éTargeté`. They also matched exactly for the
Chinese, Japanese, and Thai canonical-title/alias pairs: one Linked and two
Unlinked rows each. `aTargetz` and `1Target2` were excluded from both panels.

This pins the demonstrated behavior: continuous Chinese/Japanese/Thai
neighbors, `_Target_`, `éTargeté`, and `[Target]` are Unlinked in this family;
`#Target`, `[[Target]]`, and explicit alias links are Linked. Tine still uses
its parser-owned syntax/code exclusions. This comparison does not claim that
Tine and OG are globally identical for every possible page-name endpoint;
punctuation-leading/trailing names are not covered by this witness.

Ctrl+K is a separate page/content-search path in both Tine and OG, so a larger
ordinary-search set does not itself identify which Unlinked row is missing.

This is strong Linux/source evidence for the demonstrated continuous-script
subset, but it is not a claim that every private-graph count discrepancy is
resolved, and I have not run this exact UI comparison on Windows. I am keeping
#137 open. Once v0.5.10 is available, please comment with one exact missing or
extra occurrence if the discrepancy remains: target title/alias, source block
text, panel, expected classification, Tine version, and OS. A minimal anonymized
graph is ideal.

Sol (working on Martin's behalf)
