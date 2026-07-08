import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
  type JSX,
} from "solid-js";
import { backend } from "../backend";
import {
  parseQuery,
  toDsl,
  clauseToAdvanced,
  clauseLabel,
  addChild,
  removeAt,
  replaceAt,
  wrapAt,
  unwrapAt,
  setOp,
  MARKERS,
  PRIORITIES,
  BETWEEN_FIELDS,
  SORT_PRESETS,
  type Clause,
  type BetweenField,
  type SortPreset,
} from "../editor/queryBuilder";
import { DATE_PRESETS, previewDate } from "../editor/dateExpr";
import { pushToast, queryBuilderAutoOpen, setQueryBuilderAutoOpen } from "../ui";

// Interactive query builder: an OG-style chip-bar over a {{query}} DSL string.
// The DSL text is the single source of truth — we parse it to a tree, apply an
// immutable edit, and write the new DSL back through `onChange` (which rewrites
// the owning block). Results then re-run reactively.
//
// `stop` keeps clicks inside the bar from bubbling to the block's onClick, which
// would drop the block into raw-text edit mode and replace the builder.
const stop = (e: MouseEvent) => e.stopPropagation();
const locKey = (l: number[]) => l.join(".");

type ClauseKind = Clause["kind"];

// Sort is query-GLOBAL (not a filter chip), so it's handled separately from the
// clause tree: a single root-level `sortBy` child. These helpers read/replace it.
function rootChildren(root: Clause): Clause[] {
  return root.kind === "op" && root.op === "and" ? root.children : [root];
}
function currentSort(root: Clause): { field: string; dir: "asc" | "desc" } | null {
  const s = rootChildren(root).find((c) => c.kind === "sortBy");
  return s && s.kind === "sortBy" ? { field: s.field, dir: s.dir } : null;
}
function withSort(root: Clause, sort: { field: string; dir: "asc" | "desc" } | null): Clause {
  // Explicit Clause[] — else TS narrows the filtered array to exclude sortBy
  // (inferred type predicate) and rejects the push below.
  const kids: Clause[] = rootChildren(root).filter((c) => c.kind !== "sortBy");
  if (sort && sort.field.trim()) {
    kids.push({ kind: "sortBy", field: sort.field.trim(), dir: sort.dir });
  }
  // Re-wrap as a root `and`; toDsl simplifies a single child back to bare form.
  return { kind: "op", op: "and", children: kids };
}

// Aggregation + grouping are result-level directives (like sort), held as
// root-level `aggregate`/`groupBy` children and edited via the "+ summarize"
// control below — not as filter chips. These helpers read/replace them.
type AggState = { agg: "count" | "sum" | "avg"; field: string | null };
function currentAgg(root: Clause): AggState | null {
  const a = rootChildren(root).find((c) => c.kind === "aggregate");
  return a && a.kind === "aggregate" ? { agg: a.agg, field: a.field } : null;
}
function currentGroup(root: Clause): string | null {
  const g = rootChildren(root).find((c) => c.kind === "groupBy");
  return g && g.kind === "groupBy" ? g.field : null;
}
function withAgg(root: Clause, agg: AggState | null): Clause {
  const kids: Clause[] = rootChildren(root).filter((c) => c.kind !== "aggregate");
  if (agg) kids.push({ kind: "aggregate", agg: agg.agg, field: agg.field });
  return { kind: "op", op: "and", children: kids };
}
function withGroup(root: Clause, field: string | null): Clause {
  const kids: Clause[] = rootChildren(root).filter((c) => c.kind !== "groupBy");
  if (field && field.trim()) kids.push({ kind: "groupBy", field: field.trim() });
  return { kind: "op", op: "and", children: kids };
}

