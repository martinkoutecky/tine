# Tine conceptual model

## Part I — Core

### 1. Prior decisions and scope

This is the provisional model under D-20261115-NIGHT. D-20261115-CMCUT/-CMCUT2 require a simple
model with properties checked on actual output; D-20261115-HELP separates requests from interface
helpers; D-20261115-CMGO fixes the approved operation scope. D-20261106-MF/-UA require a complete
model and explicit universal hypotheses. D-20261112-CMSTOP freezes admitted syntax and layouts.
The pending departures from CMGO are the P1/P2/P4 text boundary, alias-held refusal, Favorites
first-name deduplication on merge, the separate views layer, the separate Settings/Day files chapters, and the O6 header-order/first-block boundaries. The maintainer confirmed these (D-20261116-CM536).

The core decides and protects the page files of a name rename or its selected merge. The views describe what is displayed: query evaluation, rows, presentation, the namespace tree, Linked References and graph links/neighbourhoods. The Settings and Day files chapters state the settings completion step and day opening/creation. Core imports neither views nor chapters; chapters may import core. Stored query/macro arguments remain typed core name sites without query evaluation.
The declaration statements for Reading and views retain their original qualifications; GUARANTEES.md
states the exact proof/adapter boundary.

Rename requests by name include a merge selected by the name dispatch. **Physical file merge is outside
this model.** The code runs the same merge writer and the same content/save checks on it; it rewrites no
outside reference or setting. This separate entry point is excluded by the prescribed core-budget cut.
Editing a title or alias line is an ordinary edit, outside these rename requests. An explicit UI
rename command is a request. The host owns flushing edits, locks, revisions, retries, atomic publication,
autocommit and recents.

### Prior art

Reading follows the accepted round-15k model. OG revision
`6e7afa8eb040686ff057156ee877193b581dd369`, `src/main/frontend/handler/page.cljs:385–642`, supplies
prior art for direct lookup, merge confirmation, namespace rewriting and home settings, as inspected
in the cited proposal/reviews. The model's laws are the rules below, not a promise to copy all OG behavior.
CITATIONS.md records the local source evidence. No application source is inspected by this proof lane.

### 2. Names, files and graph meaning

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
| `Dates.parse`, `journalDay` | Accepted spellings return actual day values; filename extraction independently returns the day or no date. It must not substitute an ambiguous rendered title for the day. |
| `stem`, `title`, `aliasDecl` | Decoded/configured effective names and native header aliases. Bare members are names. A wholly quoted alias value supplies no aliases. |
| `outline`, `occ`, stored parser | Ordered blocks, typed occurrences and stored query arguments, including applicable header and preamble blocks. |
| URL/nonblank/path conversion | URL-with-origin and nonblank tests; graph-relative unsigned UTF-8 bytes. |

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

### 2.7 Outside moves and renames

The reader accepts the new snapshot without rename history or refusal. Moving
`journals/2026_11_02.md` to `pages/Work.md`, with no overriding date title,
keeps the bytes at Work and leaves date references naming the date, normally
virtual if no claimant or date alias remains. Renaming `pages/<date>.md` to a
non-date name has the same result. Renaming a non-date file to a recognized
date makes it the journal and ensure reuses it. An unchanged date title can
continue to make a renamed pages-directory file a journal.

There is no outside-rename guarantee that references follow the moved content.


### 3. Operations

- **O0 Source.** A is renamable if it is a page (it has a file, or is referenced, or is a namespace
  parent) or an alias. Any other name is refused. A day is refused (O2).
- **O1 Substitution.** One destination spelling D defines the whole map: A becomes D and `A/…` becomes
  `D/…`, everywhere at once: file names and titles, aliases, every reference, names inside stored queries
  and macros. The Settings chapter applies this same map to its named settings. D is the typed
  name B, except in a merge, where D is the survivor's own name: its title if the title identifies the
  page the file claims, otherwise its file-stem spelling if that does; if neither does, refused. A moved
  file keeps its directory and extension; its leaf becomes the encoded new name.
- **O2 Fixed sites.** Days, task markers and priorities are never respelled. A day cannot be the source
  of a rename. A page whose name is spelled like a marker or priority can be renamed; marker and priority
  sites keep the old name. The map is per site.
