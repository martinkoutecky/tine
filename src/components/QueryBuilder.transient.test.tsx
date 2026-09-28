// Ported from master src/components/QueryBuilder.transient.test.tsx. og changes:
// the Display-panel popover family is Q4b's; registry keys are read off the
// QueryListbox add-condition picker instead of the (Q4b) vocabulary picker.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { autocompleteFacets, backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { bumpGraphEpoch, setDataRev } from "../graphSession";
import type { RegistrySnapshot } from "../editor/queryIr";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  registerTransientLayer,
} from "../transientLayers";
import {
  QueryBuilder,
  requestQueryRegistryRefresh,
  resetQueryRegistryRevisionForTests,
  type BuilderSession,
} from "./QueryBuilder";
import {
  ADVANCED_PHRASE,
  MAX_QUERY_BUILDER_DEPTH,
  priorityFilter,
  taskFilter,
} from "../editor/queryBuilder";
import type { Filter } from "../editor/queryIr";

// The builder edits the IR now, so the harness hands it a `Filter` rather than a
// DSL string: there is no frontend parser left to turn text into a tree, and the
// popover behaviour under test never depended on the text form.
function session(filter: Filter): BuilderSession {
  return { query: { anchor: "block", filter, source: { kind: "builder" } }, view: {} };
}

function nested(depth: number, leaf: Filter): Filter {
  let filter = leaf;
  for (let level = 0; level < depth; level += 1) filter = { kind: "and", items: [filter] };
  return filter;
}

/** Mount one builder. At rest it draws ONE sentence inside `host`; `open()`
 *  presses its ⚙ and hands back the sheet, which is PORTALLED to <body> (the
 *  query block's own compositing layer is a containing block for `fixed`
 *  children, so the sheet cannot live inside it). Per-instance assertions
 *  therefore go through the returned sheet, not through `host`. */
function mountBuilder(filter: Filter = taskFilter(["TODO"])) {
  const host = document.createElement("div");
  document.body.append(host);
  const [current, setCurrent] = createSignal<BuilderSession>(session(filter));
  const dispose = render(
    () => <QueryBuilder session={current} onChange={setCurrent} />,
    host
  );
  let sheetEl: HTMLElement | null = null;
  const open = (): HTMLElement => {
    if (sheetEl?.isConnected) return sheetEl;
    const before = new Set(document.querySelectorAll<HTMLElement>(".qs-sheet"));
    host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
    sheetEl =
      [...document.querySelectorAll<HTMLElement>(".qs-sheet")].find((el) => !before.has(el)) ?? null;
    if (!sheetEl) throw new Error("the sheet did not open");
    return sheetEl;
  };
  return { host, open, source: () => JSON.stringify(current()), dispose };
}

/** The popover families the sheet owns, each named by its trigger inside one
 *  open sheet. The GH #472 report was that ONE of them had hand-rolled its
 *  outside-click handling; the fix was to give them all the same one, so every
 *  case below drives all of them through the same gesture. */
interface PopoverFamily {
  name: string;
  open: (sheet: HTMLElement) => HTMLButtonElement;
  visible: string;
  /** Portalled to `<body>` rather than mounted inside the sheet. The shared
   *  Display panel is: the query block's own compositing layer is a containing
   *  block for `fixed` children, so a panel drawn inside it could not escape.
   *  It still belongs to the sheet's transient rung — which is exactly what
   *  these cases prove — so it is looked for in the document instead. */
  portalled?: boolean;
}
const FAMILIES: PopoverFamily[] = [
  { name: "anchor menu", open: (s) => s.querySelector<HTMLButtonElement>(".qs-anchor-button")!, visible: ".qs-menu" },
  { name: "row field menu", open: (s) => s.querySelector<HTMLButtonElement>(".qs-row .qs-field")!, visible: ".qs-menu" },
  { name: "row operator menu", open: (s) => s.querySelector<HTMLButtonElement>(".qs-row .qs-op")!, visible: ".qs-menu" },
  { name: "add-condition picker", open: (s) => s.querySelector<HTMLButtonElement>(".qs-add")!, visible: ".qs-menu" },
  // master's fifth family, the Display panel (`.qd-trigger`), is Q4b's surface.
];