// A small "+ sort" / "sort: field ↑" control in the bar (NOT a filter chip).
// The popover leads with one-click presets (the common cases — no typing, no
// syntax to get wrong) and keeps a free-text row for sorting by any other
// property. `SORT_PRESETS` is the single source of truth (see queryBuilder.ts).
function SortControl(props: { tree: () => Clause; apply: (c: Clause) => void }): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const cur = () => currentSort(props.tree());
  // The free-text escape hatch: sort by an arbitrary property name.
  const [field, setField] = createSignal("");
  const [dir, setDir] = createSignal<"asc" | "desc">("asc");
  let wrapEl: HTMLSpanElement | undefined;
  // Dismiss the popover when clicking anywhere outside it (another chip, the bar
  // background, or off the block). Capture phase so it fires regardless of the
  // bar's stopPropagation; only active while open.
  createEffect(() => {
    if (!open()) return;
    const onDown = (e: MouseEvent) => {
      if (wrapEl && !wrapEl.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown, true);
    onCleanup(() => document.removeEventListener("mousedown", onDown, true));
  });
  const isPreset = (c: { field: string; dir: "asc" | "desc" } | null) =>
    !!c && SORT_PRESETS.some((p) => p.field === c.field && p.dir === c.dir);
  const activePreset = (p: SortPreset) => {
    const c = cur();
    return !!c && c.field === p.field && c.dir === p.dir;
  };
  const openPopover = () => {
    const c = cur();
    // Pre-fill the free-text row only for a non-preset (custom-property) sort;
    // a preset sort is reflected by its highlighted button instead.
    setField(c && !isPreset(c) ? c.field : "");
    setDir(c?.dir ?? "asc");
    setOpen(true);
  };
  const applyPreset = (p: SortPreset) => {
    props.apply(withSort(props.tree(), { field: p.field, dir: p.dir }));
    setOpen(false);
  };
  const applyCustom = () => {
    if (!field().trim()) return;
    props.apply(withSort(props.tree(), { field: field().trim(), dir: dir() }));
    setOpen(false);
  };
  const clearSort = () => {
    props.apply(withSort(props.tree(), null));
    setOpen(false);
  };
  return (
    <span class="qb-add-wrap" ref={wrapEl}>
      {/* A stable "+ sort" affordance — it does NOT morph into the current sort
          value (the active sort shows as its own chip in the bar). It just gains
          an `active` highlight and opens the popover to change/clear. */}
      <button
        class="qb-sort"
        classList={{ active: !!cur() }}
        title={cur() ? `Sorted by ${clauseLabel({ kind: "sortBy", field: cur()!.field, dir: cur()!.dir })}. Click to change.` : "Sort results"}
        onClick={(e) => { stop(e); openPopover(); }}
      >
        + sort
      </button>
      <Show when={open()}>
        <div class="qb-picker qb-sort-picker" onClick={stop}>
          <div class="qb-picker-title">Sort by</div>
          {/* One click = applied. No typing for the common cases. */}
          <div class="qb-sort-presets">
            <For each={SORT_PRESETS}>
              {(p) => (
                <button
                  class="qb-sort-preset"
                  classList={{ active: activePreset(p) }}
                  title={p.hint}
                  onClick={() => applyPreset(p)}
                >
                  {p.label}
                </button>
              )}
            </For>
          </div>
          <div class="qb-divider" />
          {/* Escape hatch: sort by any other property (still no required syntax —
              just the bare property name + a direction). */}
          <div class="qb-sort-custom-label">Or by a property</div>
          <input
            class="qb-input"
            placeholder="property name (e.g. rating)"
            value={field()}
            onInput={(e) => setField(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter") applyCustom(); }}
          />
          <div class="qb-conn-row">
            <button class="qb-conn" classList={{ active: dir() === "asc" }} onClick={() => setDir("asc")}>Asc ↑</button>
            <button class="qb-conn" classList={{ active: dir() === "desc" }} onClick={() => setDir("desc")}>Desc ↓</button>
            <button class="qb-conn" classList={{ disabled: !field().trim() }} onClick={applyCustom}>Apply</button>
          </div>
          <Show when={cur()}>
            <button class="qb-sort-clear" onClick={clearSort}>Clear sort</button>
          </Show>
        </div>
      </Show>
    </span>
  );
}

// A "+ summarize" control: no-code aggregation (count / sum / average of a
// property) and grouping (by page or a property). Modeled on SortControl — a
// single pill + popover, dismiss-on-outside-click. Aggregate + group are
// independent (you can group by page AND count per group). The numbers are
// computed in the frontend from the returned block list (Macro.tsx); this just
// edits the DSL directive that rides along.
function SummarizeControl(props: { tree: () => Clause; apply: (c: Clause) => void }): JSX.Element {
  const [open, setOpen] = createSignal(false);
  // Two-step property choice: null = show the top-level buttons; "sum"/"avg" =
  // pick a property to aggregate; "group" = pick a property to group by.
  const [pick, setPick] = createSignal<"sum" | "avg" | "group" | null>(null);
  const [facets] = createResource(() => backend().queryFacets());
  const keys = () => (facets() ?? []).map(([k]) => k);
  const agg = () => currentAgg(props.tree());
  const group = () => currentGroup(props.tree());
  const active = () => !!agg() || !!group();
  let wrapEl: HTMLSpanElement | undefined;
  createEffect(() => {
    if (!open()) return;
    const onDown = (e: MouseEvent) => {
      if (wrapEl && !wrapEl.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown, true);
    onCleanup(() => document.removeEventListener("mousedown", onDown, true));
  });
  const openPopover = () => {
    setPick(null);
    setOpen(true);
  };
  // Each pick applies and closes the popover (like SortControl's presets). To set
  // BOTH an aggregate and a grouping, reopen — the two are independent, so the DSL
  // keeps whichever the other pick already set.
  const setAgg = (a: AggState | null) => {
    props.apply(withAgg(props.tree(), a));
    setPick(null);
    setOpen(false);
  };
  const setGroup = (f: string | null) => {
    props.apply(withGroup(props.tree(), f));
    setPick(null);
    setOpen(false);
  };
  const label = () => {
    const parts: string[] = [];
    const a = agg();
    if (a) parts.push(a.agg === "count" ? "count" : `${a.agg} of ${a.field ?? "?"}`);
    const g = group();
    if (g) parts.push(`by ${g}`);
    return parts.join(", ");
  };
  return (
    <span class="qb-add-wrap" ref={wrapEl}>
      <button
        class="qb-sort"
        classList={{ active: active() }}
        title={active() ? `Summary: ${label()}. Click to change.` : "Summarize results (count / sum / average / group)"}
        onClick={(e) => { stop(e); open() ? setOpen(false) : openPopover(); }}
      >
        {active() ? `∑ ${label()}` : "+ summarize"}
      </button>
      <Show when={open()}>
        <div class="qb-picker" onClick={stop}>
          {/* Step: pick a property for sum / avg / group-by. */}
          <Show when={pick() != null} fallback={
            <>
              <div class="qb-picker-title">Aggregate</div>
              <button class="qb-menu-item" classList={{ active: agg()?.agg === "count" }} onClick={() => setAgg({ agg: "count", field: null })}>Count</button>
              <button class="qb-menu-item" classList={{ active: agg()?.agg === "sum" }} onClick={() => setPick("sum")}>Sum of a property…</button>
              <button class="qb-menu-item" classList={{ active: agg()?.agg === "avg" }} onClick={() => setPick("avg")}>Average of a property…</button>
              <Show when={agg()}>
                <button class="qb-sort-clear" onClick={() => setAgg(null)}>Clear aggregate</button>
              </Show>
              <div class="qb-divider" />
              <div class="qb-picker-title">Group by</div>
              <button class="qb-menu-item" classList={{ active: group() === "page" }} onClick={() => setGroup("page")}>Page</button>
              <button class="qb-menu-item" classList={{ active: !!group() && group() !== "page" }} onClick={() => setPick("group")}>Property…</button>
              <Show when={group()}>
                <button class="qb-sort-clear" onClick={() => setGroup(null)}>Clear grouping</button>
              </Show>
            </>
          }>
            <div class="qb-picker-title">{pick() === "group" ? "Group by property" : `${pick() === "sum" ? "Sum" : "Average"} of property`}</div>
            <For each={keys()}>
              {(k) => (
                <button class="qb-menu-item" onClick={() => (pick() === "group" ? setGroup(k) : setAgg({ agg: pick() as "sum" | "avg", field: k }))}>
                  {k}
                </button>
              )}
            </For>
            <PropNameInput onCommit={(k) => (pick() === "group" ? setGroup(k) : setAgg({ agg: pick() as "sum" | "avg", field: k }))} />
          </Show>
        </div>
      </Show>
    </span>
  );
}

// A free-text property-name input (Enter commits) for the summarize picker, so a
// property not yet used in the graph (absent from facets) can still be chosen.
function PropNameInput(props: { onCommit: (key: string) => void }): JSX.Element {
  const [v, setV] = createSignal("");
  return (
    <input
      class="qb-input"
      placeholder="or type a property name"
      value={v()}
      onInput={(e) => setV(e.currentTarget.value)}
      onKeyDown={(e) => { if (e.key === "Enter" && v().trim()) props.onCommit(v().trim()); }}
    />
  );
}

// "Switch to advanced" converts the CURRENT builder tree to its single-line
// Datalog equivalent (clauseToAdvanced) — it must never discard the query and
// never write a multi-line macro: lsdoc `{{query …}}` macros don't span lines,
// so the old multi-line skeleton made the block stop parsing as a query at all
// and silently destroyed the user's simple query (Jul 8 data-mutation bug).
// The clause cheat-sheet lives in the pill's tooltip instead of the file.
const ADVANCED_CHEATSHEET =
  'Convert this query to an advanced (Datalog) [:find …] form. Supported clauses: ' +
  '(task ?b "TODO" "DOING"), (priority ?b "A"), (page-ref ?b "Page"), (property ?b :key "v"), ' +
  '(page-property ?b :key), (page-tags ?b "tag"), (scheduled ?b), (deadline ?b), (journal ?b), ' +
  '(page ?b "Name"), (namespace ?b "Parent"), (between ?b "2026-01-01" "2026-12-31"), ' +
  'combined with (and …) (or …) (not …). Unsupported clauses are flagged, never guessed. ' +
  'Keep it on ONE line and avoid #{…} sets — a query macro cannot span lines or contain braces.';

export function QueryBuilder(props: {
  dsl: () => string;
  onChange: (dsl: string) => void;
  blockId?: string;
}): JSX.Element {
  const tree = createMemo(() => parseQuery(props.dsl()));
  // Which popover is open, by op/clause loc + purpose. Only one at a time.
  const [openMenu, setOpenMenu] = createSignal<string | null>(null);
  // Open the root add-picker immediately when this block was just created via
  // "/Query (visual builder)" — consume the one-shot flag so only this block does.
  const autoOpen = !!props.blockId && queryBuilderAutoOpen() === props.blockId;
  if (autoOpen) setQueryBuilderAutoOpen(null);
  const [adding, setAdding] = createSignal<string | null>(autoOpen ? "add:" : null);

  const apply = (next: Clause) => {
    props.onChange(toDsl(next));
    setOpenMenu(null);
    setAdding(null);
  };

  return (
    <div class="qb-bar" onClick={stop}>
      <Node clause={tree()} loc={[]} isRoot tree={tree} apply={apply}
        openMenu={openMenu} setOpenMenu={setOpenMenu} adding={adding} setAdding={setAdding} />
      <SortControl tree={tree} apply={apply} />
      <SummarizeControl tree={tree} apply={apply} />
      <button
        class="qb-sort qb-advanced"
        title={ADVANCED_CHEATSHEET}
        onClick={(e) => {
          stop(e);
          const conv = clauseToAdvanced(tree());
          if (!conv.ok) {
            pushToast(`Can't auto-convert to Datalog: ${conv.unsupported.join(", ")} has no advanced equivalent — write the [:find …] form by hand`);
            return;
          }
          props.onChange(conv.dsl);
          pushToast(
            conv.dropped.length
              ? `Converted to an advanced Datalog query (dropped: ${conv.dropped.join(", ")}) — undo restores the simple form`
              : "Converted to an advanced Datalog query — undo restores the simple form"
          );
        }}
      >
        ⚙ advanced
      </button>
    </div>
  );
}

interface NodeCtx {
  loc: number[];
  isRoot?: boolean;
  clause: Clause;
  tree: () => Clause;
  apply: (next: Clause) => void;
  openMenu: () => string | null;
  setOpenMenu: (k: string | null) => void;
  adding: () => string | null;
  setAdding: (k: string | null) => void;
}

function Node(props: NodeCtx): JSX.Element {
  const isOp = () => props.clause.kind === "op";
  const op = () => props.clause as Clause & { kind: "op" };

  return (
    <Show when={isOp()} fallback={<Chip {...props} />}>
      <Show when={op().op === "not"} fallback={<OpGroup {...props} />}>
        <span class="qb-op-not">
          <span class="qb-bracket">NOT(</span>
          <For each={op().children}>
            {(child, i) => (
              <Node {...props} clause={child} loc={[...props.loc, i()]} isRoot={false} />
            )}
          </For>
          <AddButton {...props} />
          <ChipMenu {...props} />
          <span class="qb-bracket">)</span>
        </span>
      </Show>
    </Show>
  );
}

// An and/or operator node: optional bracket + a clickable operator pill that
// flips and<->or, its children, and a trailing "+".
function OpGroup(props: NodeCtx): JSX.Element {
  const op = () => props.clause as Clause & { kind: "op" };
  const showBracket = () => !props.isRoot;
  const showOpPill = () => op().children.length > 1 || !props.isRoot;
  const flip = () => props.apply(setOp(props.tree(), props.loc, op().op === "and" ? "or" : "and"));

  return (
    <span class="qb-group" classList={{ "qb-root": props.isRoot }}>
      <Show when={showBracket()}>
        <span class="qb-bracket">(</span>
      </Show>
      <Show when={showOpPill()}>
        <button
          class="qb-op"
          title="Toggle AND / OR"
          onClick={(e) => {
            stop(e);
            flip();
          }}
        >
          {op().op.toUpperCase()}
        </button>
      </Show>
      <For each={op().children}>
        {(child, i) => (
          <Node {...props} clause={child} loc={[...props.loc, i()]} isRoot={false} />
        )}
      </For>
      <AddButton {...props} prominent={props.isRoot && op().children.length === 0} />
      <Show when={!props.isRoot}>
        <ChipMenu {...props} />
        <span class="qb-bracket">)</span>
      </Show>
    </span>
  );
}

// A leaf filter chip. Click opens an action menu (delete / wrap).
function Chip(props: NodeCtx): JSX.Element {
  const key = () => `chip:${locKey(props.loc)}`;
  return (
    <span class="qb-chip-wrap">
      <button
        class="qb-chip"
        classList={{ "qb-chip-raw": props.clause.kind === "raw" }}
        title="Click: delete / wrap in AND·OR·NOT (to exclude or nest)"
        onClick={(e) => {
          stop(e);
          props.setOpenMenu(props.openMenu() === key() ? null : key());
        }}
      >
        {clauseLabel(props.clause)}
      </button>
      <ChipMenu {...props} />
    </span>
  );
}

// Per-clause action popover (delete, wrap in AND/OR/NOT, and for op nodes:
// unwrap). Shown for both leaf chips and operator nodes.
function ChipMenu(props: NodeCtx): JSX.Element {
  const isOpKey = () => props.clause.kind === "op";
  const key = () => `${isOpKey() ? "op" : "chip"}:${locKey(props.loc)}`;
  const open = () => props.openMenu() === key();
  const act = (f: () => Clause) => () => props.apply(f());
  // The root op has no enclosing position to delete/wrap from.
  const atRoot = () => props.loc.length === 0;

  // Result-level directives (sort / aggregate / group-by) aren't filters: they're
  // edited via the "+ sort" / "+ summarize" controls, so their chip menu offers
  // only Delete (no Edit, no wrap-in-AND/OR/NOT — wrapping one would nest it out
  // of the root and silently disable it).
  const isSort = () =>
    props.clause.kind === "sortBy" ||
    props.clause.kind === "aggregate" ||
    props.clause.kind === "groupBy";

  const [editing, setEditing] = createSignal(false);
  // Nullary clauses (scheduled/deadline/journal), result-level directives, and
  // raw/op have nothing to edit here.
  const canEdit = () =>
    !isOpKey() &&
    !["raw", "scheduled", "deadline", "journal", "sortBy", "aggregate", "groupBy"].includes(
      props.clause.kind,
    );
  const editKind = () => (props.clause.kind === "raw" ? "page" : (props.clause.kind as ClauseKind));

  return (
    <Show when={open()}>
      <div class="qb-menu" onClick={stop}>
        <Show when={editing()} fallback={
          <>
            <Show when={canEdit()}>
              <button class="qb-menu-item" onClick={() => setEditing(true)}>Edit…</button>
            </Show>
            <Show when={!atRoot()}>
              <button class="qb-menu-item" onClick={act(() => removeAt(props.tree(), props.loc))}>Delete</button>
              <Show when={!isSort()}>
                <button class="qb-menu-item" onClick={act(() => wrapAt(props.tree(), props.loc, "and"))}>Wrap in AND</button>
                <button class="qb-menu-item" onClick={act(() => wrapAt(props.tree(), props.loc, "or"))}>Wrap in OR</button>
                <button class="qb-menu-item" onClick={act(() => wrapAt(props.tree(), props.loc, "not"))}>Wrap in NOT</button>
              </Show>
            </Show>
            <Show when={isOpKey() && !atRoot()}>
              <button class="qb-menu-item" onClick={act(() => unwrapAt(props.tree(), props.loc))}>Unwrap</button>
            </Show>
          </>
        }>
          <div class="qb-picker-title">Edit value</div>
          <ValuePicker kind={editKind()} onCommit={(c) => props.apply(replaceAt(props.tree(), props.loc, c))} />
        </Show>
      </div>
    </Show>
  );
}

// "+" button that opens the add-filter picker, scoped to the op at `loc`. When
// `prominent` (an empty query), render an inviting "➕ Add filter" call-to-action
// instead of a bare "+", so leaving the bullet reveals an obvious next step.
function AddButton(props: NodeCtx & { prominent?: boolean }): JSX.Element {
  const key = () => `add:${locKey(props.loc)}`;
  const open = () => props.adding() === key();
  return (
    <span class="qb-add-wrap">
      <button
        class="qb-add"
        classList={{ "qb-add-prominent": props.prominent }}
        title="Add filter"
        onClick={(e) => {
          stop(e);
          props.setAdding(open() ? null : key());
        }}
      >
        {props.prominent ? "➕ Add filter" : "+"}
      </button>
      <Show when={open()}>
        <AddPicker
          onCommit={(c) => props.apply(addChild(props.tree(), props.loc, c))}
          onSetOp={(op) => props.apply(setOp(props.tree(), props.loc, op))}
        />
      </Show>
    </span>
  );
}

// ---------------------------------------------------------------------------
// Add-filter picker: choose a clause type, then collect its value(s).
// ---------------------------------------------------------------------------

const FILTER_TYPES: { kind: ClauseKind; label: string }[] = [
  { kind: "page", label: "Page / tag reference" },
  { kind: "task", label: "Task marker" },
  { kind: "priority", label: "Priority" },
  { kind: "property", label: "Property" },
  { kind: "scheduled", label: "Scheduled" },
  { kind: "deadline", label: "Deadline" },
  { kind: "journal", label: "On journal page" },
  { kind: "between", label: "Between dates" },
  { kind: "content", label: "Full-text search" },
  { kind: "onPage", label: "On page" },
  { kind: "namespace", label: "In namespace" },
  { kind: "pageProperty", label: "Page property" },
  { kind: "pageTags", label: "Page tags" },
];

function AddPicker(props: {
  onCommit: (c: Clause) => void;
  onSetOp: (op: "and" | "or") => void;
}): JSX.Element {
  const [step, setStep] = createSignal<ClauseKind | "type">("type");
  // When armed, the next filter is added negated (wrapped in NOT).
  const [negate, setNegate] = createSignal(false);

  const pick = (kind: ClauseKind) => {
    if (kind === "scheduled" || kind === "deadline" || kind === "journal") return commit({ kind });
    setStep(kind);
  };
  const commit = (c: Clause) => props.onCommit(negate() ? { kind: "op", op: "not", children: [c] } : c);

  return (
    <div class="qb-picker" onClick={stop}>
      <Show when={step() === "type"}>
        {/* Connectives first (OG-style): AND/OR set how this group joins;
            NOT arms negation for the filter you pick next. */}
        <div class="qb-conn-row">
          <button class="qb-conn" title="Join this group with AND" onClick={() => props.onSetOp("and")}>AND</button>
          <button class="qb-conn" title="Join this group with OR" onClick={() => props.onSetOp("or")}>OR</button>
          <button class="qb-conn" classList={{ active: negate() }} title="Exclude the next filter (NOT)" onClick={() => setNegate(!negate())}>NOT</button>
        </div>
        <div class="qb-divider" />
        <div class="qb-picker-title">{negate() ? "Exclude filter…" : "Add filter"}</div>
        <For each={FILTER_TYPES}>
          {(t) => (
            <button class="qb-menu-item" onClick={() => pick(t.kind)}>
              {t.label}
            </button>
          )}
        </For>
      </Show>
      <Show when={step() !== "type"}>
        <ValuePicker kind={step() as ClauseKind} onCommit={commit} />
      </Show>
    </div>
  );
}

// Renders the value collector for a given clause kind. Shared by the add-filter
// picker and the in-place "Edit value" flow.
function ValuePicker(props: { kind: ClauseKind; onCommit: (c: Clause) => void }): JSX.Element {
  return (
    <>
      <Show when={props.kind === "page"}>
        <PageInput placeholder="Page or tag name" onCommit={(name) => props.onCommit({ kind: "page", name })} />
      </Show>
      <Show when={props.kind === "task"}>
        <MultiPick options={MARKERS} onCommit={(markers) => props.onCommit({ kind: "task", markers })} />
      </Show>
      <Show when={props.kind === "priority"}>
        <MultiPick options={PRIORITIES} onCommit={(levels) => props.onCommit({ kind: "priority", levels })} />
      </Show>
      <Show when={props.kind === "property"}>
        <PropertyPick onCommit={(key, value) => props.onCommit({ kind: "property", key, value })} />
      </Show>
      <Show when={props.kind === "between"}>
        <BetweenPick onCommit={(field, start, end) => props.onCommit({ kind: "between", field, start, end })} />
      </Show>
      <Show when={props.kind === "onPage"}>
        <PageInput placeholder="Page name" onCommit={(name) => props.onCommit({ kind: "onPage", name })} />
      </Show>
      <Show when={props.kind === "namespace"}>
        <PageInput placeholder="Namespace (parent page)" onCommit={(ns) => props.onCommit({ kind: "namespace", ns })} />
      </Show>
      <Show when={props.kind === "pageProperty"}>
        <PropertyPick onCommit={(key, value) => props.onCommit({ kind: "pageProperty", key, value })} />
      </Show>
      <Show when={props.kind === "content"}>
        <TextInput placeholder="Text to search for" onCommit={(text) => props.onCommit({ kind: "content", text })} />
      </Show>
      <Show when={props.kind === "pageTags"}>
        <TextInput placeholder="Tag (one)" onCommit={(t) => props.onCommit({ kind: "pageTags", tags: [t] })} />
      </Show>
    </>
  );
}

// Plain free-text input that commits on Enter.
function TextInput(props: { placeholder: string; onCommit: (text: string) => void }): JSX.Element {
  const [v, setV] = createSignal("");
  return (
    <div class="qb-value">
      <input
        class="qb-input"
        autofocus
        placeholder={props.placeholder}
        value={v()}
        onInput={(e) => setV(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && v().trim()) props.onCommit(v().trim());
        }}
      />
    </div>
  );
}

// Page-name input with fuzzy autocomplete from the graph.
function PageInput(props: { placeholder: string; onCommit: (name: string) => void }): JSX.Element {
  const [q, setQ] = createSignal("");
  // Debounce the backend fuzzy-match (quick_switch lists pages from disk) so
  // holding a key doesn't fire an IPC + dir scan per character.
  const [dq, setDq] = createSignal("");
  let dqTimer: ReturnType<typeof setTimeout> | undefined;
  createEffect(() => {
    const s = q();
    clearTimeout(dqTimer);
    dqTimer = setTimeout(() => setDq(s), 120);
  });
  onCleanup(() => clearTimeout(dqTimer));
  const [matches] = createResource(dq, (s) => backend().quickSwitch(s, 8));
  return (
    <div class="qb-value">
      <input
        class="qb-input"
        autofocus
        placeholder={props.placeholder}
        value={q()}
        onInput={(e) => setQ(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && q().trim()) props.onCommit(q().trim());
        }}
      />
      <For each={matches() ?? []}>
        {(p) => (
          <button class="qb-menu-item" onClick={() => props.onCommit(p.name)}>
            {p.name}
          </button>
        )}
      </For>
    </div>
  );
}

// Multi-select (task markers, priorities) with checkboxes + an Add button.
function MultiPick(props: { options: string[]; onCommit: (picked: string[]) => void }): JSX.Element {
  const [picked, setPicked] = createSignal<string[]>([]);
  const toggle = (o: string) =>
    setPicked(picked().includes(o) ? picked().filter((x) => x !== o) : [...picked(), o]);
  return (
    <div class="qb-value">
      <For each={props.options}>
        {(o) => (
          <label class="qb-check">
            <input type="checkbox" checked={picked().includes(o)} onChange={() => toggle(o)} /> {o}
          </label>
        )}
      </For>
      <button class="qb-commit" disabled={picked().length === 0} onClick={() => props.onCommit(picked())}>
        Add
      </button>
    </div>
  );
}

// Property: choose a key (autocompleted from used properties), then a value
// (from that key's known values, "any", or free text).
function PropertyPick(props: { onCommit: (key: string, value: string | null) => void }): JSX.Element {
  const [facets] = createResource(() => backend().queryFacets());
  const [key, setKey] = createSignal("");
  const [chosen, setChosen] = createSignal<string | null>(null);
  const keys = () => (facets() ?? []).map(([k]) => k);
  const valuesFor = (k: string) => (facets() ?? []).find(([kk]) => kk === k)?.[1] ?? [];
  const [val, setVal] = createSignal("");

  return (
    <div class="qb-value">
      <Show
        when={chosen() != null}
        fallback={
          <>
            <input
              class="qb-input"
              autofocus
              placeholder="Property key"
              value={key()}
              onInput={(e) => setKey(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && key().trim()) setChosen(key().trim());
              }}
            />
            <For each={keys().filter((k) => k.toLowerCase().includes(key().toLowerCase()))}>
              {(k) => (
                <button class="qb-menu-item" onClick={() => { setKey(k); setChosen(k); }}>
                  {k}
                </button>
              )}
            </For>
          </>
        }
      >
        <div class="qb-picker-title">{chosen()}</div>
        <button class="qb-menu-item" onClick={() => props.onCommit(chosen()!, null)}>
          (any value)
        </button>
        <For each={valuesFor(chosen()!)}>
          {(v) => (
            <button class="qb-menu-item" onClick={() => props.onCommit(chosen()!, v)}>
              {v}
            </button>
          )}
        </For>
        <input
          class="qb-input"
          placeholder="Custom value"
          value={val()}
          onInput={(e) => setVal(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") props.onCommit(chosen()!, val().trim() || null);
          }}
        />
      </Show>
    </div>
  );
}

