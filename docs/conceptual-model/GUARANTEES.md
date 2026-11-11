# Guarantees, boundaries and comparison

## Current entry points and proved scope

The current reader and operations are `CM.Date13`, across `Date13.lean`,
`DateOperations14.lean`, `DateProofs14.lean`, `Journal14.lean`,
`DateEffects14.lean`, `DateSafety14.lean` and `DateTwins14.lean`.
`CM.Alias14` supplies native alias fields and complete-text accounting.
`Consolidated13` and earlier namespaces are generic/historical libraries,
not alternative current date or panel policies.

The reader is total over finite snapshots. Offered executions use a single
finite guard and an explicit syntax/text realization boundary. There is no
extra injectivity, namespace-congruence, positive-query, invariant-predicate,
fixed-outline-parent or unchanged-alias-component premise. Every current
rename theorem quantifies its explicit, unselected `DatePolicy` argument.
That proves the generic mechanism for every argument; it does not choose M2.

### Universal headline ledger

**A** is an object/domain condition or fact derived from definitions and the
finite guard. **B** is a parser/writer/representation boundary not proved by
the model. **C** is an additional semantic restriction. No current universal
headline assumes a C preservation restriction.

| Guarantee and declarations | Domain and undischarged boundaries |
|---|---|
| `target_date`, `same_day_target`, `date_not_ordinary`, `pages_date_claim`, `journal_claim` | A: the named parser answer identifies the day; directory condition identifies the physical case. B: configured parser/day extraction, effective naming and ordinary normalization. No absent-file or unclaimed-spelling condition. |
| `active_handle_opening`, `opening_idempotent`, `openTarget_reachable`, `mention_references_opened` | A: actual active file/mention where stated; otherwise arbitrary graph and identity. B: parser meanings and UTF-8 path representation. No uniqueness-of-label hypothesis. |
| `active_text_opening_when_identifying` | A: actual active file. **Explicit limitation:** textual name identifies that claim. This is a restricted textual corollary, not the former unrestricted textual-opening headline. The approved `MM-dd` counterexample is `DateWitness13.approved_label_limitation`. |
| `name_panel_is_opening_panel`, `panel_opening`, `linked_exact`, `count_panel`, `panelSource_exact` | Arbitrary graph/name/identity/Favorites setting; no extra premise. Panels and counts use the same ordered list. B: effective Favorites/name interpretation. |
| `offers_exact`, `default_offered`, `offered_executes`, `unoffered_refused`, `cancel_identity`, `automatic_unique`, `singleton_executes` | A: offered membership or singleton/success equation as the object stated. No safe preferred fallback or freshness-implies-safety premise. The ordinary singleton witness is `DateWitness13.ordinary_onestep`. |
| `memberSpelling_identifies`, `group_member_substitution` | Arbitrary physical member; its chosen spelling identifies its actual claim and drives the one substitution. B: the operation-only `MemberNames` adapter supplies and certifies accepted identifying spellings. No displayed-label identification or display parse/render round-trip premise. |
| `group_member_candidate`, `safe_group_member_offered`, `safe_group_member_reachable`, `group_choices_exact` | A: actual distinct real destination-component member; the existing finite guard for safe offers/reachability. Every such safe member is enumerated and executable, not merely every member of an already-filtered list. B: `MemberNames` as above; no M2 choice. |
| `distinct_offers_block_automatic` | A: two distinct offered candidates. Automatic execution refuses. `DateWitness13.comprehensive_ambiguous_group` checks Foo → Shared with two MM-dd journals under both fixture fixed-day extremes; both destinations plus Shared are offered. `ambiguous_group_choice_executes` checks selection and the single map. |
| `safe_root`, `safe_site`, `safe_anchor`, shared `consistent_iff_function` | A: finite guard and membership of the actual demanded site/anchor. B: legal site extraction. No namespace-congruence or injectivity premise. Date anchor selection is explicit M2 data, not an assumed universal preservation result. |
| `safe_wf`, `safe_no_new_claim_collision`, `safe_local_conflict`, `execute_map`, `fileless_cardinality`, `merge_exact_files` | A: finite guard for safety facts, actual retained-file membership where stated; assembly/cardinality equations identify raw executions as well. Claim collisions use normalized full identities. |
| `realized_journal_fixed` | A: physical journals-directory file membership. B: rewrite/merge realization. It proves path and reader claim for that physical file. **No claim of fixed identity for every pages-directory or file-less day source**: that part awaits M2. |
| `stationary_twins_opening`, `offered_twins_opening` | A: safe/offered candidate and originally conflicted normalized identity. B: `Realizes` syntax, native merge and complete-text realization. Retention, stationary keys/paths/order, no new entrants and whole-group minimum transport are proved rather than assumed. |
| `Alias14.complete_text`, `offered_complete_text` | A: actual original file/fragment, offered membership for the operation wrapper. B: fixed legal input partition and role classifier, complete before/after concatenation, retained+consumed multiplicity partition, scoped approved consumption, syntax/body/query realization. The headline itself names retained **or exactly approved alias consumption**. |
| `Alias14.opaqueText_bytes`, `opaqueText_multiplicity`, `offered_protected_text` | A: actual protected input fragment and noneditable marking. B: legal fixed role/edit marking and the certificate. Every protected byte and its multiplicity survive. No rule for arbitrary equal-property deduplication. |
| `Alias14.members_exact`, `members_unique`, `one_field`; `MergeRealization.aliases/fields` | The concrete field serializer is executable and exact by bytes; one-field requires members. Native header parser/extractor agreement for actual writer output is B. `AliasRealization14.inhabited_offered_alias_merge` inhabits the complete offered alias-merge realization, including text, header/body origins and queries. |
| `edge_exact`, `reference_exact`, `pages_exact`, `den_exact`, `queryResults_exact`, `links_exact`, `children_exact` | Arbitrary current inputs, full Boolean query, given predicates, graph and target. These recompute actual meaning, without preserving a prior answer. B: extraction, query levels/row kinds and given-predicate evaluation on the actual graph. |
| `assembled_eq`, `offered_recomputed`, `after_reference_exact`, `after_query_exact`, `after_count_partition` | The after-graph is the actual retained-file projection. Counts partition into retained and gained current panel blocks with the **substituted** setting; raw/presented query gains/losses use the shared finite `added`, `removed`, `changes_complete` facts. No invariant given predicate, parent set or alias graph. |
| `signature_reference`, `preview_complete`, `lost_complete`, `introduced_exact` | A: actual old active block/image, changed reference answer, or actual lost/introduced origin. Signature equality determines every reference answer. No requirement that the spelling itself changed, or that all preview rows change an answer. |
| `ensure_reuses`, `ensure_any_claimant_reuses`, `ensure_original_files`, `ensure_created_exact`, `opening_cons_fresh`, `ensure_idempotent` | Arbitrary snapshot/day/proposed file; reuse does not require path uniqueness or no unrelated conflicts. A successful creation proves journal-directory location, correct day, empty text, fresh unique path and exact one-file addition. Every old file survives unchanged. B: construction of a proposed configured file that passes those checks. A refused proposal can later succeed with a corrected path; idempotence is stated for the same proposal. |
| `outside_reads`, `outside_content`, `DateWitness13.outside_move_outcomes`, `outside_title_remains_effective` | Outside snapshots are read without a rename-history premise. Witnesses preserve text at the new path and recompute day/reference answers. No claim that outside moves rewrite links or carry old day references to moved content. |
| `state_setting_follows`, `state_cancel`, `favorites_follows` | Configuration is returned with the graph; its name participates in demand checking through `answerState`. B: host publication of graph and setting, as for other operation output. |

