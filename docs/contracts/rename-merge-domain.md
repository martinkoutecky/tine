# Rename and merge domain

Prior decisions: follow simultaneous namespace substitution, fixed calendar
days, source-day alias retention, complete text accounting, native parser
ownership, and unchanged-save closure. The supported syntax and layout domain
is frozen. Unsupported candidates refuse before publication with a reason;
further syntax expansion is not promised.

A merge retains both bodies and every protected original byte with its
multiplicity. Its active aliases are the survivor-first union of native
parser-decoded names after substitution. Comparison is exact and case sensitive.
The first original spelling supplies each member. Alternative spellings that
remain physically distinct survive as interpreted block properties. For example,
Markdown `C\#` and `C#` produce one active `C#` alias, in the first spelling,
with the other spelling retained in a block property.

The parser decides whether a quoted token is a member: `"literal"` in a mixed
list can be a name, including its quotes. A wholly quoted value is protected
literal text and has no members. Wrappers outside a native name span remain
protected with that member's owner.

Existing target Org drawer frames remain in order. Aliases augment the selected
native field; new non-alias properties enter the last physical segment. Source
titles, conflicting properties and protected member remnants remain native block
properties outside quotes. Only complete, parser-owned source drawer delimiters
whose entries all joined or were independently consumed may be consumed. No
target or retained day delimiter gains this permission. Delimiter indentation
uses exactly space, tab, form feed or Ctrl-Z; matching is ASCII case insensitive,
opening needs a physical terminator and closing can end at EOF.
Distinct first alias owners in different fixed target frames cannot supply one
active field without leaving their native frame; this refuses as `Q-OWNER`.
Equal-name duplicates across those frames can still merge under the alias rule.

An active alias field retains its own selected declaration's terminator.
Other inserted lines use the survivor's convention. LF, CRLF, lone CR and mixed
original endings are covered when the complete candidate passes validation.
Memberless target declarations stay in place when neither input has members.
Unchanged saves must preserve the full native document; Org also requires exact
serialized bytes. This does not promise protection after subsequent authoring
edits, or closure under Logseq's trimming writer.

Move contents keeps the day's whole native alias declaration beside its stub;
the destination gets the notes. The retained representation requires one native
header block for day aliases; multiple physical frames in that logical header
are allowed. Moving into the day's own alias keeps its existing preview rule.

Each failed candidate leaves the transaction unchanged. The following reasons
describe the frozen boundary, including imported text, incomplete native source
maps, malformed candidates and concurrent changes:

| Reason | Refusal boundary |
| --- | --- |
| Title identity (`Q-TITLE`) | The actual rebound native title names a different page from its namespace image, including a stationary namespace whose inner reference changes. References in titles alone do not refuse; case-only and unrelated-reference twins are covered. |
| Legal encoding or partition | Required edits cannot be represented by the supported nonoverlapping native name spans and encodings, including an edit outside a title producer's kept tail. |
| Whole-day header representation | Active day aliases span distinct native header blocks and cannot be retained by the single-header certificate. |
| Missing native name range (`F-LINK`) | A required member lacks parser-owned inner-name coordinates, such as an escaped linked Markdown name. Plain escaped members with truthful maps remain covered. |
| Fused line ending (`Q4`) | Assembly would form CRLF from unrelated original CR and LF coordinates, or an inserted half would borrow an original ending. Mixed endings alone do not refuse. |
| First spelling or owner validation (`Q-OWNER` for lost native ownership) | Native field/range or member/owner cardinalities disagree, or the active field selects a later decoded-name representative. A spelling collision itself is allowed. |
| Native extraction or geometry | Complete truthful native range, role, member, container or outline observations are unavailable, or their bounds, UTF-8 cuts or original identities fail validation. Multiple physical drawers alone do not refuse. |
| Text, meaning, assembly or query validation | The actual candidate fails complete-text, native member, occurrence, body/origin, inserted syntax/ending or indexed-query validation. All gates apply to the same output. |
| Unchanged save validation | Native read/classification/serialization/reparse fails document equality, nonpromotion or exact Org bytes. The writer must choose a safe frame or refuse. |
| Semantic safety or unavailable choice | The existing rules reject ambiguous physical claims, unsafe destinations, incompatible formats, forbidden date mutation, or an unavailable unique source. |
| Host publication | Readable-page bounds, native path allocation, base revisions, locks or the audited transaction fail. No partial write is authorized. |

The mathematical guarantees depend on truthful native extraction and fixed
original partitions, identifying spellings for physical members, actual codec
observations, legal writer construction, opaque-query evaluation and audited
host publication. Bounded model examples establish those supplied examples;
they do not prove every production parser or writer can construct a certificate.
