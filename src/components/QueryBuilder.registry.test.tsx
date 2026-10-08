// Ported from master src/components/QueryBuilder.registry.test.tsx. og: a registry
// failure is a plain Error (no typed index readiness).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ErrorBoundary, createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { bumpGraphEpoch, setDataRev } from "../graphSession";
import type { Filter, RegistrySnapshot } from "../editor/queryIr";
import { propertyFilter } from "../editor/queryBuilder";
import { clearTransientLayersForTest } from "../transientLayers";
import {
  QueryBuilder,
  resetQueryRegistryRevisionForTests,
  type BuilderSession,
} from "./QueryBuilder";

// **What a registry read that does not succeed looks like in the sheet.**
//
// `query_registry` is SQL-only on both backends, so it answers one of three
// ways: a snapshot, typed readiness while the index is indexing/recovering, or
// a typed terminal failure. Before this file the third answer had no rendering
// at all — it rejected the shared resource, and `Resource.latest` RETHROWS a
// rejected resource, so the throw escaped the reactive graph (an error boundary
// in the app) and the field picker could not be opened at all, while every
// pending-gated control sat on "Reading this graph's properties…" forever.
//
// The contract these tests hold: readiness retries itself and keeps the draft;
// a terminal failure STOPS retrying, says which failure it was, and offers the
// one control that starts another read; neither ever looks like a graph with no
// properties in it; and type-dependent edits stay disabled until a healthy
// registry for the CURRENT graph and declaration revision has landed.

function session(filter: Filter): BuilderSession {
  return { query: { anchor: "block", filter, source: { kind: "builder" } }, view: {} };
}

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

const FAILURE_MESSAGE = "The index could not be rebuilt.";
const unavailable = () => new Error(FAILURE_MESSAGE);

/** Let the shared registry request run through its promise chain. */
async function settle(ticks = 12): Promise<void> {
  for (let tick = 0; tick < ticks; tick += 1) await Promise.resolve();
}

/** Mount one builder inside an error boundary, so "the failure crashed the
 *  sheet" is an ASSERTION rather than a stray unhandled rejection in the log. */
