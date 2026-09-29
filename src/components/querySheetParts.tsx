// The query sheet's parts: the resting sentence, the node model, the row
// controls, value editors, the anchor line and the add-condition flow (SPEC
// §7.2-§7.5). Split from `QuerySheet.tsx` along its seams (I-24).
import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  createUniqueId,
  onCleanup,
  onMount,
  type Accessor,
  type JSX,
} from "solid-js";
import { backend } from "../backend";
import {
  MAX_QUERY_BUILDER_DEPTH,
  betweenFilter,
  contentFilter,
  encodePropertyLeaf,
  filterChildren,
  groupWithPrevious,
  isDisabledAt,
  journalFilter,
  namespaceFilter,
  onPageFilter,
  pageRefFilter,
  pageTagsFilter,
  planningFilter,
  priorityFilter,
  propertyOperatorArity,
  propertyOperators,
  querySentence,
  searchFilter,
  taskFilter,
  BETWEEN_FIELDS,
  MARKERS,
  PRIORITIES,
  type BetweenField,
  type BuilderLeafKind,
  type GroupChoice,
  type PhraseSegment,
  type PropertyLeafTest,
  type PropertyOperatorId,
} from "../editor/queryBuilder";
import type { Anchor, Cardinality, Diagnostic, Filter, ObservedType, Query, RegistryRow } from "../editor/queryIr";
import { Listbox, stop, type ListboxOption } from "./QueryListbox";
import { QueryVocabularyPicker, type VocabularyChoice } from "./QueryVocabularyPicker";
import { PropertyType } from "./PropertyType";
import { registryRowFor, effectiveTypeOf } from "../editor/queryPropertyType";
import { DATE_PRESETS, previewDate } from "../editor/dateExpr";
import type { QuerySheetDropTarget } from "./querySheetReorder";
import { dismissOnOutsidePointer, registerTransientLayer, type TransientLayer } from "../transientLayers";

export { stop, Listbox, type ListboxOption };

/** `openMenu`'s key for the "Group selected ▾" chooser. */
export const GROUP_SELECTED_MENU_KEY = "group-selected";

export { registryRowFor, effectiveTypeOf } from "../editor/queryPropertyType";

export type { VocabularyChoice } from "./QueryVocabularyPicker";

/** The condition picker delegates vocabulary and keyboard behaviour to the
 * shared registry picker. Registry status is supplied by the sheet's one read. */
export function QueryFieldPicker(props: {
  id: string;
  anchor: Anchor;
  rows: Accessor<RegistryRow[] | undefined>;
  pending: Accessor<boolean>;
  failure: Accessor<Error | null>;
  onRetry: () => void;
  current?: VocabularyChoice | null;
  placeholder?: string;
  rootRef?: (element: HTMLDivElement) => void;
  onPick: (choice: VocabularyChoice) => void;
}): JSX.Element {
  return <QueryVocabularyPicker {...props} />;
}

/** The property registry for the current graph. An open sheet shares one read
 *  per graph, data revision and declaration revision. Rows stay visible during
 *  a re-read; `pending()` tells callers they may be stale. Type-dependent edits
 *  wait for the current read, and a failed read exposes an explicit retry. */
export interface RegistryAccess {
  rows: Accessor<RegistryRow[] | undefined>;
  /** The current revision's read is in flight; it ends without user action. */
  pending: Accessor<boolean>;
  /** The read for the current graph/declaration revision FAILED terminally. */
  failure: Accessor<Error | null>;
  /** There is no healthy registry for the current graph/revision — `pending()` or `failure()`. */
  unavailable: Accessor<boolean>;
  /** Bump the shared declaration revision and re-read in every open builder. */
  request: () => void;
  /** Re-read after a terminal failure; affects every open builder. */
  retry: () => void;
}

export const locKey = (l: number[]) => l.join(".");

/** `openMenu`'s key for the add-condition chooser. */
export const ADD_MENU_KEY = "add";

/** Every popover in the sheet — anchor menu, field chooser, operator menu, value
*  editors, and the sort/summarize pickers in the footer — registers here, so
*  all of them answer Escape/Back AND "the user pressed somewhere else" the same
*  way. GH #472 is what happens when they do not: two of the four had
*  hand-rolled the outside-press effect and two had not, so a menu stayed open
*  while the user clicked into and edited a different block. The trigger is
*  passed as inside-the-popover so its own click can toggle. */
export function registerVisiblePopover(open: () => boolean, layer: TransientLayer) {
  createEffect(() => {
    if (!open()) return;
    const unregister = registerTransientLayer(layer);
    onCleanup(unregister);
  });
  dismissOnOutsidePointer({
    open,
    inside: () => [layer.root?.(), layer.trigger?.()],
    dismiss: () => layer.dismiss("explicit"),
  });
}

// The resting sentence (§7.2)

/** A single condition rather than a group — the unit that renders as ONE row,
*  and the unit a `not`/`off` wrapper decorates without costing a level. */
export function isLeafLike(filter: Filter): boolean {
  return (
    filter.kind === "leaf" ||
    filter.kind === "raw" ||
    filter.kind === "true" ||
    filter.kind === "false"
  );
}

/** Where a retained leaf sits, and whether an `Off` encloses it. */
export interface RawLeafSite {
  leaf: Filter & { kind: "raw" };
  loc: number[];
  /** Derived from the CURRENT tree, never stored on the node (§4.3.2): a
  *  diagnostic inside an `Off` subtree does not invalidate the query. */
  disabled: boolean;
}

/** The `raw` leaves of a tree, in order — the red rows, and the conditions the anchor prompt names. */
export function rawLeaves(filter: Filter, loc: number[] = [], disabled = false): RawLeafSite[] {
  if (filter.kind === "raw") return [{ leaf: filter, loc, disabled }];
  const children = filterChildren(filter);
  if (!children) return [];
  const off = disabled || filter.kind === "off";
  return children.flatMap((child, index) => rawLeaves(child, [...loc, index], off));
}

