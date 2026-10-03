// **The query sheet (SPEC §7.2-§7.5).** At rest a query block is one
// plain-English sentence; opened, it is this sheet of rows and groups. The
// parts it composes live in `querySheetParts.tsx` (I-24 split).
import { For, Show, createEffect, createMemo, createSignal, createUniqueId, onCleanup, onMount, type Accessor, type JSX } from "solid-js";
import {
  ADVANCED_PHRASE,
  addChild,
  filterValueLabel,
  propertyFilter,
  builderLeafKind,
  encodePropertyLeaf,
  filterLabel,
  filterPhrase,
  groupSelected,
  journalFilter,
  moveAcross,
  moveSibling,
  planningFilter,
  propertyLeafTest,
  propertyOperatorArity,
  propertyOperatorLabel,
  propertyOperators,
  removeAt,
  replaceAt,
  setOp,
  toggleDisabledAt,
  unwrapAt,
  wrapAt,
  type BuilderLeafKind,
  type GroupChoice,
  type PropertyLeafTest,
  type PropertyOperatorId,
} from "../editor/queryBuilder";
import type { Anchor, Filter } from "../editor/queryIr";
import { beginQuerySheetReorder, cancelQuerySheetReorder, parseLocKey, type QuerySheetDropTarget } from "./querySheetReorder";
import {
  type RegistryAccess,
  locKey,
  ADD_MENU_KEY,
  registerVisiblePopover,
  rawLeaves,
  countConditions,
  diagnosticFor,
  QuerySentence,
  FIELD_LABELS,
  KIND_PHRASE,
  type SheetNode,
  advancedCount,
  buildNodes,
  type SiblingPos,
  posOf,
  type SheetSelection,
  type SheetControls,
  dropClasses,
  DragHandle,
  SelectBox,
  EnabledSwitch,
  offLabel,
  moveOptions,
  pickMoveOption,
  Popover,
  ValueEditor,
  type AnchorPrompt,
  type QuerySheetProps,
  AnchorLine,
  AnchorPromptPanel,
  SelectionBar,
  effectiveFor,
  PropertyValueCell,
  AddCondition,
  QueryFieldPicker,
  VocabularyChoice,
  stop,
  Listbox,
  ListboxOption,
} from "./querySheetParts";

export {
  QuerySentence,
  rawLeaves,
  countConditions,
  registerVisiblePopover,
  stop,
  Listbox,
  type AnchorPrompt,
  type RegistryAccess,
  type QuerySheetProps,
  type ListboxOption,
};


