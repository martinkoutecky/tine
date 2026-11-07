# Tine conceptual model

## 1. Prior decisions and the user contract

This specification follows the conceptual-model design's file-based identity,
full alias groups, literal page-name queries, finite page universe, namespace
closure, stored-query rewriting and configured Favorites exclusion. It follows
the decisions in the decision journal D-20261106-CMS, D-20261106-R13,
D-20261106-R13-UI, CM-TWINS, R13-C4, D-20261107-OWN2 and D-20261107-KD. The journal records authority and
provenance; this document specifies the resulting behavior. It follows
CM-COUNTS (reference count equals the Linked References panel size), the
2026-10-13 query top-level-filter decision, and D-20261019-M2(6) for
same-key cross-kind conflicts. Ordinary date pages retain the literal
identities specified by D-20261107-KD. The files
remain ordinary Logseq Markdown or Org files, and an operation must preserve
user text.

A **name** is something you can write in a page link. Different spellings can
have the same literal name key, including configured case handling. File claims
use those literal keys. Reference interpretation additionally recognizes dates
and gives an existing ordinary file precedence at its own literal spelling.
A **file** is the actual Markdown or Org document. A **page** can
have a file, or exist because a link or namespace name uses it. An alias by
itself does not create another empty page.

An ordinary page file is named by its title property, when present, otherwise
by its decoded filename. A journal file is named by its filename's date; a
journal title does not change that identity. A file opens itself when you use
its name, even if the file is empty and another page declares that name as an
alias. When no file claims the name, an alias opens a page that directly
declares it. The choice among several direct declarers uses their current
graph-relative file paths in unsigned UTF-8 order.

Aliases join names into **groups**. If A declares B and B declares C, all three
names belong to one group. Several real files can belong to one group; joining
their names does not combine their text. The reference relation is shared;
Linked References projects it by excluding one self source and the configured
Favorites source, so different members' panels and counts may differ. Opening
a particular real page still opens its own file. A literal
page-name query asks for that name, rather than following an alias to a different
file. Thus a file-less alias can have Linked References without producing a
separate row in a page-name query.

A namespace link such as `[[Work/Projects/Tine]]` also references `Work/Projects`
and `Work`. Those parents can exist without files. Nonblank active files also
support their filename/title namespace parents. Relative paths and URLs with an
origin are not treated as namespaces. Empty prefixes do not create pages.

When two files claim the same name, Tine reports a conflict. Only the file that
the name opens contributes aliases, blocks, references or namespace parents.
The other file remains intact on disk and contributes nothing to the graph's
meaning until the conflict is resolved. It must never be silently overwritten.

Renaming is an operation you choose after seeing its effects. The dialog shows
the destination's whole alias group and every safe option: use the typed name,
or combine the source into any eligible real page of the group. A file-less
source has no content to append. Selecting a group member uses that member's
own name for the entire operation, including children and stored queries.

An ordinary page named `2026-11-06` remains its own page. A journal for that
day can coexist under its configured journal name. Other accepted spellings
of the date mean that journal, or a virtual journal when no active journal file supplies it.
Opening the ordinary page or showing its Linked References uses its own
identity; it is not overridden by a date panel redirect. Explicit aliases can
still join the ordinary page and journal into a shared reference concept.

The preferred option uses the typed name literally. If it is an alias with no
file, renaming an ordinary source file there makes that name its own real page.
It does not silently append the source into an alias owner. If a stationary
file already claims the typed name, the content-bearing option is an explicit
merge into that file. An unsafe preferred option is not offered.

Substitution is simultaneous. Renaming A to A/x moves an existing ordinary
A/x child to A/x/x; it does not overwrite that child. Journal identities, task
markers and priorities are fixed. Other names, aliases, covered references,
stored query name arguments and favorite entries use the chosen substitution.
The preview includes every block reference whose target changes, including
changes caused by aliases and derived namespace parents. Such changes need not
be limited to links whose spelling is edited. An unattended caller executes
only when exactly one safe candidate exists. Cancel changes nothing.

## 2. Inputs and graph meaning

### 2.1 Representation and parser boundary

A graph is a finite collection of files. A file has a graph-relative path,
directory kind (`pages` or `journals`), format (Markdown or Org) and complete
raw text. Distinct files have distinct paths. A page identity is `(kind, key)`.
A block identity is `(file path, outline position)`; it is an ephemeral model
identity, not a newly persisted ID.

The supplied functions are:

