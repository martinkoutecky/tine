# Search and query unification implementation plan

Accepted 2026-07-13 for GH #98, #99, and #69. This plan records the intended
product slope and engineering boundaries behind ADR 0042 so later slices do not
collapse back into separate search/query systems.

## Product progression

The primary path should remain uncluttered:

1. Type friendly text in Ctrl+K.
2. Inspect reusable page and block result rows with bounded excerpts and real
   match evidence.
3. Open the graph-result portion in a persistent query tab.
4. Add filters in compact chips or a spacious Advanced modal.
5. Choose search/list/table/board presentation independently of membership.
6. Name the workspace to materialize a normal page with one `{{query ...}}`
   block. The saved page is ordinary, syncable, exportable graph content.

The query tab initially shows an editable friendly search field, result status,
the result list, a subtle Filters/Advanced action, view choice, and a title field
whose placeholder says that naming saves it as a page. It does not show friendly
text and raw DSL at the same time.

## Semantic model

`QueryPlan` is the semantic source of truth:

- target: blocks, pages, or both;
- predicate tree: content, content regex, explicit fuzzy page name, references,
  task/priority/property/date/page/namespace predicates, and boolean groups;
- scope: graph, current page, namespace, and later bounded scopes;
- ordering, limit/paging, grouping, aggregation, and presentation hint;
- diagnostics and optional explanation instrumentation.

Frontends compile to/from this plan only where lossless:

- friendly search text (AND, OR, phrase, exclusion, regex; later field syntax);
- compact visual chips;
- full visual predicate tree;
- canonical Logseq/Tine query DSL;
- future natural-language or plugin-supplied declarative frontends.

Unsupported clauses remain visible in a raw/advanced representation; no frontend
may silently omit them. Existing `{{query}}` text remains non-fuzzy and block-only
unless it explicitly requests another target or fuzzy predicate.

Rust is authoritative for parsing regex, matching, spans, and diagnostics. A hit
is typed as Page or Block and includes stable identity, matched field/spans, and
an optional relevance score. Ctrl+K's page ranking preserves its current order:
prefix, then substring, then subsequence, with shorter names preferred and stable
relevance sorting. Commands and Create page remain launcher providers.

## Reusable result presentation (#98)

Use one strong default row, not density preferences. A block row separates the
page/breadcrumb context from matched content and renders a bounded excerpt around
the useful match region. It highlights all positive terms or regex evidence that
actually caused the hit; negated terms are not highlighted. Multiple terms,
phrases, regex, and repeated hits have deterministic selection/merging rules.

Excerpt work is bounded by bytes/chars, number of spans, result count, and DOM
nodes so pathological blocks do not dominate the popup or slow interactive
search. The same row feeds Ctrl+K and the workspace's `tine.view:: search`
presentation. Keyboard navigation and accessible names include both page context
and matched text.

## Query workspace (#99)

Add a route shaped like:

```ts
{
  kind: "query",
  sourceKind: "search" | "dsl",
  source: string,
  presentation: "search" | "list" | "table" | "board"
}
```

It is persisted in the existing graph-scoped session store, on this app instance,
and reevaluated when restored. No hidden or temporary graph page is created.
Result snapshots are not persisted.

Naming the workspace on Enter:

1. validates/canonicalizes the name and checks for a collision;
2. compiles the current plan to canonical query DSL;
3. saves a brand-new normal page through `backend.savePage` with the no-baseline
   audited create path;
4. writes a single query block plus presentation properties when needed;
5. replaces the query route in place with the page route.

## Friendly and advanced query construction (#69)

Friendly search stays the default frontend. Compact filters summarize active
clauses. Advanced opens a Gmail-like modal with draft Apply/Cancel semantics,
keyboard/focus management, precise validation, and cancellable preview/counts.
The modal has room for:

- all/any/exact/exclude/regex/fuzzy text;
- blocks/pages/both and graph/page/namespace scope;
- tasks, properties, references, schedules/deadlines/journal dates;
- nested boolean groups;
- sort, group, aggregate, limit, and view.

An explanation surface provides a plain-language summary and predicate tree.
On demand, it can show per-clause counts and elapsed time; individual results can
show Why matched with field, spans, and score. Invalid drafts keep the last valid
results visible and show precise diagnostics. A sample-block/page tester is a
later slice over the same evaluator, not a second engine. Rhai or arbitrary
scripting is deliberately excluded.

## Performance and verification

Execution profiles share semantics but not budgets:

- Ctrl+K preview: debounce, supersession cancellation, small cap/early stop;
- query tab: paged results and virtualized/bounded DOM;
- inline query: current compatibility semantics;
- explanations/counts/timings: explicitly on demand.

Implementation order:

1. Add catalog entries and prove fail-before tests.
2. Extract typed plan/evidence and compatibility adapters in Rust.
3. Reuse bounded result rows in Ctrl+K and query search view.
4. Add session-backed route and Open results in tab.
5. Add save-by-naming.
6. Move the existing builder into the spacious modal and add explanations.
7. Run focused unit/render/core tests, screenshots, real-app route/save E2E,
   full Linux release E2E, TypeScript, build, benchmark, and sanctioned deploy.

Future consumers may include current-page find, backlinks/unlinked references,
pinned searches, sheets membership, CLI/MCP, declarative plugin predicates, and
static export. They are not part of this implementation unless needed to keep
the shared plan honest. GH #100 remains a later search-entry-point decision.

## Precedents

- Gmail advanced search and operators: a friendly field, compact operators, and
  a spacious advanced form.
- VS Code Search Editor: promote transient search into a durable editor tab.
- Obsidian Search: explainable search terms and durable embedded queries.
- Emacs Occur: promote a transient search into a navigable result buffer.
