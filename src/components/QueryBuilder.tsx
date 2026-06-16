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
  wrapAt,
  unwrapAt,
  setOp,
  MARKERS,
  PRIORITIES,
  type Clause,
} from "../editor/queryBuilder";

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

  return (
    <Show when={open()}>
      <div class="qb-menu" onClick={stop}>
        <Show when={!atRoot()}>
          <button class="qb-menu-item" onClick={act(() => removeAt(props.tree(), props.loc))}>
            Delete
          </button>
          <button class="qb-menu-item" onClick={act(() => wrapAt(props.tree(), props.loc, "and"))}>
            Wrap in AND
          </button>
          <button class="qb-menu-item" onClick={act(() => wrapAt(props.tree(), props.loc, "or"))}>
            Wrap in OR
          </button>
          <button class="qb-menu-item" onClick={act(() => wrapAt(props.tree(), props.loc, "not"))}>
            Wrap in NOT
          </button>
        </Show>
        <Show when={isOpKey() && !atRoot()}>
          <button class="qb-menu-item" onClick={act(() => unwrapAt(props.tree(), props.loc))}>
            Unwrap
          </button>
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
          onClose={() => props.setAdding(null)}
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
  { kind: "between", label: "Between dates" },
  { kind: "content", label: "Full-text search" },
  { kind: "onPage", label: "On page" },
  { kind: "namespace", label: "In namespace" },
  { kind: "pageProperty", label: "Page property" },
  { kind: "pageTags", label: "Page tags" },
];

function AddPicker(props: {
  onCommit: (c: Clause) => void;
  onClose: () => void;
}): JSX.Element {
  const [step, setStep] = createSignal<ClauseKind | "page" | "type">("type");

  const pick = (kind: ClauseKind | "page") => {
    if (kind === "scheduled") return props.onCommit({ kind: "scheduled" });
    if (kind === "deadline") return props.onCommit({ kind: "deadline" });
    setStep(kind);
  };

  return (
    <div class="qb-picker" onClick={stop}>
      <Show when={step() === "type"}>
        <div class="qb-picker-title">Add filter</div>
        <For each={FILTER_TYPES}>
          {(t) => (
            <button class="qb-menu-item" onClick={() => pick(t.kind)}>
              {t.label}
            </button>
          )}
        </For>
      </Show>
      <Show when={step() === "page"}>
        <PageInput placeholder="Page or tag name" onCommit={(name) => props.onCommit({ kind: "page", name })} />
      </Show>
      <Show when={step() === "task"}>
        <MultiPick options={MARKERS} onCommit={(markers) => props.onCommit({ kind: "task", markers })} />
      </Show>
      <Show when={step() === "priority"}>
        <MultiPick options={PRIORITIES} onCommit={(levels) => props.onCommit({ kind: "priority", levels })} />
      </Show>
      <Show when={step() === "property"}>
        <PropertyPick onCommit={(key, value) => props.onCommit({ kind: "property", key, value })} />
      </Show>
      <Show when={step() === "between"}>
        <BetweenPick onCommit={(start, end) => props.onCommit({ kind: "between", start, end })} />
      </Show>
      <Show when={step() === "onPage"}>
        <PageInput placeholder="Page name" onCommit={(name) => props.onCommit({ kind: "onPage", name })} />
      </Show>
      <Show when={step() === "namespace"}>
        <PageInput placeholder="Namespace (parent page)" onCommit={(ns) => props.onCommit({ kind: "namespace", ns })} />
      </Show>
      <Show when={step() === "pageProperty"}>
        <PropertyPick onCommit={(key, value) => props.onCommit({ kind: "pageProperty", key, value })} />
      </Show>
      <Show when={step() === "content"}>
        <TextInput placeholder="Text to search for" onCommit={(text) => props.onCommit({ kind: "content", text })} />
      </Show>
      <Show when={step() === "pageTags"}>
        <TextInput placeholder="Tag (one)" onCommit={(t) => props.onCommit({ kind: "pageTags", tags: [t] })} />
      </Show>
    </div>
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

// Between two journal dates (page-name inputs with autocomplete).
function BetweenPick(props: { onCommit: (start: string, end: string) => void }): JSX.Element {
  const [start, setStart] = createSignal("");
  const [end, setEnd] = createSignal("");
  return (
    <div class="qb-value">
      <input class="qb-input" autofocus placeholder="Start (journal page)" value={start()} onInput={(e) => setStart(e.currentTarget.value)} />
      <input class="qb-input" placeholder="End (journal page)" value={end()} onInput={(e) => setEnd(e.currentTarget.value)} />
      <button class="qb-commit" disabled={!start().trim() || !end().trim()} onClick={() => props.onCommit(start().trim(), end().trim())}>
        Add
      </button>
    </div>
  );
}