/** How many conditions a tree holds — what the anchor prompt counts against. */
export function countConditions(filter: Filter): number {
  if (isLeafLike(filter)) return 1;
  const children = filterChildren(filter);
  if (!children) return 1;
  return children.reduce((total, child) => total + countConditions(child), 0);
}

/** Match a retained leaf to its source diagnostic by kind and exact span.
 *  With no spans, only an unambiguous single diagnostic of that kind is shown. */
export function diagnosticFor(query: Query | undefined, leaf: Filter & { kind: "raw" }): Diagnostic | undefined {
  const candidates = (query?.diagnostics ?? []).filter((d) => d.kind === leaf.diagnostic_kind);
  if (leaf.span) return candidates.find((d) => d.span?.start === leaf.span?.start && d.span?.end === leaf.span?.end);
  return candidates.length === 1 && !candidates[0].span ? candidates[0] : undefined;
}

/**
* **The resting state: one sentence, the count, and a ⚙ (§7.2, design §2.1).**
*
* Nothing here is a control except the ⚙ and the sentence itself. The sentence
* IS the affordance — clicking it, or pressing Enter or Space on it, opens the
* sheet — which is why it carries a button role and a visible focus ring rather
* than looking like text that happens to be clickable.
*/
export function QuerySentence(props: {
  query: Query;
  total?: JSX.Element;
  onOpen: () => void;
  open?: boolean;
  sentenceRef?: (element: HTMLSpanElement) => void;
}): JSX.Element {
  const segments = createMemo<PhraseSegment[]>(() =>
    querySentence({ anchor: props.query.anchor, filter: props.query.filter }),
  );
  // A retained leaf reads as its decoded text; the diagnostic is why it is red, so it is the hover text rather …
  const rawTitles = createMemo(() => {
    const titles = new Map<string, string>();
    for (const site of rawLeaves(props.query.filter)) {
      const diagnostic = diagnosticFor(props.query, site.leaf);
      if (diagnostic) titles.set(site.leaf.text, diagnostic.message);
    }
    return titles;
  });
  return (
    <div class="qs-line" onClick={stop}>
      <span
        ref={props.sentenceRef}
        class="qs-sentence"
        role="button"
        tabindex="0"
        aria-expanded={props.open ? "true" : "false"}
        title="Click to edit this query"
        onClick={(e) => {
          stop(e);
          props.onOpen();
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            e.stopPropagation();
            props.onOpen();
          }
        }}
      >
        <For each={segments()}>
          {(segment) => (
            <span
              class="qs-seg"
              classList={{
                "qs-seg-value": segment.kind === "value",
                "qs-seg-field": segment.kind === "field",
                "qs-seg-advanced": segment.kind === "advanced",
              }}
              title={segment.title ?? rawTitles().get(segment.text)}
            >
              {segment.text}
            </span>
          )}
        </For>
      </span>
      <Show when={props.total != null}>
        <span class="qs-count-slot">{props.total}</span>
      </Show>
      <button
        type="button"
        class="qs-gear"
        aria-label="Edit query filter"
        aria-expanded={props.open ? "true" : "false"}
        title="Edit query filter"
        onClick={(e) => {
          stop(e);
          props.onOpen();
        }}
      >
        ⚙
      </button>
    </div>
  );
}

// Row and group model — the tree, as the sheet draws it

