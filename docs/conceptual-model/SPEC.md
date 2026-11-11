# Tine conceptual model

## 1. Prior decisions and user contract

This specification follows file-based identity, full alias groups, literal name
queries, namespace closure, stored-query rewriting, CM-TWINS, R13-C4,
D-20261107-OWN2, D-20261110-CM (CM-DATE, CM-ORDER, CM-ONESTEP, CM-PLAIN),
D-20261109-AL, D-20261110-CMF and D-20261111-CMR14 (CM-DAYFILE and CM-ALIASDUP).
D-20261106-CMS governs the length measure; D-20261106-MF/UA govern model-first
work and universal guarantees. These decisions are followed. Ordinary files
remain Logseq Markdown or Org. No operation silently merges conflicting files.

**CM-DATEFIXED / M2 is open.** The right to rename a date source and which day
identities must be fixed are parameters awaiting Martin's decision (§4.2).
No default answer is selected. The reader, outside-file changes, ordinary
non-date renames, text protection and journal creation do not require it.

A name can be written in a page link. An ordinary name has its normalized
ordinary key. Every accepted spelling of a calendar date names the same day,
which is a journal identity. A file named by a date under `pages/` is that
journal, at its existing path. A day need not have a file. An alias by itself
does not create an additional empty page.

A real file opens before an alias owner, even when empty. Without a real file,
a name opens its least-path direct alias declarer, or itself as a virtual
identity. Paths are graph-relative unsigned UTF-8 bytes. Physical claimants
prefer the journals directory, then path bytes; direct alias declarers use
path bytes across both directories. Losing claimants remain intact and are
shown as conflicts, but contribute no meaning.

Aliases connect names into undirected groups; they do not combine file text.
A literal name query tests the named identity, rather than following opening.
A name's Linked References list is exactly the list of the page that name
opens. That list excludes that opening page's own source and the configured
Favorites source. Different real members of one alias group can therefore
have different lists and counts. A count is the size of its displayed list.

A namespace mention references its nonempty namespace prefixes as well as
its full spelling. Relative paths and URLs with an origin are not namespaces.
A nonblank active file also supports its effective name's namespace parents.

Ordinary renaming substitutes simultaneously, including children, aliases,
covered occurrences, stored query arguments and the Favorites setting.
For example, A → A/x moves an existing A/x child to A/x/x. The user can choose
any offered safe destination option. A safe ordinary fresh rename with one
option executes in one step, without a choice dialog. The timing of a reference
preview is a UI question outside this specification. An unattended caller
executes only a singleton safe option; cancel changes nothing.

Merging keeps both bodies and protected header text. Its active aliases form
one native declaration with the exact-byte union of the substituted spellings.
Only redundant recognized alias syntax and duplicate rendered alias tokens
with identical retained member witnesses may be consumed (§5.2).

### Prior art

At OG revision `6e7afa8eb040686ff057156ee877193b581dd369`,
`deps/graph-parser/src/logseq/graph_parser/block.cljs:273–284` converts parsed
page names to journal days; `extract.cljc:159–162` applies it to effective
file-page names independently of directory. `date_time_util.cljs:15–41`
supplies the configured title format and title/ISO/underscore fallbacks.
Tine also accepts its configured filename format, per the retained date-list
decision. This is a classification principle, not authority to merge files or
move imports. OG's `src/main/frontend/db/model.cljs:161–184` selects the first
eligible result of an unordered alias query; the deterministic Tine path order
is Tine's decided rule.

## 2. Names, files and graph meaning

### 2.1 Representation and extraction

A graph is a finite list of complete Markdown/Org files. A file has a
relative path, physical directory class, format and raw text. Well-formed
published graphs have unique paths. Readers are total even on conflicting or
malformed snapshots; operation safety checks paths separately. A block's model
identity is `(path, outline position)`, not a new persisted identifier.