| Input | Required interpretation |
|---|---|
| `key`, canonical `spell` | Literal name equality is key equality; `key(spell(k)) = k`. No namespace-congruence law is implicit. |
| `stem` | Decoded effective filename, without the extension, using the graph's filename/date configuration. |
| `title`, `aliasDecl` | Title and alias members from the file header, respecting Markdown/Org syntax. Whole quoted alias values produce no alias members. Bare alias members are names too. |
| `outline`, `occ` | Ordered blocks and typed occurrences, including header/pre-block text when applicable. |
| `inputNonblank`, `urlHasOrigin` | Nonblank input and URL recognition with a non-null origin. |
| `kindPref`; generic compatibility orders | Physical claimants use `kindPref` across kinds and unsigned UTF-8 path order within a kind (§2.2). Direct alias declarers use path bytes across kinds. Equal-path ties use the fixed file totalization. Generic `pageLe`, `fileLe` and `refKind` fields remain available to the checked generic/raw-support library; they do not supply the production date or claimant policy. |
| `Dates.canonical` | Recognize the configured accepted date spellings, after literal name normalization, and return the literal key of that day’s configured canonical journal title. Use the same date and title configuration as `stem`. This is an input/parser boundary, not permission to fold physical file claims by day. |
| Filesystem path representation | Graph-relative UTF-8 paths. Their unsigned bytes define the order; production path conversion is a conformance boundary. No extension, basename, path-component, filename-match or canonical-date rank participates in the order. |
| Query inputs | Outline depths, pre-block recognition, stored-query extraction and readings of opaque given leaves. `QueryRowParser.kind` distinguishes body, page-property header and preamble prose positions from the same extraction; it must agree with the outline, depths and header recognition. `QueryPresentation` supplies the requested header visibility and child-removal option. |
| Rewrite/merge inputs | The text writer and its syntax, path, query, layout and complete-text realization laws in §5. |

The Lean development does not prove a production Markdown/Org parser, filename
decoder, date recognizer, URL parser or serializer correct. Their contracts are
part of this model's size and its conformance obligations.

**Literal claims and date interpretation.** `key : String → K` is literal
name normalization. Physical claims, conflict detection, substitution demands
and writer contracts use this fixed key. An ordinary effective date name does
not claim every spelling of its day. Journals’ stems are their configured
canonical date titles; an ordinary file’s title property retains its ordinary
name precedence over its filename.

The semantic vertex of a name, `target(D,G,s)`, is computed as follows:

1. If a physical file claims `key(s)`, use the selected file’s `(kind,key)`
   claim, including when that file is empty or when the key is conflicted.
2. Otherwise, if the date recognizer returns canonical journal key `j`, use
   `(journal,j)`.
3. Otherwise use `(page,key(s))`.

Aliases, references, literal-name queries and virtual namespace seeds use
these full vertices. Namespace closure precedes this interpretation. The
interpretation is recomputed on each graph; creating or removing an ordinary
date page can change an unedited reference, which must be previewed.

The kind matters even when literal keys coincide: an ordinary page named
`Nov 6th, 2026` and that day’s **virtual** journal are different vertices.
An alternate accepted date spelling can name the virtual journal without
referencing or opening the ordinary page. Two **physical** files with the
same literal key, however, are a conflict across kinds under §2.2. Selecting
an active claimant is independent of reference interpretation. Every existing
ordinary spelling gets its own precedence; normalized equal spellings share
that precedence. Intentional alias edges may join these otherwise distinct
vertices; no blanket promise of separate reference sets survives such a join.
Semantic targets retain their kind across consumers; a displayed canonical
label is not a substitute for its full target vertex.

### 2.2 Claims, conflicts and active files

Directory determines a file's kind. For ordinary pages, `name(f)` is title if
present, otherwise stem. For journals it is stem. The claim is
`(kind(f), key(name(f)))`. An actual file counts even when its text is empty.

All files claiming a key, across both kinds, use one lexicographic order:
first prefer `kindPref`, then compare graph-relative paths by unsigned UTF-8
bytes. Within a kind only the path comparison matters. `openingFile(G,k)`
selects the least physical claimant. A key is conflicted when two distinct
files claim it, including an ordinary and a journal file.

Equal paths are invalid inputs. The fixed full-file order on
`[path,text,directory-code,format-code]` totalizes those ties only; no
text-dependent tie affects a valid graph. Filename format and configured date
rendering play no role in selection.

`activeGraph(G)` contains exactly the files selected by `openingFile`.
Every static reading below is evaluated on this active graph. In particular,
an inactive twin supplies no filename-parent seed, alias declaration, query
block or reference. Selection never deletes its raw file. Mutation checks
still inspect all physical files to prevent overwrite or damage.