/** The 13 field types a row can be (§7.4's P3 vocabulary). */
export const FILTER_TYPES: { kind: BuilderLeafKind; label: string }[] = [
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

export const FIELD_LABELS: Record<BuilderLeafKind, string> = {
  page: "Page / tag reference",
  task: "Task marker",
  priority: "Priority",
  property: "Property",
  scheduled: "Scheduled",
  deadline: "Deadline",
  journal: "On journal page",
  between: "Between dates",
  content: "Full-text search",
  onPage: "On page",
  namespace: "In namespace",
  pageProperty: "Page property",
  pageTags: "Page tags",
  search: "Full-text search",
};

/** The fixed operator phrase for a non-property row, and its negation. */
export const KIND_PHRASE: Record<BuilderLeafKind, { positive: string; negative: string }> = {
  page: { positive: "references", negative: "does not reference" },
  task: { positive: "is any of", negative: "is none of" },
  priority: { positive: "is any of", negative: "is none of" },
  property: { positive: "is", negative: "is not" },
  scheduled: { positive: "is set", negative: "is not set" },
  deadline: { positive: "is set", negative: "is not set" },
  journal: { positive: "is a journal page", negative: "is not a journal page" },
  between: { positive: "between", negative: "not between" },
  content: { positive: "contains", negative: "does not contain" },
  onPage: { positive: "on page", negative: "not on page" },
  namespace: { positive: "in namespace", negative: "not in namespace" },
  pageProperty: { positive: "is", negative: "is not" },
  pageTags: { positive: "is any of", negative: "is none of" },
  search: { positive: "matches", negative: "does not match" },
};

/** One thing the sheet draws. */
export type SheetNode =
  | {
      kind: "row";
      /** The OUTERMOST node — the `off`/`not` wrapper when there is one. */
      loc: number[];
      filter: Filter;
      /** The condition inside the wrappers. */
      core: Filter;
      negated: boolean;
      /** Where the `not` ITSELF is, when the row has one. */
      negLoc: number[] | null;
      /** This node carries its OWN `off` — the state its enabled control owns. */
      disabled: boolean;
      /** An ANCESTOR is disabled, so this condition does not run whatever its own switch says. */
      inherited: boolean;
    }
  | {
      kind: "group";
      loc: number[];
      /** Where the `and`/`or` itself lives, which is inside a `not` for the `none of` / `not all of` headers. */
      opLoc: number[];
      header: "all of" | "any of" | "none of" | "not all of";
      negated: boolean;
      /** Where the `not` itself is — see the row's own note. */
      negLoc: number[] | null;
      disabled: boolean;
      inherited: boolean;
      children: SheetNode[];
    }
  | { kind: "advanced"; loc: number[]; filter: Filter; disabled: boolean; inherited: boolean };

/**
* **The tree, as rows and groups (§7.4, design §2.5).**
*
* The stored form is rendered HONESTLY: §3.5 forbids De Morgan rewriting, so a
* `not` over an `or` reads "none of" and a `not` over an `and` reads "not all
* of" — the builder never silently restates the user's query as its dual.
*
* A `not`/`off` around a SINGLE condition is not a level: it is the row's
* negative operator and the row's greyed state. Around a group it is the
* group's header and the group's greyed state.
*/
export function buildNodes(filter: Filter, loc: number[], depth: number, inherited = false): SheetNode {
  if (depth >= MAX_QUERY_BUILDER_DEPTH) {
    return { kind: "advanced", loc, filter, disabled: isDisabledAt(filter, []), inherited };
  }
  let node = filter;
  let at = loc;
  let negated = false;
  let negLoc: number[] | null = null;
  let disabled = false;
  // Peel the decorations.
  for (;;) {
    if (node.kind === "off" && !disabled) {
      disabled = true;
      node = node.inner;
      at = [...at, 0];
      continue;
    }
    if (node.kind === "not" && !negated && (isLeafLike(node.inner) || node.inner.kind === "off")) {
      negated = true;
      negLoc = at;
      node = node.inner;
      at = [...at, 0];
      continue;
    }
    break;
  }
  // Everything below a disabled node is disabled too, whatever its own wrapper says: `Off` is structural …
  const under = inherited || disabled;
  if (node.kind === "and" || node.kind === "or") {
    return {
      kind: "group",
      loc,
      opLoc: at,
      header: negated ? (node.kind === "or" ? "none of" : "not all of") : node.kind === "or" ? "any of" : "all of",
      negated,
      negLoc,
      disabled,
      inherited,
      children: node.items.map((item, index) => buildNodes(item, [...at, index], depth + 1, under)),
    };
  }
  if (node.kind === "not") {
    // A `not` over a group: the group's own header carries it.
    const inner = node.inner;
    if (inner.kind === "and" || inner.kind === "or") {
      return {
        kind: "group",
        loc,
        opLoc: [...at, 0],
        header: inner.kind === "or" ? "none of" : "not all of",
        negated: true,
        negLoc: at,
        disabled,
        inherited,
        children: inner.items.map((item, index) => buildNodes(item, [...at, 0, index], depth + 1, under)),
      };
    }
    return { kind: "row", loc, filter, core: inner, negated: true, negLoc: at, disabled, inherited };
  }
  return { kind: "row", loc, filter, core: node, negated, negLoc, disabled, inherited };
}

// Selecting, disabling and reordering (§7.4 remainder, P6)

/** A node's place among its siblings — everything selection, moving and
*  dropping need, derived from the node's own `loc` plus how many siblings the
*  list it lives in has. */
export interface SiblingPos {
  /** The boolean node whose child list this item is in. */
  parentLoc: number[];
  index: number;
  count: number;
}

export const posOf = (loc: number[], count: number): SiblingPos => ({
  parentLoc: loc.slice(0, -1),
  index: loc[loc.length - 1],
  count,
});

/** **What is selected, in ONE rendered boolean list (§7.4).**
*
*  Selection is sibling-local by construction, not by a check afterwards: it
*  names one parent and indices inside it, so "select a row in a different
*  group" cannot express a selection that spans two lists and a group operation
*  can never move a condition between them. Choosing a sibling elsewhere starts
*  a new selection rather than extending this one. */
export interface SheetSelection {
  parentLoc: number[];
  indices: number[];
}

/** The controls the rows share. */
export interface SheetControls {
  selection: () => SheetSelection | null;
  isSelected: (loc: number[]) => boolean;
  toggleSelected: (loc: number[]) => void;
  clearSelection: () => void;
  /** Add or remove this node's own `Off`. */
  toggleEnabled: (loc: number[]) => void;
  /** Move within the list, by the index the item ends at. */
  move: (pos: SiblingPos, to: number) => void;
  startDrag: (event: PointerEvent, pos: SiblingPos) => void;
  dropTarget: () => QuerySheetDropTarget | null;
}

/** The drop indicator classes for one item, or nothing when the drag is elsewhere. */
export function dropClasses(controls: SheetControls, pos: SiblingPos): Record<string, boolean> {
  const target = controls.dropTarget();
  const mine = !!target && target.parent === locKey(pos.parentLoc) && target.index === pos.index;
  return {
    "qs-drop-before": mine && target!.before,
    "qs-drop-after": mine && !target!.before,
  };
}

/** **The drag handle, and the keyboard operation that equals it (§7.4, §7.7).**
*
*  A drag starts HERE and nowhere else, so typing a value, pressing a menu,
*  scrolling and selecting text are untouched. It is a button rather than a
*  decorated `<span>` because the keyboard has to reach the same operation: Up
*  and Down on a focused handle move the item exactly as a drop would, through
*  the same `moveSibling`, and focus stays on the handle that moved so a second
*  press continues rather than starting over. */
export function DragHandle(props: { pos: SiblingPos; label: string; controls: SheetControls }): JSX.Element {
  const loc = () => [...props.pos.parentLoc, props.pos.index];
  return (
    <button
      type="button"
      class="qs-drag-handle"
      data-qs-handle={locKey(loc())}
      aria-label={props.label}
      aria-keyshortcuts="ArrowUp ArrowDown"
      title="Drag to reorder, or use the up and down arrow keys"
      onPointerDown={(event) => props.controls.startDrag(event, props.pos)}
      onKeyDown={(event) => {
        if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
        event.preventDefault();
        event.stopPropagation();
        props.controls.move(props.pos, props.pos.index + (event.key === "ArrowUp" ? -1 : 1));
      }}
      onClick={stop}
    >
      ⋮⋮
    </button>
  );
}

/** **The selection box — NOT the enabled control (§7.4).**
*
*  They were one control in the chip bar's descendants and in most filter
*  builders: a checkbox that both picked the row and switched it off. Two
*  questions ("which rows am I about to group?" and "which conditions run?")
*  answered by one box means every grouping gesture silently changes what the
*  query returns. */
export function SelectBox(props: { pos: SiblingPos; label: string; controls: SheetControls }): JSX.Element {
  const loc = () => [...props.pos.parentLoc, props.pos.index];
  return (
    <label class="qs-select-target" onClick={stop}>
      <input
        type="checkbox"
        class="qs-select"
        aria-label={props.label}
        checked={props.controls.isSelected(loc())}
        onChange={() => props.controls.toggleSelected(loc())}
      />
    </label>
  );
}

/** **The enabled control: this node's own `Off`, told honestly (§3.5, §7.4).**
*
*  `aria-checked` is the node's OWN state, because that is the only state this
*  switch owns. When an ancestor is disabled the row does not run whatever this
*  switch says, and the row says so in words beside it rather than letting the
*  switch imply that one press here would bring the condition back. */
export function EnabledSwitch(props: {
  loc: number[];
  disabled: boolean;
  inherited: boolean;
  label: string;
  controls: SheetControls;
}): JSX.Element {
  return (
    <button
      type="button"
      role="switch"
      class="qs-enabled"
      aria-label={props.label}
      aria-checked={props.disabled ? "false" : "true"}
      title={
        props.inherited
          ? "The group above is disabled, so this does not run"
          : props.disabled
            ? "Disabled — kept in the query, not run"
            : "Running — press to disable without removing it"
      }
      onClick={(event) => {
        stop(event);
        props.controls.toggleEnabled(props.loc);
      }}
    >
      {props.disabled ? "○" : "●"}
    </button>
  );
}

/** Why this item is greyed, in the sheet's own words. `null` when it runs. */
export function offLabel(node: { disabled: boolean; inherited: boolean }): string | null {
  if (node.inherited) return "disabled by group";
  return node.disabled ? "disabled" : null;
}

/** The move entries every item's ⋮ menu carries, offered only where they mean
*  something: the first item has nothing above it and the last has nothing
*  below. They move the SAME siblings the drag does, through the same helper. */
export function moveOptions(pos: SiblingPos): ListboxOption[] {
  const options: ListboxOption[] = [];
  if (pos.index > 0) options.push({ key: "move-up", label: "Move up" });
  if (pos.index < pos.count - 1) options.push({ key: "move-down", label: "Move down" });
  if (pos.index > 0) {
    options.push(
      { key: "group-above-all", label: "Group with row above — all of" },
      { key: "group-above-any", label: "Group with row above — any of" },
      { key: "group-above-none", label: "Group with row above — none of" },
    );
  }
  return options;
}

export const GROUP_ABOVE_CHOICE: Record<string, GroupChoice> = {
  "group-above-all": "all",
  "group-above-any": "any",
  "group-above-none": "none",
};

/** Handle one of {@link moveOptions}' keys. */
export function pickMoveOption(
  key: string,
  pos: SiblingPos,
  node: { loc: number[] },
  sheet: QuerySheetProps,
  controls: SheetControls,
): boolean {
  if (key === "move-up") {
    controls.move(pos, pos.index - 1);
    return true;
  }
  if (key === "move-down") {
    controls.move(pos, pos.index + 1);
    return true;
  }
  const choice = GROUP_ABOVE_CHOICE[key];
  if (!choice) return false;
  sheet.apply(groupWithPrevious(sheet.root(), node.loc, choice));
  return true;
}

/** A popover anchored to a trigger button, registered in the dismissal ladder. */
export function Popover(props: {
  open: () => boolean;
  close: () => void;
  parentId?: string;
  trigger: () => HTMLElement | null;
  children: (rootRef: (element: HTMLDivElement) => void) => JSX.Element;
}): JSX.Element {
  let rootEl: HTMLDivElement | undefined;
  const layerId = `query-sheet-menu-${createUniqueId()}`;
  registerVisiblePopover(props.open, {
    id: layerId,
    parentId: props.parentId,
    root: () => rootEl ?? null,
    trigger: props.trigger,
    dismiss: () => {
      props.close();
      return true;
    },
  });
  return (
    <Show when={props.open()}>
      {props.children((element) => {
        rootEl = element;
      })}
    </Show>
  );
}

// Value editors — lifted intact from the chip bar into the row's value cell

/** Plain free-text input that commits on Enter. */
export function TextInput(props: {
  placeholder: string;
  initial?: string;
  onCommit: (text: string) => void;
}): JSX.Element {
  const [v, setV] = createSignal(props.initial ?? "");
  return (
    <div class="qs-value-editor">
      <input
        class="qs-input"
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

/** Page-name input with fuzzy autocomplete from the graph. */
export function PageInput(props: {
  placeholder: string;
  initial?: string;
  onCommit: (name: string) => void;
}): JSX.Element {
  const [q, setQ] = createSignal(props.initial ?? "");
  const [dq, setDq] = createSignal(props.initial ?? "");
  let dqTimer: ReturnType<typeof setTimeout> | undefined;
  createEffect(() => {
    const s = q();
    clearTimeout(dqTimer);
    dqTimer = setTimeout(() => setDq(s), 120);
  });
  onCleanup(() => clearTimeout(dqTimer));
  const [matchesResource] = createResource(dq, (s) => backend().quickSwitch(s, 8));
  // A failed lookup offers no completions; the next keystroke re-runs it.
  const matches = () => (matchesResource.error === undefined ? matchesResource() : undefined);
  return (
    <div class="qs-value-editor">
      <input
        class="qs-input"
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
          <button type="button" class="qs-option" onClick={() => props.onCommit(p.name)}>
            {p.name}
          </button>
        )}
      </For>
    </div>
  );
}

/** Multi-select (task markers, priorities) with checkboxes + an Add button. */
export function MultiPick(props: {
  options: string[];
  initial?: string[];
  onCommit: (picked: string[]) => void;
}): JSX.Element {
  const [picked, setPicked] = createSignal<string[]>(props.initial ?? []);
  const toggle = (o: string) =>
    setPicked(picked().includes(o) ? picked().filter((x) => x !== o) : [...picked(), o]);
  return (
    <div class="qs-value-editor">
      <For each={props.options}>
        {(o) => (
          <label class="qs-check">
            <input type="checkbox" checked={picked().includes(o)} onChange={() => toggle(o)} /> {o}
          </label>
        )}
      </For>
      <button
        type="button"
        class="qs-commit"
        disabled={picked().length === 0}
        onClick={() => props.onCommit(picked())}
      >
        Apply
      </button>
    </div>
  );
}

/** A single date-bound input with a live resolved-date preview underneath. */
export function DateBoundInput(props: {
  placeholder: string;
  value: string;
  onInput: (v: string) => void;
  onEnter: () => void;
  autofocus?: boolean;
}): JSX.Element {
  const preview = createMemo(() => previewDate(props.value));
  return (
    <div class="qs-bound">
      <input
        class="qs-input"
        autofocus={props.autofocus}
        placeholder={props.placeholder}
        value={props.value}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") props.onEnter();
        }}
      />
      <span class="qs-bound-preview">{preview() ? `→ ${preview()}` : " "}</span>
    </div>
  );
}