/** Where a family's panel is drawn. */
const shown = (family: PopoverFamily, sheet: HTMLElement): Element | null =>
  (family.portalled ? document : sheet).querySelector(family.visible);

/** Let the shared registry request resolve through its promise chain, and
 *  report how many backend calls it took. */
async function settleRegistry(registry: { mock: { calls: unknown[][] } }): Promise<number> {
  for (let tick = 0; tick < 8; tick += 1) await Promise.resolve();
  return registry.mock.calls.length;
}

/** A registry snapshot of exactly these keys, each with a block count. */
function snapshot(keys: [string, number][]): RegistrySnapshot {
  return {
    generation: 1,
    rows: keys.map(([normalized_name, count_blocks]) => ({
      normalized_name,
      cardinality: "one" as const,
      observed_type: "text" as const,
      count_blocks,
      count_pages: 0,
      mismatch_count: 0,
    })),
  };
}

afterEach(() => {
  clearTransientLayersForTest();
  resetSharedQueryResultsForTests();
  resetQueryRegistryRevisionForTests();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});


beforeEach(() => {
  // The vocabulary list is virtualized, so a picker in a zero-height jsdom
  // viewport would mount overscan alone (N2). Production sizing is unchanged.
  resetQueryRegistryRevisionForTests();
  vi.spyOn(backend(), "queryFacets").mockResolvedValue([]);
  vi.spyOn(backend(), "queryRegistry").mockResolvedValue(snapshot([]));
  // The text pane's contents are PRINTED BY RUST (I-12); these tests are about
  // the popovers above it, so the printer is stubbed rather than exercised.
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(and (task TODO))");
});

