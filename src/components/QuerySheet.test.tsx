// Ported from master src/components/QuerySheet.test.tsx. og changes: no mock query
// fixture seam (the spies already answer print/parse) and no index readiness
// error type (a registry read failure is a plain error on og).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { clearTransientLayersForTest, dismissTopTransient } from "../transientLayers";
import { QueryBuilder, type BuilderSession } from "./QueryBuilder";
import { anyTaskFilter, encodePropertyLeaf, betweenFilter, journalFilter, pageRefFilter, propertyFilter, taskFilter } from "../editor/queryBuilder";
import { diagnosticFor, PropertyValueCell } from "./querySheetParts";
import type { Filter, ParsedQuery, RegistrySnapshot } from "../editor/queryIr";

// **The sheet itself** (SPEC §7.2–§7.4): what the resting sentence expands into.
//
// The three things pinned here are the ones that have no other home. The
// popover ladder and the facet sharing live in `QueryBuilder.transient.test.tsx`
// with the rest of GH #472; the phrase lives in `queryBuilder.test.ts`.

function session(filter: Filter, anchor: "block" | "page" = "block"): BuilderSession {
  return { query: { anchor, filter, source: { kind: "builder" } }, view: {} };
}

function mountBuilder(filter: Filter, blockId?: string) {
  const host = document.createElement("div");
  document.body.append(host);
  const [current, setCurrent] = createSignal<BuilderSession>(session(filter));
  // Every SAVE the sheet performs, in order. A gesture that must not write —
  // selecting, hovering a drop target, cancelling a drag — is proven by this
  // list staying empty, which a snapshot of the markup could never show.
  const changes: BuilderSession[] = [];
  const dispose = render(
    () => (
      <QueryBuilder
        session={current}
        onChange={(next) => {
          changes.push(next);
          setCurrent(next);
        }}
        blockId={blockId}
      />
    ),
    host,
  );
  let sheetEl: HTMLElement | null = null;
  const open = (): HTMLElement => {
    if (sheetEl?.isConnected) return sheetEl;
    const before = new Set(document.querySelectorAll<HTMLElement>(".qs-sheet"));
    host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
    sheetEl = [...document.querySelectorAll<HTMLElement>(".qs-sheet")].find((el) => !before.has(el)) ?? null;
    if (!sheetEl) throw new Error("the sheet did not open");
    return sheetEl;
  };
  return { host, open, session: current, changes, setSession: setCurrent, dispose };
}