An effective pages-directory name is its title property, otherwise its decoded
filename. A journal filename supplies its parsed day independently of its
displayed title; a journal's title property does not rename that day. Thus a
pages-directory file with effective title Work is Work even if its filename
looks like a date. A title that is an accepted date gives that file a day claim.
The physical directory is separate from the semantic identity.

The identity of a non-date name is `(page, ordinaryKey)`. The identity of a date
is `(journal, dayValue)`. Day values are calendar days, not rendered labels.
The same interpretation is used by file claims, references, aliases, queries,
opening, collision checks and journal creation. There is no ordinary-date
claim ahead of date interpretation.

The configured parser supplies accepted-date recognition and journal filename
day extraction. A non-date journal-directory filename has no parsed day and
falls back to the extracted effective filename name. Reading requires no rename
history, and does not refuse a graph because a file moved outside the app.

**Approved label limitation:** with title format `MM-dd`, 2026-11-02 and
2027-11-02 can both display `11-02`. They remain distinct day identities,
openable by their day/path handles. Typing the ambiguous, non-identifying label
cannot select one. Textual self-opening applies when the displayed name
identifies the file's claim. It is not a theorem for all journal labels.

| Supplied boundary | Required interpretation |
|---|---|
| ordinary `key`, `spell` | Configured ordinary normalization; ordinary representative spelling. Date parsing must factor through the literal key used by its adapter. No namespace-congruence law is assumed. |
| `MemberNames.identifying`, `identifies` | Operation adapter supplies an accepted spelling for each physical claim and certifies that its normalized identity equals that claim. It need not equal the displayed label. Reading snapshots requires no such adapter. |
| `Dates.parse`, `journalDay` | Accepted spellings return actual day values; filename extraction independently returns the day or no date. It must not substitute an ambiguous rendered title for the day. |
| `stem`, `title`, `aliasDecl` | Decoded/configured effective names and native header aliases. Bare members are names. A wholly quoted alias value supplies no aliases. |
| `outline`, `occ`, stored parser | Ordered blocks, typed occurrences and stored query arguments, including applicable header and preamble blocks. |
| URL/nonblank/path conversion | URL-with-origin and nonblank tests; graph-relative unsigned UTF-8 bytes. |
| query row/depth parser | Header, preamble and body classification and body ancestry from the same extraction. |
| fixed text partition/classifier | Whole raw input split into legal name-edit, narrow alias-syntax/member and protected spans, independently of the chosen operation. |

### 2.2 Claimants, active files and page rows

Physical claimants compete at the full normalized identity. The selected file
is the least claimant: journals-directory first, then unsigned UTF-8 relative
path. A fixed full-file totalization handles invalid equal-path input. It is
not a configurable ordinary-first policy.

Every physical file remains on disk. Only selected files contribute aliases,
blocks, namespace seeds, references or query rows. Input mutation and text
certificates still cover all physical files, including losing claimants.
Two accepted date spellings can therefore be a visible same-day conflict.
If both `journals/2026_11_02.md` and `pages/2026-11-02.md` claim that day, the
journals-directory file opens; neither is rewritten or merged on discovery.

The page universe contains active real identities, plus namespace-closed
reference/name seeds that have no real file and are not alias targets.
All active real identities remain page rows, including empty ones. An alias
alone has no additional page row. Namespace seeds are derived before their
individual spellings are interpreted as ordinary names or days.

### 2.3 Aliases and opening

Every active file, in either directory, can declare aliases. Each declaration
joins the file's normalized claim to the normalized declared name. The concept
of a name is its connected component in this undirected graph, including the
name itself. Transitive and reciprocal declarations do not create content merges.

Opening an identity returns itself if real; otherwise it returns the claim of
the least-path direct declarer, or the original virtual identity. Opening is
idempotent and stays in the starting name's alias component. A mention references
what it opens. Opening a day physically does not relocate its file.