describe("QueryBuilder transient ownership (post-GH #161)", () => {
  // Sharing across instances is proven separately, by the Harvest W4-P1 item 3
  // test below; this one pins the per-revision refresh for a single builder.
  //
  // **P4 rehomed this from `query_facets(false)` to `query_registry`.** The
  // facets read is gone with the two-stage property chooser it fed; the
  // registry is now the sheet's only graph-level question, and it inherited the
  // lifecycle the facets read used to provide — nothing at rest, once on open,
  // once per data revision — plus the declaration refresh it never had.
  it("asks the graph nothing at rest, reads the registry when the sheet opens, and again per data revision", async () => {
    const registry = vi.mocked(backend().queryRegistry);
    const facets = vi.mocked(backend().queryFacets);
    const { open, dispose } = mountBuilder();
    try {
      await Promise.resolve();
      // A page of RESTING sentences is the common case, and it costs the graph
      // nothing: the vocabulary is the editor's, not the sentence's.
      expect(registry).toHaveBeenCalledTimes(0);

      const sheet = open();
      await Promise.resolve();
      await Promise.resolve();
      expect(registry).toHaveBeenCalledTimes(1);

      // Opening the picker and typing in its search is pure frontend work over
      // the snapshot already in hand — never a keystroke-per-request (I-13).
      sheet.querySelector<HTMLButtonElement>(".qs-add")!.click();
      const search = sheet.querySelector<HTMLInputElement>(".qs-menu-filter")!;
      for (const text of ["o", "ow", "own", "owne"]) {
        search.value = text;
        search.dispatchEvent(new Event("input", { bubbles: true }));
      }
      await Promise.resolve();
      expect(registry).toHaveBeenCalledTimes(1);

      setDataRev((revision) => revision + 1);
      await Promise.resolve();
      await Promise.resolve();
      expect(registry).toHaveBeenCalledTimes(2);

      // The facets read the builder used to make is gone entirely. The
      // autocomplete producer's `queryFacets(true)` is a different question and
      // keeps its own path (`Block.propertyAutocomplete.test.tsx`).
      expect(facets).toHaveBeenCalledTimes(0);
    } finally {
      dispose();
    }
  });

  it("renders a bounded ⟨advanced⟩ chip instead of recursing through a hostile query tree", () => {
    // 64 levels still PARSE (`QUERY_NESTING_MAX`); what is bounded here is the
    // drawing. Both the sentence and the rows stop at the rendering cap, so a
    // query written by outside content cannot make either of them big (I-22).
    const depth = 64;
    const { host, open, dispose } = mountBuilder(nested(depth, taskFilter(["TODO"])));
    try {
      const sentence = host.querySelector(".qs-sentence")!;
      expect(sentence.textContent).toContain(ADVANCED_PHRASE);
      expect(sentence.querySelectorAll(".qs-seg").length).toBeLessThanOrEqual(8);

      const sheet = open();
      expect(sheet.querySelectorAll(".qs-row-advanced").length).toBe(1);
      expect(sheet.querySelectorAll(".qs-group").length).toBeLessThanOrEqual(
        MAX_QUERY_BUILDER_DEPTH,
      );
      expect(sheet.querySelectorAll(".qs-row").length).toBeLessThanOrEqual(
        MAX_QUERY_BUILDER_DEPTH + 1,
      );
    } finally {
      dispose();
    }
  });

  it("gives every popover family one Escape or Back rung above the sheet, and the sheet one above a lower owner", () => {
    // Three rungs, in order: the menu, then the sheet, then whatever owned the
    // screen before either — and no rung edits the query.
    for (const [index, family] of FAMILIES.entries()) {
      const reason = index % 2 === 0 ? "escape" : "back";
      const { open, source, dispose } = mountBuilder();
      const original = source();
      const lower = vi.fn(() => true);
      const unregisterLower = registerTransientLayer({
        id: `query-builder-lower-${index}`,
        dismiss: lower,
      });
      try {
        const sheet = open();
        family.open(sheet).click();
        expect(shown(family, sheet), `${family.name} did not open`).not.toBeNull();

        expect(dismissTopTransient(reason)).toBe(true);
        expect(shown(family, sheet), `${family.name} survived its own rung`).toBeNull();
        expect(sheet.isConnected, "the sheet went with the menu").toBe(true);
        expect(lower).not.toHaveBeenCalled();

        expect(dismissTopTransient(reason)).toBe(true);
        expect(document.querySelector(".qs-sheet")).toBeNull();
        expect(lower).not.toHaveBeenCalled();
        expect(source()).toBe(original);
      } finally {
        unregisterLower();
        dispose();
      }
    }
  });

  it("keeps two builder instances independent: a press in one closes only the other's menu", () => {
    // Reactivation of an older visible peer by an inside pointer is pinned
    // generically in transientRegistry.p1d1.lifecycle.test.tsx. What is specific
    // here is that two builders on one page own separate sheets and separate
    // popover state, and that a press inside one is an OUTSIDE press for the
    // other (GH #472) — so they cannot both stay open, and the one pressed
    // survives.
    const first = mountBuilder(taskFilter(["TODO"]));
    const second = mountBuilder(priorityFilter(["A"]));
    try {
      const firstSheet = first.open();
      const secondSheet = second.open();
      firstSheet.querySelector<HTMLButtonElement>(".qs-row .qs-field")!.click();
      secondSheet.querySelector<HTMLButtonElement>(".qs-row .qs-field")!.click();
      expect(firstSheet.querySelector(".qs-menu")).not.toBeNull();
      expect(secondSheet.querySelector(".qs-menu")).not.toBeNull();

      firstSheet.querySelector(".qs-menu")!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      expect(firstSheet.querySelector(".qs-menu")).not.toBeNull();
      expect(secondSheet.querySelector(".qs-menu")).toBeNull();

      expect(dismissTopTransient("escape")).toBe(true);
      expect(firstSheet.querySelector(".qs-menu")).toBeNull();
    } finally {
      first.dispose();
      second.dispose();
    }
  });
});