const settle = async () => {
  for (let i = 0; i < 8; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
};

beforeEach(() => {
  vi.spyOn(backend(), "queryFacets").mockResolvedValue([["cost", ["10"]]]);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("@block and cost > 10");
});

afterEach(() => {
  clearTransientLayersForTest();
  resetSharedQueryResultsForTests();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("one block, two sheets", () => {
  // The same query block can be on screen twice — main pane and sidebar, or two
  // split panes. `transientLayers` keys by ID and a LATER registration REPLACES
  // an earlier one, so a layer id derived from the block would silently
  // unregister the first sheet and Escape would close the wrong one.
  it("gives each mount of one block its own layer, so Escape closes the sheet that is open", () => {
    const first = mountBuilder(taskFilter(["TODO"]), "block-42");
    const second = mountBuilder(taskFilter(["TODO"]), "block-42");
    try {
      const secondSheet = second.open();
      expect(document.querySelectorAll(".qs-sheet")).toHaveLength(1);

      expect(dismissTopTransient("escape")).toBe(true);
      expect(secondSheet.isConnected).toBe(false);
      expect(document.querySelectorAll(".qs-sheet")).toHaveLength(0);

      // The first mount was never registered, so its sentence is untouched and
      // still opens.
      const firstSheet = first.open();
      expect(firstSheet.isConnected).toBe(true);
      expect(dismissTopTransient("escape")).toBe(true);
      expect(document.querySelectorAll(".qs-sheet")).toHaveLength(0);
    } finally {
      first.dispose();
      second.dispose();
    }
  });
});

describe("a typed property leaf reopens as the row that wrote it", () => {
  it("shows the operator it was saved with, not a generic equality", async () => {
    vi.spyOn(backend(), "queryRegistry").mockResolvedValue({
      rows: [
        {
          normalized_name: "cost",
          display_name: "cost",
          observed_type: "number",
          declared_type: null,
          cardinality: "one",
          count: 3,
        },
      ],
      generation: 1,
    } as unknown as RegistrySnapshot);
    const leaf = encodePropertyLeaf({ id: "gt", key: "cost", values: ["10"], type: "number" })!;
    expect(leaf).toBeTruthy();
    const { open, dispose } = mountBuilder(leaf);
    try {
      const sheet = open();
      await settle();
      const row = sheet.querySelector(".qs-row")!;
      expect(row.querySelector(".qs-field")!.textContent).toContain("Property");
      expect(row.querySelector(".qs-op")!.textContent).toContain("is more than");
      expect(row.querySelector(".qs-property-key")!.textContent).toBe("cost");
      // The row is a listitem in the conditions listbox, and its menus are
      // listboxes of their own.
      expect(row.getAttribute("role")).toBe("listitem");
      row.querySelector<HTMLButtonElement>(".qs-op")!.click();
      const listbox = sheet.querySelector('[role="listbox"]')!;
      expect(listbox.getAttribute("aria-label")).toBe("Condition operator");
      expect(
        [...listbox.querySelectorAll(".qs-option")].map((option) => option.textContent),
      ).toContain("is at least");
    } finally {
      dispose();
    }
  });
});

it("adds a numeric between condition with both bounds", async () => {
  vi.spyOn(backend(), "queryRegistry").mockResolvedValue({
    rows: [{ normalized_name: "cost", observed_type: "number", cardinality: "one", count_blocks: 3, count_pages: 0, mismatch_count: 0 }],
    generation: 1,
  });
  const builder = mountBuilder({ kind: "and", items: [] });
  try {
    const sheet = builder.open();
    await settle();
    sheet.querySelector<HTMLButtonElement>(".qs-add")!.click();
    const cost = [...document.querySelectorAll<HTMLElement>('[role="option"]')]
      .find((item) => item.textContent?.trim() === "cost");
    expect(cost).toBeDefined();
    cost!.click();
    const between = [...document.querySelectorAll<HTMLElement>('[role="option"]')]
      .find((item) => item.textContent?.includes("between"));
    expect(between).toBeDefined();
    between!.click();
    const from = document.querySelector<HTMLInputElement>('input[aria-label="From"]')!;
    const to = document.querySelector<HTMLInputElement>('input[aria-label="To"]')!;
    expect(from).toBeDefined();
    expect(to).toBeDefined();
    from.value = "3"; from.dispatchEvent(new InputEvent("input", { bubbles: true }));
    to.value = "7"; to.dispatchEvent(new InputEvent("input", { bubbles: true }));
    document.querySelector<HTMLButtonElement>(".qs-commit")!.click();
    expect(builder.changes).toHaveLength(1);
    expect(builder.session().query.filter).toEqual({ kind: "and", items: [encodePropertyLeaf({ id: "between", key: "cost", values: ["3", "7"], type: "number" })] });
  } finally {
    builder.dispose();
  }
});

it("matches two same-kind raw diagnostics by their distinct spans", () => {
  const query = session({ kind: "true" }).query;
  query.diagnostics = [
    { kind: "syntax", span: { start: 1, end: 2 }, message: "first" },
    { kind: "syntax", span: { start: 5, end: 6 }, message: "second" },
  ];
  expect(diagnosticFor(query, { kind: "raw", text: "b", diagnostic_kind: "syntax", span: { start: 5, end: 6 } })?.message).toBe("second");
});

it("does not commit a property value when its untouched input blurs", () => {
  const host = document.createElement("div");
  document.body.append(host);
  const onCommit = vi.fn();
  const dispose = render(() => <PropertyValueCell
    test={{ id: "is", key: "cost", values: ["3"], throughPage: false }}
    effective={{ type: "number", cardinality: "one" }}
    disabled={false}
    registry={{ rows: () => [], pending: () => false, failure: () => null, unavailable: () => false, request: () => {}, retry: () => {} }}
    onCommit={onCommit}
  />, host);
  try {
    host.querySelector<HTMLInputElement>("input")!.dispatchEvent(new FocusEvent("blur"));
    expect(onCommit).not.toHaveBeenCalled();
  } finally {
    dispose();
  }
});

describe("switching what the query selects re-validates it and says how much it costs", () => {
  const notApplicable = (): ParsedQuery => ({
    query: {
      anchor: "page",
      filter: {
        kind: "and",
        items: [
          { kind: "raw", text: "task = 'TODO'", diagnostic_kind: "not_applicable" },
          propertyFilter("owner", "Ada"),
        ],
      },
      diagnostics: [
        { kind: "not_applicable", message: "`task` does not apply to pages", disabled: false },
      ],
      source: { kind: "tql", original: "@page and task = 'TODO' and prop('owner') = 'Ada'" },
    },
    view: {},
  }) as unknown as ParsedQuery;

  it("counts the conditions that stop applying and offers a way out of each", async () => {
    // jsdom has no engine, so the print-then-parse round trip is stubbed.
    vi.spyOn(backend(), "printQuery").mockResolvedValue("@page and task = 'TODO'");
    vi.spyOn(backend(), "parseQuery").mockResolvedValue(notApplicable());

    const filter: Filter = {
      kind: "and",
      items: [taskFilter(["TODO"]), propertyFilter("owner", "Ada")],
    };
    const { open, session: current, dispose } = mountBuilder(filter);
    try {
      const sheet = open();
      sheet.querySelector<HTMLButtonElement>(".qs-anchor-button")!.click();
      [...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")]
        .find((option) => option.textContent?.startsWith("pages"))!
        .click();
      await settle();

      const prompt = sheet.querySelector(".qs-anchor-prompt")!;
      expect(prompt.getAttribute("role")).toBe("alertdialog");
      // The number is the POINT: "1 of your 2 conditions", not "some".
      expect(prompt.textContent).toContain("1 of your 2 conditions");
      expect(prompt.textContent).toContain("task = 'TODO'");
      // Nothing is committed while the prompt is up.
      expect(current().query.anchor).toBe("block");

      prompt.querySelector<HTMLButtonElement>(".qs-commit")!.click();
      await settle();
      expect(current().query.anchor).toBe("page");
      // "Remove them" removed exactly the leaf that stopped applying.
      expect(JSON.stringify(current().query.filter)).not.toContain("not_applicable");
      expect(JSON.stringify(current().query.filter)).toContain("owner");
    } finally {
      dispose();
    }
  });

  it("cancels back to the query it had, with nothing changed", async () => {
    vi.spyOn(backend(), "printQuery").mockResolvedValue("@page and task = 'TODO'");
    vi.spyOn(backend(), "parseQuery").mockResolvedValue(notApplicable());

    const filter: Filter = {
      kind: "and",
      items: [taskFilter(["TODO"]), propertyFilter("owner", "Ada")],
    };
    const { open, session: current, dispose } = mountBuilder(filter);
    const before = JSON.stringify(current());
    try {
      const sheet = open();
      sheet.querySelector<HTMLButtonElement>(".qs-anchor-button")!.click();
      [...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")]
        .find((option) => option.textContent?.startsWith("pages"))!
        .click();
      await settle();

      const actions = [...sheet.querySelectorAll<HTMLButtonElement>(".qs-anchor-prompt-actions button")];
      actions.find((button) => button.textContent === "Cancel")!.click();
      await settle();
      expect(sheet.querySelector(".qs-anchor-prompt")).toBeNull();
      expect(JSON.stringify(current())).toBe(before);
    } finally {
      dispose();
    }
  });
});


describe("anchor preview belongs to the current open sheet", () => {
  it.each(["close", "reselect", "newer"])("drops an old success after %s", async (action) => {
    const replies: ((value: ParsedQuery) => void)[] = [];
    vi.spyOn(backend(), "parseQuery").mockImplementation(() => new Promise((resolve) => replies.push(resolve)));
    const builder = mountBuilder(propertyFilter("owner", "Ada"));
    const pick = (sheet: HTMLElement, label: string) => {
      sheet.querySelector<HTMLButtonElement>(".qs-anchor-button")!.click();
      [...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")]
        .find((option) => option.textContent?.startsWith(label))!.click();
    };
    try {
      const sheet = builder.open();
      pick(sheet, "pages");
      await settle();
      if (action === "close") dismissTopTransient("escape");
      else pick(sheet, action === "reselect" ? "blocks" : "pages");
      await settle();
      replies[0]({ query: session(propertyFilter("owner", "Ada"), "page").query, view: {} } as ParsedQuery);
      await settle();
      expect(builder.session().query.anchor).toBe("block");
    } finally { builder.dispose(); }
  });
});

// ---------------------------------------------------------------------------
// P6: the selection box, the enabled switch, grouping and reordering (§7.4).
//
// The assertions below are about the IR the gesture SAVED, not about the markup
// it drew: a checkbox that is checked proves nothing, and a row that moved in
// the DOM proves nothing either — what matters is which tree reached
// `onChange`, and how many times.
// ---------------------------------------------------------------------------

const A = pageRefFilter("Alpha");
const B = pageRefFilter("Beta");
const C = pageRefFilter("Gamma");

const filterOf = (builder: { session: () => BuilderSession }) => builder.session().query.filter;

/** The items of the sheet's OUTERMOST condition list, in order. */
function rootItems(sheet: HTMLElement): HTMLElement[] {
  return [...sheet.querySelector<HTMLElement>(".qs-rows")!.children] as HTMLElement[];
}

function pointer(type: string, x: number, y: number): PointerEvent {
  const Ctor = (window as { PointerEvent?: typeof PointerEvent }).PointerEvent ?? MouseEvent;
  return new Ctor(type, {
    bubbles: true,
    cancelable: true,
    clientX: x,
    clientY: y,
    button: 0,
    buttons: 1,
  }) as PointerEvent;
}

/** jsdom lays nothing out, so the geometry a drop reads is stated here. Rows are
 *  40px tall and stacked, which is all `rowReorder` needs to decide a side. */
function stackRects(items: HTMLElement[]) {
  items.forEach((item, index) => {
    const top = index * 40;
    Object.defineProperty(item, "getBoundingClientRect", {
      configurable: true,
      value: () => ({
        x: 0, y: top, width: 300, height: 40,
        left: 0, top, right: 300, bottom: top + 40, toJSON: () => ({}),
      }) as DOMRect,
    });
  });
}

/** Drag `from` onto `over`, dropping on its lower half unless `before`. */
function dragOnto(items: HTMLElement[], from: number, over: number, before = false) {
  stackRects(items);
  const handle = items[from]!.querySelector<HTMLElement>(".qs-drag-handle")!;
  handle.dispatchEvent(pointer("pointerdown", 10, from * 40 + 10));
  const y = over * 40 + (before ? 5 : 35);
  const previous = document.elementFromPoint;
  try {
    document.elementFromPoint = () => items[over]!;
    document.dispatchEvent(pointer("pointermove", 10, y));
    return () => {
      document.dispatchEvent(pointer("pointerup", 10, y));
      document.elementFromPoint = previous;
    };
  } catch (error) {
    document.elementFromPoint = previous;
    throw error;
  }
}

const arrow = (element: HTMLElement, key: "ArrowUp" | "ArrowDown") =>
  element.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));

describe("the selection box and the enabled switch are two controls", () => {
  it("selects without saving, and disables without selecting", () => {
    const builder = mountBuilder({ kind: "and", items: [A, B] });
    try {
      const sheet = builder.open();
      const rows = rootItems(sheet);
      expect(rows).toHaveLength(2);

      // Two DISTINCT controls on the row, each named.
      const select = rows[0]!.querySelector<HTMLInputElement>(".qs-select")!;
      const enabled = rows[0]!.querySelector<HTMLButtonElement>(".qs-enabled")!;
      expect(select.getAttribute("aria-label")).toBe("Select condition");
      expect(enabled.getAttribute("role")).toBe("switch");
      expect(enabled.getAttribute("aria-label")).toBe("Condition enabled");
      expect(enabled.getAttribute("aria-checked")).toBe("true");

      // Selecting is not an edit: nothing is saved and the query is untouched.
      select.click();
      expect(builder.changes).toHaveLength(0);
      expect(filterOf(builder)).toEqual({ kind: "and", items: [A, B] });
      expect(sheet.querySelector(".qs-selection-count")!.textContent).toBe("1 selected");

      // Disabling IS an edit, and it wraps only this row.
      rootItems(sheet)[0]!.querySelector<HTMLButtonElement>(".qs-enabled")!.click();
      expect(builder.changes).toHaveLength(1);
      expect(filterOf(builder)).toEqual({ kind: "and", items: [{ kind: "off", inner: A }, B] });

      const after = rootItems(sheet)[0]!;
      expect(after.querySelector(".qs-enabled")!.getAttribute("aria-checked")).toBe("false");
      expect(after.querySelector(".qs-off-label")!.textContent).toBe("disabled");
      // The row is still there, still selectable and still movable.
      expect(after.querySelector(".qs-select")).not.toBeNull();
      expect(after.querySelector(".qs-drag-handle")).not.toBeNull();

      // Enabling gives the condition back exactly as it was.
      after.querySelector<HTMLButtonElement>(".qs-enabled")!.click();
      expect(filterOf(builder)).toEqual({ kind: "and", items: [A, B] });
    } finally {
      builder.dispose();
    }
  });

  it("tells a row inside a disabled group that its own switch cannot bring it back", () => {
    const builder = mountBuilder({
      kind: "and",
      items: [{ kind: "off", inner: { kind: "and", items: [A, B] } }, C],
    });
    try {
      const sheet = builder.open();
      const group = sheet.querySelector<HTMLElement>(".qs-group")!;
      const header = group.querySelector<HTMLElement>(".qs-group-header")!;
      expect(header.querySelector(".qs-enabled")!.getAttribute("aria-label")).toBe("Group enabled");
      expect(header.querySelector(".qs-enabled")!.getAttribute("aria-checked")).toBe("false");

      const inner = [...group.querySelector<HTMLElement>(".qs-rows")!.children] as HTMLElement[];
      expect(inner).toHaveLength(2);
      for (const row of inner) {
        // Its own state is honest — it carries no `off` of its own …
        expect(row.querySelector(".qs-enabled")!.getAttribute("aria-checked")).toBe("true");
        // … and so is the reason it is not running.
        expect(row.querySelector(".qs-off-label")!.textContent).toBe("disabled by group");
      }

      // The group's own switch is the one that brings them back.
      header.querySelector<HTMLButtonElement>(".qs-enabled")!.click();
      expect(filterOf(builder)).toEqual({ kind: "and", items: [{ kind: "and", items: [A, B] }, C] });
      const revived = [...sheet.querySelector<HTMLElement>(".qs-group .qs-rows")!.children] as HTMLElement[];
      expect(revived.every((row) => row.querySelector(".qs-off-label") === null)).toBe(true);
    } finally {
      builder.dispose();
    }
  });
});

describe("grouping a selection", () => {
  it("groups two selected rows under the header the user picked, in one save", () => {
    const builder = mountBuilder({ kind: "and", items: [A, B, C] });
    try {
      const sheet = builder.open();
      // Non-contiguous: the first and the last.
      rootItems(sheet)[0]!.querySelector<HTMLInputElement>(".qs-select")!.click();
      rootItems(sheet)[2]!.querySelector<HTMLInputElement>(".qs-select")!.click();
      expect(sheet.querySelector(".qs-selection-count")!.textContent).toBe("2 selected");
      expect(builder.changes).toHaveLength(0);

      sheet.querySelector<HTMLButtonElement>(".qs-group-selected")!.click();
      const options = [...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")];
      options.find((option) => option.textContent === "Any of")!.click();

      // One save, one undoable edit: the group lands where the topmost selected
      // row was, and the unselected row keeps its own order around it.
      expect(builder.changes).toHaveLength(1);
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [{ kind: "or", items: [A, C] }, B],
      });
      // The selection is spent.
      expect(sheet.querySelector(".qs-selection")).toBeNull();
    } finally {
      builder.dispose();
    }
  });

  it("offers grouping only from two, and starts over when a sibling elsewhere is picked", () => {
    const builder = mountBuilder({
      kind: "and",
      items: [A, { kind: "and", items: [B, C] }],
    });
    try {
      const sheet = builder.open();
      rootItems(sheet)[0]!.querySelector<HTMLInputElement>(".qs-select")!.click();
      expect(sheet.querySelector<HTMLButtonElement>(".qs-group-selected")!.disabled).toBe(true);

      // A row in the nested list is not a sibling of the one already picked, so
      // it REPLACES the selection rather than spanning two lists.
      const nested = [...sheet.querySelector<HTMLElement>(".qs-group .qs-rows")!.children] as HTMLElement[];
      nested[0]!.querySelector<HTMLInputElement>(".qs-select")!.click();
      expect(sheet.querySelector(".qs-selection-count")!.textContent).toBe("1 selected");
      expect(rootItems(sheet)[0]!.querySelector<HTMLInputElement>(".qs-select")!.checked).toBe(false);

      nested[1]!.querySelector<HTMLInputElement>(".qs-select")!.click();
      expect(sheet.querySelector<HTMLButtonElement>(".qs-group-selected")!.disabled).toBe(false);
      sheet.querySelector<HTMLButtonElement>(".qs-group-selected")!.click();
      [...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")]
        .find((option) => option.textContent === "None of")!
        .click();

      // The nested rows grouped inside their own list; the root row is untouched.
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [A, { kind: "and", items: [{ kind: "not", inner: { kind: "or", items: [B, C] } }] }],
      });
      expect(builder.changes).toHaveLength(1);
    } finally {
      builder.dispose();
    }
  });

  it("groups a row with the row above it from the row menu, with the same three choices", () => {
    const builder = mountBuilder({ kind: "and", items: [A, B, C] });
    try {
      const sheet = builder.open();
      rootItems(sheet)[1]!.querySelector<HTMLButtonElement>(".qs-row-menu")!.click();
      const labels = [...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")].map((o) => o.textContent);
      expect(labels).toEqual([
        "Move up",
        "Move down",
        "Group with row above — all of",
        "Group with row above — any of",
        "Group with row above — none of",
        "Remove",
      ]);
      [...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")]
        .find((option) => option.textContent === "Group with row above — none of")!
        .click();
      expect(builder.changes).toHaveLength(1);
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [{ kind: "not", inner: { kind: "or", items: [A, B] } }, C],
      });
    } finally {
      builder.dispose();
    }
  });

  it("offers no move or group entry to the only row in a list", () => {
    const builder = mountBuilder({ kind: "and", items: [A] });
    try {
      const sheet = builder.open();
      rootItems(sheet)[0]!.querySelector<HTMLButtonElement>(".qs-row-menu")!.click();
      expect([...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")].map((o) => o.textContent))
        .toEqual(["Remove"]);
    } finally {
      builder.dispose();
    }
  });
});

describe("reordering: the drag and the keyboard reach the same tree", () => {
  const start = (): Filter => ({ kind: "and", items: [A, B, C] });

  it("moves a row down with the keyboard and keeps the focus on it", () => {
    const builder = mountBuilder(start());
    try {
      const sheet = builder.open();
      const handle = rootItems(sheet)[0]!.querySelector<HTMLElement>(".qs-drag-handle")!;
      expect(handle.getAttribute("aria-keyshortcuts")).toBe("ArrowUp ArrowDown");
      handle.focus();
      arrow(handle, "ArrowDown");

      expect(builder.changes).toHaveLength(1);
      expect(filterOf(builder)).toEqual({ kind: "and", items: [B, A, C] });
      // The rows were rebuilt from the new tree; the keyboard is on the control
      // that moved, at its new place, so a second press continues the move.
      expect((document.activeElement as HTMLElement).dataset.qsHandle).toBe("1");
      arrow(document.activeElement as HTMLElement, "ArrowDown");
      expect(filterOf(builder)).toEqual({ kind: "and", items: [B, C, A] });
      expect((document.activeElement as HTMLElement).dataset.qsHandle).toBe("2");
    } finally {
      builder.dispose();
    }
  });

  it("drops pending focus when the anchor changes but the root object does not", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const initial = session(start());
    const [current, setCurrent] = createSignal<BuilderSession>(initial);
    let submitted: BuilderSession | undefined;
    let finish!: (saved: boolean) => void;
    const save = new Promise<boolean>((resolve) => { finish = resolve; });
    const dispose = render(
      () => (
        <QueryBuilder
          session={current}
          onChange={(next) => {
            submitted = next;
            return save;
          }}
        />
      ),
      host,
    );

    try {
      host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
      const sheet = document.querySelector<HTMLElement>(".qs-sheet")!;
      const sourceRoot = current().query.filter;
      const handle = rootItems(sheet)[0]!.querySelector<HTMLElement>(".qs-drag-handle")!;
      handle.focus();
      arrow(handle, "ArrowDown");
      if (!submitted) throw new Error("the deferred reorder was not submitted");

      // The anchor effect must invalidate the path even though the filter still
      // has the exact source object identity. Returning to the old anchor before
      // the save resolves must not resurrect that discarded intent.
      setCurrent({ ...initial, query: { ...initial.query, anchor: "page" } });
      expect(current().query.filter).toBe(sourceRoot);
      await Promise.resolve();
      setCurrent({ ...submitted, query: { ...submitted.query, anchor: "block" } });
      finish(true);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();

      expect((document.activeElement as HTMLElement).dataset.qsHandle).not.toBe("1");
    } finally {
      dispose();
    }
  });

  it("saves nothing at the boundary of the list", () => {
    const builder = mountBuilder(start());
    try {
      const sheet = builder.open();
      const first = rootItems(sheet)[0]!.querySelector<HTMLElement>(".qs-drag-handle")!;
      arrow(first, "ArrowUp");
      const last = rootItems(sheet)[2]!.querySelector<HTMLElement>(".qs-drag-handle")!;
      arrow(last, "ArrowDown");
      expect(builder.changes).toHaveLength(0);
      expect(filterOf(builder)).toEqual(start());
    } finally {
      builder.dispose();
    }
  });

  it("a drop and the arrow key produce the same IR, and a drop is exactly one edit", () => {
    const dragged = mountBuilder(start());
    let byDrag: Filter;
    try {
      const sheet = dragged.open();
      const drop = dragOnto(rootItems(sheet), 0, 1);
      // Moving the pointer is not an edit; the indicator is drawn, nothing else.
      expect(dragged.changes).toHaveLength(0);
      expect(rootItems(sheet)[1]!.classList.contains("qs-drop-after")).toBe(true);
      drop();
      expect(dragged.changes).toHaveLength(1);
      byDrag = filterOf(dragged);
      expect(rootItems(sheet)[0]!.classList.contains("qs-drop-after")).toBe(false);
    } finally {
      dragged.dispose();
    }

    const typed = mountBuilder(start());
    try {
      const sheet = typed.open();
      arrow(rootItems(sheet)[0]!.querySelector<HTMLElement>(".qs-drag-handle")!, "ArrowDown");
      expect(filterOf(typed)).toEqual(byDrag);
      expect(byDrag).toEqual({ kind: "and", items: [B, A, C] });
    } finally {
      typed.dispose();
    }
  });

  it("draws no drop bar at a slot beside the dragged condition itself (GH #619)", () => {
    // Two conditions, dragging the LOWER one up. Only one slot changes anything: above the first. The slot
    // between the two (the first's lower edge or the dragged row's own upper edge) and the slot below the
    // dragged row are no-ops, and a bar there reads as a hidden third condition.
    const builder = mountBuilder({ kind: "and", items: [A, B] });
    try {
      const sheet = builder.open();
      const bars = () => rootItems(sheet).map((item) => [item.classList.contains("qs-drop-before"), item.classList.contains("qs-drop-after")]);
      const none = [[false, false], [false, false]];
      for (const [over, before] of [[0, false], [1, true], [1, false]] as const) {
        const drop = dragOnto(rootItems(sheet), 1, over, before);
        expect(bars()).toEqual(none);
        drop();
      }
      expect(builder.changes).toHaveLength(0);
      const drop = dragOnto(rootItems(sheet), 1, 0, true);
      expect(bars()).toEqual([[true, false], [false, false]]);
      drop();
      expect(builder.changes).toHaveLength(1);
      expect(filterOf(builder)).toEqual({ kind: "and", items: [B, A] });
    } finally {
      builder.dispose();
    }
  });

  it("moves a whole group, wrappers and subtree together", () => {
    const group: Filter = { kind: "off", inner: { kind: "or", items: [B, C] } };
    const builder = mountBuilder({ kind: "and", items: [A, group] });
    try {
      const sheet = builder.open();
      const header = sheet.querySelector<HTMLElement>(".qs-group-header .qs-drag-handle")!;
      arrow(header, "ArrowUp");
      expect(filterOf(builder)).toEqual({ kind: "and", items: [group, A] });
    } finally {
      builder.dispose();
    }
  });

  it("refuses a drop whose target is in another list", () => {
    const builder = mountBuilder({
      kind: "and",
      items: [A, { kind: "and", items: [B, C] }],
    });
    try {
      const sheet = builder.open();
      const outer = rootItems(sheet);
      const nested = [...sheet.querySelector<HTMLElement>(".qs-group .qs-rows")!.children] as HTMLElement[];
      // The pointer is over a row of the nested list. `closest()` resolves it to
      // the GROUP's item in the dragged row's own list, so the only thing this
      // drop can express is "after the group" — never "into it".
      stackRects([...outer, ...nested]);
      const handle = outer[0]!.querySelector<HTMLElement>(".qs-drag-handle")!;
      handle.dispatchEvent(pointer("pointerdown", 10, 10));
      const previous = document.elementFromPoint;
      try {
        document.elementFromPoint = () => nested[1]!;
        document.dispatchEvent(pointer("pointermove", 10, 130));
        document.dispatchEvent(pointer("pointerup", 10, 130));
      } finally {
        document.elementFromPoint = previous;
      }
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [{ kind: "and", items: [B, C] }, A],
      });
      expect(builder.changes).toHaveLength(1);
    } finally {
      builder.dispose();
    }
  });
});

describe("a drag that does not finish changes nothing", () => {
  const start = (): Filter => ({ kind: "and", items: [A, B, C] });

  it("cancels on Escape, through the sheet's own dismissal ladder", () => {
    const builder = mountBuilder(start());
    try {
      const sheet = builder.open();
      const drop = dragOnto(rootItems(sheet), 0, 2);
      // Escape reaches the DRAG first; the sheet under it stays open.
      expect(dismissTopTransient("escape")).toBe(true);
      expect(sheet.isConnected).toBe(true);
      expect(rootItems(sheet).some((item) => item.classList.contains("qs-drop-after"))).toBe(false);
      drop();
      expect(builder.changes).toHaveLength(0);
      expect(filterOf(builder)).toEqual(start());
    } finally {
      builder.dispose();
    }
  });

  it("cancels on pointercancel", () => {
    const builder = mountBuilder(start());
    try {
      const sheet = builder.open();
      const drop = dragOnto(rootItems(sheet), 0, 2);
      document.dispatchEvent(pointer("pointercancel", 10, 90));
      drop();
      expect(builder.changes).toHaveLength(0);
      expect(filterOf(builder)).toEqual(start());
    } finally {
      builder.dispose();
    }
  });

  it("refuses a drop whose tree was replaced under it", () => {
    const builder = mountBuilder(start());
    try {
      const sheet = builder.open();
      const drop = dragOnto(rootItems(sheet), 0, 2);
      // The block was re-read — a text-pane save, an external file edit, a graph
      // transition. Every index the drag is holding is a path into the OLD
      // revision, so the drop is refused rather than remapped.
      builder.setSession(session({ kind: "and", items: [C, B, A] }));
      drop();
      expect(builder.changes).toHaveLength(0);
      expect(filterOf(builder)).toEqual({ kind: "and", items: [C, B, A] });
    } finally {
      builder.dispose();
    }
  });

  it("drops a selection when the tree is replaced under it", () => {
    const builder = mountBuilder(start());
    try {
      const sheet = builder.open();
      rootItems(sheet)[0]!.querySelector<HTMLInputElement>(".qs-select")!.click();
      rootItems(sheet)[1]!.querySelector<HTMLInputElement>(".qs-select")!.click();
      expect(sheet.querySelector(".qs-selection-count")!.textContent).toBe("2 selected");

      builder.setSession(session({ kind: "and", items: [C, B, A] }));
      expect(sheet.querySelector(".qs-selection")).toBeNull();
      expect(rootItems(sheet).every((item) => !item.querySelector<HTMLInputElement>(".qs-select")!.checked))
        .toBe(true);
    } finally {
      builder.dispose();
    }
  });
});

describe("a folded subtree is a sibling like any other", () => {
  it("shows a folded not-off subtree as disabled and enables only its own Off", () => {
    const payload: Filter = { kind: "and", items: [C, A] };
    const nested = (inner: Filter): Filter => ({
      kind: "and", items: [A, { kind: "and", items: [B, { kind: "and", items: [inner, B] }] }],
    });
    const builder = mountBuilder(nested({ kind: "not", inner: { kind: "off", inner: payload } }));
    try {
      const sheet = builder.open();
      const chip = sheet.querySelector<HTMLElement>(".qs-row-advanced")!;
      const enabled = chip.querySelector<HTMLButtonElement>(".qs-enabled")!;
      expect(enabled.getAttribute("aria-checked")).toBe("false");
      expect(chip.querySelector(".qs-off-label")?.textContent).toBe("disabled");
      enabled.click();
      expect(filterOf(builder)).toEqual(nested({ kind: "not", inner: payload }));
      expect(builder.changes).toHaveLength(1);
    } finally {
      builder.dispose();
    }
  });

  //     and[ A, and[ B, and[ and[C, A], B ] ] ]
  // The innermost list is at the rendering cap, so both of its items draw as
  // `⟨advanced⟩` chips — presentation only; the IR under them is untouched.
  const deep = (): Filter => ({
    kind: "and",
    items: [A, { kind: "and", items: [B, { kind: "and", items: [{ kind: "and", items: [C, A] }, B] }] }],
  });

  it("selects, disables and moves without re-reading the payload it cannot draw", () => {
    const builder = mountBuilder(deep());
    try {
      const sheet = builder.open();
      const chips = [...sheet.querySelectorAll<HTMLElement>(".qs-row-advanced")];
      expect(chips).toHaveLength(2);
      expect(chips[0]!.querySelector(".qs-drag-handle")).not.toBeNull();
      expect(chips[0]!.querySelector(".qs-select")).not.toBeNull();

      chips[0]!.querySelector<HTMLButtonElement>(".qs-enabled")!.click();
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [
          A,
          {
            kind: "and",
            items: [
              B,
              {
                kind: "and",
                items: [{ kind: "off", inner: { kind: "and", items: [C, A] } }, B],
              },
            ],
          },
        ],
      });

      const disabledChip = [...sheet.querySelectorAll<HTMLElement>(".qs-row-advanced")][0]!;
      expect(disabledChip.querySelector(".qs-off-label")!.textContent).toBe("disabled");
      arrow(disabledChip.querySelector<HTMLElement>(".qs-drag-handle")!, "ArrowDown");
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [
          A,
          {
            kind: "and",
            items: [
              B,
              {
                kind: "and",
                items: [B, { kind: "off", inner: { kind: "and", items: [C, A] } }],
              },
            ],
          },
        ],
      });
      expect(builder.changes).toHaveLength(2);
    } finally {
      builder.dispose();
    }
  });
});

