// Ported from master src/components/Page.queryUndoLifecycle.test.tsx (7fcd4c98d,
// 93ff682a3). og adaptations: the routed DTO is a `PageRead` (carries `id`), and
// og re-reads a loaded page by path, so `getPageByPath` is mocked beside `getPage`.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { clearTransientLayersForTest } from "../transientLayers";
import { endEdit } from "../editorController";
import type { Filter, Query } from "../editor/queryIr";
import { initParser } from "../render/parse";
import { backendReadsQueries, blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { redo, resetStore, setRaw, undo } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import type { PageRead } from "../types";
import { PageView } from "./Page";

beforeAll(async () => {
  await initParser();
});

const PAGE = "P6 undo continuity";
const INITIAL_ARGUMENT =
  "@block and content = 'alpha' and content = 'beta' and content = 'gamma' and content = 'delta'";
const DISABLED_ARGUMENT =
  "@block and Off(content = 'alpha') and content = 'beta' and content = 'gamma' and content = 'delta'";
const FIRST_MOVE_ARGUMENT =
  "@block and content = 'beta' and content = 'alpha' and content = 'gamma' and content = 'delta'";
const SECOND_MOVE_ARGUMENT =
  "@block and content = 'beta' and content = 'gamma' and content = 'alpha' and content = 'delta'";
const INITIAL_RAW = `{{tine-query ${INITIAL_ARGUMENT}}}`;
const DISABLED_RAW = `{{tine-query ${DISABLED_ARGUMENT}}}`;
const FIRST_MOVE_RAW = `{{tine-query ${FIRST_MOVE_ARGUMENT}}}`;
const SECOND_MOVE_RAW = `{{tine-query ${SECOND_MOVE_ARGUMENT}}}`;

const content = (text: string): Filter => ({
  kind: "leaf",
  leaf: { kind: "attr", attr: "content", op: "eq", value: { kind: "text", text } },
});
const enabledFilter: Filter = {
  kind: "and",
  items: [content("alpha"), content("beta"), content("gamma"), content("delta")],
};
const disabledFilter: Filter = {
  kind: "and",
  items: [{ kind: "off", inner: content("alpha") }, content("beta"), content("gamma"), content("delta")],
};
const firstMoveFilter: Filter = {
  kind: "and",
  items: [content("beta"), content("alpha"), content("gamma"), content("delta")],
};
const secondMoveFilter: Filter = {
  kind: "and",
  items: [content("beta"), content("gamma"), content("alpha"), content("delta")],
};

function node(id: string, raw: string): StoreNode {
  return { id, raw, collapsed: false, parent: null, page: PAGE, children: [] };
}

function feedPage(): FeedPage {
  return {
    name: PAGE,
    kind: "page",
    title: PAGE,
    preBlock: null,
    roots: ["query", "unrelated"],
    format: "md",
    readOnly: false,
    guide: false,
  };
}

function pageDto(): PageRead {
  return {
    id: `pages/${PAGE}.md`,
    name: PAGE,
    kind: "page",
    title: PAGE,
    pre_block: null,
    format: "md",
    blocks: [
      { id: "query", raw: INITIAL_RAW, collapsed: false, children: [] },
      { id: "unrelated", raw: "Unrelated bytes stay exact", collapsed: false, children: [] },
    ],
  };
}

function printedArgument(query: Query): string {
  const first = query.filter.kind === "and" ? query.filter.items[0] : undefined;
  if (first?.kind === "off") return DISABLED_ARGUMENT;
  if (query.filter.kind !== "and") return INITIAL_ARGUMENT;
  const order = query.filter.items.map((item) => {
    if (item.kind !== "leaf" || item.leaf.kind !== "attr" || item.leaf.value.kind !== "text") return "";
    return item.leaf.value.text;
  }).join(",");
  if (order === "beta,alpha,gamma,delta") return FIRST_MOVE_ARGUMENT;
  if (order === "beta,gamma,alpha,delta") return SECOND_MOVE_ARGUMENT;
  return INITIAL_ARGUMENT;
}

function queryReadings() {
  return {
    [INITIAL_ARGUMENT]: { form: INITIAL_ARGUMENT, kind: "tql" as const, filter: enabledFilter },
    [DISABLED_ARGUMENT]: { form: DISABLED_ARGUMENT, kind: "tql" as const, filter: disabledFilter },
    [FIRST_MOVE_ARGUMENT]: { form: FIRST_MOVE_ARGUMENT, kind: "tql" as const, filter: firstMoveFilter },
    [SECOND_MOVE_ARGUMENT]: { form: SECOND_MOVE_ARGUMENT, kind: "tql" as const, filter: secondMoveFilter },
  };
}

function arrowDown(handle: HTMLElement): void {
  handle.dispatchEvent(new KeyboardEvent("keydown", {
    key: "ArrowDown",
    bubbles: true,
    cancelable: true,
  }));
}

function activeHandle(): HTMLElement | null {
  const active = document.activeElement;
  return active instanceof HTMLElement && active.matches(".qs-drag-handle") ? active : null;
}

function focusedRowValue(): string | null {
  return activeHandle()?.closest(".qs-row")?.querySelector(".qs-value")?.textContent?.trim() ?? null;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

function firstEnabled(): HTMLButtonElement {
  const control = document.querySelector<HTMLButtonElement>(
    '.qs-sheet > .qs-rows > .qs-row[data-qs-parent=""][data-row-index="0"] .qs-enabled',
  );
  if (!control) throw new Error("the first P6 enabled switch is absent");
  return control;
}

function rootHandle(index: number): HTMLElement {
  const handle = document.querySelector<HTMLElement>(
    `.qs-sheet > .qs-rows > .qs-row[data-qs-parent=""][data-row-index="${index}"] .qs-drag-handle`,
  );
  if (!handle) throw new Error(`root query handle ${index} is absent`);
  return handle;
}

function seedPage(): void {
  const dto = pageDto();
  setDoc({
    byId: {
      query: node("query", INITIAL_RAW),
      unrelated: node("unrelated", dto.blocks[1]!.raw),
    },
    pages: [feedPage()],
    feed: [PAGE],
    loaded: true,
  });
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
  // og: once loaded the route re-reads by the file path (loadedPage.id).
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
  backendReadsQueries(queryReadings());
}

async function mountOpenSheet(): Promise<{
  host: HTMLDivElement;
  sheet: HTMLElement;
  dispose: () => void;
}> {
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: PAGE, pageKind: "page" });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <PageView />, host);
  const gear = await vi.waitFor(() => {
    const found = host.querySelector<HTMLButtonElement>(".qs-gear");
    if (!found) throw new Error("the routed query block did not render its builder");
    return found;
  });
  gear.click();
  const sheet = await vi.waitFor(() => {
    const found = document.querySelector<HTMLElement>('.qs-sheet[aria-label="Query filter"]');
    if (!found) throw new Error("the query sheet did not open");
    return found;
  });
  await vi.waitFor(() => expect(rootHandle(0)).toBeTruthy());
  return { host, sheet, dispose };
}