A non-date page can declare a date alias. Before a physical day file exists,
that date spelling may open the declarer. Creating the journal finds physical
day claimants, not alias owners: it creates a missing day file and thereafter
the real day opens first. This can change opening while preserving the alias
connection. The page/name query remains a test of the date identity throughout.

### 2.4 References and Linked References

A block references a target when one of its namespace-closed occurrence names
reaches the target in the active alias graph. Alias members share this relation.

Panel self is the opening identity, using the same vertex-level function as
navigation. The panel keeps deduplicated active blocks that reference the
requested concept and whose actual source differs from panel self and the
configured nonempty Favorites identity. It does not exclude a whole alias
group. Order is the active block order after deduplication. Counts use this list.
The Favorites setting follows renaming; exclusion uses the resulting setting
and resulting graph, without assuming the old source or alias topology remains.

## 3. Queries, links and presentation

### 3.1 Raw queries and displayed rows

The page anchor universe is §2.2's pages; the block anchor universe is active
blocks. A literal `name(s)` leaf tests the normalized identity s, without alias
routing. A `refs(s)` block leaf tests paths through the block's own and body
ancestors' references plus its containing page. Page wrapping evaluates a page
leaf on the actual containing identity. Supplied predicates evaluate on the
current graph. Conjunction, disjunction and negation evaluate normally over
that anchor universe. There is no positivity or invariant-predicate restriction.

Stored query name arguments receive the same simultaneous substitution as
covered graph names. Their actual output denotations are recomputed.

Displayed query rows filter the complete raw denotation. Header rows are
hidden unless requested. The default child filter removes a body row only if
its immediate body parent is itself a raw hit. A matching grandparent does
not suffice. Header/preamble blocks are not body ancestors. Explicitly disabling
the child filter is honored. Page rows are unaffected. Rendering views preserve
the projected membership; raw hits and displayed rows need not be equal.

### 3.2 Links, neighborhoods and namespaces

A directed link has distinct resulting page endpoints, a source other than
the configured Favorites page, and an active source block referencing the
target. Outgoing and incoming walks repeatedly add neighbors from the finite
page universe. The both-direction view concatenates those walks. Every radius
uses the actual recomputed directed relation.

An immediate namespace parent uses the effective spelling's raw parent and
looks up that full interpreted identity in the page universe. Children,
full trees and the namespace-only tree derive from that lookup. Body parents
and namespace parents are separate. Parent support may change when a merge
makes an empty file nonblank. No fixed-parent-set assumption is imposed.

### 3.3 Actual changes

Added members are after-members absent before; removed members are
before-members absent after. Outside those two sets, membership is unchanged.
This applies to raw and displayed queries, references, panels, links and tree
readings separately. It does not promise that arbitrary Boolean queries,
namespace closure or alias components are transported unchanged by renaming.

## 4. Operations

### 4.1 Substitution, candidates and physical assembly

A request has an old spelling and a destination spelling. The spelling map is
simultaneous namespace substitution using normalized identity equality and the
shared URL/namespace test. It affects the entire operation, including stored
queries. A source file is the unique physical source claimant eligible under
§4.2; a file-less source supplies no content.

The preferred literal candidate keeps the typed destination. It does not
follow an alias to its owner. If a distinct stationary unique file claims that
destination, it is the explicit content survivor. A physically journal-directory
survivor is stationary independently of an ambiguous displayed label. A moving
ordinary own-child is rewritten, not overwritten.

Additional candidates select each distinct real member of the destination
component by its actual identity and an identifying spelling. The operation
adapter `MemberNames` certifies that spelling against the member's physical
claim. Use the effective name when it identifies the claim; otherwise use the
adapter's accepted spelling. The displayed label need not identify the member
and is never reparsed as an unrelated ordinary destination. That one chosen
spelling drives the whole substitution, including namespaces and stored queries.
Every safe member choice is enumerated and offered, independently of whether
its label identifies it. The adapter is an explicit naming boundary; it cannot
be implemented by assuming a rendered-title parse round trip.