export function QuerySheet(props: QuerySheetProps): JSX.Element {
  const nodes = createMemo(() => buildNodes(props.root(), [], 0));
  const isEmpty = createMemo(() => {
    const node = nodes();
    return node.kind === "group" && node.children.length === 0;
  });

  // -- selection, disabling and reordering (§7.4 remainder, P6) --------------

  const [selection, setSelection] = createSignal<SheetSelection | null>(null);
  const [dropTarget, setDropTarget] = createSignal<QuerySheetDropTarget | null>(null);
  let sheetEl: HTMLDivElement | undefined;
  let reorderFocusGeneration = 0;
  let reorderFocusIntent: {
    generation: number;
    anchor: Anchor;
    sourceRoot: Filter;
    expectedRoot: string;
    target: number[];
    origin: HTMLElement | null;
    saved: boolean;
    scheduled: boolean;
  } | null = null;

  // A successful save is re-parsed into fresh objects.
  const reorderRootRevision = (filter: Filter): string =>
    JSON.stringify(filter, (key, value) => key === "span" ? undefined : value);
  const cancelReorderFocus = () => {
    reorderFocusGeneration += 1;
    reorderFocusIntent = null;
  };
  const handleFocusChange = (event: FocusEvent) => {
    const intent = reorderFocusIntent;
    if (!intent || event.target === intent.origin) return;
    cancelReorderFocus();
  };
  const restoreReorderFocus = (
    intent: NonNullable<typeof reorderFocusIntent>,
    immediate = false,
  ) => {
    if (reorderFocusIntent !== intent || reorderFocusGeneration !== intent.generation) return;
    const root = props.root();
    if (props.anchor() !== intent.anchor) {
      cancelReorderFocus();
      return;
    }
    if (root === intent.sourceRoot) return;
    if (reorderRootRevision(root) !== intent.expectedRoot) {
      cancelReorderFocus();
      return;
    }
    // A matching tree can arrive from outside while this save is pending.
    if (!intent.saved || intent.scheduled) return;
    const focus = () => {
      if (reorderFocusIntent !== intent || reorderFocusGeneration !== intent.generation) return;
      const renderedRoot = props.root();
      if (
        props.anchor() !== intent.anchor
        || renderedRoot === intent.sourceRoot
        || reorderRootRevision(renderedRoot) !== intent.expectedRoot
      ) {
        cancelReorderFocus();
        return;
      }
      const active = document.activeElement;
      if (active !== intent.origin && active !== document.body) {
        cancelReorderFocus();
        return;
      }
      const target = sheetEl?.querySelector<HTMLElement>(
        `[data-qs-handle="${locKey(intent.target)}"]`,
      ) ?? null;
      reorderFocusIntent = null;
      target?.focus();
    };
    intent.scheduled = true;
    if (immediate) focus();
    else queueMicrotask(focus);
  };
  onMount(() => document.addEventListener("focusin", handleFocusChange, true));

  /** **A loc is a path into a ROOT REVISION, not a node's identity.**
  *
  *  The moment the tree on screen is replaced — by this sheet's own edit, by a
  *  save in the text pane, by an anchor switch, by the block being re-read
  *  after an external file change or a graph transition — every index a
  *  selection or a drag is holding addresses a position that may now mean
  *  something else. There is no remapping that could be right, so both are
  *  dropped and an in-flight drag is cancelled without applying. This is also
  *  what clears the selection after a successful group: the edit lands, the
  *  root changes, the boxes empty. */
  createEffect(() => {
    props.root();
    props.anchor();
    setSelection(null);
    cancelQuerySheetReorder();
    const intent = reorderFocusIntent;
    if (!intent) return;
    // Effects observe the new root before Solid has necessarily reconciled its keyed rows.
    restoreReorderFocus(intent);
  });
  onCleanup(() => {
    document.removeEventListener("focusin", handleFocusChange, true);
    cancelReorderFocus();
    cancelQuerySheetReorder();
  });

  const controls: SheetControls = {
    selection,
    isSelected: (loc) => {
      const current = selection();
      return (
        !!current
        && locKey(current.parentLoc) === locKey(loc.slice(0, -1))
        && current.indices.includes(loc[loc.length - 1])
      );
    },
    toggleSelected: (loc) => {
      const parentLoc = loc.slice(0, -1);
      const index = loc[loc.length - 1];
      setSelection((current) => {
        // A sibling of a DIFFERENT list starts a new selection: conditions are never implicitly carried out of the …
        if (!current || locKey(current.parentLoc) !== locKey(parentLoc)) {
          return { parentLoc, indices: [index] };
        }
        const indices = current.indices.includes(index)
          ? current.indices.filter((other) => other !== index)
          : [...current.indices, index].sort((a, b) => a - b);
        return indices.length ? { parentLoc, indices } : null;
      });
    },
    clearSelection: () => setSelection(null),
    toggleEnabled: (loc) => props.apply(toggleDisabledAt(props.root(), loc)),
    move: (pos, to) => {
      const root = props.root();
      const next = moveSibling(root, [...pos.parentLoc, pos.index], to);
      // `moveSibling` hands the tree back unchanged for a boundary or a stale path, and an unchanged tree is not …
      if (next === root) return;
      const source = [...pos.parentLoc, pos.index];
      const origin = sheetEl?.querySelector<HTMLElement>(
        `[data-qs-handle="${locKey(source)}"]`,
      ) ?? null;
      const intent = {
        generation: ++reorderFocusGeneration,
        anchor: props.anchor(),
        sourceRoot: root,
        expectedRoot: reorderRootRevision(next),
        target: [...pos.parentLoc, to],
        origin: document.activeElement === origin ? origin : null,
        saved: false,
        scheduled: false,
      };
      reorderFocusIntent = intent;
      let outcome: void | Promise<boolean>;
      try {
        outcome = props.apply(next);
      } catch (error) {
        cancelReorderFocus();
        throw error;
      }
      if (outcome instanceof Promise) {
        void outcome.then((saved) => {
          if (reorderFocusIntent !== intent) return;
          if (!saved) cancelReorderFocus();
          else {
            intent.saved = true;
            restoreReorderFocus(intent);
          }
        }, () => {
          if (reorderFocusIntent === intent) cancelReorderFocus();
        });
      } else {
        // Controlled builders publish synchronously.
        intent.saved = true;
        restoreReorderFocus(intent, true);
      }
    },
    startDrag: (event, pos) => {
      const root = props.root();
      beginQuerySheetReorder(event, {
        parent: locKey(pos.parentLoc),
        from: pos.index,
        isCurrent: () => props.root() === root,
        setTarget: setDropTarget,
        commit: (to) => controls.move(pos, to),
        commitAcross: (parent, slot) => {
          const current = props.root();
          const next = moveAcross(current, [...pos.parentLoc, pos.index], parseLocKey(parent), slot);
          if (next === current) return;
          // A move that pushes a subtree past the drawing depth would fold it into an "advanced" chip the user
          // never asked for, so it is refused like any other move that cannot be shown honestly.
          if (advancedCount(buildNodes(next, [], 0)) > advancedCount(buildNodes(current, [], 0))) return;
          props.apply(next);
        },
      });
    },
    dropTarget,
  };

  const groupSelection = (choice: GroupChoice) => {
    const current = selection();
    props.setOpenMenu(null);
    if (!current || current.indices.length < 2) return;
    const root = props.root();
    const next = groupSelected(
      root,
      current.indices.map((index) => [...current.parentLoc, index]),
      choice,
    );
    if (next === root) return;
    setSelection(null);
    props.apply(next);
  };
  // The add-condition chooser is one of the sheet's menus, not a signal of its own: sharing `openMenu` is what …
  const adding = () => props.openMenu() === ADD_MENU_KEY;
  const setAdding = (open: boolean) => props.setOpenMenu(open ? ADD_MENU_KEY : null);

  const addAtRoot = (filter: Filter) => {
    props.apply(addChild(props.root(), [], filter));
    setAdding(false);
  };

  return (
    <div
      ref={(element) => {
        sheetEl = element;
        props.sheetRef?.(element);
      }}
      class="qs-sheet"
      classList={{ "qs-sheet-stale": props.stale }}
      role="group"
      aria-label="Query filter"
      onClick={stop}
    >
      <AnchorLine
        anchor={props.anchor}
        onAnchor={props.onAnchor}
        both={props.both}
        empty={isEmpty()}
        openMenu={props.openMenu}
        setOpenMenu={props.setOpenMenu}
        layerId={props.layerId}
      />
      <Show when={props.anchorPrompt()}>{(prompt) => <AnchorPromptPanel prompt={prompt()} />}</Show>
      <NodeList
        node={nodes()}
        isRoot
        sheet={props}
        controls={controls}
        adding={adding}
        setAdding={setAdding}
        onAdd={addAtRoot}
      />
      <Show when={selection()}>
        {(current) => (
          <SelectionBar
            selection={current()}
            sheet={props}
            controls={controls}
            onGroup={groupSelection}
          />
        )}
      </Show>
      <Show when={isEmpty() && props.suggestions().length > 0}>
        <div class="qs-try">
          <span class="qs-try-label">Try:</span>
          <For each={props.suggestions()}>
            {(key) => (
              <button
                type="button"
                class="qs-try-item"
                onClick={() => addAtRoot(propertyFilter(key, null))}
              >
                {key}
              </button>
            )}
          </For>
        </div>
      </Show>
      {/* Read ONCE. */}
      <Show when={props.footer}>{(footer) => <div class="qs-footer">{footer()}</div>}</Show>
    </div>
  );
}