A safe operation does not match any conflicted claimant’s effective name:
its original key would fail the local conflict check. Such claimants retain
their name keys, kinds and paths under realization. Kind preference depends
only on the fixed kinds, and the path comparison
depends only on the fixed paths. Their relative order therefore stays fixed
even when title spelling, alias or reference text changes. Neither claimant
can be a content source or survivor, and no new distinct claimant key can join
them. Every safe realized rename or merge keeps the active physical twin's
path. An explicit resolution of the conflict is a separate operation.

### 2.3 Namespace closure and pages

A namespace name contains `/`, starts with neither `./` nor `../`, and is not
an origin-bearing URL. For such a name, closure consists of the name and its
nonempty proper slash prefixes. Otherwise closure contains just the name.
For example, `/n/y` closes to `/n/y` and `/n`, without an empty name.

The seven occurrence kinds are page link, tag, page embed, property key,
linkable property value, task marker and priority. Ordinary plain mentions
do not reference a page. UUID block references are outside this occurrence
universe. All seven kinds contribute reference names through namespace closure.

The page universe consists of:

1. Every active file's real page.
2. Interpreted occurrence-closure vertices and nonempty namespace parents of
   nonblank active files, except vertices claimed by an active file or declared
   as an alias member.

Each virtual page has the kind and key returned by `target`. Alias members
do not generate pages
merely by being declared, even when linked. A real file with such a name
remains a real page. Repetitions in executable lists do not create distinct
page identities; observable membership and counts use sets/deduplication.

Nodes are those pages and the active files' outline blocks.

### 2.4 Alias groups, opening and references

Both ordinary and journal files can declare aliases. Each active declaration
adds an undirected edge from the declaring file’s full claim vertex to the
alias member’s interpreted `target` vertex. A group is the full connected
component, including transitive chains,
cycles, repeated declarations, self declarations and several real declarers.

To open a name, first compute its `target` vertex:

1. If an active file claims that vertex, open it. Physical conflicts use §2.2;
   content emptiness plays no part.
2. Otherwise choose a real page directly declaring that vertex, ranked by
   current unsigned UTF-8 graph-relative owner path.
3. Otherwise open the virtual target itself.

Path ranking is recomputed from the current graph after each operation.
It does not choose a content-merge destination.

A name’s concept is the connected component of its interpreted vertex. A block
references
a page iff one of its occurrence-closure names belongs to that page's
component. This does not depend on the direct declarer selected for opening.
The unprojected reference hits are the qualifying active block identities,
deduplicated. Linked References applies these source exclusions to that set:

- For a requested vertex claimed by a file, exclude blocks of that vertex
  alone. Do not exclude other members of its alias group.
- With no file at the requested vertex, the panel’s self vertex is the claim of the
  lexicographically least effective name among active real files in its
  component. This is independent of direct-declarer navigation path ranking.
  If there is no real member, use the requested vertex; no actual source file
  then has a vertex in that component.
- Also exclude blocks whose containing file's own key equals the configured
  Favorites arrangement name's key. An absent or normalized empty setting
  adds no exclusion. Do not expand this name through aliases or navigation.

Thus an alias-only name may have a panel without a separate query page row.
Two real members A and B share the reference relation but A's panel excludes
A's sources and B's excludes B's sources. They need not have equal panels or
counts. A page's reference count is the length of this very panel list,
computed once and shared by consumers; it counts blocks, not occurrences,
spellings, or source-page groups. Raw panel requests use `target(D,G,s)`
and the same self-source selection. A recognized date does not override an
existing ordinary page’s panel or count.

For a real requested page, the self exclusion has the same scope as OG
0.10.9 `frontend.db.model/get-page-referenced-blocks` and its `-full` variant
(`c67b8b5fa`, `src/main/frontend/db/model.cljs:1227–1290`): they remove
blocks whose containing-page entity is the requested entity, not its entire
alias set. OG also keeps sources at a different alias entity when the
requested alias has no file. Tine's single least-name fallback in that case
(`query.rs:445–485`) and its Favorites arrangement exclusion
(`tine-core/src/refs.rs:78–96`) are explicit presentation policies.

## 3. Queries, links and presentation

### 3.1 Queries

Queries have page or block anchors, leaves `refs(name)`, `name(name)`,
`page(leaf)` and given predicates, combined with `and`, `or` and `not`.
Page and block readings are evaluated separately.

For a block, path references include its own reference vertices, its outline
ancestors’ reference vertices and its containing page vertex. An earlier node is an
ancestor when its depth is smaller and every intervening node has greater
depth than that earlier node.

| Leaf | Page reading | Block reading |
|---|---|---|
| `refs(s)` | False | Some path-reference vertex lies in the alias component of `target(D,G,s)`. |
| `name(s)` | Page vertex equals `target(D,G,s)` | False |
| `page(l)` | Read `l` on this page | Read `l` on the block's containing page, or false if absent. |
| Given leaf | Supplied page predicate | Supplied block predicate |

