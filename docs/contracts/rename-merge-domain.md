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
since dropping them would lose text. The interface names the file and affected
declaration and asks for plain comma-separated alias names before retrying.

The parser decides whether a quoted token is a member: `"literal"` in a mixed
list can be a name, including its quotes. A wholly quoted value has no members.
Alias allocation reads `reference_evidence::linkable_property_members`, the
native-slot view of the index and observation's `linkable_property_references_in`.
It uses the reader's decoded names, native name runs and complete member tokens.
A file, search or asset link in an alias value is no member (Logseq's reading),
so `alias:: [x](A.md)` alone is a declaration without members, and in a
combined declaration such a link is text that is neither a member nor alias
syntax. A macro is read with macros skipped, so `alias:: {{embed x}}` declares
no member either and merges as other header text. A member without a
contiguous native name run (a Markdown tag with an escape, `#a\_b`, which
declares `a_b`) refuses as `MissingNativeNameRange` (`aliasSpelling` in the
window). Duplicate
members' complete native tokens are consumed by their reader identity; only
bytes between tokens are delimiter syntax.

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

**Rename helpers** (D-20261115-HELP, D-20261116-TAGH) are the window performing
ordinary page edits before one ordinary rename or merge request; the model is
unchanged. The rename question (`rename_offer`) is the only source of what a helper
may write: for a merge of two pages with `tags::` lines, the kept page's `tags`
value (the union, source spellings appended in order) and the removal of the
source's `tags::` line; for the alias dialog's "use the name for this page", one
`alias` value per other declaring page, the typed name removed. The dialog lists
the typed name's alias component (`InventoryScope::AliasGroup`) and offers one
ordinary merge per component page other than the source that the planner allows,
plus "use the name"; a refused request is listed with its reason, never offered.
Each merge choice is planned as that exact merge on the view the typed request's
refusal refreshed: one graph-wide stamp walk per dialog, whatever its choices.
Each "links open a different page" row is a name whose page changes under the
planned request, asked through the store's one owner rule (`NameOwners::opens`:
the first real-file claimant, else the declarer with the least path), which
`resolve` itself uses. The request changes a name's owner inputs only through its
files: each moved file (children included) claims its new name and declares its
aliases from its new path, a merge's source goes and its declarations join the
survivor, a declaration of a renamed name (a page with no file included) is
respelled to declare the new name (a declarer the request skips for VCS conflict
markers keeps declaring the old one), and "use the name" drops the typed name from the
other declarers. The names asked are the aliases the moved and merged files
declare, the names the request gives, and the released typed name, minus the names
the request rewrites. A moved page is the same page; a merged source is not. A name
with no owner after the request opens its page with no file, and the row shows the
name. The row lists the pages whose links name the name in any
spelling (a day: every rendering of it), found through the reference index, under
the names they have after the request; a page's own alias declaration is the owner
input, not a link, so a declaring page (or the merge source) is listed only when the
parser's reading of its file (`cm_check::observe`: every site, Org headline tags
included) has a site of the name outside its alias declarations. Cost per name: one
owner-inputs read, two rule calls and one referrer lookup over its spellings; each
declaring file is read and parsed once per request, however many of its names are
asked.

A helper writes only through `setPageProperty` and the page host's guarded save
(`flushPage`); there is no other write path. Its pages are pinned in the working set
(`pinPageNearViewport`) before the first is loaded and stay until the request and
any put-back are over, on every exit, so neither many pages nor the request's own
loads evict one; a put-back that finds a page gone counts as failed. Before writing, every page must still
read the value the question read, and the graph binding must be current; that check
happens with no wait before the first write. The edits are then made one page at a
time, in order, each page checked again, written and published before the next is
edited, and the kept page's tag union comes before the removal of the source's
`tags::` line, so a crash between two saves never leaves a tag on no page. After each
save the binding is checked again. If a save fails, a page changed meanwhile, the
binding retires, or the request does not happen (refused, busy, an unsaved page, an
error), the helper puts back its own values in reverse order, on the pages it has
edited so far: each property that still holds the helper's value gets its old value back and is saved before the next;
anything else typed meanwhile stays. A retired binding puts nothing back
(the window no longer holds that graph). An `uncertain` request keeps the edits,
since it may have committed.