For Foo → Shared with two journals declaring Shared and both displaying
`11-02`, full identifying spellings `2026-11-02` and `2027-11-02` select their
respective physical survivors. Both are offered when safe, alongside the typed
Shared choice; unattended execution refuses the multiple choices. Typing
`11-02` still cannot select either day. This does not choose a date-source policy.
Exact duplicate serialized candidates are removed. Different output spellings
can remain different candidates even when they name the same day.

Execution merges only an actual source/survivor pair; otherwise it rewrites
all physical files without changing file count. A content pair removes its
source, keeps the survivor, and rewrites every other retained physical file.
This retained-file map is the actual after-graph. All readings select its
active files again. No hidden content union is performed on opening.

### 4.2 Finite safety and the open date-source policy

Every candidate passes one finite check:

1. The source identity is in play.
2. Demanded identity mappings are consistent: the root, every direct name/site,
   extra/configured name and every fixed anchor agree on one image per input
   identity. Namespace congruence and injectivity are not assumptions.
3. Touched original and image identities have no unresolved physical conflicts.
   Unrelated existing conflicts do not block an operation.
4. A content pair has distinct files, equal formats, a unique destination
   claimant and a stationary survivor.
5. Two retained files cannot newly collide in normalized claim unless their
   original claims were already equal.
6. A moved non-root virtual page descendant cannot capture stationary content.
7. Before and after physical paths are unique.

Physical journal-directory files, task markers and priorities remain fixed
anchors. Marker/priority spellings are not rewritten. The additional fixed-day
policy is unresolved. `DatePolicy.fixed(G, day)` isolates that question:

- A would fix every day, including file-less dates.
- B would fix days with physical files, independent of directory.
- C would fix only days claimed from the journals directory.

There is no selected instance. Under the supplied policy, a fixed date cannot
be a moving date source, and its identity demands stay fixed. The generic
proofs apply to every policy argument; they do not establish a product right
to rename date sources or an unconditional all-date identity guarantee.
Ordinary non-date source behavior does not decide M2.

### 4.3 Choice, cancellation and preview

The choice UI shows the destination component, real file identities, aliases,
virtual versus alias-only names, conflicts and the selected physical claimant.
For each safe candidate it shows the chosen destination, content pair, moves,
changed aliases and stored arguments, and reference effects. The safe literal
option and every safe distinct real-member option are offered, with no
identifying-display-label condition; a preferred unsafe option is absent. The ordinary fresh
singleton case needs no choice dialog. Unattended execution accepts exactly
one safe option. Zero and multiple options refuse; cancel returns the original
state, including the Favorites setting.

Reference preview compares, for each old active block and actual output image,
its closed raw names, interpreted target vertices, opening targets and active
alias edges. Every changed reference membership answer has a preview row.
Extra rows are permitted. Lost and newly active origins are listed separately.
A changed answer can arise from an unedited link. Query-result preview timing
and a general query-result-preview UI are not additional model obligations.

The state operation returns the after-graph and the substituted Favorites
setting together. The setting participates in the same name demands, as an
extra/configured site. Subsequent panels and links read that resulting state.

### 4.4 Ensure a journal

To open/create day d, find physical claimants of `(journal,d)` using §2's exact
identity and claimant order. If one exists, return the selected file and the
unchanged graph, with all claimants available as conflict information. Reuse
works in any directory and does not require the graph's other conflicts to be
resolved. Do not follow an alias owner as the physical day file.

With no claimant, propose an empty native file in the configured journals
directory. Check its parsed claim is d, its path is unoccupied and input paths
are well formed. Add it while retaining every original file byte-for-byte.
Failure returns the unchanged graph with no selected file; never overwrite an
occupied path. Repeated ensure with the same proposal is idempotent. Once a
day is represented, any fresh proposal is ignored and no second file is created.