### Hypotheses that may be too strong if implemented carelessly

These boundaries are not silently discharged by Boolean allowedness:

- Accepted dates must factor through the supplied literal-key adapter. If a
  production parser distinguishes spellings that ordinary normalization
  collapses, the adapter must be repaired; the model has not proved that law
  about an arbitrary parser.
- Day extraction must use actual filename dates independently of displayed
  labels. Replacing day values with rendered titles would be too strong and
  false under the approved `MM-dd` configuration. There is no round-trip or
  unique-display-label assumption. An unparsed journals-directory filename
  has an explicit fallback, not an arbitrary fabricated day.
- Operation enumeration takes `MemberNames`, a total accepted-spelling
  provider with a checked identity law for physical members. This is an explicit
  **B boundary** on the offer/answer/automatic APIs and their offered wrappers;
  raw execution, safety, reading and creation do not require it. The current
  parser adapter must construct it, including for filename-derived days whose
  labels are ambiguous. If a filename extractor produced a day having no
  accepted identifying spelling at all, this adapter could not be constructed;
  the model does not prove arbitrary supplied parsers avoid that inconsistency.
  This is stronger than the reader's total-input contract, but requires no
  unique display labels and adds no exception to comprehensive member choice.
  The concrete fixture providers are proved for all physical files, not just
  the witness graph. Every original display-opening limitation is unchanged.
- The fixed whole-text partition, editable-site legality and alias-role
  classification are substantial B obligations. A malicious classifier could
  mislabel prose as alias syntax/member. The fixed whitelist, retained-member
  witness, physical merge scope, complete concatenations and multiplicity
  partition constrain the certificate; they do not prove the production
  parser supplies a truthful partition.
