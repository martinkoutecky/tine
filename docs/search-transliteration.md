# Page-name transliteration adapters

Native page search owns relevance. Optional expansions use the `Transliteration`
class, below Exact, Prefix, Substring and Fuzzy and above BodyEvidence. Adaptive
launcher ranking may reorder only inside that band. Persistent queries and block
matching receive no expansion.

`crates/tine-store/src/transliteration.rs` owns adapter selection and bounded
`NameKeys`; add an enum variant, its eligibility rule and key builder there.
The scorer consumes `cache_identity` and keys without knowing the script or dictionary. Mandarin uses the
MIT-licensed `pinyin` 0.11 crate with only its `plain` feature (no phrase dictionary
or heteronyms). It emits exactly two toneless keys per Han-containing normalized
name: full Pinyin and initials. Each character uses the dictionary's default
reading. Latin text is preserved; whitespace, hyphens and apostrophes are omitted.
Only ASCII Latin queries expand. Matches are prefixes or contiguous substrings,
never subsequences. Kanji receive Mandarin readings, not Japanese romaji.

`ReadSnapshot::build_name_keys` builds keys before publication using existing
title, alias and explicit reference-name projections. The cache is ephemeral,
bounded by names in that snapshot, shares unchanged keys with the prior snapshot,
and is rebuilt after checkpoint restore. Latin-only publications reuse the
empty cache after inspecting only the changed pages’ parser-owned names. No persisted shape changes. Search does
no dictionary work or cache mutation. Latin-only strings allocate no keys.

The request-local flag defaults off for legacy callers. Frontend device preference
`pinyin-page-matching` is in `launcherRanking.ts`; Settings and its messages live
in the existing Editor/Advanced area and Fluent catalogs. Mock and published web
backends accept and ignore the flag; the wasm parser has no dictionary dependency.

For each adapter add multilingual and mixed-script fixtures, native class
precedence and collision tests, a default/ambiguous reading policy, negative/regex
behavior, aliases, referenced names, invalidation/checkpoint tests, and preference
plus launcher component tests. Keep fixed-baseline native latency, cache memory
and release binary size evidence with the lane receipt. Do not add whole-graph
or per-keystroke transliteration or change the query dialect.