- **O3 Outcomes.** Let the *holders* of B be the files other than the source's file that claim B (a
  journal holds every accepted spelling of its day).
  - *Several holders:* refused (contested).
  - *One holder:* merge into it, after one confirm.
  - *No holder,* and a page other than the source declares B as an alias, and the source's own file does
    not claim B: refused, naming the page B opens. This holds when several pages declare B, and when the
    source has no file.
  - *No holder otherwise* (B is fresh, or differs from A only in what name comparison ignores, or is an
    alias declared only by the source itself): rename in one step. A respelling moves the file. The
    source's own declaration of B stays as it is (P1).
  - *A source without a file* (reference-only or alias-only name): no file is ever created. No holder:
    its references and alias declarations are rewritten. One holder: after the confirm, they are
    repointed (a merge with nothing to move).
  - The only answers are confirm and cancel. An answer where nothing was asked is refused.
- **O4 Safety.** An outcome is available only if all hold; otherwise it is refused with the reason:
  - no *touched* name newly lands on a *claimed* name other than the merge survivor. For the
    source identity itself, claims made only by its own file do not count as capture: O3 permits
    renaming it to its own alias. This exception does not apply to a child identity landing on an
    alias that the source file declares. Touched names are
    the source, every name under it (`A/…`) however it exists (a file, a reference, an alias that any
    page declares), the aliases the source's file declares, and the images of all of these under O1; a
    name is touched whether or not the request changes its spelling. A day under the source is not
    touched: it stays (O2), and the rename proceeds. A name is claimed when a file claims it or a page
    declares it as an alias; a name that is only referenced is free, here as in O3 (D-20261105-CM11);
  - no touched name is contested, and the operation creates no new pair of files with one name
    (unrelated existing twins do not block);
  - the destination is not a claimed name under the source;
  - a merge pair has one format and neither file has conflict markers, and an Org file that Tine cannot save unchanged is not touched;
  - no title would name two pages at once. A title is one name site whose value may contain references.
    Its new value starts with what O1 puts in place of the renamed part and keeps the rest of the old
    title, with the references in that kept part respelled by O1 as anywhere else. The result must name
    the page that O1 gives the old name (`Old/[[Old]]` under a case-only rename of `Old` to `old`
    becomes `old/[[old]]`, the same page; under a rename to `New` it would read `New/[[New]]` where O1
    says `New/[[Old]]`: refused). A reference that O1 would respell inside the replaced part: refused;
  - every new file path is free, also against another file's folded spelling on a case-folding file
    system (a move onto the file's own folded spelling is free);
  - a moved file with conflict markers needs no rewrite (it moves verbatim).
- **O5 No dialog.** A caller that cannot ask (command line, agents) gets the outcome only when there is
  nothing to ask. Otherwise it is refused, also when it supplies the answer in advance.
- **O6 Merge meaning.** The survivor keeps its name, path, header and blocks. Source header entries are
  allocated in order:
  - alias declarations that have members, of both files, become one declaration holding every member
    once, survivor's first; a member that reads as the same name as one already kept is dropped. Where
    in the header that declaration stands is the writer's. A declaration without members is other
    header text;
  - a property whose key neither the survivor nor an earlier joined entry has joins the survivor's
    header and stays a property in its native form. Keys are compared as the parser reads them,
    after O1/O7 transformations. A key is an observed field, including when it has no typed name site. Where
    a joined entry stands is the writer's: the merged header holds the survivor's entries and the
    joined ones, each exactly once, and their order is not promised;
  - a source title, a clashing or repeated property and any other header text are the *leftover lines*,
    in order.
  If there are leftover lines, one leftover block follows the survivor's blocks, at the top level. Its
  content is fixed: an empty first line (in Org, a headline with an empty title), then in Markdown the
  leftover lines, in Org a line `#+BEGIN_QUOTE`, the leftover lines, and a line `#+END_QUOTE`. A
  leftover line is the entry's own line without its structural prefix, whatever its native form (a
  drawer entry too). A blank line and a bare bullet are not leftover lines. In the leftover the lines
  are no longer page properties; the references inside them stay references, and a merge whose
  leftover could not keep one (in Org, a property line inside the quote) is refused (`leftoverReference`). Then the
  source's blocks, in order, at their depths. Blocks that hung under the source's header hang under
  the leftover block; if there are such blocks and no leftover lines, the merge is refused. Text
  before the first block of a file is read as that file's first block, at the top level, so a
  merge keeps it like any other block, in its place before that file's other blocks.
  (D-20261102-MC, D-20261109-AL.)