The denotation filters the anchor universe by the Boolean value. Negation is
complement within that universe. `page(name(alias))` does not navigate through
an alias and does not invent a row for a file-less alias. A page with a file
is still eligible even when its name is also an alias.

Opaque given predicates can inspect the graph; locality or transport of their
results requires an explicit agreement law. The model does not infer that
agreement from their function type. Property-value semantics beyond these
given predicates are not defined here.

Stored queries are independently extracted from actual block text. Rename
maps every refs/name argument recursively through `page` and Boolean syntax.
Opaque given tokens remain unchanged. Query-only names do not seed pages.
Syntax rewriting does not imply unchanged query results, especially for
negation, literal names, joined groups or externally supplied predicates.

The query UI displays a defined projection of that raw denotation, in its
existing order:

1. Suppress page-property header blocks unless header rows were requested.
   Preamble prose blocks remain eligible. Both kinds of pre-block have no
   body parent or children. In Tine a query's explicitly requested block
   anchor enables header rows; an implicit conversion from a page anchor to
   block results does not (`query/exec.rs:210–230,508–514`).
2. By default, suppress a matching body block if its **immediate parent**
   also belongs to the raw denotation. Test raw parent membership before any
   presentation hiding. A matched grandparent with an unmatched parent does
   not hide the child. `:remove-block-children? false` disables this filter.
3. Page rows pass through unchanged. Header visibility and child removal
   are independent options; neither changes query truth or raw membership.

`queryResults` is `queryProjection` applied to `den`; `QueryPresentation`
defaults to child removal enabled and headers hidden. The parent is the last
body ancestor in the parsed outline. OG's `get-query-result` calls
`tree/filter-top-level-blocks` with the same immediate-parent raw-hit test
(`c67b8b5fa`, `components/query/result.cljs:58–73`,
`modules/outliner/tree.cljs:102–108`). Raw query inspection is a programmatic
surface, if provided, not a separate unfiltered UI mode. Honoring an explicit
false child-removal option remains required. Matching rows can consequently
be absent from the UI without being absent from the query denotation.

### 3.2 Links and neighborhoods

A directed link S→T exists when both endpoints are pages, S and T are distinct,
S's key is not the fixed configured Favorites exclusion name, and an active
owner block of S references T. Favorites entries and the configured exclusion
name are different state: entries follow substitution; the exclusion setting
does not change when its currently named page is renamed.

Out and in neighborhoods are directed reachability within the requested
number of steps, including seeds at distance zero. “Both” is the union of the
out and in neighborhoods. It is not a walk that alternates directions.

### 3.3 Trees and views

Namespace trees use deduplicated existing page identities. A page's parent is
the interpreted vertex of its immediate canonical-spelling namespace parent
iff that page exists in the
universe. There is no alias substitution and no jump over a missing parent.
Children are the inverse relation. The namespace-only tree contains pages
having a parent or children and omits isolated pages.

List, table, sort, group, board and collapsed views preserve the membership
of the projected result list in §3.1. Collapsing hides presentation, not
projected membership or denotation. A kind-uniformity
test compares only existing page/pre-block pairs.

## 4. Rename and merge candidates

### 4.1 Request, substitution and candidate set

A request supplies an existing graph G, source spelling `old`, destination
spelling `new`, and configured favorite entries. No candidate mutates an
entirely unsupported source. The operation API takes literal spellings: a supported source key occurs in
the raw active name-support view as a physical claim, raw alias endpoint,
occurrence closure or filename parent. Physical safety uses this literal
view; a navigation result is not an implicit source-content selection. This includes
virtual date-shaped names. An actual journal's identity is a fixed anchor.

For a chosen destination d, `m = nsSubst(old,d)` replaces the first nonempty
slash prefix with the source key, once. For flat/nonnamespace names only a
whole-name match is replaced. Every original spelling is processed once,
without recursively processing the result. All files are transformed from
the same before-graph.

An ordinary source file S exists only when one physical file across both
kinds uniquely claims the source key and that file is ordinary. There is no
source-content file for an alias-only or reference-only source.

A candidate contains a destination spelling d and an optional survivor D.
For a destination d, choose D precisely when S exists, d's key differs from
old's, one file uniquely claims d's key, and that file's proposed name stays
at its current key under m. Otherwise there is no survivor. An ordinary own
child that moves is therefore not a stationary survivor.

