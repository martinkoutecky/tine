# Living Guide/demo documentation program

Status: implementation plan for branch `docs/new-docs`.

## 1. Goal

Turn Tine's existing bundled Guide graph into the canonical user documentation:

1. the first-run demo graph;
2. the read-only in-app Guide, with an explicit copy-to-graph sandbox;
3. the public HTML export at `https://tine.page/demo/`.

The result should help a new user succeed quickly, help an experienced user find
precise behavior and combinations, and stay current as Tine changes. It should
not become a second maintainer-memory system or a prose copy of Logseq's manual.

GH #201 asks for a Wiki. Its needs are valid; a separate Wiki is not the chosen
storage mechanism because it would duplicate the Guide/demo source already
protected by ADR 0036.

## 2. Product principles

### 2.1 One source, three surfaces

`crates/tine-core/src/templates/*.md`, registered in `GUIDE_TEMPLATES` in
`crates/tine-core/src/onboarding.rs`, remain canonical. A content change is not
complete until it works in all three surfaces and `website/demo/` is regenerated.

### 2.2 Teach tasks before syntax

Lead with an outcome: capture a thought, find unfinished work, link two ideas,
annotate a paper, structure repeated information, or extend Tine. Introduce raw
syntax at the point where it helps. The user should not need to understand a DSL
or internal model before obtaining a useful result.

### 2.3 Executable examples, not decorative examples

Guide blocks are real Tine/Logseq graph content. An example should normally be
copyable or directly interactive and state what the user should observe. Prefer
small examples whose result can be inspected over screenshots or claims about
behavior.

### 2.4 Progressive disclosure without duplicated manuals

The Guide has three linked lenses:

- **Start**: the shortest coherent path to a useful graph;
- **Workflows**: task-oriented recipes that combine features;
- **Reference**: precise behavior, settings, limits, compatibility, and related
  workflows.

Several lenses may link to one canonical example. They must not copy and then
independently maintain the same explanation.

### 2.5 Explain user-relevant boundaries

Document surprising Tine/Logseq differences, platform availability, whether
state is graph-scoped or device-local, what is written to disk, and safety or
experimental status where users encounter it. Do not expose internal
architecture unless it changes a user's decision or recovery action.

### 2.6 User docs are not engineering memory

ADRs, regression catalogs, tests, release-impact records, and issue discussions
preserve implementation intent. The Guide may explain *why* a behavior helps the
user, but should not accumulate issue numbers, code owners, internal incidents,
or private agent notes. Never source public copy from `tine-agents/` except for
an explicitly approved public plan such as this one. Never quote Martin's real,
anonymized, test, or recovery graphs into public templates.

## 3. Current architecture and owners

| Concern | Current owner |
|---|---|
| Canonical Guide/demo Markdown | `crates/tine-core/src/templates/*.md` |
| Page and asset manifest | `GUIDE_TEMPLATES` / `GUIDE_ASSETS` in `crates/tine-core/src/onboarding.rs` |
| Read-only virtual Guide and copy semantics | ADR 0036 plus `src/guide.ts` and onboarding tests |
| Public generated docs | `website/demo/` |
| Generator and freshness/link check | `scripts/build-guide-demo.mjs`; `npm run docs:build`, `npm run docs:check` |
| Full feature inventory | `docs/FEATURES.md` |
| Settings/default inventory | `docs/SETTINGS-INVENTORY.md` and current Settings source/tests |
| Accepted release changes and docs dispositions | `CHANGELOG.md`, `docs/releases/*-impact.json` |
| User-visible regression evidence | `tests/regressions/catalog.json` and referenced inventories/tests |
| Durable design and compatibility decisions | `docs/adr/`, `docs/BACKLOG.md` |
| Community need and terminology | open/closed GitHub issues and discussions; not authoritative for current behavior |

When these sources disagree, current tested product behavior and an accepted ADR
outrank older prose. Record a discrepancy rather than silently choosing a story.