- **O7 All or nothing, with named exceptions.** Either every site changes or nothing does, except the
  following, after which the result is "done", each exception reported with its own location:
  - a *referrer* file with unresolved conflict markers is skipped;
  - a site that cannot spell the new name where it stands is left as it is. Whether it can is a
    function of the site's kind and format and of the new name, given with the parser (examples: an
    Org headline tag cannot hold a name with a space or a colon; a query or macro argument cannot hold
    a name that would end the macro; a property key cannot hold a colon; a name split across lines
    never can; a Markdown tag can, with its bracket wrapper). A site that can is rewritten;
  - a property key is written in key spelling (lower-cased, whitespace hyphenated), reported when that
    differs from the new name.

### 4. Text properties, checked on the actual result

The *observations* of a file are what the parser reports: its names; its alias members and how many
alias declarations have members; its other header entries in order, each with key, value text and
native form; its blocks in order, including nonblank text before the first outline marker as the first top-level block, each with its depth and its content
exactly as the outline reader gives it (line endings normalised, structural prefix and indentation
removed, everything else verbatim, including markup, code, comments and the framing lines of a quote);
and the typed name sites inside all of these, each with its byte span in the file. A title is one site
spanning its value; the references inside it are sites too, nested in it, and a nested site may have
the same span as its title (a title that is exactly one reference). Two sites that are not nested have
different spans. Every block is observed the
same way: the leftover block has no special observation, the model computes its expected content as O6
fixes it. A header entry's optional key is the parser-normalized property key, present even
for built-in properties without a property-key site; non-property header text has no key. Where a
property-key site exists, the observed key agrees with that site's name. The expected entry's key is
`some` of the first property-key site's name after `siteResult` (O1/O2/O7, including key spelling,
fixed/skipped/unspellable cases); without such a site the input key is unchanged. Actual keys come
from reparsing the output. Key fields participate in P2's expected/observed comparison.

- **P1 Outside a merge pair, only names change.** For every file that is not one of a merge pair: the
  output equals the input except inside the spans of the name sites O1 rewrites there. A span is the
  site's native extent on one line (the wrapper included, as in `#Old` becoming `#[[Two Words]]`); a
  replacement contains no line break. A rewritten title is one span, with the references inside it. A
  file with no rewritten site is byte-identical. No rename adds or removes a line.
- **P2 Reads as intended.** The expected observations are computed from input observations alone
  (O1 at every typed site; O2 fixed sites and O7 exceptions; O6 for a merge). The expected function
  receives no output, writer or writer reports. Outside the merged survivor, observations and header
  order are exact, with site coordinates erased. In a merged survivor the header is a permutation
  of the survivor's entries plus the sequentially allocated joining entries: every occurrence is
  kept exactly once, without an order promise. Non-header observations stay exact, including typed sites
  inside the prescribed leftover block. A merge is refused before writing when reparsing that
  input-derived leftover with the same reader would not retain its reference occurrences.
- **P3 Stable.** Saving each changed file through Tine's save path with no edit leaves it byte-identical
  and does not rename it.
- **P4 Nothing lost in a merge.** After legal O1/O2/O7 transformations, every block and every
  content-bearing header entry of both files occurs exactly once in the survivor, with its content
  (an entry as an entry or as its leftover line). Blocks retain O6's order and depths. Source header
  entries are allocated in source order; the only drops are duplicate alias members by Reading
  identity and the syntax of combined alias declarations. P2 transfers these properties to the
  actual output. Merged-file promises exclude parser-unreported bytes (blank lines, indentation,
  bullets, line endings, header syntax and whether a header is before the first block or in it),
  header-entry order. P1 continues to
  protect every file outside the merge pair. P3 applies to every changed output.

