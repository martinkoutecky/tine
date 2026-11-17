# Rename and merge domain

Prior decisions: follow simultaneous namespace substitution, fixed calendar
days, source-day alias retention, complete text accounting, native parser
ownership, and unchanged-save closure. The supported syntax and layout domain
is frozen. Unsupported candidates refuse before publication with a reason;
further syntax expansion is not promised.

A reference inside another reference's name cannot change independently:
renaming `Old` leaves `[[a [[Old]]]]` and a glued tag such as
`#[[Old]]Tail` unchanged, and reports the inner site as unspellable (O7).
The enclosing reference follows its own name: `[[Old/[[Old]]]]` becomes
`[[New/[[Old]]]]` when renaming `Old` to `New`, with the inner site left and
reported. Ordinary references in the same file still follow the rename.
Reports identify the input file, native byte span and site kind. The reader
supplies the decoded name and native name slot, so an escaped spelling
written by one rename remains eligible for subsequent renames.
Macro arguments decide enclosure from all reader evidence before selecting
their page-link rewrite targets. A candidate spelling must read as the new
name with exactly the inner sites the substitution retains: replacing the
whole name removes its old inner sites; a namespace substitution retains
those in its untouched suffix. Markdown escapes are used where they give
that reading; a site with no faithful spelling stays and is reported.

A merge writes what the conceptual model's merge rule O6 says
(`cm-cut-model-r6`). The survivor keeps its name, path, header and blocks; no
survivor byte moves into the leftover block. Every original byte keeps its
multiplicity except what P4 lets a merge drop: an alias member that reads as a
kept one, the syntax of combined alias declarations, and layout (blank header
lines, structural prefixes and property-drawer frame lines).

The alias declarations with members of both files become one: the survivor's
first such declaration, else the source's, keeps its key and spelling and
receives the other declarations' kept member tokens. Members are compared as
the reader identifies the page a name opens: a journal day by its day,
any other name by its page key (case, Unicode normalization). The first spelling
stays, survivor's first. For example, Markdown `C\#` and `C#` produce one `C#`
alias in the first spelling, and nothing else. A declaration without members
is header text: the survivor's stays where it is, the source's is a leftover
line. A combined declaration whose bytes outside its member tokens are not
alias syntax (a key, separators, link and tag wrappers, spacing) refuses,
since dropping them would lose text. This is a writer failure to realize O6:
the interface names the file and asks for a bug report, as for P2.

The parser decides whether a quoted token is a member: `"literal"` in a mixed
list can be a name, including its quotes. A wholly quoted value has no members.