async function expectRowState(enabled: boolean): Promise<void> {
  await vi.waitFor(() => {
    expect(firstEnabled().getAttribute("aria-checked")).toBe(enabled ? "true" : "false");
  });
}

afterEach(() => {
  clearTransientLayersForTest();
  resetSharedQueryResultsForTests();
  endEdit("blur");
  resetStore();
  resetTabsToJournals();
  vi.restoreAllMocks();
  localStorage.clear();
  document.body.replaceChildren();
});

describe("PageView query undo lifecycle", () => {
  it("keeps the open sheet mounted while Off save, undo, and redo replay its page snapshot", async () => {
    const dto = pageDto();
    setDoc({
      byId: {
        query: node("query", INITIAL_RAW),
        unrelated: node("unrelated", dto.blocks[1]!.raw),
      },
      pages: [feedPage()],
      feed: [PAGE],
      loaded: true,
    });

    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    // og: once loaded the route re-reads by the file path (loadedPage.id).
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
    vi.spyOn(backend(), "printQuery").mockImplementation(async (query) => printedArgument(query));
    backendReadsQueries(queryReadings());

    mainPaneRouter.replaceActiveRoute({ kind: "page", name: PAGE, pageKind: "page" });
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <PageView />, host);

    try {
      const gear = await vi.waitFor(() => {
        const found = host.querySelector<HTMLButtonElement>(".qs-gear");
        if (!found) throw new Error("the routed query block did not render its builder");
        return found;
      });
      gear.click();
      const sheet = await vi.waitFor(() => {
        const found = document.querySelector<HTMLElement>('.qs-sheet[aria-label="Query filter"]');
        if (!found) throw new Error("the query sheet did not open");
        return found;
      });
      expect(gear.getAttribute("aria-expanded")).toBe("true");
      await expectRowState(true);

      firstEnabled().click();
      await vi.waitFor(() => expect(doc.byId.query.raw).toBe(DISABLED_RAW));
      await expectRowState(false);
      expect(host.querySelector(".qs-gear")).toBe(gear);
      expect(document.querySelector('.qs-sheet[aria-label="Query filter"]')).toBe(sheet);
      expect(doc.byId.unrelated.raw).toBe("Unrelated bytes stay exact");

      undo();
      await vi.waitFor(() => expect(doc.byId.query.raw).toBe(INITIAL_RAW));
      await expectRowState(true);
      expect(host.querySelector(".qs-gear")).toBe(gear);
      expect(document.querySelector('.qs-sheet[aria-label="Query filter"]')).toBe(sheet);
      expect(gear.getAttribute("aria-expanded")).toBe("true");
      expect(sheet.isConnected).toBe(true);
      expect(doc.byId.unrelated.raw).toBe("Unrelated bytes stay exact");

      redo();
      await vi.waitFor(() => expect(doc.byId.query.raw).toBe(DISABLED_RAW));
      await expectRowState(false);
      expect(host.querySelector(".qs-gear")).toBe(gear);
      expect(document.querySelector('.qs-sheet[aria-label="Query filter"]')).toBe(sheet);
      expect(gear.getAttribute("aria-expanded")).toBe("true");
      expect(sheet.isConnected).toBe(true);
      expect(doc.byId.unrelated.raw).toBe("Unrelated bytes stay exact");
    } finally {
      dispose();
    }
  });

  it("restores focus to the moved row after each deferred saved root is rendered", async () => {
    seedPage();
    const pending: Array<ReturnType<typeof deferred<string>>> = [];
    vi.spyOn(backend(), "printQuery").mockImplementation(async (query, _view, dialect) => {
      const argument = printedArgument(query);
      if (dialect !== "tql_macro") return argument;
      const job = deferred<string>();
      pending.push(job);
      return job.promise;
    });
    const { host, sheet, dispose } = await mountOpenSheet();

    try {
      const first = rootHandle(0);
      first.focus();
      arrowDown(first);
      await vi.waitFor(() => expect(pending).toHaveLength(1));
      expect(doc.byId.query.raw).toBe(INITIAL_RAW);

      pending[0]!.resolve(FIRST_MOVE_ARGUMENT);
      await vi.waitFor(() => {
        expect(doc.byId.query.raw).toBe(FIRST_MOVE_RAW);
        expect(activeHandle()?.dataset.qsHandle).toBe("1");
        expect(focusedRowValue()).toBe("alpha");
      });

      const moved = activeHandle();
      if (!moved) throw new Error("the first saved reorder did not restore its handle focus");
      arrowDown(moved);
      await vi.waitFor(() => expect(pending).toHaveLength(2));
      pending[1]!.resolve(SECOND_MOVE_ARGUMENT);
      await vi.waitFor(() => {
        expect(doc.byId.query.raw).toBe(SECOND_MOVE_RAW);
        expect(activeHandle()?.dataset.qsHandle).toBe("2");
        expect(focusedRowValue()).toBe("alpha");
      });
      expect(document.querySelector('.qs-sheet[aria-label="Query filter"]')).toBe(sheet);
      expect(host.querySelector(".qs-gear")?.getAttribute("aria-expanded")).toBe("true");
      expect(doc.byId.unrelated.raw).toBe("Unrelated bytes stay exact");
    } finally {
      dispose();
    }
  });

  it("discards a rejected reorder before a later matching external root arrives", async () => {
    seedPage();
    const save = deferred<string>();
    let requested = 0;
    vi.spyOn(backend(), "printQuery").mockImplementation(async (query, _view, dialect) => {
      if (dialect !== "tql_macro") return printedArgument(query);
      requested += 1;
      return save.promise;
    });
    const { dispose } = await mountOpenSheet();

    try {
      const first = rootHandle(0);
      first.focus();
      arrowDown(first);
      await vi.waitFor(() => expect(requested).toBe(1));
      save.reject(new Error("printer refused the reorder"));
      await vi.waitFor(() => {
        expect(document.querySelector(".query-print-refused")?.textContent).toContain(
          "printer refused the reorder",
        );
      });
      expect(doc.byId.query.raw).toBe(INITIAL_RAW);

      setRaw("query", FIRST_MOVE_RAW);
      await vi.waitFor(() => {
        expect(rootHandle(1).closest(".qs-row")?.querySelector(".qs-value")?.textContent?.trim()).toBe("alpha");
      });
      expect(activeHandle()?.dataset.qsHandle).not.toBe("1");
    } finally {
      dispose();
    }
  });

  it("does not arm focus when the page becomes read-only before the deferred write", async () => {
    seedPage();
    const save = deferred<string>();
    let requested = 0;
    vi.spyOn(backend(), "printQuery").mockImplementation(async (query, _view, dialect) => {
      if (dialect !== "tql_macro") return printedArgument(query);
      requested += 1;
      return save.promise;
    });
    const { dispose } = await mountOpenSheet();

    try {
      const first = rootHandle(0);
      first.focus();
      arrowDown(first);
      await vi.waitFor(() => expect(requested).toBe(1));

      setDoc("pages", 0, "readOnly", true);
      expect(doc.pages[0]?.readOnly).toBe(true);
      save.resolve(FIRST_MOVE_ARGUMENT);
      await save.promise;
      await Promise.resolve();
      await Promise.resolve();
      expect(doc.byId.query.raw).toBe(INITIAL_RAW);

      // An external replacement that happens to equal the refused move must
      // not receive the abandoned destination focus. Direct store publication
      // here models a read-only page changing under external graph refresh.
      setDoc("byId", "query", "raw", FIRST_MOVE_RAW);
      await vi.waitFor(() => {
        expect(rootHandle(1).closest(".qs-row")?.querySelector(".qs-value")?.textContent?.trim()).toBe("alpha");
      });
      expect(activeHandle()?.dataset.qsHandle).not.toBe("1");
    } finally {
      dispose();
    }
  });

  it("does not steal focus when the user leaves a reorder handle while save is pending", async () => {
    seedPage();
    const save = deferred<string>();
    let requested = 0;
    vi.spyOn(backend(), "printQuery").mockImplementation(async (query, _view, dialect) => {
      if (dialect !== "tql_macro") return printedArgument(query);
      requested += 1;
      return save.promise;
    });
    const { dispose } = await mountOpenSheet();

    try {
      const outside = document.createElement("button");
      outside.textContent = "Outside query sheet";
      document.body.append(outside);
      const first = rootHandle(0);
      first.focus();
      arrowDown(first);
      await vi.waitFor(() => expect(requested).toBe(1));
      outside.focus();
      expect(document.activeElement).toBe(outside);

      save.resolve(FIRST_MOVE_ARGUMENT);
      await vi.waitFor(() => {
        expect(doc.byId.query.raw).toBe(FIRST_MOVE_RAW);
        expect(rootHandle(1).closest(".qs-row")?.querySelector(".qs-value")?.textContent?.trim()).toBe("alpha");
      });
      expect(document.activeElement).toBe(outside);
    } finally {
      dispose();
    }
  });
});