| Helper refusal | In-scope threat |
| --- | --- |
| A page changed since the question (`rename-helper-page-changed`) | Typing in the window, an external editor, or a concurrent honest instance changed the property after the plan. |
| A save failed, put back (`rename-helper-unsaved`) | A disk error or a sync conflict during the helper's save. |
| Put-back failed (`rename-helper-not-restored`, sticky, names the pages), after a failed request or a failed helper save | A disk error during the put-back save. |

The helpers are not atomic. A crash, a closed window or a graph switch between the
helper's saves and the request leaves the helper's edits saved and the request not
done; each edit is then an ordinary property edit, readable by Logseq, and the user
can repeat the rename. Cost: one page read per helper page not loaded, one
property write and one guarded save per page, then the request; the question adds
one plan per offered merge and one for "use the name", with linear header reads
(each tag line read once, union membership by set lookup).

**Day move** (D-20261111-CMDF, -MA, -MOA): renaming a day offers to move its
notes to the typed name's page with one cross-page block move, a guarded
two-page host operation. The offer names the receiving file (`path`: the
existing page's, or the file `WholeGraph::proposed_file` says a new page is
created as), and the window holds a new page in that file's format. What
moves: one planner (`dayMovePlan`) answers which root is the retained header
and which blocks move: every root but the page-header block, and the header's
current children, promoted to roots at the destination; the header node stays
with the day, which gains one `Moved to [[name]]` block. The header is decided
once, before any helper edit, so releasing an alias that was the header's only
line does not turn the header into an ordinary block that moves. For the day's
own alias the helper edit releases the alias first; the move itself puts back
only that value, into that header node, and only while the line still reads
the helper's value, inside the same operation, so other typing in the header
survives. When the move does not happen (refused by the host, different
formats, the receiving file changed, a failed read), the helper's put-back
writes the value into that same header node too, under the same guard, so the
day reads as before. Each parent's moved children are detached once. The day
and the receiving page are taken from the working set only when the loaded
copy holds the file the dialog named (none, for a new page); otherwise that
file is admitted into the name's slot (`admitPageFile`, refused when the
loaded copy has unsaved input), so a twin merged away by the contested-name
helper cannot receive the notes. After the confirmation the receiving page
must be the file the dialog named, or still have no file, by the store's name
answer (`resolve`); otherwise nothing moves and the rename is asked again.
A crash after the alias save and before the move leaves the helper state above (the
day without the alias line, its notes all on it). The move drafts the receiving
page's text first, then saves the page, then the day; a crash between them reopens
with the notes on both pages, the duplicate an interrupted move leaves
(`docs/storage-model/spec/SPEC-s2.md` §1), never with a note on neither (proof
`src/renameHelpers.interrupt.test.ts`, and a real kill at every boundary with a cold
relaunch, `page_host/production_move_crash_tests.rs`).
Cost: one read of the destination when not loaded, one name resolution, one
host move, one write per file, plus the alias save for MOA.

