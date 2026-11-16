# 0077. Unlinked mentions count text inside code, math and other literal regions

- **Status:** Accepted
- **Date:** 2026-10-20

Supersedes the code-exclusion sentence of the Decision in
[ADR 0043](0043-canonical-reference-evidence.md) only. The rest of 0043 stands.

## Context

ADR 0043 said that code, verbatim, opaque export/comment/drawer content and
escaped syntax do not become plain occurrences of a page name. Tine implemented
that by searching only an allowlist of parser block kinds. The allowlist lost
coverage whenever lsdoc gained a block type, and it disagreed with Logseq (OG),
which users compare against (GH #270: a page name written in a code block was
missing from Unlinked References).

Commit 2624c2ef reversed the rule in code and tests but did not update 0043.
Martin decided on 2026-10-19 (journal D-20261019-QC item 4) to record this in a
new ADR.

## Decision

A plain occurrence may appear anywhere in the block source except where the
parser already claimed the text. The search region is the block's raw text minus:

- explicit page references (links, tags, property values that link);
- block references `((uuid))`;
- structural property text (`id::`, `collapsed::`, `logseq.*`), a deliberate
  divergence from OG, which scans it;
- `:LOGBOOK:` drawers (OG strips these too).

Everything else counts: fenced and inline code, math, raw HTML, example and
export blocks, comments, other drawers, hiccup.

Two OG prefix rules apply directly to the raw text, including inside code where
no link is parsed: a match preceded by `#`, or by `[[`, is not plain. So
`` `#Target` `` and `\[[Target]]` give nothing, while `` `Target` `` and a bare
`Target` in a fence do. Match boundaries follow OG's ASCII rule (an ASCII letter
or digit next to the name blocks the match; see
`plain_boundaries_match_logseq_ascii_edge_rules`).

The rule is implemented in `plain_search_ranges` and `og_prefix_allows` in
`crates/tine-core/src/reference_evidence.rs`. There is still one matcher, over
the cached lsdoc projection, with exact UTF-16 source spans; nothing else in
0043 changes (evidence shape, one result row per block, scoped invalidation).

## OG evidence

`get-page-unlinked-references` in `src/main/frontend/db/model.cljs` (OG 1.0.0
and the current checkout) filters every `:block/content` datom with a regex
built by `pattern` in the same file:
`(?i)(^|[^\[#0-9a-zA-Z]|((^|[^\[])\[))NAME($|[^0-9a-zA-Z])`. The only
preprocessing is `drawer/remove-logbook` (`src/main/frontend/util/drawer.cljs`).
It sees raw content, so code and math text matches, and the pattern itself
rejects `#NAME` and `[[NAME`. Blocks on the page itself are removed afterwards.

## Consequences

Unlinked References shows more blocks than before, matching OG, and new lsdoc
block types are covered without another edit here. A page named after a common
word can now match code samples; that is what OG shows too, and the cost is
noise, not data loss. Tests that encoded the old rule were rewritten, notably
`escaped_link_is_not_plain_but_code_mentions_are` and
`unlinked_mentions_include_literal_regions_without_counting_link_syntax`.
Scan time dropped (285 to 36 ms for one lookup on the 1,045-file anonymized
graph) thanks to an ASCII prefilter in the matcher. The Guide already says
Unlinked References includes plain mentions in code and math.