Thus a pages-directory file produced by Foo → an accepted date spelling is
already the journal. Subsequent ensure reuses it. External tools can still add
another claimant; discovery exposes a conflict and preserves both files.

### 4.5 Outside moves and renames

The reader accepts the new snapshot without rename history or refusal. Moving
`journals/2026_11_02.md` to `pages/Work.md`, with no overriding date title,
keeps the bytes at Work and leaves date references naming the date, normally
virtual if no claimant or date alias remains. Renaming `pages/<date>.md` to a
non-date name has the same result. Renaming a non-date file to a recognized
date makes it the journal and ensure reuses it. An unchanged date title can
continue to make a renamed pages-directory file a journal.

There is no outside-rename guarantee that references follow the moved content.
A later in-app date-page command may offer weaker guarantees than ordinary
renaming. Its design and date-source rules await M2; no exploratory design is
an operation authorized by this specification.

## 5. Text realization and interoperability

### 5.1 Native rewriting and merging

Rewriting preserves directory and format, rebinds ordinary effective names,
and preserves physical journal-directory names/paths. Unchanged literal-name
keys retain paths. Aliases in every physical file, including inactive twins,
and covered occurrence/query name spans use the chosen substitution. Fixed
marker and priority spellings remain unchanged.

A merge keeps the survivor's path, directory, format and effective-name key.
Alias spelling union is survivor-first after substitution, deduplicated only
by exact rendered bytes. X and x remain distinct. The result has one native
alias field when members exist; other properties are not deduplicated.
Markdown uses `alias::`; Org uses native `#+alias:` (recognized native variants
are classified by the parser). The native field parser and alias extractor
must agree with the concrete serialized result.

Every output block has valid origins, possibly multiple origins for a merged
header; each input block/occurrence has an image. The layout is at most one
combined header, target body blocks in order, at most one leftover source-header
block, then source body blocks in order. Bodies remain separate. Source titles,
clashing properties and unknown header text survive as ordinary text;
nonclashing properties join the target header. Equal non-alias properties
remain protected with multiplicity. Stored queries follow the same origins
and substitution. Namespace support comes from the actual merged text/name.

General header parsing and property promotion are serializer boundaries. This
model includes a bounded native alias serializer and concrete certificates,
not a full production Markdown/Org property parser or format translator.

### 5.2 Complete text with exactly the alias exception

A fixed input partition accounts for every raw byte before a candidate is
chosen. Legal editable spans render under substitution. Every other original
span either has an original placement or is explicitly approved consumption.
The original spans form a permutation, with multiplicity, of retained spans
plus consumed spans. Input and rendered-output concatenations equal the complete
physical file texts. Inserted serialization syntax is explicit.

Consumption is confined to the actual merge pair. It permits only:

- Redundant recognized alias declaration keys, separators and line endings,
  with a retained native alias declaration witness.
- A duplicate rendered alias-name token with a retained alias-member token
  whose rendered bytes are identical.

The bounded syntax classifier accepts native alias keys, comma separators and
LF/CRLF. Odd spacing, bracket wrappers, quoted or unrecognized remainders,
examples and non-alias properties are protected unless separately, explicitly
classified as approved narrow alias syntax. They are not discarded with a
whole second line. The concrete prototype keeps them protected.

The complete-text theorem states the retained-or-approved-consumed alternative.
Protected byte multiplicities survive unchanged. The body law additionally
protects block order and separation. Production partition legality and writer
certificate construction remain explicit conformance boundaries; a Boolean
candidate check alone proves neither.

The proof ledger, hypotheses and exhaustive comparison are in
[GUARANTEES.md](GUARANTEES.md). The source/statement inventory and measurement
are evidence alongside the Lean development, not additional product rules.

This is a pure graph/state specification. Publication, crash recovery, locks,
revisions and synchronization use the host's audited storage path. No new
persisted identity, hidden graph database, storage mode or non-Logseq format is
authorized.