function NodeList(props: {
  node: SheetNode;
  isRoot?: boolean;
  sheet: QuerySheetProps;
  controls: SheetControls;
  /** Where this group sits among its own siblings; absent at the root, which
  *  is the sheet's implicit list and has no controls of its own. */
  pos?: SiblingPos;
  adding?: Accessor<boolean>;
  setAdding?: (open: boolean) => void;
  onAdd?: (filter: Filter) => void;
}): JSX.Element {
  return (
    <Show
      when={props.node.kind === "group"}
      fallback={<SheetItem node={props.node} sheet={props.sheet} controls={props.controls} pos={props.pos!} />}
    >
      {(() => {
        const group = props.node as Extract<SheetNode, { kind: "group" }>;
        const count = () => group.children.length;
        const body = (
          <>
            <div
              class="qs-rows"
              role="list"
              aria-label={props.isRoot ? "Query conditions" : `Conditions, ${group.header}`}
            >
              <For each={group.children}>
                {(child, index) => {
                  const pos = (): SiblingPos => posOf(child.loc, count());
                  return (
                    <Show
                      when={child.kind === "group"}
                      fallback={
                        <SheetItem node={child} sheet={props.sheet} controls={props.controls} pos={pos()} />
                      }
                    >
                      {/* This group's item is the drop target for its own list;
                          nested rows cannot become targets in a grandparent list. */}
                      <div
                        class="qs-listitem"
                        role="listitem"
                        data-qs-parent={locKey(group.opLoc)}
                        data-row-index={index()}
                        classList={dropClasses(props.controls, pos())}
                      >
                        <NodeList node={child} sheet={props.sheet} controls={props.controls} pos={pos()} />
                      </div>
                    </Show>
                  );
                }}
              </For>
            </div>
            <Show when={props.isRoot}>
              <AddCondition
                sheet={props.sheet}
                open={props.adding!}
                setOpen={props.setAdding!}
                onAdd={props.onAdd!}
              />
            </Show>
          </>
        );
        return props.isRoot ? (
          body
        ) : (
          <div class="qs-group" classList={{ "qs-off": group.disabled || group.inherited }}>
            <GroupHeader group={group} sheet={props.sheet} controls={props.controls} pos={props.pos!} />
            {body}
          </div>
        );
      })()}
    </Show>
  );
}