The finite candidate set is the literal candidate for `new`, plus the
candidate for the own name of every active real file in `new`'s alias group
whose key differs from old's. Remove duplicate candidates. There is no
automatic direct-declarer routing. The preferred candidate is the safe literal
candidate, if it passes; preference does not authorize execution.

| Candidate presented to the user | Transformation | Additional safety condition |
|---|---|---|
| Rename/make the typed name its own page | Apply m to every file; an ordinary root file rebinds to d. No file is created for a file-less source. | No root survivor; every common check in §4.2 passes. An occupied stationary destination cannot be overwritten. |
| Merge into the stationary file at the typed name | Delete S only after its text has been represented in D; rewrite D and all other files using m. | S and D are distinct, same format, D uniquely claims d and remains stationary; every common check passes. |
| Merge into a selected real member of the destination group | Use the selected member's own name as d for the entire substitution and merge, including descendants and query arguments. | Same stationary-survivor and common checks. A moving member is not a content survivor. |
| Redirect a file-less source to a selected group member's name | Rewrite all files under the substitution to the selected name. No content file is deleted or appended. | Every common check; no source-format condition exists without a source file. |
| Cancel | Return G exactly. | Always available. |

The first and third descriptions can produce the same candidate; display one
operation. A same-key request is a respelling, without root-content merging.
An actual journal can participate in a consistent same-key respelling of
references; its own path/date identity remains fixed. Ordinary pages can be
merged into journals when all conditions hold.

### 4.2 One finite safety check

Safety is evaluated separately for each candidate and its computed after-graph.
The parser/writer must satisfy §5's realization contract. Under that contract,
the finite predicate checks all of the following:

1. **Supported source.** The source is supported as defined in §4.1.
2. **Consistent key substitution.** Collect every physical file's effective
   name, aliases, occurrence spellings of all seven kinds, stored query name
   arguments, and favorite entries. For each spelling s demand
   `(key(s),key(m(s)))`. Add `(key(old),key(d))`. Add identity demands for
   every actual journal name and task-marker/priority spelling. Demands sharing
   an input key must agree on the output key. The resulting finite function
   is identity outside the table.
3. **Local conflict freedom.** Check old, d, every used/name-site spelling
   matched by the source-prefix map, and their images. None of these keys may
   be claimed by two distinct physical files. This includes moved alias-only
   images, even though aliases do not create pages. Unrelated conflicts do
   not forbid the operation.
4. **Valid content pair.** A survivor requires a distinct ordinary source,
   equal source/target formats, unique literal destination ownership and a
   stationary survivor key.
5. **No new claimant collisions.** Remove only the confirmed source of a
   content pair. If two remaining files' computed output claims have the same
   key, their original claims must already have had the same key. Physical
   output paths must be distinct.
6. **No virtual-child capture.** A moved page descendant cannot land on a
   stationary real file. The root is exempt for the explicit root operation;
   a file that itself moves out of the way is not stationary. This applies
   to raw name-support page keys, not raw alias-only endpoints. This literal
   eligibility analysis is independent of date reference folding.
7. **Path well-formedness.** Before and after physical file paths are unique.

Consistency does not demand that old namespace ancestor sets map to new ones.
Ancestors are recomputed. A move from A/x to B/z can therefore remove an
induced reference to A, including a journal A. That change must be previewed.
A journal's editable alias is not another fixed journal-date anchor.

### 4.3 Dialog and unattended execution

The dialog must show the entire destination component: real pages and their
file identities, alias names, and names without files. Mark which file-less
names have virtual page rows and which are alias-only. Include conflicts and
the selected physical claimant, not only a preferred alias owner.

For each safe candidate show its chosen destination, any source/survivor,
all file moves including simultaneous child moves, affected journals, changed
aliases and stored query arguments, and the reference preview below. Refused
candidates can be explained by their failed finite checks. Offer every safe
candidate; never omit a safe group member because another member is preferred.

Only an offered candidate may execute. Cancel returns the original graph.
A CLI, plugin or agent without a choice dialog receives a mutating answer
only when the safe-candidate list has exactly one member. Zero and multiple
members are refusals. The preferred option is not an unattended fallback.

### 4.4 Reference preview

For each old active block compute its output block images using rewritten
file paths, or the merger's source/target origin table. For every old/image
pair compare its reference signature:

* Namespace-closed raw reference spellings.
* Their interpreted reference vertices.
* The current opening targets of the original closed reference spellings.
* The entire active alias-edge list.

Include each unequal signature as a preview row. This is conservative: an
unrelated alias-edge change may include a block whose final targets are equal.
Equal signatures imply equal reference membership for every target page, so
every changed target is covered. A reference change need not involve changed
link text. Separately show old blocks with no images and new active blocks
without old origins. Complete text and origin realization is required before
publishing an operation; a missing source block is not an acceptable merge.