export const BETWEEN_FIELD_LABEL: Record<BetweenField, string> = {
  journal: "Journal date",
  scheduled: "Scheduled",
  deadline: "Deadline",
  any: "Any date",
};

/** Date-range editor: which date, one-click relative presets, and two bound
*  inputs that accept keywords (`today`), relative offsets (`-30d`), ISO dates
*  or a journal-page title — each with a live resolved-date preview. */
export function BetweenPick(props: {
  onCommit: (field: BetweenField, start: string, end: string) => void;
}): JSX.Element {
  const [field, setField] = createSignal<BetweenField>("journal");
  const [start, setStart] = createSignal("");
  const [end, setEnd] = createSignal("");
  const ready = () => !!start().trim() && !!end().trim();
  const submit = () => {
    if (ready()) props.onCommit(field(), start().trim(), end().trim());
  };
  return (
    <div class="qs-value-editor qs-between">
      <div class="qs-between-field">
        <For each={BETWEEN_FIELDS}>
          {(f) => (
            <button
              type="button"
              class="qs-conn"
              classList={{ active: field() === f }}
              onClick={() => setField(f)}
            >
              {BETWEEN_FIELD_LABEL[f]}
            </button>
          )}
        </For>
      </div>
      <div class="qs-between-presets">
        <For each={DATE_PRESETS}>
          {(p) => (
            <button
              type="button"
              class="qs-preset"
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
      <DateBoundInput
        placeholder="Start — today, -30d, 2026-06-01, or a page"
        value={start()}
        onInput={setStart}
        onEnter={submit}
        autofocus
      />
      <DateBoundInput
        placeholder="End — today, +7d, 2026-06-30, or a page"
        value={end()}
        onInput={setEnd}
        onEnter={submit}
      />
      <button type="button" class="qs-commit" disabled={!ready()} onClick={submit}>
        Apply
      </button>
    </div>
  );
}

/** The value collector for a filter kind, and the IR leaf `og.rs` builds for the same intent. */
export function ValueEditor(props: {
  kind: BuilderLeafKind;
  onCommit: (filter: Filter) => void;
}): JSX.Element {
  return (
    <>
      <Show when={props.kind === "page"}>
        <PageInput placeholder="Page or tag name" onCommit={(name) => props.onCommit(pageRefFilter(name))} />
      </Show>
      <Show when={props.kind === "task"}>
        <MultiPick options={MARKERS} onCommit={(markers) => props.onCommit(taskFilter(markers))} />
      </Show>
      <Show when={props.kind === "priority"}>
        <MultiPick options={PRIORITIES} onCommit={(levels) => props.onCommit(priorityFilter(levels))} />
      </Show>
      <Show when={props.kind === "between"}>
        <BetweenPick onCommit={(field, start, end) => props.onCommit(betweenFilter(field, start, end))} />
      </Show>
      <Show when={props.kind === "onPage"}>
        <PageInput placeholder="Page name" onCommit={(name) => props.onCommit(onPageFilter(name))} />
      </Show>
      <Show when={props.kind === "namespace"}>
        <PageInput placeholder="Namespace (parent page)" onCommit={(ns) => props.onCommit(namespaceFilter(ns))} />
      </Show>
      <Show when={props.kind === "content"}>
        <TextInput placeholder="Text to search for" onCommit={(text) => props.onCommit(contentFilter(text))} />
      </Show>
      <Show when={props.kind === "search"}>
        <TextInput placeholder="Search words and operators" onCommit={(source) => props.onCommit(searchFilter(source))} />
      </Show>
      <Show when={props.kind === "pageTags"}>
        <TextInput placeholder="Tag (one)" onCommit={(t) => props.onCommit(pageTagsFilter([t]))} />
      </Show>
    </>
  );
}

// The sheet

/** The counted prompt an anchor switch raises (§7.4, design §2.3). */
export interface AnchorPrompt {
  anchor: Anchor;
  /** How many conditions do not apply, and how many there are in total. */
  count: number;
  total: number;
  /** The phrases of the conditions that do not apply. */
  names: string[];
  onRemove: () => void;
  onKeep: () => void;
  onCancel: () => void;
}

export const ANCHOR_OPTIONS: { key: Anchor; label: string; hint: string }[] = [
  { key: "block", label: "blocks", hint: "individual bullets, anywhere in the graph" },
  { key: "page", label: "pages", hint: "whole pages" },
];

export interface QuerySheetProps {
  anchor: () => Anchor;
  onAnchor: (anchor: Anchor) => void;
  anchorPrompt: () => AnchorPrompt | null;
  /** The `and`/`or` root the sheet edits. */
  root: () => Filter;
  /** The whole query, for the diagnostics a red row shows. */
  query: () => Query | undefined;
  apply: (next: Filter) => void | Promise<boolean>;
  registry: RegistryAccess;
  /** Open and focus the query text pane — the route the `⟨advanced⟩` control
  *  takes, and the route a row that cannot be edited here always has (§7.4). */
  onEditText?: () => void;
  /** The four most frequent property keys, for the empty state's "Try:" line. */
  suggestions: () => string[];
  /** Which menu inside the sheet is open, by row loc + purpose. */
  openMenu: () => string | null;
  setOpenMenu: (key: string | null) => void;
  /** The layer every menu in here parents to. */
  layerId?: string;
  /** Open the field chooser on an empty add row the moment the sheet appears — the `/query` entry (§7.3). */
  autoOpenChooser?: boolean;
  footer?: JSX.Element;
  sheetRef?: (element: HTMLDivElement) => void;
  stale?: boolean;
}

export function AnchorLine(props: {
  anchor: () => Anchor;
  onAnchor: (anchor: Anchor) => void;
  empty: boolean;
  openMenu: () => string | null;
  setOpenMenu: (key: string | null) => void;
  layerId?: string;
}): JSX.Element {
  let triggerEl: HTMLButtonElement | undefined;
  const menuId = `qs-anchor-${createUniqueId()}`;
  const open = () => props.openMenu() === "anchor";
  const label = () => (props.anchor() === "page" ? "pages" : "blocks");
  return (
    <div class="qs-anchor">
      {/* Not a row and not deletable: it is the sentence's subject (§7.4). */}
      <span class="qs-anchor-lead">Find</span>
      <span class="qs-anchor-wrap">
        <button
          ref={triggerEl}
          type="button"
          class="qs-anchor-button"
          aria-haspopup="listbox"
          aria-expanded={open() ? "true" : "false"}
          aria-controls={menuId}
          title="What this query selects"
          onClick={(e) => {
            stop(e);
            props.setOpenMenu(open() ? null : "anchor");
          }}
        >
          {label()} ▾
        </button>
        <Popover
          open={open}
          close={() => props.setOpenMenu(null)}
          parentId={props.layerId}
          trigger={() => triggerEl ?? null}
        >
          {(rootRef) => (
            <Listbox
              id={menuId}
              label="What this query selects"
              rootRef={rootRef}
              options={ANCHOR_OPTIONS.map((option) => ({
                key: option.key,
                label: option.label,
                hint: option.hint,
                active: option.key === props.anchor(),
              }))}
              onPick={(key) => {
                props.setOpenMenu(null);
                props.onAnchor(key as Anchor);
              }}
            />
          )}
        </Popover>
      </span>
      <Show when={props.empty}>
        <span class="qs-anchor-tail">where …</span>
      </Show>
    </div>
  );
}

export function AnchorPromptPanel(props: { prompt: AnchorPrompt }): JSX.Element {
  let panelEl: HTMLDivElement | undefined;
  onMount(() => panelEl?.focus());
  const row = () => (props.prompt.anchor === "page" ? "pages" : "blocks");
  return (
    <div
      ref={panelEl}
      class="qs-anchor-prompt"
      role="alertdialog"
      aria-label="Switching what this query selects"
      tabindex="-1"
    >
      <p class="qs-anchor-prompt-text">
        Switching to <strong>{row()}</strong> — {props.prompt.count} of your {props.prompt.total}{" "}
        conditions don't apply to {row()}
        <Show when={props.prompt.names.length > 0}>
          {" "}
          (
          <For each={props.prompt.names}>
            {(name, index) => (
              <>
                <Show when={index() > 0}>, </Show>
                <code>{name}</code>
              </>
            )}
          </For>
          )
        </Show>
        .
      </p>
      <div class="qs-anchor-prompt-actions">
        <button type="button" class="qs-commit" onClick={() => props.prompt.onRemove()}>
          Remove them
        </button>
        <button type="button" class="qs-conn" onClick={() => props.prompt.onKeep()}>
          Keep anyway
        </button>
        <button type="button" class="qs-conn" onClick={() => props.prompt.onCancel()}>
          Cancel
        </button>
      </div>
    </div>
  );
}

// Rows and groups

/** **"Group selected ▾", and what is selected right now (§7.4).**
*
*  One bar for the whole sheet rather than a control per list: the selection is
*  already one list's, and a phone-width row has no space for a sixth control.
*  It states the count, because a non-contiguous selection two groups down is
*  otherwise invisible, and it offers the same three headers a group can carry.
*  Grouping needs two; with one selected the action is offered but refused, so
*  the rule is visible rather than mysterious. */
export function SelectionBar(props: {
  selection: SheetSelection;
  sheet: QuerySheetProps;
  controls: SheetControls;
  onGroup: (choice: GroupChoice) => void;
}): JSX.Element {
  let trigger: HTMLButtonElement | undefined;
  const menuId = `qs-group-selected-${createUniqueId()}`;
  const open = () => props.sheet.openMenu() === GROUP_SELECTED_MENU_KEY;
  const count = () => props.selection.indices.length;
  return (
    <div class="qs-selection" role="group" aria-label="Selected conditions">
      <span class="qs-selection-count">{count()} selected</span>
      <span class="qs-menu-wrap">
        <button
          ref={trigger}
          type="button"
          class="qs-group-selected"
          disabled={count() < 2}
          title={count() < 2 ? "Select two or more conditions to group them" : undefined}
          aria-haspopup="listbox"
          aria-expanded={open() ? "true" : "false"}
          aria-controls={menuId}
          onClick={(e) => {
            stop(e);
            props.sheet.setOpenMenu(open() ? null : GROUP_SELECTED_MENU_KEY);
          }}
        >
          Group selected ▾
        </button>
        <Popover
          open={open}
          close={() => props.sheet.setOpenMenu(null)}
          parentId={props.sheet.layerId}
          trigger={() => trigger ?? null}
        >
          {(rootRef) => (
            <Listbox
              id={menuId}
              label="Group the selected conditions"
              rootRef={rootRef}
              options={GROUP_CHOICES}
              onPick={(key) => props.onGroup(key as GroupChoice)}
            />
          )}
        </Popover>
      </span>
      <button type="button" class="qs-selection-clear" onClick={(e) => {
        stop(e);
        props.controls.clearSelection();
      }}>
        Clear
      </button>
    </div>
  );
}

/** The three group headers, in the order the sheet's own menus use. */
export const GROUP_CHOICES: ListboxOption[] = [
  { key: "all", label: "All of" },
  { key: "any", label: "Any of" },
  { key: "none", label: "None of" },
];

/** The effective type of the key a property row tests, from the ONE registry read. */
export function effectiveFor(
  registry: RegistryAccess,
  key: string | undefined,
): { type: ObservedType; cardinality: Cardinality } {
  const row = key ? registryRowFor(registry.rows(), key) : undefined;
  return row ? effectiveTypeOf(row) : { type: "text", cardinality: "one" };
}

/** A property row's key and zero to two plain text inputs. Values come from the
 *  current leaf; `effective` and `registry` are reserved for Q4b's type badge.
 *  Enter or blur commits only a changed value, avoiding a reprint and undo step
 *  when focus leaves an untouched input. */
export function PropertyValueCell(props: {
  test: PropertyLeafTest;
  effective: { type: ObservedType; cardinality: Cardinality };
  disabled: boolean;
  registry: RegistryAccess;
  onCommit: (values: string[]) => void;
}): JSX.Element {
  const arity = () => propertyOperatorArity(props.test.id);
  const [low, setLow] = createSignal(props.test.values[0] ?? "");
  const [high, setHigh] = createSignal(props.test.values[1] ?? "");
  createEffect(() => {
    setLow(props.test.values[0] ?? "");
    setHigh(props.test.values[1] ?? "");
  });
  const commit = () => {
    const values = arity() === 2 ? [low(), high()] : arity() === 1 ? [low()] : [];
    if (values.length === props.test.values.length && values.every((value, i) => value === props.test.values[i])) return;
    props.onCommit(values);
  };
  return (
    <span class="qs-property-value">
      <span class="qs-property-key">{props.test.key}</span>
      <PropertyType propertyKey={props.test.key} rows={props.registry.rows}
        onDeclarationWritten={props.registry.request} readOnly={props.disabled} />
      <Show when={arity() > 0}>
        <input
          class="qs-input"
          aria-label={arity() === 2 ? "From" : "Value"}
          placeholder={arity() === 2 ? "From" : "Value"}
          disabled={props.disabled}
          value={low()}
          onInput={(e) => setLow(e.currentTarget.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
          }}
        />
      </Show>
      <Show when={arity() === 2}>
        <input
          class="qs-input"
          aria-label="To"
          placeholder="To"
          disabled={props.disabled}
          value={high()}
          onInput={(e) => setHigh(e.currentTarget.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
          }}
        />
      </Show>
    </span>
  );
}

/** The add-a-condition row: the ONE vocabulary picker, then the value editor for what was chosen. */
export function AddCondition(props: {
  sheet: QuerySheetProps;
  open: Accessor<boolean>;
  setOpen: (open: boolean) => void;
  onAdd: (filter: Filter) => void;
}): JSX.Element {
  const [chosen, setChosen] = createSignal<VocabularyChoice | null>(null);
  const [propertyId, setPropertyId] = createSignal<PropertyOperatorId>("is");
  const [propertyValues, setPropertyValues] = createSignal<string[]>([]);
  let triggerEl: HTMLButtonElement | undefined;
  const chooserId = `qs-add-${createUniqueId()}`;
  const reset = () => {
    setChosen(null);
    setPropertyValues([]);
  };
  const close = () => {
    reset();
    props.setOpen(false);
  };
  const property = () => {
    const choice = chosen();
    return choice?.kind === "property" ? choice : null;
  };
  const effective = createMemo(() => effectiveFor(props.sheet.registry, property()?.key));
  /** The chosen key's operators and value encoding are registry answers, so
  *  while there is no healthy registry the editor keeps the key and the draft
  *  on screen and refuses to commit — rather than encoding a `number` key, or
  *  one whose declaration has just been written, as text (§6.3). A terminal
  *  failure holds the commit exactly as an in-flight read does; what differs
  *  is what the editor SAYS, and that only `retry()` ends it. */
  const registryFailure = () => props.sheet.registry.failure();
  const registryUnavailable = () => props.sheet.registry.unavailable();
  const commitProperty = () => {
    const choice = property();
    if (!choice || registryUnavailable()) return;
    const filter = encodePropertyLeaf({
      id: propertyId(),
      key: choice.key,
      values: propertyValues(),
      type: effective().type,
      throughPage: choice.throughPage,
    });
    if (!filter) return;
    props.onAdd(filter);
    // Enter on a completed value commits the row and reopens the chooser for the next condition (design §2.9).
    reset();
    props.setOpen(true);
  };
  const choose = (choice: VocabularyChoice) => {
    if (choice.kind === "builtin") {
      const next = choice.leaf;
      if (next === "scheduled" || next === "deadline") return props.onAdd(planningFilter(next));
      if (next === "journal") return props.onAdd(journalFilter());
      setChosen(choice);
      return;
    }
    setChosen(choice);
    // The family's first identity is pre-selected the moment the key is chosen, so the common case needs no …
    if (!registryUnavailable()) setPropertyId(propertyOperators(effective())[0]?.id ?? "is");
  };
  // The key stays chosen across the wait — through a readiness retry, through a failure, and through an …
  createEffect(() => {
    if (registryUnavailable() || !property()) return;
    if (!propertyOperators(effective()).some((operator) => operator.id === propertyId())) {
      setPropertyId(propertyOperators(effective())[0]?.id ?? "is");
    }
  });
  return (
    <div class="qs-add-wrap">
      <button
        ref={triggerEl}
        type="button"
        class="qs-add"
        aria-haspopup="listbox"
        aria-expanded={props.open() ? "true" : "false"}
        aria-controls={chooserId}
        onClick={(e) => {
          stop(e);
          props.open() ? close() : props.setOpen(true);
        }}
      >
        + Add condition
      </button>
      <Popover open={props.open} close={close} parentId={props.sheet.layerId} trigger={() => triggerEl ?? null}>
        {(rootRef) => (
          <Show
            when={chosen()}
            fallback={
              <QueryFieldPicker
                id={chooserId}
                anchor={props.sheet.anchor()}
                rows={props.sheet.registry.rows}
                pending={props.sheet.registry.pending}
                failure={props.sheet.registry.failure}
                onRetry={props.sheet.registry.retry}
                placeholder="Type to add a condition"
                rootRef={rootRef}
                onPick={choose}
              />
            }
          >
            {(choice) => (
              <div ref={rootRef} class="qs-menu" onClick={stop}>
                <Show
                  when={property()}
                  fallback={
                    <ValueEditor
                      kind={(choice() as VocabularyChoice & { kind: "builtin" }).leaf}
                      onCommit={(filter) => {
                        props.onAdd(filter);
                        reset();
                        props.setOpen(true);
                      }}
                    />
                  }
                >
                  {(pick) => (
                    <div class="qs-value-editor">
                      <div class="qs-menu-title">
                        {pick().key}
                        <Show when={pick().throughPage}>
                          <span class="qs-menu-scope"> (page property)</span>
                        </Show>
                      </div>
                      {/* Q4b seam: the key\'s type badge + "declare type…" (PropertyType) mounts here. */}
                      <Show
                        when={!registryUnavailable()}
                        fallback={
                          /* The key and draft stay where the user left them. The
                             comparison waits because its options come from the
                             registry; a failed read offers an explicit retry. */
                          <Show
                            when={registryFailure()}
                            fallback={
                              <div class="qs-registry-pending" role="status">
                                Reading this graph's properties…
                              </div>
                            }
                          >
                            {(failure) => (
                              <div class="qs-registry-failure" role="alert">
                                <span class="qs-registry-failure-message">
                                  This graph's properties could not be read. {failure().message}
                                </span>
                                <button
                                  type="button"
                                  class="qs-registry-retry"
                                  onClick={(e) => {
                                    stop(e);
                                    props.sheet.registry.retry();
                                  }}
                                >
                                  Try again
                                </button>
                              </div>
                            )}
                          </Show>
                        }
                      >
                        <Listbox
                          class="qs-inline-list"
                          id={`${chooserId}-op`}
                          label="Condition operator"
                          options={propertyOperators(effective()).map((operator) => ({
                            key: operator.id,
                            label: operator.label,
                            active: operator.id === propertyId(),
                          }))}
                          onPick={(picked) => {
                            setPropertyId(picked as PropertyOperatorId);
                            if (propertyOperatorArity(picked as PropertyOperatorId) === 0) {
                              queueMicrotask(commitProperty);
                            }
                          }}
                        />
                      </Show>
                      <Show when={propertyOperatorArity(propertyId()) > 0}>
                        <input
                          class="qs-input"
                          autofocus
                          aria-label={propertyOperatorArity(propertyId()) === 2 ? "From" : "Value"}
                          placeholder={propertyOperatorArity(propertyId()) === 2 ? "From" : "Value"}
                          disabled={registryUnavailable()}
                          value={propertyValues()[0] ?? ""}
                          onInput={(e) => setPropertyValues([e.currentTarget.value, propertyValues()[1] ?? ""])}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") commitProperty();
                          }}
                        />
                      </Show>
                      <Show when={propertyOperatorArity(propertyId()) === 2}>
                        <input
                          class="qs-input"
                          aria-label="To"
                          placeholder="To"
                          disabled={registryUnavailable()}
                          value={propertyValues()[1] ?? ""}
                          onInput={(e) => setPropertyValues([propertyValues()[0] ?? "", e.currentTarget.value])}
                          onKeyDown={(e) => { if (e.key === "Enter") commitProperty(); }}
                        />
                      </Show>
                      <button
                        type="button"
                        class="qs-commit"
                        disabled={registryUnavailable()}
                        onClick={commitProperty}
                      >
                        Add
                      </button>
                    </div>
                  )}
                </Show>
              </div>
            )}
          </Show>
        )}
      </Popover>
    </div>
  );
}