function GroupHeader(props: {
  group: Extract<SheetNode, { kind: "group" }>;
  sheet: QuerySheetProps;
  controls: SheetControls;
  pos: SiblingPos;
}): JSX.Element {
  let menuTrigger: HTMLButtonElement | undefined;
  const menuKey = () => `group-menu:${locKey(props.group.loc)}`;
  const open = () => props.sheet.openMenu() === menuKey();
  const menuId = `qs-group-menu-${createUniqueId()}`;
  const root = () => props.sheet.root();
  const cycle = () => {
    // Click cycles all ↔ any.
    const next = props.group.header === "all of" || props.group.header === "not all of" ? "or" : "and";
    props.sheet.apply(setOp(root(), props.group.opLoc, next));
  };
  return (
    <div class="qs-group-header">
      <DragHandle pos={props.pos} label="Move group" controls={props.controls} />
      <SelectBox pos={props.pos} label="Select group" controls={props.controls} />
      <button
        type="button"
        class="qs-group-op"
        title="Switch between all of / any of"
        onClick={(e) => {
          stop(e);
          cycle();
        }}
      >
        {props.group.header}
      </button>
      <span class="qs-menu-wrap">
        <button
          ref={menuTrigger}
          type="button"
          class="qs-row-menu"
          aria-label="Group actions"
          aria-haspopup="listbox"
          aria-expanded={open() ? "true" : "false"}
          aria-controls={menuId}
          onClick={(e) => {
            stop(e);
            props.sheet.setOpenMenu(open() ? null : menuKey());
          }}
        >
          ⋮
        </button>
        <Popover
          open={open}
          close={() => props.sheet.setOpenMenu(null)}
          parentId={props.sheet.layerId}
          trigger={() => menuTrigger ?? null}
        >
          {(rootRef) => (
            <Listbox
              id={menuId}
              label="Group actions"
              rootRef={rootRef}
              options={[
                ...moveOptions(props.pos),
                { key: "none", label: props.group.negated ? "Remove none of" : "None of" },
                // A group inside a `not`/`off` has no list to splice its rows into, and inventing one is a De Morgan rewrite …
                ...(unwrapAt(root(), props.group.opLoc) !== root()
                  ? [{ key: "ungroup", label: "Ungroup" }]
                  : []),
                { key: "remove", label: "Remove" },
              ]}
              onPick={(key) => {
                props.sheet.setOpenMenu(null);
                if (pickMoveOption(key, props.pos, props.group, props.sheet, props.controls)) return;
                if (key === "none") {
                  props.sheet.apply(
                    props.group.negated && props.group.negLoc
                      ? unwrapAt(root(), props.group.negLoc)
                      : wrapAt(root(), props.group.opLoc, "not"),
                  );
                } else if (key === "ungroup") {
                  props.sheet.apply(unwrapAt(root(), props.group.opLoc));
                } else {
                  props.sheet.apply(removeAt(root(), props.group.loc));
                }
              }}
            />
          )}
        </Popover>
      </span>
      {/* A group is disabled like a row: one `Off` around the node
          named by the header, using the same control and helper. */}
      <EnabledSwitch
        loc={props.group.loc}
        disabled={props.group.disabled}
        inherited={props.group.inherited}
        label="Group enabled"
        controls={props.controls}
      />
      <Show when={offLabel(props.group)}>
        {(label) => <span class="qs-off-label">{label()}</span>}
      </Show>
    </div>
  );
}

function SheetItem(props: {
  node: SheetNode;
  sheet: QuerySheetProps;
  controls: SheetControls;
  pos: SiblingPos;
}): JSX.Element {
  return (
    <Show
      when={props.node.kind === "advanced"}
      fallback={
        <QueryRow
          node={props.node as Extract<SheetNode, { kind: "row" }>}
          sheet={props.sheet}
          controls={props.controls}
          pos={props.pos}
        />
      }
    >
      <AdvancedChip
        node={props.node as Extract<SheetNode, { kind: "advanced" }>}
        sheet={props.sheet}
        controls={props.controls}
        pos={props.pos}
      />
    </Show>
  );
}