- General header promotion, occurrence coverage, stored-query syntax, body
  separation/order and origin construction remain writer boundaries. Concrete
  complete-text and fully inhabited offered merge fixtures establish that
  these contracts are satisfiable; they do not verify the application writer.
- UTF-8 graph-relative path conversion and configured stem/title/query/header
  extraction are external interpretations. A lawful Boolean comparator does
  not verify OS path conversion. A proposed file's directory class must truthfully
  describe its configured physical directory: creation checks that metadata
  field, rather than proving a separate path-prefix/configuration theorem.
  Malformed input snapshots remain readable.
- Neither pure graph/state transformations nor their certificates prove crash
  recovery, locking, base-revision safety or publication atomicity. The host
  must use the audited storage path.

M2 is not a theorem hypothesis masquerading as discharged product policy:
its unresolved argument is visible on the API. A, B and C would instantiate
`DatePolicy.fixed` as all days, physically represented days or
journals-directory days respectively. No such production instance exists.
The day-source offer set and unconditional all-day identity guarantee remain
open. Exploratory UI operations are not used by any proof.

## Exhaustive comparison

`ReaderInputs` records primitive real/block/seed rows, optional virtual rows,
closed reference names and individual alias-path answers, leaf truth, source
eligibility/incidence, namespace-parent input/lookup, header/body-parent inputs,
and initial/actually-used direct opening selections. It contains no complete
query, reference, panel, link, tree or neighborhood output.

`primitiveDifference` gives an indexed row difference (retaining order and
multiplicity), or a concrete argument with a differing primitive answer.
`inputs_eq_of_no_difference` proves complete primitive agreement otherwise.
`structural_comparison_exhaustive` consequently proves either one of those
causes or agreement of all ordered page/block lists, reference functions,
all full Boolean denotations, panels/counts, displayed queries, links,
every neighborhood direction/radius/seed, full/namespace trees and navigation.
`reader_inputs_realize` proves this evaluator is the current reader, across
all those surfaces. Duplicate-path and full-vertex-parent behavior are retained
as inputs, without a well-formed-graph assumption.

The private helpers in `R13Baseline14.lean` express only the accepted r13
primitive equations; the sole exported reader value is a comparison snapshot.
They cannot select an old date or panel policy in current reading or execution.
`R13Baseline14.current_vs_r13` instantiates the structural comparison with
those accepted equations, including literal-before-date targets, raw physical
claims, old claimant preference and the independent panel self. Date-label
interpretation is a supplied accepted-r13 canonical-key function, separately
from the current day parser; equality between those functions is not assumed.
Initial-target-only and direct-only opening differences, and a changed target
masked by a common direct declarer, have executable witnesses. Positive causes
are conservative: they need not change the final answer.

`presented_comparison_exhaustive_final` directly compares a projection to its
raw baseline reading: it identifies an excluded source hit or a
header/immediate-parent-suppressed raw query row, or proves equal lists/counts.
Combining it with structural agreement compares current projections with
accepted raw results. It never calls a historical raw-hit length a panel count.

`Date13.operation_comparison_exhaustive` quantifies every raw old selected
candidate, and thus includes every authorized accepted-r13 candidate, including
file-less sources. It returns a concrete destination/survivor/source-selection
difference, a spelling-map witness, a named failed current finite check, or a
safe current literal default whose full physical result equals the old raw
execution. No result equality is a premise or diagnostic. Cancellation is
identity in both APIs. Earlier `Consolidated13Coverage` comparisons against
r12 remain checked with their original exact domain and spelling-map witness;
they are not relabeled as normalized day-operation proofs.

Existential witnesses over arbitrary configured functions can be checked
individually. Their absence over every spelling or supplied query leaf is not
claimed to be a finite Boolean decision. Journal creation has no r13 operation
predecessor and is proved separately; it is not hidden in an old equality branch.

## Historical sufficient conditions

Historical fresh/injective rename, mapped-edge transport, positive merge-query,
`GivenSafe`, fixed-outline and link-transport theorems retain their expressly
stated sufficient conditions in the library. They are not current operation
eligibility rules or universal preservation headlines. The length ledger
reports their additional 22 U separately.

## Preserved accepted §5.3–5.4 record

The following is the exact text received from accepted r13 SPEC §5.3–5.4,
pinned by the input snapshot `baseline/SPEC-r13.md` (SHA-256
`b939b41185e0f31de7a83d508f215070c467e12d5313071a35bc23c97d605c13`).
It preserves the complete earlier ledger and comparisons, including unfavorable
boundaries. Its old `Date13` names and literal-date/text claims are historical
statements. The current ledger and comparison above specify their replacements;
this appendix is not an alternative product policy.

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