describe("the P6 controls are not type-dependent, so a registry that has not landed does not gate them", () => {
  const start = (): Filter => ({ kind: "and", items: [propertyFilter("cost", "3"), A] });

  it.each([
    ["failed", () => Promise.reject(new Error("The registry could not be read."))],
    ["pending", () => new Promise<never>(() => {})],
  ])("keeps disabling and reordering usable while the read has %s", async (_case, reply) => {
    vi.spyOn(backend(), "queryRegistry").mockImplementation(reply as () => Promise<never>);
    const builder = mountBuilder(start());
    try {
      const sheet = builder.open();
      await settle();
      const rows = rootItems(sheet);
      // The accepted behaviour is untouched: a property row's operator and value
      // still wait for a healthy registry, because both are type answers.
      expect(rows[0]!.querySelector<HTMLButtonElement>(".qs-op")!.disabled).toBe(true);

      // Neither `Off` nor an index is a type question, so neither waits.
      rows[0]!.querySelector<HTMLButtonElement>(".qs-enabled")!.click();
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [{ kind: "off", inner: propertyFilter("cost", "3") }, A],
      });
      arrow(rootItems(sheet)[0]!.querySelector<HTMLElement>(".qs-drag-handle")!, "ArrowDown");
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [A, { kind: "off", inner: propertyFilter("cost", "3") }],
      });
      expect(builder.changes).toHaveLength(2);
    } finally {
      builder.dispose();
    }
  });
});