// Date-range picker. A field selector (which date to test), one-click relative
// presets, and two bound inputs that accept keywords (`today`), relative offsets
// (`-30d`), ISO dates, or a journal-page title — each with a live resolved-date
// preview so the free-text accepts more than its placeholder hints.
const FIELD_LABEL: Record<BetweenField, string> = {
  journal: "Journal date",
  scheduled: "Scheduled",
  deadline: "Deadline",
  any: "Any date",
};
function BetweenPick(props: { onCommit: (field: BetweenField, start: string, end: string) => void }): JSX.Element {
  const [field, setField] = createSignal<BetweenField>("journal");
  const [start, setStart] = createSignal("");
  const [end, setEnd] = createSignal("");
  const ready = () => !!start().trim() && !!end().trim();
  const submit = () => {
    if (ready()) props.onCommit(field(), start().trim(), end().trim());
  };
  return (
    <div class="qb-between">
      <div class="qb-between-field">
        <For each={BETWEEN_FIELDS}>
          {(f) => (
            <button class="qb-conn" classList={{ active: field() === f }} onClick={() => setField(f)}>
              {FIELD_LABEL[f]}
            </button>
          )}
        </For>
      </div>
      <div class="qb-between-presets">
        <For each={DATE_PRESETS}>
          {(p) => (
            <button
              class="qb-preset"
              title={`${p.start} → ${p.end}`}
              onClick={() => {
                setStart(p.start);
                setEnd(p.end);
              }}
            >
              {p.label}
            </button>
          )}
        </For>
      </div>
      <DateBoundInput placeholder="Start — today, -30d, 2026-06-01, or a page" value={start()} onInput={setStart} onEnter={submit} autofocus />
      <DateBoundInput placeholder="End — today, +7d, 2026-06-30, or a page" value={end()} onInput={setEnd} onEnter={submit} />
      <button class="qb-commit" disabled={!ready()} onClick={submit}>
        Add
      </button>
    </div>
  );
}

// A single date-bound input with a live resolved-date preview underneath.
function DateBoundInput(props: {
  placeholder: string;
  value: string;
  onInput: (v: string) => void;
  onEnter: () => void;
  autofocus?: boolean;
}): JSX.Element {
  const preview = createMemo(() => previewDate(props.value));
  return (
    <div class="qb-bound">
      <input
        class="qb-input"
        autofocus={props.autofocus}
        placeholder={props.placeholder}
        value={props.value}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") props.onEnter();
        }}
      />
      <span class="qb-bound-preview">{preview() ? `→ ${preview()}` : " "}</span>
    </div>
  );
}
