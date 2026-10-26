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

`ReadSnapshot::build_name_keys` updates a persistent tree from the publication's
changed page paths. Per-page sets of parser-owned titles, aliases and references
contribute ownership counts; removing one owner cannot remove another's keys.
Unchanged tree structure and keys are shared. A body edit inspects only the
changed documents' old/new names and retains the cache root when names do not
change. Checkpoint restore builds this ephemeral cache once; no persisted shape
changes. Latin-only strings allocate no keys.

Inline completion holds supplemental keys for its original block under a unique
editor session, shared across publications. The first request parses that
baseline and reuses published keys; later keystrokes perform no dictionary work.
`quick_switch` with limit zero releases the session on completion close, trigger
replacement or component disposal. An IPC reply arriving after close releases
again, so a late blocking query cannot retain obsolete names. Both native
commands return transient completion match classes; `orderAcItems` keeps every
native row and its existing Create policy ahead of the transliteration band.

The request-local flag defaults off for legacy callers. Frontend device preference
`pinyin-page-matching` is in `launcherRanking.ts`; Settings and its messages live
in the existing Editor/Advanced area and Fluent catalogs. Mock and published web
backends accept and ignore the flag; the wasm parser has no dictionary dependency.
Persisted changes use the same Tauri event propagation pattern as UI language;
each open WebView rereads the device preference, and Capture also refreshes on show.

For each adapter add multilingual and mixed-script fixtures, native class
precedence and collision tests, a default/ambiguous reading policy, negative/regex
behavior, aliases, referenced names, invalidation/checkpoint tests, and preference
plus launcher component tests. Keep fixed-baseline native latency, cache memory
and release binary size evidence with the lane receipt. Do not add whole-graph
or per-keystroke transliteration or change the query dialect.