## 5. Text realization, interoperability and guarantees

### 5.1 Writer obligations

Every file rewrite preserves directory and format. It rebinds an ordinary
file's effective name under m; journals retain their effective name and path.
An unchanged-key ordinary file retains its path. Alias members in every file,
including inactive twins, are rewritten. Covered occurrence kinds use m;
task markers and priorities keep their spelling. Stored query arguments use
the same m. The ordered block-occurrence lists obey these laws exactly.

A content merge retains the survivor's path, directory, format and name.
Alias members from both files remain active under substitution. Every output
block has valid source/target origins; every input block and occurrence has
an output origin and the required rewritten occurrence. The survivor has at
most one combined header, then target body blocks in order, at most one
leftover source-header block, then source body blocks in order. Ordinary body
blocks remain separate. Source title and clashing header properties remain as
ordinary text; nonclashing properties join the target header. Unknown header
text must survive. Stored queries follow block origins using the same m.

Namespace parents are recomputed from the merged survivor’s resulting
complete text and effective name. They are not required to equal the parents
of the separately rewritten target. Appending text to an empty `B/x` therefore
activates parent `B`. All other merge laws describe the writer’s identity,
alias, occurrence, origin, layout or stored-query behavior; none assumes
reference, alias-component, query-result or parent-set preservation.

General header parsing/property promotion is a specified serializer boundary,
not a computed general property-value implementation in this Lean model.
Equal-format merging is required because no Markdown/Org translator is supplied.

### 5.2 Complete-text certificate

Occurrence extraction is insufficient to protect opaque prose. A fixed input
span partition, independent of the selected operation, accounts for every byte
of every physical file, including headers and malformed/unknown input. Only
legal name-edit spans are marked editable; everything else is protected.
The span parser must establish that grammar interpretation.

The text certificate records tagged original fragments and their placements
in output files, with explicit inserted serialization syntax. Concatenating
input fragments equals each complete input text; concatenating rendered
placements equals each complete output text. Original fragments in the two
graphs form a permutation with multiplicity. Editable fragments render under
m; protected fragments render byte-for-byte unchanged. This provides finite,
inspectable evidence rather than equating “all occurrences survived” with
“all text survived”. The body-layout contract additionally protects block order
and separation; fragment conservation alone would permit reordering.

The Lean certificate theorem proves preservation of marked original fragments
and unchanged protected bytes. Fidelity of the production span partition and
construction of a certificate for every proposed writer result remain
conformance obligations. A Boolean candidate check alone cannot certify an
arbitrary text writer. A writer that does not realize these contracts is not
a valid instance of the offered-operation safety theorem.

### 5.3 Universal guarantees and their boundaries

For every offered candidate, the after-graph is the retained physical files
mapped by the candidate’s output-file function: the source is removed only
for a content pair, the survivor is merged, and other files are rewritten.
All static readings are recomputed on that actual output, after selecting its
active files again. No injectivity, namespace-closure transport, fixed query
result, unchanged outline-depth, or unchanged alias-component hypothesis is
required for these guarantees.

The universal actual-effect equations specify:

- Alias edges are exactly the active output files’ declarations. Components
  are reachability in those recomputed edges; classes may split or join.
- A block references a target exactly when one of its recomputed closed
  occurrence spellings reaches the full target vertex in those edges. Linked References
  contain precisely those deduplicated active block identities whose source
  passes §2's exact self/Favorites exclusions, computed on the after-graph.
  Counts are the size of that panel and equal retained plus gained panel blocks,
  with no assumption that source names or alias topology stayed the same.
- Raw query denotations consist exactly of nodes in the recomputed anchor universe
  whose Boolean query evaluates true on the recomputed leaves. This covers
  negation, page wrapping, path references and supplied predicates. Stored
  arguments use the chosen substitution; result changes are the exact set
  differences of before and after denotations. Given predicates are evaluated
  on each graph, without assuming that they keep their answers. The presented
  results are exactly that after-denotation filtered by the header and raw
  immediate-parent rules in §3.1. Their gained/lost rows are computed separately
  from gained/lost raw hits. No result-invariance or parent-invariance premise
  is required.
- A link exists exactly for distinct resulting page endpoints, with the
  configured Favorites source excluded, and a resulting source-page block
  referencing the target. Neighborhoods use these recomputed directed links.
- Navigation uses resulting real files first, then current direct-declarer
  path ranks, then virtual fallback. Namespace support uses resulting text.
- For every old block/image pair and every target, a changed reference answer
  appears in the reference preview. Lost and newly active origins have separate
  rows. The preview may include extra rows; query-result changes need not be
  displayed, although changed stored arguments must be displayed.