| Day-move refusal | In-scope threat |
| --- | --- |
| The receiving file is not the confirmed one (asked again) | An external editor, sync delivery or a concurrent honest instance created or replaced the destination's file while the dialog was open. |
| The day and the page use different formats (`rename-refused-different-formats`) | Malformed input: an Org outline written into Markdown (or the reverse) would not read back as the same blocks. |

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
the marker, or the continuation dedent the reader removed.
The next root chooses its own offset. The document reader returns its own
source map (`tine_core::doc::SourceMap`, built in the same outline pass): each
physical line's native range, the bytes the reader removed in front of each
line it read, each line's offset in its owner's text (pre-block or block
`raw`), and each block's lines and indentation. `doc::native::NativeDocument`
assembles, per header line, the fields, drawers and alias members the reader
found, placed through that map, plus prose and line endings. Merge policy
stays with the writer: its partition cuts, the join terminator, the
removable prefixes of indented roots, the insertion point of joined fields
(after the survivor's last nonblank header line, blank as the reader reads
it) and the output's directive check. The certificate (`text_trace`) owns
which alias fragments are syntax (`alias_nonsyntax`). An Org header entry's
references, which the leftover check compares, are the rename's own reading
of the entry: a field in its original context, and an edited entry or a
non-field line read as it will stand on its own. Input descriptions
come from the rename reader's existing outline pass. Allocation places those
ranges; it does not recognize Markdown or Org. One output read checks the
intended header, block content, order and tree. The writer compares every placed block's
raw content, children and order against its native input content slices with
legal trace substitutions applied. The survivor body, prose and the fixed
leftover payload are checked too, using the existing assembled-output parse.
Every retained header line keeps its own reader-derived structural prefix;
blank layout does not take another line's prefix. A childless Markdown header
becomes a preamble, consuming its structural prefixes and leading layout.
Interior blank lines remain. A removed alias declaration's reader-derived
continuation prefix can be reused by a joined field, before inserting new layout.
Joined fields use their input reader's payload coordinates and the destination's
native header syntax. The writer records allocation's intended logical lines,
including the united alias tokens, and checks the output reader's header for
exact payload, field order, normalized keys and native property forms. Any
unexpected pre-block prose refuses `BodyLaw` naming the survivor. An input's
header fields and its pre-block prose never overlap: both are read from one
answer, the parser's page properties (Markdown: the property lines of the first
top-level property node, when only blank lines precede it, up to its first line
that is no property, and the prose starts after the last of them; Org: the
top-level directives before the first headline and the head drawer, except a
directive or drawer strictly between two lines of other text, and the prose runs
from the first to the last of that other text, so page properties may follow
it). Every other non-blank byte of a pre-block is prose, so it is indexed,
renamed and kept like any block text. That prose is the page's opening text and
is shown exactly as written wherever it appears: the page's opening row, its
Linked References row and a query's List row all render it with the block
renderer (`AstBody` with `opening`, one parse per row), so every construct
renders as it does in a block. The one difference: nothing is lifted into
block chrome. A property node in it (a later Markdown property group, i.e.
lines after the header's blank line, or a `#+key: value` line) shows as its
exact source lines, whole lines from the parser's span with their spacing and
no typography, never property chips or page properties; a leading task word or
priority keeps its text (D-20261119-PG, a deliberate difference from Logseq, which shows
such a group as nothing). While the header is edited, the frontend keeps the
parser's answer (`pageOpening`) instead of splitting the remaining pre-block
again; display, the properties panel, and the copy and edit checks on a
result's header and opening rows all read that one answer. A Markdown `#+key: value` line is prose
whose references count, as Logseq hoists it. The format is the file's, never
inferred from the text.
Header checking costs O(header bytes), uses the same native description as the
body comparison, and retains temporary intended header text until comparison.
There is one outline read per input and one for the output, including mapped
titles; native title coordinates are carried to the title span writer. A Rust
syntax-tree guard allow-lists every method, called function and macro the
merge writer uses (some only on a fixed receiver or argument), and rejects
cfg-attributed production modules, renamed function imports, macro
definitions and byte literals. It has no types and does not look inside
macros or at operators, so it bounds the writer's named operations, not every
possible reading of its inputs.
The writer's acceptance certifies placement: the output reads as the header
intent and blocks it emitted. It does not certify the meaning of what it
emitted: an emitter fault that writes a member outside the alias union
(`alias:: A, B, Ghost`) corrupts output and intent alike. Only P2, the
independent model check, certifies the alias union and every other meaning,
and it refuses such a survivor before any write. P2 independently
checks the final allocated header and body.
Only layout bytes are consumed, through the existing trace; P2/P3 still check
the result. Prefixes crossing a rewritten name span and remaining unrealizable
body placements are explained limitations.
After an Org survivor's final outline block, its run of line breaks keeps one
original terminator at the join; the remaining breaks are consumed as layout.
This preserves the last block's observed content (O6), consuming only
parser-unreported bytes (P4). Pre-block prose emission and its content check use
both endpoints of the input reader's prose range. Markdown's reader excludes
trailing whitespace-only pre-block lines; Org keeps their spaces and tabs and
excludes only edge line breaks. Interior whitespace remains content in both
formats. The original terminator stays as the separator; the remaining excluded
suffix is consumed as layout. Header classification uses
the reader's pre-block or first-root text; leftover lines use that reading's
content coordinates, without terminators or reader-owned continuation prefixes.
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
| Missing native name range (`F-LINK`) | A required member lacks a contiguous parser-owned name run, such as a Markdown tag with an escape (`#a\_b`, member `a_b`), or a combined alias declaration holds text (such as a file link, `alias:: [x](A.md), B`) that is neither a member nor alias syntax. Escaped members with truthful native slots remain covered. |
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