### 3.1 Decisions and constraints for this program

- Martin has already chosen one complete graph/template set for onboarding,
  in-app Guide, copied sandbox, and public demo. New registered Guide pages
  therefore also become files in a newly created demo graph. Keep the first
  program small: no more than three new pages before a manager reviews the
  resulting first-run graph as a whole.
- Existing public page titles and generated URLs are frozen in v1. New workflow
  pages link to existing `Features/*` pages; they do not rename/move them.
- `Tine Guide` is the v1 navigation owner. In-app Guide full-text search and a
  hierarchical public side navigation would be product changes; this program
  records but does not implement them.
- Managed-storage/sync status copy is frozen unless Martin separately provides
  current wording. The documentation worker must not infer status from code.
- Kimi may edit Markdown templates and generated `website/demo/` output. It may
  append a manager-approved entry to `GUIDE_TEMPLATES`, but may not otherwise
  edit Rust production logic.
- `crates/tine-core/src/publish.rs`, onboarding/copy logic outside the manifest,
  `scripts/build-guide-demo.mjs` validation semantics, and
  `crates/tine-core/src/templates/config.edn` are manager-owned. A worker reports
  a defect there instead of repairing or weakening it.
- Generated `website/demo/` files are committed with their template source.
  Reviewers treat them as derived output and review the source plus rendered
  pages, not thousands of generated lines as independent authorship.

## 4. Information architecture

Keep `Welcome to Tine` as the public and first-run landing page and `Tine Guide`
as the in-app documentation index. Organize the remaining graph by these lenses.
Names are a starting taxonomy, not a requirement to create empty pages. Existing
page titles remain stable; only the first manager-approved workflow pages receive
new names.

### 4.1 Start

- **Welcome to Tine**: the existing first ten minutes, kept short.
- **Start/Bring an existing graph**: what Tine reads, coexistence with Logseq,
  graph selection, and the first safety checks.
- **Start/Where things are**: page, journal, block, tabs, right sidebar,
  Settings, and Help—only enough orientation to follow recipes.

### 4.2 Workflows

- **Workflows/Capture and plan**: journals, TODO workflow, priorities,
  scheduling, quick capture, agenda/carry-over.
- **Workflows/Find and revisit**: Ctrl+K, in-page find, links, tags, block
  references, linked/unlinked references, search/query workspaces.
- **Workflows/Research a document**: assets, PDF reading and highlights,
  block references, side-by-side panes, export.
- **Workflows/Structure repeated information**: tags/properties, queries,
  Sheets table/board/grid, formulas; begin visually and reveal DSL later.
- **Workflows/Extend Tine**: plugins and themes, capabilities, settings,
  platform/distribution boundaries.

### 4.3 Feature and concept reference

Retain and improve existing `Features/*` pages. Add a page only when the
inventory finds a material gap. Likely families are:

- pages, blocks, references, embeds, properties and tags;
- search, queries and persistent result workspaces;
- Sheets and formulas;
- tabs, panes, sidebar, focus and multi-window behavior;
- journals, tasks, scheduling, agenda and quick capture;
- media, PDF annotation, diagrams, publishing and export;
- plugins and themes;
- Android/mobile and desktop/platform differences;
- files, backups, external edits, graph compatibility and recovery;
- managed storage/sync, clearly labelled according to its current status.

`Feature showcase` remains the rendering/conformance kitchen sink. It is not a
substitute for task-oriented explanation.

### 4.4 Reference indexes

- **Reference/Shortcuts and commands**: discovery paths and high-value defaults;
  do not manually duplicate every configurable binding if the app is the better
  searchable authority.
- **Reference/Settings and scope**: where settings live, their default, and
  graph/device/platform scope for settings users plausibly need to look up.
- **Reference/Logseq compatibility**: concise intentional differences and
  interoperability/safety boundaries, linking to feature pages.
