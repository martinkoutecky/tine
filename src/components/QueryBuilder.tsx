import {
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
  type JSX,
} from "solid-js";
import { backend } from "../backend";
import {
  parseQuery,
  toDsl,
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
  type Clause,
  type BetweenField,
} from "../editor/queryBuilder";
import { DATE_PRESETS, previewDate } from "../editor/dateExpr";

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

export function QueryBuilder(props: {
  dsl: () => string;
  onChange: (dsl: string) => void;
}): JSX.Element {
  const tree = createMemo(() => parseQuery(props.dsl()));
  // Which popover is open, by op/clause loc + purpose. Only one at a time.
  const [openMenu, setOpenMenu] = createSignal<string | null>(null);
  const [adding, setAdding] = createSignal<string | null>(null);

  const apply = (next: Clause) => {
    props.onChange(toDsl(next));
    setOpenMenu(null);
    setAdding(null);
  };

  return (
    <div class="qb-bar" onClick={stop}>
      <Node clause={tree()} loc={[]} isRoot tree={tree} apply={apply}
        openMenu={openMenu} setOpenMenu={setOpenMenu} adding={adding} setAdding={setAdding} />
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
      <AddButton {...props} />
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

  const [editing, setEditing] = createSignal(false);
  // Nullary clauses (scheduled/deadline/journal) and raw/op have nothing to edit.
  const canEdit = () =>
    !isOpKey() && !["raw", "scheduled", "deadline", "journal"].includes(props.clause.kind);
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
              <button class="qb-menu-item" onClick={act(() => wrapAt(props.tree(), props.loc, "and"))}>Wrap in AND</button>
              <button class="qb-menu-item" onClick={act(() => wrapAt(props.tree(), props.loc, "or"))}>Wrap in OR</button>
              <button class="qb-menu-item" onClick={act(() => wrapAt(props.tree(), props.loc, "not"))}>Wrap in NOT</button>
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

// "+" button that opens the add-filter picker, scoped to the op at `loc`.
function AddButton(props: NodeCtx): JSX.Element {
  const key = () => `add:${locKey(props.loc)}`;
  const open = () => props.adding() === key();
  return (
    <span class="qb-add-wrap">
      <button
        class="qb-add"
        title="Add filter"
        onClick={(e) => {
          stop(e);
          props.setAdding(open() ? null : key());
        }}
      >
        +
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
  const [matches] = createResource(q, (s) => backend().quickSwitch(s, 8));
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