A source property joins the survivor's header when neither the survivor nor an
earlier joined entry has its key, keys compared as the reader compares them
(`property_key_norm`). In a rename-selected merge both files are read after
the name map, before allocation: a key the rename rewrites is compared as its
new key, and the source's own title, written as a leftover line, names the
destination (the title's one producer, `RenameProposal::rebind_title`).
Existing target Org drawer frames remain in order;
joined drawer entries enter the target's last drawer, else one new drawer.
Source titles, clashing or repeated properties and other source header lines
are the leftover lines, in order, each its own line without its structural
prefix, a drawer entry included. A leftover block exists only when there are
leftover lines: Markdown `-` and the lines indented under it, Org a headline
`*` and the lines inside `#+BEGIN_QUOTE`/`#+END_QUOTE`. Blocks under the source's
header hang under it. The leftover lines must keep their references: what
Tine's reader reads in the leftover block (kind and name) must equal what it
read on those lines where they came from (a drawer entry inside its drawer),
else the merge is refused (model `leftoverReference`). An Org `#+key:` line
is still read in the quote; a drawer entry, outside its drawer, is not.

Inserted lines use the survivor's line-ending convention. LF, CRLF, lone CR and
mixed original endings are covered when the complete candidate passes
validation. Unchanged saves must preserve the full native document; Org also
requires exact serialized bytes. This does not promise protection after
subsequent authoring edits, or closure under Logseq's trimming writer.

Move contents keeps the day's whole native alias declaration beside its stub;
the destination gets the notes. The retained representation requires one native
header block for day aliases; multiple physical frames in that logical header
are allowed. Moving into the day's own alias keeps its existing preview rule.

Each failed candidate leaves the transaction unchanged. The following reasons
describe the frozen boundary, including imported text, incomplete native source
maps, malformed candidates and concurrent changes:

The window receives named planner, check and merge refusals as typed data,
localized by its Fluent catalogs. Error toasts stay until dismissed. A P3 refusal
names the on-disk file and offers **Save it and rename**: on the user's click,
the page host saves exactly that file without an edit through `PagePlan::save_page`
(normal preparation, revision guard, lock, temp write, fsync and rename), then
repeats the original rename or file merge with the same confirmed choices. The
lower bound is one extra page read and ordinary save, then the existing rename;
one IPC carries both steps. There is no automatic loop. Another refused file
requires another click; a failed save prevents the repeat. These are two legal
operations, not one atomic repair: a successful save remains if the subsequent
rename refuses, fails, or the app closes between the two steps. P2 says Tine's
own check failed, names the file, and asks for a bug report without a retry.
The plain toast surface names files as text; it does not support inline links.

The CLI has no standalone unchanged-save verb. Its P3 human message names the
file and asks the user to open it in the app, save once and repeat the rename.
P2 asks for a report. Both keep `code: refused`, `detail.reason: rename-refused`
and `detail.admitted: false`, and add `detail.guarantee` (`P2` or `P3`) and
`detail.file` (the graph-relative on-disk path).

`RewriteBlocked` carries the remaining pre-write context: the two files for
incompatible formats, the file with conflict markers, the duplicate file in a
self-merge, stale source or survivor selections, an unreadable file, an Org
file that cannot be saved unchanged, or several files claiming the selected
source page. The window uses the existing sticky toast and catalog
boundary; CLI messages name the same files and legal next step. These are
existing refusal conditions, not additional restrictions.

`MergeRefused` retains the exact realization reason and its provenance for
diagnostics. The writer attaches the source or survivor while reading that
input; checks of assembled output name the survivor. `BodyLaw`,
`DirectiveEscape` and `MissingNativeNameRange` can occur in ordinary files.
Each source root supplies its own indentation offset for its subtree. Removal
is bounded by each block line's reader-owned layout: leading whitespace before
the marker, or the continuation prefix returned by `doc::continuation_raw`.
The next root chooses its own offset. The writer compares every placed block's
raw content, children and order against its native input content slices with
legal trace substitutions applied. The survivor body, prose and the fixed
leftover payload are checked too, using the existing assembled-output parse;
the allocated header's fields remain the independent model check's question.
Only layout bytes are consumed, through the existing trace; P2/P3 still check
the result. Prefixes crossing a rewritten name span and remaining unrealizable
body placements are explained limitations.
They explain the writer's limitation and a legal next step in the named file:
align top-level blocks, move or remove the stray Org end-quote line, or rewrite
the affected alias declaration as plain comma-separated names. They never ask
for a bug report. `check_refused` classifies `FusedLineEnding` and other trace
failures as internal checks, with no retry action. A trace's `TitleIdentity` is O4's named
title refusal instead: changing a title's references would change its page
identity. It explains how to edit that title before retrying. The other named
merge refusals remain header children, leftover references and survivor identity.
Classification uses variants, never error-message text. These mappings add no
file reads, parses, writes, locks or IPC calls. Normalizing an indented source
uses the native coordinates from the header's existing outline read. Body
acceptance costs O(input and output body bytes plus trace pieces), without an
extra document parse or I/O, and holds at most one block's comparison text.
The read-only rename-question command preserves the same typed refusals as
the rename and file-merge commands. The window localizes a refused question
before reaching any rename call. Preparation can save pending edits before a
later refusal: messages promise only that nothing was renamed or merged,
never that no file changed. The flush policy and both output checks stay intact.
Transaction preflight attaches the original input file to a bare text-certificate
failure before discarding its step index. The same typed realization boundary
therefore covers non-merge rename failures, including a stationary title whose
references would change its page identity. No file is guessed from the rename's
source; a failing referrer's own path survives to both callers.
The planner's duplicate-destination guard is also an internal check failure:
O1's namespace substitution and the reversible filename codec cannot map two
uncontested source identities to one destination path.

| Reason | Refusal boundary |
| --- | --- |
| Title identity (`Q-TITLE`) | The actual rebound native title names a different page from its namespace image, including a stationary namespace whose inner reference changes. References in titles alone do not refuse; case-only and unrelated-reference twins are covered. |
| Legal encoding or partition | Required edits cannot be represented by the supported nonoverlapping native name spans and encodings, including an edit outside a title producer's kept tail. |
| Whole-day header representation | Active day aliases span distinct native header blocks and cannot be retained by the single-header certificate. |
| Missing native name range (`F-LINK`) | Malformed or externally edited input: a required member lacks parser-owned inner-name coordinates, such as an escaped linked Markdown name, or a combined alias declaration holds text that is neither a member nor alias syntax. Plain escaped members with truthful maps remain covered. |
| Fused line ending (`Q4`) | Assembly would form CRLF from unrelated original CR and LF coordinates, or an inserted half would borrow an original ending. Mixed endings alone do not refuse. |
| Header children (`HeaderChildren`, model `headerChildren`) | Blocks hang under the source's header and the header leaves no leftover line to hold them: writing would lose their parent. |
| Leftover reference (`HiddenOccurrence`, model `leftoverReference`) | Tine reads the Org leftover block's references differently from the leftover lines where they came from (a clashing drawer entry with a reference, its key being one, loses its drawer). Writing would lose or add references a user made. |
| Survivor identity (`SurvivorIdentity`) | The survivor's own header, written as a preamble, would give it another title on the next read (a childless first-root `title::`). |
| Model check (`R-RENAME-CHECK`, P2/P3) | A written file (the survivor, a referrer, a moved file) does not read as the model expects from its old reading, or Tine's own unchanged save would rewrite it; checked on every step's final bytes by `tine_core::cm_check`, which shares no code with the writers. It covers the survivor's united alias members in one declaration (the merge writer's former `AliasField` re-read). |
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