describe("a drag whose owner goes away", () => {
  const start = (): Filter => ({ kind: "and", items: [A, B, C] });

  it("cancels when the pointer capture is taken, and when the sheet unmounts", () => {
    const first = mountBuilder(start());
    let drop: () => void;
    try {
      drop = dragOnto(rootItems(first.open()), 0, 2);
      document.dispatchEvent(new Event("lostpointercapture"));
      drop();
      expect(first.changes).toHaveLength(0);
    } finally {
      first.dispose();
    }

    const second = mountBuilder(start());
    const sheet = second.open();
    drop = dragOnto(rootItems(sheet), 0, 2);
    second.dispose();
    drop();
    expect(second.changes).toHaveLength(0);
  });
});

describe("a group that has been switched off is still a group", () => {
  it("keeps its header actions working on the right wrapper, and offers only what it can do", () => {
    // `none of`, then disabled: the sheet draws one group wearing two wrappers.
    const builder = mountBuilder({
      kind: "and",
      items: [{ kind: "off", inner: { kind: "not", inner: { kind: "or", items: [A, B] } } }, C],
    });
    try {
      const sheet = builder.open();
      const header = sheet.querySelector<HTMLElement>(".qs-group-header")!;
      expect(header.querySelector(".qs-group-op")!.textContent).toBe("none of");
      expect(header.querySelector(".qs-enabled")!.getAttribute("aria-checked")).toBe("false");

      header.querySelector<HTMLButtonElement>(".qs-row-menu")!.click();
      // Ungroup is NOT offered: two rows cannot be spliced into a unary wrapper
      // and rewriting them is not the builder's to decide.
      expect([...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")].map((o) => o.textContent))
        .toEqual(["Move down", "Remove none of", "Remove"]);

      // "Remove none of" removes the NOT and leaves the Off exactly where it was
      // — the wrapper the action names, not whichever one happens to be outermost.
      [...sheet.querySelectorAll<HTMLButtonElement>(".qs-option")]
        .find((option) => option.textContent === "Remove none of")!
        .click();
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [{ kind: "off", inner: { kind: "or", items: [A, B] } }, C],
      });

      // And the header's own press still switches the operator inside the Off.
      sheet.querySelector<HTMLButtonElement>(".qs-group-op")!.click();
      expect(filterOf(builder)).toEqual({
        kind: "and",
        items: [{ kind: "off", inner: { kind: "and", items: [A, B] } }, C],
      });
      expect(builder.changes).toHaveLength(2);
    } finally {
      builder.dispose();
    }
  });
});