### 5. Guarantee and boundaries

Whenever the decision function returns done, O0–O7 and P1–P4 hold for its actual page-file candidate,
under the truthful parser/configuration/codec/save/host interpretation in GUARANTEES.md. Each exception
has an input path, native byte span and site kind. Nested title/reference spans may coincide;
independent sites and sibling nested references have different spans. Report identities include kind. The model checks distinct report locations; input/output native
slices and complete title nesting are checked. Settings completion is specified in Part III; it cannot change this page outcome.

This is a conditional acceptance guarantee. It proves that the candidate passing all checks has these
properties. It does not prove a writer always produces an accepted candidate. A refused request changes
nothing at the host publication boundary. The host must publish the exact checked bytes atomically.
The syntax domain is frozen at 15k; `supported` is its truthful boundary, never a reason to omit ordinary
required cases. The bounded reader fixtures are separate non-vacuity evidence, not the production domain.

### Interface helpers outside the core


The alias dialog: "merge into that page" is the request "rename A to that page's name" and its confirm;
"use the name for this page" removes the alias declaration from every other page that declares it, an
ordinary edit, and then renames. The day move and the contested-name helper as before. A helper adds no
model outcome and promises no atomicity across its steps.


## Part II — Views

This layer has no incoming dependency from the core. Its rules apply to the
core Reading snapshot, with the same query and presentation parameters as 15k.

### 6.1 Linked References

Panel self is the opening identity, using the same vertex-level function as
navigation. The panel keeps deduplicated active blocks that reference the
requested concept and whose actual source differs from panel self and the
configured nonempty Favorites identity. It does not exclude a whole alias
group. Order is the active block order after deduplication. Counts use this list.
The Favorites setting follows renaming; exclusion uses the resulting setting
and resulting graph, without assuming the old source or alias topology remains.

### 6.2 Raw queries and displayed rows

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

### 6.3 Links, neighborhoods and namespaces

A directed link has distinct resulting page endpoints, a source other than
the configured Favorites page, and an active source block referencing the
target. Outgoing and incoming walks repeatedly add neighbors from the finite
page universe. The both-direction view concatenates those walks. Every radius
uses the actual recomputed directed relation.

### Namespace tree

A row's tree parent candidate is the typed page that its last proper prefix with a nonempty page
key other than its own names, before alias opening: the prefix's real owner, else the day or
ordinary page the prefix spells. The candidate is dropped when it is the row itself or when its
page key has as many `/`-separated parts as the row's page key or more; parts are counted on the
folded page key, empty parts included, so `2026/11/` counts two. The displayed tree parent is the
candidate only when that typed identity is present in the page universe. There is no alias or
grandparent fallback. Children are the deduplicated pages with that parent; the global namespace
tree includes rows with a parent or at least one child, and a day whose title has a prefix with a
nonempty page key; other isolated pages are excluded. Namespace closure itself remains core.

### 6.4 Actual changes

Added members are after-members absent before; removed members are
before-members absent after. Outside those two sets, membership is unchanged.
This applies to raw and displayed queries, references, panels, links and tree
readings separately. It does not promise that arbitrary Boolean queries,
namespace closure or alias components are transported unchanged by renaming.


## Part III — Chapters

These separately counted write rules import core. No core definition imports a chapter.

### Settings (O8)

After the files: Favorites entries follow O1; after a merge the list then keeps only
  the first entry of each name. The Favorites-page setting follows. The home page follows a rename and
  stays on a merge. The settings file is written in its own step after the pages, all of it or none; if
  any of it cannot be written the result is "pages done, settings not updated". The text properties do not cover the settings file.


`finishSettings` consumes the core's done page result, the previous settings record and the honest
settings-write success function. It returns either the complete intended settings record and updated
status, or the complete old record and pages-done/settings-not-updated status. There is no partial
record. The exact source home identity stays on a merge; home descendants still follow the shared
namespace substitution. The page decision has no settings argument or settings fields. Projecting
away settings from the combined operation gives exactly that page result.

### Day files

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