describe("GH #472: every Query Builder popover closes on an outside press", () => {
  // The reported failure: the leftmost (clause) menu stayed open when the user
  // clicked away — "the menu even stays open after clicking into and editing a
  // different block" — while the sort popover next to it closed correctly. The
  // difference was that two of the popovers had hand-rolled an outside-click
  // effect and two had not, so the cases below drive ALL of them through the
  // same gesture rather than only the reported one. The sheet added a rung
  // under them, so an outside press now takes the menu FIRST and the sheet on
  // the press after — never both at once.
  const popovers = FAMILIES;

  // A press elsewhere in the document — the page background, another block.
  const pressOutside = (type: "mousedown" | "pointerdown") => {
    const elsewhere = document.createElement("div");
    document.body.append(elsewhere);
    elsewhere.dispatchEvent(new MouseEvent(type, { bubbles: true }));
    elsewhere.remove();
  };

  const expectAllClosedBy = (type: "mousedown" | "pointerdown") => {
    for (const popover of popovers) {
      const { open, source, dispose } = mountBuilder();
      const original = source();
      try {
        const sheet = open();
        popover.open(sheet).click();
        expect(shown(popover, sheet), `${popover.name} did not open`).not.toBeNull();

        pressOutside(type);

        expect(shown(popover, sheet), `${popover.name} stayed open`).toBeNull();
        expect(sheet.isConnected, `${popover.name} took the sheet with it`).toBe(true);

        pressOutside(type);
        expect(document.querySelector(".qs-sheet"), "the sheet stayed open").toBeNull();
        expect(source()).toBe(original);
      } finally {
        dispose();
      }
    }
  };

  // Both event types, because touch and pen deliver only the first and some
  // synthesized/compatibility paths only the second.
  it("closes each popover on an outside mousedown without changing the query", () => {
    expectAllClosedBy("mousedown");
  });

  it("closes each popover on an outside pointerdown without changing the query", () => {
    expectAllClosedBy("pointerdown");
  });

  it("keeps a popover open when the press lands inside it", () => {
    for (const popover of popovers) {
      const { open, dispose } = mountBuilder();
      try {
        const sheet = open();
        popover.open(sheet).click();
        const panel = shown(popover, sheet)!;
        panel.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        expect(shown(popover, sheet), `${popover.name} closed on an inside press`).not.toBeNull();
      } finally {
        dispose();
      }
    }
  });

  it("keeps the sheet open when a press lands on a CLOSED popover's trigger", () => {
    // The gesture every one of these popovers starts with: the sheet is open,
    // nothing else is, and the user presses `+ sort`. That pointerdown is
    // inside the sheet, so the sheet must not treat it as "the user pressed
    // somewhere else" — if it does, the sheet closes under the press and the
    // click that follows lands on nothing at all.
    for (const popover of popovers) {
      const { open, dispose } = mountBuilder();
      try {
        const sheet = open();
        const trigger = popover.open(sheet);
        trigger.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
        expect(
          document.querySelector(".qs-sheet"),
          `the sheet closed on the press that opens ${popover.name}`,
        ).not.toBeNull();
        trigger.click();
        expect(shown(popover, sheet), `${popover.name} never opened`).not.toBeNull();
      } finally {
        dispose();
      }
    }
  });

  it("lets the trigger of an open popover toggle it shut instead of reopening it", () => {
    // The trigger must count as INSIDE: dismissing on its press would close the
    // popover, and the click that follows would immediately reopen it.
    for (const popover of popovers) {
      const { open, dispose } = mountBuilder();
      try {
        const sheet = open();
        const trigger = popover.open(sheet);
        trigger.click();
        expect(shown(popover, sheet)).not.toBeNull();

        trigger.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        expect(shown(popover, sheet), `${popover.name} closed before its own click`).not.toBeNull();
        trigger.click();
        expect(shown(popover, sheet), `${popover.name} did not toggle shut`).toBeNull();
      } finally {
        dispose();
      }
    }
  });
});

