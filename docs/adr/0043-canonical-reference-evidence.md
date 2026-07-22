# 0043. Canonical reference occurrences and evidence

- **Status:** Accepted
- **Date:** 2026-07-14

## Context

Linked References used lsdoc's parsed reference set, while Unlinked References
lowercased the raw block and ran a separate byte-oriented word search. The two
paths could disagree about code, escaping, Unicode boundaries, aliases, and a
block containing both an explicit link and a separate plain mention. They also
returned only blocks, so the UI had to show the entire block and could not say
which occurrence caused inclusion. A second diagnostic matcher would make that
drift harder to detect rather than easier.

## Decision

Tine has one reference-occurrence engine over the cached lsdoc projection. It
returns source-addressed occurrences, not a second derived result set. Every
occurrence carries the matched canonical page, the matched title or alias,
`explicit` or `plain` classification, a UTF-16 source span in the block's raw
text, and a stable rule identifier. A result remains one block; multiple
occurrences are evidence on that block and never duplicate its row.

Explicit membership follows lsdoc's page-reference projection. Source spans
come from the same parser nodes and are mapped from Tine's re-bulleted parse
input back to the authoritative block raw text. Plain membership is searched
only within parser-recognized visible text spans, uses Unicode-alphanumeric
boundaries, and excludes ranges already owned by explicit references. Thus a
block may correctly contribute to both surfaces when it contains a link and a
different plain mention. Code, verbatim, opaque export/comment/drawer content,
and escaped syntax do not become plain occurrences merely because their raw
bytes contain a page name.

Linked and unlinked APIs return the same evidence-bearing group shape. Page
property references use a synthetic stable block identity and the same engine.
Alias resolution, self-page exclusion, grouping, ordering, and scoped cache
invalidation share the same canonical-name set. Diagnostic output is a verbose
view of these exact occurrences and exclusions, versioned as
`reference-evidence/v1`; it is not independently matched. Any Help-with-Tine
export remains target-scoped, explicit opt-in, and passes through verified
anonymization.

A slow test oracle reconstructs reference occurrences from parser events and
source spans without using the production projection cache. Fixtures cover
aliases, mixed explicit/plain mentions, properties, nested and UUID-shaped
syntax, Unicode boundaries, code and escaping, multiple occurrences, block
counts, and invalidation.

## Consequences

Reference panels, search excerpts, counts, exact-occurrence navigation, and
diagnostics can share one reason-for-inclusion contract. The wire payload grows
only for matching blocks and is bounded per block. Existing callers that only
need blocks can ignore the optional evidence field.

The unresolved aggregate discrepancy in GitHub #137 still needs a concrete
disputed source occurrence. This architecture removes known ambiguity but does
not claim an unknown private-graph difference is fixed.