function mountBuilder(filter: Filter = propertyFilter("cost", "3")) {
  const host = document.createElement("div");
  document.body.append(host);
  const [current, setCurrent] = createSignal<BuilderSession>(session(filter));
  const changes: BuilderSession[] = [];
  let caught: unknown = null;
  const dispose = render(
    () => (
      <ErrorBoundary
        fallback={(error) => {
          caught = error;
          return <div class="test-error-boundary" />;
        }}
      >
        <QueryBuilder
          session={current}
          onChange={(next) => {
            changes.push(next);
            setCurrent(next);
          }}
        />
      </ErrorBoundary>
    ),
    host,
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
  return { host, open, changes, dispose, boundary: () => caught };
}

/** Open the add-condition chooser and hand back its popover. */
function openChooser(sheet: HTMLElement): HTMLElement {
  sheet.querySelector<HTMLButtonElement>(".qs-add")!.click();
  const menu = document.querySelector<HTMLElement>(".qs-vocab, .qs-menu");
  if (!menu) throw new Error("the add-condition chooser did not open");
  return menu;
}

function pickKey(key: string): void {
  const option = [...document.querySelectorAll<HTMLElement>(".qs-vocab-option, [role='option']")]
    .find((element) => element.textContent?.includes(key));
  if (!option) throw new Error(`no vocabulary row for ${key}`);
  option.click();
}

function retryButton(): HTMLButtonElement {
  const button = document.querySelector<HTMLButtonElement>(".qs-vocab-retry, .qs-registry-retry");
  if (!button) throw new Error("no registry retry control on screen");
  return button;
}


beforeEach(() => {
  resetQueryRegistryRevisionForTests();
  vi.spyOn(backend(), "queryFacets").mockResolvedValue([]);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(and (property cost 3))");
});

afterEach(() => {
  clearTransientLayersForTest();
  resetSharedQueryResultsForTests();
  resetQueryRegistryRevisionForTests();
  setDataRev(0);
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("QueryBuilder registry failure handling (RET2-UI)", () => {
  it("reports an initial terminal read failure instead of crashing or indexing forever", async () => {
    const registry = vi.spyOn(backend(), "queryRegistry").mockRejectedValue(unavailable());
    const builder = mountBuilder();
    try {
      const sheet = builder.open();
      await settle();

      // FAIL-BEFORE: `latest` rethrew, so the throw reached the boundary.
      expect(builder.boundary()).toBeNull();
      expect(document.querySelector(".test-error-boundary")).toBeNull();

      // FAIL-BEFORE: the picker's own render threw, so it never opened.
      openChooser(sheet);
      const failure = document.querySelector<HTMLElement>(".qs-registry-failure");
      expect(failure).not.toBeNull();
      expect(failure!.textContent).toContain(FAILURE_MESSAGE);
      expect(failure!.getAttribute("role")).toBe("alert");
      // Neither of the two things a failure must never be mistaken for.
      expect(document.querySelector(".qs-registry-pending")).toBeNull();
      expect(retryButton()).not.toBeNull();

      // And nothing is still retrying behind it.
      const attempts = registry.mock.calls.length;
      await settle(40);
      expect(registry.mock.calls.length).toBe(attempts);
    } finally {
      builder.dispose();
    }
  });

  it("keeps type-dependent edits disabled while the registry is unavailable", async () => {
    vi.spyOn(backend(), "queryRegistry").mockRejectedValue(unavailable());
    const builder = mountBuilder();
    try {
      const sheet = builder.open();
      await settle();
      // The operator control is the row's one type-dependent edit, and its
      // fallback with no rows is the untyped `text` family — so it stays shut,
      // and says which failure it is waiting on rather than "reading…".
      const op = sheet.querySelector<HTMLButtonElement>(".qs-row .qs-op")!;
      expect(op.disabled).toBe(true);
      expect(op.title).toContain(FAILURE_MESSAGE);
      expect(op.title).not.toContain("Reading this graph's properties");
      expect(builder.changes).toHaveLength(0);
    } finally {
      builder.dispose();
    }
  });

  it("reports a REFRESH failure after a healthy read without publishing the stale vocabulary", async () => {
    const registry = vi.spyOn(backend(), "queryRegistry")
      .mockResolvedValueOnce(snapshot([["cost", 12]]))
      .mockRejectedValue(unavailable());
    const builder = mountBuilder();
    try {
      const sheet = builder.open();
      await settle();
      openChooser(sheet);
      expect(document.querySelector(".qs-menu")!.textContent).toContain("cost");

      // A save bumps `dataRev`; the re-read for the NEW revision fails.
      setDataRev((revision) => revision + 1);
      await settle();
      expect(registry.mock.calls.length).toBe(2);
      expect(builder.boundary()).toBeNull();
      const failure = document.querySelector<HTMLElement>(".qs-registry-failure");
      expect(failure).not.toBeNull();
      expect(failure!.textContent).toContain(FAILURE_MESSAGE);
      // The previous revision's rows are not this revision's answer.
      expect(sheet.querySelector<HTMLButtonElement>(".qs-row .qs-op")!.disabled).toBe(true);
    } finally {
      builder.dispose();
    }
  });

  // master's two readiness tests ("retries typed readiness by itself…", "stops the
  // automatic retry when readiness turns into a terminal failure") are X-class:
  // og's query_registry walks the files and has no index readiness to wait on.

  it("recovers on explicit retry without reopening the sheet or resetting the draft", async () => {
    const registry = vi.spyOn(backend(), "queryRegistry")
      .mockResolvedValueOnce(snapshot([["cost", 12]]))
      .mockRejectedValueOnce(unavailable())
      .mockResolvedValue(snapshot([["cost", 12]]));
    const builder = mountBuilder();
    try {
      const sheet = builder.open();
      await settle();
      openChooser(sheet);
      pickKey("cost");
      await settle();
      const input = document.querySelector<HTMLInputElement>(".qs-value-editor .qs-input")!;
      input.value = "42";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await settle();

      setDataRev((revision) => revision + 1);
      await settle();
      expect(document.querySelector(".qs-registry-failure")).not.toBeNull();
      // Commits are refused for as long as the types are unavailable.
      document.querySelector<HTMLButtonElement>(".qs-commit")!.click();
      await settle();
      expect(builder.changes).toHaveLength(0);

      retryButton().click();
      await settle();
      expect(registry.mock.calls.length).toBe(3);

      // Same sheet element, same chosen key, same draft — nothing reopened.
      expect(document.querySelector(".qs-sheet")).toBe(sheet);
      expect(document.querySelector(".qs-registry-failure")).toBeNull();
      expect(document.querySelector(".qs-value-editor .qs-menu-title")!.textContent)
        .toContain("cost");
      expect(document.querySelector<HTMLInputElement>(".qs-value-editor .qs-input")!.value)
        .toBe("42");
      const commit = document.querySelector<HTMLButtonElement>(".qs-commit")!;
      expect(commit.disabled).toBe(false);
      commit.click();
      await settle();
      expect(builder.changes).toHaveLength(1);
    } finally {
      builder.dispose();
    }
  });

  it("cannot land a previous graph's rows or its failure", async () => {
    let settleStale: ((snapshot: RegistrySnapshot) => void) | null = null;
    let failStale: ((error: unknown) => void) | null = null;
    const registry = vi.spyOn(backend(), "queryRegistry")
      .mockImplementationOnce(() => new Promise((resolve, reject) => {
        settleStale = resolve;
        failStale = reject;
      }))
      .mockResolvedValue(snapshot([["owner", 3]]));
    const builder = mountBuilder();
    try {
      const sheet = builder.open();
      await settle();
      expect(registry.mock.calls.length).toBe(1);

      // The graph is replaced while the first read is still outstanding.
      bumpGraphEpoch();
      await settle();
      await vi.waitFor(() => expect(registry.mock.calls.length).toBe(2));

      // The old graph answers late — with rows, and then with a failure. The
      // new graph's sheet must see neither.
      settleStale!(snapshot([["stale", 99]]));
      failStale!(unavailable());
      await settle(20);

      openChooser(sheet);
      const list = document.querySelector<HTMLElement>(".qs-menu")!;
      expect(list.textContent).toContain("owner");
      expect(list.textContent).not.toContain("stale");
      expect(document.querySelector(".qs-registry-failure")).toBeNull();
      expect(document.querySelector(".qs-registry-pending")).toBeNull();
      expect(builder.boundary()).toBeNull();
    } finally {
      builder.dispose();
    }
  });

  it("shares one failing read and one retry across two mounted builders", async () => {
    const registry = vi.spyOn(backend(), "queryRegistry")
      .mockRejectedValueOnce(unavailable())
      .mockResolvedValue(snapshot([["cost", 12]]));
    const first = mountBuilder();
    const second = mountBuilder();
    try {
      const sheetA = first.open();
      const sheetB = second.open();
      await settle();
      // ONE request for the scope/key, not one per mounted builder.
      expect(registry.mock.calls.length).toBe(1);
      expect(sheetA.querySelector<HTMLButtonElement>(".qs-row .qs-op")!.disabled).toBe(true);
      expect(sheetB.querySelector<HTMLButtonElement>(".qs-row .qs-op")!.disabled).toBe(true);

      // Retrying in ONE builder is the shared owner's retry: still one request,
      // and both builders recover from it.
      openChooser(sheetA);
      retryButton().click();
      await settle();
      expect(registry.mock.calls.length).toBe(2);
      expect(sheetA.querySelector<HTMLButtonElement>(".qs-row .qs-op")!.disabled).toBe(false);
      expect(sheetB.querySelector<HTMLButtonElement>(".qs-row .qs-op")!.disabled).toBe(false);
      expect(first.boundary()).toBeNull();
      expect(second.boundary()).toBeNull();
    } finally {
      first.dispose();
      second.dispose();
    }
  });

  it("keeps the last registry choices visible during a save-triggered re-read", async () => {
    let finish: ((value: RegistrySnapshot) => void) | undefined;
    vi.spyOn(backend(), "queryRegistry")
      .mockResolvedValueOnce(snapshot([["cost", 12]]))
      .mockImplementationOnce(() => new Promise<RegistrySnapshot>((resolve) => { finish = resolve; }));
    const builder = mountBuilder();
    try {
      const sheet = builder.open();
      openChooser(sheet);
      await settle();
      const choices = () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].map((item) => item.textContent?.trim());
      expect(choices()).toContain("cost");
      setDataRev(1);
      await settle();
      expect(finish).toBeDefined();
      expect(choices()).toContain("cost");
      finish!(snapshot([["cost", 13], ["owner", 2]]));
      await settle();
      expect(choices()).toContain("owner");
    } finally {
      builder.dispose();
    }
  });
});