/** A subtree past the rendering cap: ONE control, its phrase, and a ×. */
function AdvancedChip(props: {
  node: Extract<SheetNode, { kind: "advanced" }>;
  sheet: QuerySheetProps;
  controls: SheetControls;
  pos: SiblingPos;
}): JSX.Element {
  const phrase = () => `${ADVANCED_PHRASE} ${filterLabel(props.node.filter)}`;
  return (
    <div
      class="qs-row qs-row-advanced"
      role="listitem"
      data-qs-parent={locKey(props.pos.parentLoc)}
      data-row-index={props.pos.index}
      classList={{
        "qs-off": props.node.disabled || props.node.inherited,
        ...dropClasses(props.controls, props.pos),
      }}
    >
      {/* A folded subtree is a sibling like any other: it selects, it moves and it disables. */}
      <DragHandle pos={props.pos} label="Move condition" controls={props.controls} />
      <SelectBox pos={props.pos} label="Select condition" controls={props.controls} />
      <Show
        when={props.sheet.onEditText}
        fallback={<span class="qs-advanced" title="edit in the query text below">{phrase()}</span>}
      >
        <button
          type="button"
          class="qs-advanced qs-advanced-open"
          title="Edit this part in the query text below"
          aria-label={`Edit in the query text: ${filterLabel(props.node.filter)}`}
          onClick={(e) => {
            stop(e);
            props.sheet.onEditText?.();
          }}
        >
          {phrase()}
        </button>
      </Show>
      <EnabledSwitch
        loc={props.node.loc}
        disabled={props.node.disabled}
        inherited={props.node.inherited}
        label="Condition enabled"
        controls={props.controls}
      />
      <Show when={offLabel(props.node)}>
        {(label) => <span class="qs-off-label">{label()}</span>}
      </Show>
      <AdvancedMenu node={props.node} sheet={props.sheet} controls={props.controls} pos={props.pos} />
      <button
        type="button"
        class="qs-row-remove"
        aria-label="Remove condition"
        title="Remove"
        onClick={(e) => {
          stop(e);
          props.sheet.apply(removeAt(props.sheet.root(), props.node.loc));
        }}
      >
        ×
      </button>
    </div>
  );
}

/** The folded subtree's ⋮: the move and group entries every sibling has, and
*  nothing that would edit a payload this row cannot read. */
function AdvancedMenu(props: {
  node: Extract<SheetNode, { kind: "advanced" }>;
  sheet: QuerySheetProps;
  controls: SheetControls;
  pos: SiblingPos;
}): JSX.Element {
  let trigger: HTMLButtonElement | undefined;
  const menuKey = () => `advanced-menu:${locKey(props.node.loc)}`;
  const open = () => props.sheet.openMenu() === menuKey();
  const menuId = `qs-advanced-menu-${createUniqueId()}`;
  return (
    <Show when={moveOptions(props.pos).length > 0}>
      <span class="qs-menu-wrap">
        <button
          ref={trigger}
          type="button"
          class="qs-row-menu"
          aria-label="Row actions"
          aria-haspopup="listbox"
          aria-expanded={open() ? "true" : "false"}
          aria-controls={menuId}
          onClick={(e) => {
            stop(e);
            props.sheet.setOpenMenu(open() ? null : menuKey());
          }}
        >
          ⋮
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
              label="Row actions"
              rootRef={rootRef}
              options={moveOptions(props.pos)}
              onPick={(key) => {
                props.sheet.setOpenMenu(null);
                pickMoveOption(key, props.pos, props.node, props.sheet, props.controls);
              }}
            />
          )}
        </Popover>
      </span>
    </Show>
  );
}