For any finite result sets, added members are exactly after-members absent
before; removed members are exactly before-members absent after. Membership
is unchanged exactly outside those two sets. These are actual-effect and
coverage guarantees, rather than promises that every reading is preserved.

The semantic Lean API is `Date13`. It uses `Consolidated13` for literal
substitution, finite physical safety, execution and text realization. Its
offers, group enumeration, readings and preview use the date interpretation.
`Date13.offered_safe` transfers every offered choice to the existing physical
safety theorems; `offered_twins` and `offered_text_safety` state the resulting
ownership and text guarantees for these offers. The following table is the
complete hypothesis ledger
for current headlines. **A** follows from definitions/safety; **B** is a named
input/parser/writer boundary; **C** is an extra semantic restriction.

| Guarantee | Hypotheses and classification |
|---|---|
| Candidate authorization, cancellation, unattended uniqueness | A: offered choice or successful call as stated; no external hypothesis. |
| Fixed-key lookup, paths, no new claim collision, local-conflict isolation | A: finite candidate safety; well-formed before/after paths follow from it. |
| Physical journal identity, lawful claimant order and stationary conflicting ownership | A: safety, file membership/conflict as the objects described; all comparison inputs are proved stationary and whole-group opening paths agree. B: rewrite/merge syntax realization; fixed literal name extraction and graph-relative UTF-8 path interpretation as in §2.1. No abstract order-preservation or unchanged-date hypothesis. |
| Protected original text survives completely | A: offered choice and actual file/fragment membership. B: fixed legal span partition, complete-text certificate, syntax/body/query realization. |
| Literal claims/date precedence and physical conflicts | A: target’s physical/date/fallback branches and full `(kind,key)` vertices; literal conflict definition, exact opening selection. Branch lemmas identify an actual selected file or parser answer, without restricting operation eligibility. B: literal normalization, configured accepted-date recognition and canonical-title agreement with journal stems. |
| Recomputed aliases, reference relation, raw query denotations, links, navigation and exact added/removed results | No premise beyond the definitions; the equations also hold for raw candidate executions. B: production interpretation of parsers, configured date recognition, query predicates and writers. Offered execution is identified with the actual after-graph by authorization. |
| Linked References and panel counts | No premise: arbitrary target and optional Favorites setting; exact after-source lookup, self-key selection, exclusion, deduplication and panel length. B: configured Favorites/key interpretation. There is no whole-alias-group exclusion or unchanged-panel hypothesis. |
| Query presentation and rendering views | No premise: arbitrary query, anchor, row parser and presentation options; raw-hit filtering and exact after/gained equations. B: row kinds/depths/header recognition from the same production extraction, requested header/child-removal flags. Views preserve the projected list; no promise that the UI displays every raw hit. |
| Changed-reference preview coverage | A: old active block and its computed image, or a computed lost/introduced origin. No reference-preservation premise. |
| Date-aware structural comparison | No premise: concrete differing reference paths, anchor-universe membership or query-leaf truth, or source-exclusion input; otherwise the corresponding answer agrees. Every positive diagnostic names a finite object; query leaves and source rules expand its cause. |
| Generic literal-operation old/new exhaustive comparison | No semantic premise: compare a named old choice; classify refusal/cancel, an explicit diagnostic, or equal physical after-graphs. Static comparison has the separate diagnostics below. |

There are **no C premises** on current operation headlines. Equal-format content
merges and fixed-anchor/local-collision checks are explicit eligibility rules,
not hidden theorem restrictions. Parser/interchange fidelity, legal editable
span marking and general header promotion are B obligations that must be checked
by implementation conformance; they are not proved by a Boolean safety test.

Historical injective/fresh-rename, mapped-edge, positive-merge-query and
link-transport preservation results are special-case lemmas. Their stronger
transport assumptions are not current operation guarantees or eligibility rules.

### 5.4 Exhaustive comparison and diagnostic interpretation

The old API has several user choices and no default. Comparison therefore names
an old choice and compares it to the safe literal default, treating cancellation
as cancellation. For every request and old choice, either the old call has no
answer, it is cancel, an independently defined diagnostic holds, or the new
default exists and its complete physical after-graph equals the old answer.
File-less old “merge” operations are included as substitutions with no content pair.

The diagnostics include the alias/group, moving-target, anchor, key-consistency,
image-conflict, journal-closure, source-kind, literal-spelling and twin cases;
legacy own-page routing; different physical content-pair selection; and each
failed common safety check individually. A differing query-map spelling has a
concrete spelling witness, including noncongruent namespace keys. Diagnostics
are conservative causes, not a claim that each positive flag forces a different
result. Every category has a kernel-executed witness.