describe("QueryBuilder registry sharing (Harvest W4-P1 item 3)", () => {
  // Drive the production vocabulary picker and read back the property keys it
  // offers, so a "one call" bound cannot be met by starving four of the five
  // builders. **Rehomed from `query_facets(false)` in P4:** the request being
  // shared is the registry read, and its key gained two terms the facets key
  // never had — the shared declaration revision, and the graph root.
  function propertyKeysOffered(sheet: HTMLElement): string[] {
    const add = sheet.querySelector<HTMLButtonElement>(".qs-add")!;
    add.click();
    const keys = listboxPropertyKeys(sheet);
    add.click(); // The trigger toggles: leave the picker closed for the next read.
    return keys;
  }

  it("issues one shared registry request per (graph scope, dataRev, declaration) for five mounted builders", async () => {
    const payloads = [
      snapshot([["revision-one", 3]]),
      snapshot([["revision-two", 3]]),
      snapshot([["revision-three", 3]]),
      snapshot([["revision-four", 3]]),
    ];
    let current = 0;
    const registry = vi.mocked(backend().queryRegistry);
    registry.mockReset();
    registry.mockImplementation(async () => payloads[current]);
    const facets = vi.mocked(backend().queryFacets);

    const builders = Array.from({ length: 5 }, () => mountBuilder(taskFilter(["TODO"])));
    try {
      await Promise.resolve();
      await Promise.resolve();
      // Five RESTING sentences ask nothing at all; the shared request is made
      // when the first sheet opens and served to the other four from the cache.
      expect(registry.mock.calls.length).toBe(0);
      const sheets = builders.map((builder) => builder.open());
      const mounted = await settleRegistry(registry);
      for (const sheet of sheets) {
        expect(propertyKeysOffered(sheet)).toEqual(["revision-one"]);
      }

      // A new data revision: one fresh shared call, and every builder sees it.
      // The facets path used to be the only thing keyed on `dataRev`; removing
      // it without giving the registry the same key would have left every open
      // sheet showing the vocabulary the graph had before the last save.
      registry.mockClear();
      current = 1;
      setDataRev((revision) => revision + 1);
      const perRevision = await settleRegistry(registry);
      for (const sheet of sheets) {
        expect(propertyKeysOffered(sheet)).toEqual(["revision-two"]);
      }

      // A declaration written in ONE sheet is a fact about the graph: it
      // refreshes every open builder, through one shared call, and it cannot be
      // served from the already-resolved entry the previous key holds.
      registry.mockClear();
      current = 2;
      requestQueryRegistryRefresh();
      const perDeclaration = await settleRegistry(registry);
      for (const sheet of sheets) {
        expect(propertyKeysOffered(sheet)).toEqual(["revision-three"]);
      }

      // A graph switch: the shared scope changes, so one fresh call again — and
      // the previous graph's rows are never shown under the new one.
      registry.mockClear();
      current = 3;
      bumpGraphEpoch();
      const perGraphScope = await settleRegistry(registry);
      for (const sheet of sheets) {
        expect(propertyKeysOffered(sheet)).toEqual(["revision-four"]);
      }

      // The autocomplete producer asks a DIFFERENT question, on a different
      // command, and is not served by any of this.
      facets.mockClear();
      facets.mockImplementation(async (autocomplete?: boolean) =>
        autocomplete ? [["autocomplete-only", ["a"]]] : []
      );
      expect(await autocompleteFacets()).toEqual([["autocomplete-only", ["a"]]]);
      const autocompleteCalls = facets.mock.calls.map(([flag]) => flag ?? false);

      // eslint-disable-next-line no-console -- the measurement IS the receipt.
      console.log(
        `w4_p1_query_registry builders=5 mounted=${mounted} perDataRev=${perRevision} ` +
          `perDeclaration=${perDeclaration} perGraphScope=${perGraphScope} ` +
          `autocomplete=${JSON.stringify(autocompleteCalls)}`
      );

      expect({ mounted, perRevision, perDeclaration, perGraphScope, autocompleteCalls }).toEqual({
        mounted: 1,
        perRevision: 1,
        perDeclaration: 1,
        perGraphScope: 1,
        autocompleteCalls: [true],
      });
    } finally {
      for (const builder of builders) builder.dispose();
    }
  });
});