function QueryRow(props: {
  node: Extract<SheetNode, { kind: "row" }>;
  sheet: QuerySheetProps;
  controls: SheetControls;
  pos: SiblingPos;
}): JSX.Element {
  const core = () => props.node.core;
  const root = () => props.sheet.root();
  const raw = () => (core().kind === "raw" ? (core() as Filter & { kind: "raw" }) : null);
  const kind = () => builderLeafKind(core());
  const property = createMemo<PropertyLeafTest | null>(() => {
    const test = propertyLeafTest(props.node.filter);
    if (!test) return null;
    // Re-read with the key's effective type so `references` and `is` — the one pair of identities with the same …
    return propertyLeafTest(props.node.filter, effectiveFor(props.sheet.registry, test.key));
  });
  const effective = createMemo(() => effectiveFor(props.sheet.registry, property()?.key));
  const fieldLabel = () => {
    const test = property();
    if (test) return test.throughPage ? "Page property" : "Property";
    const k = kind();
    return k ? FIELD_LABELS[k] : "Condition";
  };
  const operatorLabel = () => {
    const test = property();
    if (test) return propertyOperatorLabel(test.id, effective().cardinality);
    const k = kind();
    if (!k) return props.node.negated ? "not" : "matches";
    const phrase = KIND_PHRASE[k];
    return props.node.negated ? phrase.negative : phrase.positive;
  };
  const diagnostic = () => {
    const leaf = raw();
    return leaf ? diagnosticFor(props.sheet.query(), leaf) : undefined;
  };
  /** **A property row's edits wait for a HEALTHY registry.** Its operator menu
  *  and its value encoding are both `effectiveTypeOf` answers, and the fallback
  *  when there is no row is the untyped `text` family — so editing while the
  *  read is in flight, or after it failed, would silently retype a `number`
  *  key, or a key whose declaration was written a moment ago, as text. Nothing
  *  here is a NEW state: the row keeps its own draft and reads exactly as it
  *  did (§6.3, I-20). */
  const propertyPending = () => !!property() && props.sheet.registry.unavailable();
  /** Why the row is waiting, in the control's own tooltip: an indexing read
  *  ends by itself, a failure does not, and the two must not read alike. */
  const propertyWaitReason = () =>
    props.sheet.registry.failure()?.message ?? "Reading this graph's properties…";

  const key = (purpose: string) => `${purpose}:${locKey(props.node.loc)}`;
  const menuOpen = (purpose: string) => props.sheet.openMenu() === key(purpose);
  const toggle = (purpose: string) =>
    props.sheet.setOpenMenu(menuOpen(purpose) ? null : key(purpose));

  let fieldTrigger: HTMLButtonElement | undefined;
  let opTrigger: HTMLButtonElement | undefined;
  let valueTrigger: HTMLButtonElement | undefined;
  let rowMenuTrigger: HTMLButtonElement | undefined;
  const fieldMenuId = `qs-field-${createUniqueId()}`;
  const opMenuId = `qs-op-${createUniqueId()}`;
  const rowMenuId = `qs-rowmenu-${createUniqueId()}`;

  /** Replace this row's whole node — the wrappers included, because the negative
  *  operator IS a `not` wrapper (§7.4). */
  const replaceRow = (filter: Filter) =>
    props.sheet.apply(replaceAt(root(), props.node.loc, filter));

  const setPropertyOperator = (id: PropertyOperatorId) => {
    const test = property();
    if (!test || propertyPending()) return;
    const arity = propertyOperatorArity(id);
    const next = encodePropertyLeaf({
      id,
      key: test.key,
      values: test.values.slice(0, arity),
      type: effective().type,
      throughPage: test.throughPage,
    });
    // An identity that needs a value the row does not have yet keeps the row on screen with its editor open …
    if (next) replaceRow(next);
    props.sheet.setOpenMenu(null);
  };

  const setPropertyValues = (values: string[]) => {
    const test = property();
    if (!test || propertyPending()) return;
    const next = encodePropertyLeaf({
      id: test.id,
      key: test.key,
      values,
      type: effective().type,
      throughPage: test.throughPage,
    });
    if (next) replaceRow(next);
  };

  /** What this row currently tests, in the vocabulary picker's terms, so the
  *  list can mark the row the user is already on. */
  const currentChoice = (): VocabularyChoice | null => {
    const test = property();
    if (test) return { kind: "property", key: test.key, throughPage: test.throughPage };
    const k = kind();
    return k ? { kind: "builtin", leaf: k } : null;
  };

  /** Point this row at a different property key. */
  const setPropertyKey = (choice: VocabularyChoice & { kind: "property" }) => {
    if (props.sheet.registry.unavailable()) return;
    const target = effectiveFor(props.sheet.registry, choice.key);
    const offered = propertyOperators(target).map((operator) => operator.id);
    const test = property();
    const values = test?.values ?? [];
    const id = test && offered.includes(test.id) ? test.id : offered[0] ?? "is_set";
    const encode = (candidate: PropertyOperatorId) =>
      encodePropertyLeaf({
        id: candidate,
        key: choice.key,
        values,
        type: target.type,
        throughPage: choice.throughPage,
      });
    const next = encode(id) ?? encode("is_set");
    if (next) replaceRow(next);
  };

  const toggleNegated = () => {
    // The positive leaf is what a negative operator wraps, so flipping is wrapping or unwrapping exactly one `not`.
    props.sheet.apply(
      props.node.negated && props.node.negLoc
        ? unwrapAt(root(), props.node.negLoc)
        : wrapAt(root(), props.node.loc, "not"),
    );
    props.sheet.setOpenMenu(null);
  };

  return (
    <div
      class="qs-row"
      role="listitem"
      data-qs-parent={locKey(props.pos.parentLoc)}
      data-row-index={props.pos.index}
      classList={{
        "qs-row-raw": !!raw(),
        "qs-off": props.node.disabled || props.node.inherited,
        ...dropClasses(props.controls, props.pos),
      }}
    >
      {/* A disabled row is still a row: it can be selected, moved and dragged. */}
      <DragHandle pos={props.pos} label="Move condition" controls={props.controls} />
      <SelectBox pos={props.pos} label="Select condition" controls={props.controls} />
      <Show
        when={!raw()}
        fallback={
          <>
            {/* A retained leaf: the decoded text, the diagnostic, and a ×. */}
            <span class="qs-raw-text">{raw()!.text}</span>
            <span class="qs-raw-message" role={props.node.disabled ? undefined : "alert"}>
              {diagnostic()?.message ?? "This condition was not understood."}
            </span>
            <Show when={diagnostic()?.suggestions?.length}>
              {/* Rust's OWN alternatives (§4.3.2). */}
              <span class="qs-raw-suggestions">
                Did you mean{" "}
                <For each={diagnostic()!.suggestions!.slice(0, 4)}>
                  {(suggestion, index) => (
                    <>
                      <Show when={index() > 0}>, </Show>
                      <code>{suggestion}</code>
                    </>
                  )}
                </For>
                ?
              </span>
            </Show>
            {/* A retained leaf cannot be edited in place, so it offers
                the text editor as the way out. */}
            <Show when={props.sheet.onEditText}>
              <button
                type="button"
                class="qs-raw-edit"
                onClick={(e) => {
                  stop(e);
                  props.sheet.onEditText?.();
                }}
              >
                Edit as text
              </button>
            </Show>
          </>
        }
      >
        <span class="qs-cell qs-cell-field">
          <button
            ref={fieldTrigger}
            type="button"
            class="qs-field"
            aria-haspopup="listbox"
            aria-expanded={menuOpen("field") ? "true" : "false"}
            aria-controls={fieldMenuId}
            disabled={props.node.disabled}
            onClick={(e) => {
              stop(e);
              toggle("field");
            }}
          >
            {fieldLabel()} ▾
          </button>
          <Popover
            open={() => menuOpen("field")}
            close={() => props.sheet.setOpenMenu(null)}
            parentId={props.sheet.layerId}
            trigger={() => fieldTrigger ?? null}
          >
            {(rootRef) => (
              /* The SAME picker the add-condition flow uses (§7.5). */
              <QueryFieldPicker
                id={fieldMenuId}
                anchor={props.sheet.anchor()}
                rows={props.sheet.registry.rows}
                pending={props.sheet.registry.pending}
                failure={props.sheet.registry.failure}
                onRetry={props.sheet.registry.retry}
                current={currentChoice()}
                rootRef={rootRef}
                onPick={(choice) => {
                  // A property pick needs the key's effective type to encode a leaf.
                  if (choice.kind === "property" && props.sheet.registry.unavailable()) return;
                  props.sheet.setOpenMenu(null);
                  if (choice.kind === "property") return setPropertyKey(choice);
                  // The filter sheet offers only the filter vocabulary; a display field cannot reach this picker.
                  if (choice.kind === "field") return;
                  const next = choice.leaf;
                  if (next === "scheduled" || next === "deadline") return replaceRow(planningFilter(next));
                  if (next === "journal") return replaceRow(journalFilter());
                  // Anything that needs a value re-opens the value editor with the new field selected; the row is not …
                  props.sheet.setOpenMenu(`value:${locKey(props.node.loc)}:${next}`);
                }}
              />
            )}
          </Popover>
        </span>
        <span class="qs-cell qs-cell-op">
          <button
            ref={opTrigger}
            type="button"
            class="qs-op"
            aria-haspopup="listbox"
            aria-expanded={menuOpen("op") ? "true" : "false"}
            aria-controls={opMenuId}
            disabled={props.node.disabled || propertyPending()}
            title={propertyPending() ? propertyWaitReason() : undefined}
            onClick={(e) => {
              stop(e);
              toggle("op");
            }}
          >
            {operatorLabel()} ▾
          </button>
          <Popover
            open={() => menuOpen("op")}
            close={() => props.sheet.setOpenMenu(null)}
            parentId={props.sheet.layerId}
            trigger={() => opTrigger ?? null}
          >
            {(rootRef) => (
              <Show
                when={property()}
                fallback={
                  <Listbox
                    id={opMenuId}
                    label="Condition operator"
                    rootRef={rootRef}
                    options={(() => {
                      const k = kind();
                      const phrase = k ? KIND_PHRASE[k] : { positive: "matches", negative: "does not match" };
                      return [
                        { key: "positive", label: phrase.positive, active: !props.node.negated },
                        { key: "negative", label: phrase.negative, active: props.node.negated },
                      ];
                    })()}
                    onPick={(picked) => {
                      if ((picked === "negative") !== props.node.negated) toggleNegated();
                      else props.sheet.setOpenMenu(null);
                    }}
                  />
                }
              >
                {(test) => (
                  <Listbox
                    id={opMenuId}
                    label="Condition operator"
                    rootRef={rootRef}
                    options={(() => {
                      const offered = propertyOperators(effective());
                      const listed = offered.map((operator) => ({
                        key: operator.id,
                        label: operator.label,
                        active: operator.id === test().id,
                      }));
                      // A reopen-only identity (P2's atom-level `!=`) is shown as the current selection so the row says what it …
                      return listed.some((option) => option.active)
                        ? listed
                        : [
                            {
                              key: test().id,
                              label: propertyOperatorLabel(test().id, effective().cardinality),
                              active: true,
                            },
                            ...listed,
                          ];
                    })()}
                    onPick={(picked) => setPropertyOperator(picked as PropertyOperatorId)}
                  />
                )}
              </Show>
            )}
          </Popover>
        </span>
        <span class="qs-cell qs-cell-value">
          <Show
            when={property()}
            fallback={
              <>
                <button
                  ref={valueTrigger}
                  type="button"
                  class="qs-value"
                  disabled={props.node.disabled}
                  aria-label="Condition value"
                  onClick={(e) => {
                    stop(e);
                    const k = kind();
                    props.sheet.setOpenMenu(
                      menuOpen(`value`) || !k ? null : `value:${locKey(props.node.loc)}:${k}`,
                    );
                  }}
                >
                  {filterValueLabel(core())}
                </button>
                <Popover
                  open={() => (props.sheet.openMenu() ?? "").startsWith(`value:${locKey(props.node.loc)}:`)}
                  close={() => props.sheet.setOpenMenu(null)}
                  parentId={props.sheet.layerId}
                  trigger={() => valueTrigger ?? null}
                >
                  {(rootRef) => (
                    <div ref={rootRef} class="qs-menu" onClick={stop}>
                      <ValueEditor
                        kind={(props.sheet.openMenu() ?? "").split(":").pop() as BuilderLeafKind}
                        onCommit={(filter) => {
                          props.sheet.setOpenMenu(null);
                          replaceRow(props.node.negated ? { kind: "not", inner: filter } : filter);
                        }}
                      />
                    </div>
                  )}
                </Popover>
              </>
            }
          >
            {(test) => (
              <PropertyValueCell
                test={test()}
                effective={effective()}
                disabled={props.node.disabled || propertyPending()}
                registry={props.sheet.registry}
                onCommit={setPropertyValues}
              />
            )}
          </Show>
        </span>
      </Show>
      <EnabledSwitch
        loc={props.node.loc}
        disabled={props.node.disabled}
        inherited={props.node.inherited}
        label="Condition enabled"
        controls={props.controls}
      />
      <span class="qs-menu-wrap">
        <button
          ref={rowMenuTrigger}
          type="button"
          class="qs-row-menu"
          aria-label="Row actions"
          aria-haspopup="listbox"
          aria-expanded={menuOpen("row") ? "true" : "false"}
          aria-controls={rowMenuId}
          onClick={(e) => {
            stop(e);
            toggle("row");
          }}
        >
          ⋮
        </button>
        <Popover
          open={() => menuOpen("row")}
          close={() => props.sheet.setOpenMenu(null)}
          parentId={props.sheet.layerId}
          trigger={() => rowMenuTrigger ?? null}
        >
          {(rootRef) => (
            <Listbox
              id={rowMenuId}
              label="Row actions"
              rootRef={rootRef}
              options={[...moveOptions(props.pos), { key: "remove", label: "Remove" }]}
              onPick={(picked) => {
                props.sheet.setOpenMenu(null);
                if (pickMoveOption(picked, props.pos, props.node, props.sheet, props.controls)) return;
                props.sheet.apply(removeAt(root(), props.node.loc));
              }}
            />
          )}
        </Popover>
      </span>
      <button
        type="button"
        class="qs-row-remove"
        aria-label="Remove condition"
        title="Remove"
        onClick={(e) => {
          stop(e);
          props.sheet.apply(removeAt(root(), props.node.loc));
        }}
      >
        ×
      </button>
      <Show when={offLabel(props.node)}>
        {(label) => <span class="qs-off-label">{label()}</span>}
      </Show>
    </div>
  );
}

/** The bounded phrase for a filter — exported so the host's sentence and the sheet's chip cannot drift apart. */
export { filterPhrase };