// GH #619 items 2, 3 and 5 at the sheet: what the user sees and clicks.
describe("GH #619: Any status, In a journal page, no advanced chip for builder shapes", () => {
  it("offers Any status on the Task value menu, writes every OG marker, and reads it back as Any status", async () => {
    const builder = mountBuilder({ kind: "and", items: [taskFilter(["TODO"])] });
    try {
      const sheet = builder.open();
      await settle();
      sheet.querySelector<HTMLButtonElement>(".qs-value")!.click();
      const any = document.querySelector<HTMLButtonElement>(".qs-any-option");
      expect(any).not.toBeNull();
      expect(any!.textContent).toBe("Any status");
      any!.click();
      await settle();
      expect(builder.changes).toHaveLength(1);
      expect(builder.session().query.filter).toEqual({ kind: "and", items: [anyTaskFilter()] });
      expect(sheet.querySelector(".qs-value")!.textContent).toBe("Any status");
    } finally {
      builder.dispose();
    }
  });

  it("names the journal condition 'In a journal page' and shows it as a condition, not as dates", async () => {
    const builder = mountBuilder({ kind: "and", items: [journalFilter()] });
    try {
      const sheet = builder.open();
      await settle();
      const row = sheet.querySelector(".qs-row")!;
      expect(row.querySelector(".qs-field")!.textContent).toContain("In a journal page");
      expect(row.textContent).not.toMatch(/2000|-2000y|\+2000y/);
    } finally {
      builder.dispose();
    }
  });

  it("does not call a builder-made scheduled condition inside an any-of group advanced", async () => {
    const nested: Filter = {
      kind: "and",
      items: [pageRefFilter("x"), { kind: "or", items: [betweenFilter("scheduled", "-7d", "+7d"), betweenFilter("journal", "today", "+7d")] }],
    };
    const builder = mountBuilder(nested);
    try {
      const text = builder.host.textContent ?? "";
      expect(text).not.toMatch(/advanced/i);
      expect(text).toContain("scheduled: 7 days ago to 7 days ahead");
      expect(text).toContain("journal date: next 7 days");
    } finally {
      builder.dispose();
    }
  });
});