describe("QueryBuilder registry landing (I-20)", () => {
  it.each(["data", "declaration"])("withdraws obsolete rows while the %s revision is pending", async (kind) => {
    const registry = vi.mocked(backend().queryRegistry);
    registry.mockReset();
    let release!: (value: RegistrySnapshot) => void;
    registry.mockResolvedValueOnce(snapshot([["obsolete-type", 4]]));
    registry.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    const builder = mountBuilder();
    try {
      const sheet = builder.open();
      await settleRegistry(registry);
      expect(propertyKeys(sheet)).toEqual(["obsolete-type"]);
      if (kind === "data") setDataRev((n) => n + 1);
      else requestQueryRegistryRefresh();
      await settleRegistry(registry);
      expect(propertyKeys(sheet)).toEqual([]);
      release(snapshot([["current-type", 4]]));
      await settleRegistry(registry);
      expect(propertyKeys(sheet)).toEqual(["current-type"]);
    } finally { builder.dispose(); }
  });
  // A graph switch does not answer instantly. While the new read is in flight
  // the resource still HOLDS the previous graph's snapshot — that is what
  // `.latest` is for — and offering it would let a reader build a condition on
  // a key this graph has never had, from a list that looks entirely current.
  // So the landed payload carries the scope it was fetched for, and the rows
  // are read only while that scope is still the live one.
  it("shows no rows from the previous graph while the new registry read is in flight", async () => {
    const registry = vi.mocked(backend().queryRegistry);
    registry.mockReset();
    let release: ((snapshot: RegistrySnapshot) => void) | null = null;
    registry.mockImplementationOnce(async () => snapshot([["before-the-switch", 400]]));
    registry.mockImplementationOnce(
      () => new Promise<RegistrySnapshot>((resolve) => { release = resolve; }),
    );

    const builder = mountBuilder(taskFilter(["TODO"]));
    try {
      const sheet = builder.open();
      await settleRegistry(registry);
      expect(propertyKeys(sheet)).toEqual(["before-the-switch"]);

      // The graph changes. The new read has not answered yet.
      bumpGraphEpoch();
      await settleRegistry(registry);
      expect(propertyKeys(sheet)).toEqual([]);

      release!(snapshot([["after-the-switch", 5]]));
      await settleRegistry(registry);
      expect(propertyKeys(sheet)).toEqual(["after-the-switch"]);
    } finally {
      builder.dispose();
    }
  });
});

/** The property keys the production picker offers inside an open sheet. */
function propertyKeys(sheet: HTMLElement): string[] {
  const add = sheet.querySelector<HTMLButtonElement>(".qs-add")!;
  add.click();
  const keys = listboxPropertyKeys(sheet);
  add.click();
  return keys;
}

/** og: the property keys are the QueryListbox options under "Properties". */
function listboxPropertyKeys(sheet: HTMLElement): string[] {
  const keys: string[] = [];
  let section = "";
  for (const el of sheet.querySelectorAll<HTMLElement>(".qs-options > *")) {
    if (el.classList.contains("qs-option-section")) section = el.textContent ?? "";
    else if (section === "Properties") keys.push(el.childNodes[0]?.textContent ?? "");
  }
  return keys;
}