Since key and writer functions range over infinitely many spellings, equality
of their entire spelling maps is not generally a finite Boolean test. The exact
comparison includes an existential, executable spelling witness for a map split;
it does not silently assume function equality from equal destination keys.

Static comparison separately accounts for inactive-file masking, configured
reader normalization (concrete claimant order, both-kind alias policy, current
navigation ranks), and empty-real-file navigation. Normalization differences
are exactly a differing order comparison or alias-policy value, each with a
finite witness; deciding absence over an arbitrary function is not a finite
check. Outside these explicit causes all references, page/block membership,
raw query denotations and links agree, and navigation agrees for declared as
well as undeclared names. The historical model has no explicit panel/query
presentation projection. Applying the current projections to its equal raw
readings yields equal panels, counts and displayed query rows; this does not
assert that historical raw-hit lengths were panel counts. Direct comparison
with those historical raw readings additionally identifies every projected-away
input: a reference hit rejected by the source exclusions, or a query block
rejected by the header/immediate-parent rule. Outside these explicit
suppression witnesses and the reader diagnostics, panels, counts and displayed
queries equal the historical raw readings. Suppression tests the input row and
the stated rule, not inequality of complete result sets. These generic
comparison theorems remain checked for `Consolidated13`.
The date-aware reference comparison additionally returns a concrete closed
spelling with a path in one interpretation and no closed-spelling path in the
other, or equal reference answers. Raw-query comparison returns a differing
anchor-universe membership, a concrete query leaf with a differing truth
value, or equal denotation membership. Panel comparison returns a block with
a differing reference path answer or source-exclusion answer, or equal panels
and counts. These are universal structural diagnostics, with the target,
edge, page-seed, leaf and source rules above expanding their causes. Query
presentation uses the same header/immediate-parent projection of the resulting
denotation. Date-aware and historical key-only readings need not be equal.

Final-reader navigation has two additional finite diagnostics. The **target**
diagnostic compares the literal/date vertex with the generic real-first
target (preferred real kind, other real kind, otherwise the configured raw-key
reference kind). The **direct-declarer** diagnostic compares the optional
selected direct declarer in the two readers; a real target bypasses that
selection. If both inputs agree, navigation agrees. Together with inactive-file,
reader-normalization and empty-real diagnostics, these exhaust navigation
differences from the historical reader for every graph and spelling, including
unknown names and empty graphs. A changed target can be masked by a shared
direct declarer, so a positive diagnostic does not require different openings.

The complete final static reader also has an exhaustive comparison
(`Date13.static_comparison_exhaustive_final` in `DateComparison13.lean`).
Besides those diagnostics it identifies an individual primitive difference:

- A raw page-creation spelling produces a different optional virtual vertex,
  through target interpretation, real-file suppression or alias-target suppression.
- A closed reference spelling has a different alias-path answer for a target.
- A query leaf differs on a node in an actual page or block anchor universe.
- A source-exclusion answer differs for an actual block and requested panel.
- A block belongs to different source pages under path lookup and owner-block
  enumeration. This diagnostic includes malformed graphs with duplicate paths;
  the comparison does not assume path uniqueness.
- An immediate raw-parent lookup differs between full-vertex matching and
  key-only matching, including the historical duplicate-removal step.

Outside these causes, the comparison certificate gives equal page and block
lists, reference answers, all raw query denotations, links, directed neighborhoods
in every direction and radius, full trees, namespace trees, and the selected
navigation request. Panels/counts and displayed queries equal the same current
projections of the historical raw readings. Direct comparison with unprojected
historical readings additionally identifies each source-excluded reference hit
or header/immediate-parent-suppressed query block
(`Date13.presented_comparison_exhaustive_final`).

The diagnostics inspect primitive inputs or finite selections, not inequality
of complete result graphs, queries, trees or neighborhoods. They are
conservative causes. Each family has a kernel-executed witness, including a
target-only navigation change and a direct-declarer-only navigation change
with equal targets and no generic static diagnostic. Existential witnesses
over arbitrary configured functions are executable individually; deciding
their absence over all spellings or query leaves is not a finite check.
These comparisons have no semantic-operation, graph-validity, date-preservation,
canonical-spelling or realization premise. The parser/writer and configured
date-interpretation conformance boundaries remain as stated above.

This is a pure graph transformation specification. It does not implement
publication, disk crash recovery, revisions, locks or synchronization. The host
must publish through Tine's audited storage path. No new persisted identity,
hidden graph database, storage mode, or non-Logseq serialization is authorized.