- **Reference/Troubleshooting and recovery**: only evidenced recovery paths and
  diagnostics; no generic guesses.

## 5. Pre-registered user journeys

Do not create a second feature/coverage catalogue. Release-impact records already
force a Guide/docs disposition for accepted user-visible changes. The program is
judged against these manager-defined journeys rather than topics the worker
invents, prioritizes, and marks complete itself:

| ID | A user can… | Principal current pages |
|---|---|---|
| J01 | create a demo graph and perform the basic edit/nest/task/link loop | `Welcome to Tine` |
| J02 | open an existing Logseq graph and understand what Tine reads/writes | Welcome plus a future Start/reference page |
| J03 | capture and plan work using journals, tasks and quick capture | `Features/Quick capture`, tips |
| J04 | link, find and revisit information using pages, tags, refs and search | tips, showcase, future workflow |
| J05 | structure repeated information visually, then reach queries/DSL when needed | Sheets, Formulas, future workflow |
| J06 | read and annotate a PDF and connect highlights to notes | `Features/PDF annotation` |
| J07 | use tabs, panes and the right sidebar to keep context visible | tips, future workflow/reference |
| J08 | install and understand the boundary of plugins/themes | `Features/Plugins` |
| J09 | understand files, external edits, backups, export and recovery | future reference page |
| J10 | identify what works on Android/mobile versus desktop | future reference page |

A journey is accepted only by manager review. The worker may report it as
`candidate`, `partial`, or `blocked`, with quoted evidence; it may not declare
the program complete. The initial Kimi task is deliberately limited to J05 and
the existing Guide index.

## 6. Implementation phases

### Phase 0 — manager hardening before any new page is registered

1. Repair `guide_link_set_is_closed_over_demo_pages`: today its `guide` and
   `demo` sets are both built from `GUIDE_TEMPLATES`, so its implication is
   tautological. Validate every extracted Guide target against the manifest or
   a small explicit deliberate-stub allowlist.
2. Make generated-demo link validation distinguish deliberate click-to-create
   stubs from accidental missing Guide pages. Do not retain the current broad
   “all `.ref` links may be missing” exemption as the only proof.
3. Add focused fail-before/pass-after tests for both corrections.

This prerequisite is manager/frontier implementation work. The first Kimi slice
may proceed without adding or renaming links; no new page may be registered until
Phase 0 lands.

### Phase A — editorial calibration on two existing pages

1. Audit only `Tine Guide` and `Features/Sheets`, using the sources named in the
   Kimi dossier.
2. Rewrite the Guide index over existing page titles only. Do not add placeholder
   links or rename a page.
3. Restructure Sheets to teach the easiest visual workflow first and advanced
   query/DSL concepts later. Preserve correct facts and the established
   `Create one yourself` → numbered steps → `What you should see` convention.
4. Generate the public demo and produce a compact evidence/provenance report.
5. Iterate within these two pages until the worker's semantic checks pass, then
   stop for manager editorial and rendered-page review.

This answers whether the lane produces prose worth shipping before it creates a
large branch or new information architecture.

### Phase B — one new workflow page

If Phase A is accepted and Phase 0 has landed, add exactly
`Workflows/Structure repeated information`. It links the accepted Sheets and
Formulas pages and introduces visual query/search construction before raw DSL.
The manager grants the single `GUIDE_TEMPLATES` append. Existing page titles and
URLs remain unchanged.

### Phase C — family-sized expansion

Select one pre-registered journey at a time. Inventory only that family; do not
produce a 180-topic whole-product audit. A dossier names the journey, allowed
templates, authoritative sources, new-page allowance, and stop conditions.
Limit each unattended assignment to at most three coherent commits and three
template pages. Manager review chooses the next family.

### Phase D — editorial and product review

Review the whole graph as a user journey:

- can a novice reach a useful result without reading reference material?
- can an experienced user find exact scope/default/compatibility information?
- do workflows link to precise reference without repeating it?
- are claims current and source-backed?
- are experimental or platform-limited features unmistakable?
- is every paragraph useful, concrete, and reasonably short?
- do public pages avoid private engineering history and vague marketing?

Visual acceptance, in-app probe execution, and the GH #201 response are manager
work. Only after this review should the manager explain that the living
Guide/demo fulfills the Wiki need through one canonical source.

## 7. Worker loop and stop conditions

An asynchronous documentation worker continues inside one pre-defined slice,
but does not select the next product area. Its loop is:

1. read the dossier's named journey, templates, exclusions and sources;
2. record each behavioral claim with `path:line` and a short exact quote;
3. edit only the allowed templates (plus their generated output);
4. regenerate and mechanically inspect the HTML structure and links;
5. run the focused gates and correct causal failures;
6. adversarially reread for unsupported claims, duplication, buried easy paths,
   and lost limitations; revise;
7. commit the coherent slice and update the progress report;
8. repeat only for the next pre-authorized sub-slice in the same dossier.

Stop and record a blocker when:

- authoritative sources materially disagree;
- the task requires deciding product semantics, defaults, status, or scope;
- the task requires adding or renaming a page not explicitly authorized;
- generated output reveals a product/export bug outside documentation;
- a claim cannot be verified on the available platform;
- the worker would need to alter application behavior rather than documentation
  content;
- the next change touches a manager-owned path listed in section 3.1.

The worker reports generator/checker/product defects; it does not fix them in
the same assignment.

## 8. Required proof

For every coherent content slice:

- record source paths, line numbers and short exact quotes in the progress
  report;
- for template-only edits, run `source scripts/env.sh && cargo test -p
  tine-core onboarding`; run the full `cargo test -p tine-core` when manifest or
  Guide/copy logic changes;
- run `source scripts/env.sh && npm run docs:build`;
- run `source scripts/env.sh && npm run docs:check`;
- mechanically check changed generated pages exist, contain their expected
  heading/links/examples, and introduce no missing local asset/Guide target;
- keep `website/demo/` checked in and synchronized.

If Rust manifest formatting changed, run `source scripts/env.sh && cargo fmt
--all -- --check`. Run `npm test` or `npx tsc --noEmit` only if TypeScript/TSX
changed. Pure template edits do not justify them.

At stable manager checkpoints:

- the manager visually inspects generated pages with the screenshot/browser
  harness, including mobile width;
- the manager runs `scripts/probe-guide.mjs` when the in-app Guide, namespace
  rewriting, or copy journey changed;
- broader unit/typecheck gates follow only if their source families changed.

Pure content edits do not justify the full release E2E corpus or hosted CI.

## 9. Deliverables

1. Revised/new canonical templates and narrowly authorized manifest entries.
2. Regenerated `website/demo/` with link/freshness checks green.
3. A concise progress ledger listing completed journeys, quoted sources, checks,
   contradictions and deliberate deferrals.
4. One commit per coherent slice, followed by a final integration report.
5. Manager-owned repairs to the link-closure and missing-target gates before any
   new page registration.

## 10. Acceptance criteria

- One canonical template set still feeds onboarding, in-app Guide, copied
  sandbox, and public demo.
- `Welcome to Tine` remains a short successful onboarding path.
- Existing page titles and public URLs remain stable.
- `Tine Guide` exposes clear Start, Workflows, Reference, and showcase lenses
  using only pages that exist.
- The assignment's pre-registered journey is accepted by the manager, not merely
  marked complete by its author.
- Each accepted workflow contains an actual example and observable result.
- Important platform, persistence, compatibility and safety boundaries appear
  where relevant.
- No public content depends on private agent notes or makes unsupported claims.
- Generated HTML is current and linked; the manager has visually accepted the
  changed desktop/mobile pages.
- The non-vacuous Guide/copy/link tests and focused content gates pass.
