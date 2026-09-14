import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { loadSingle, resetStore, setRaw } from "../store";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { MARKERS } from "../markers";
import { parseBody } from "./facets";
import { bumpDataRev } from "../ui";
import * as router from "../router";
import * as ui from "../ui";

// GH #518: an unaliased `((block-reference))` must visibly preserve the
// referenced block's task state, the same way the source block does.

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function setupTaskTarget(id: string, raw: string, refText: string, format: "md" | "org" = "md") {
  const target = {
    id,
    raw,
    marker: (() => { const parsed = parseBody(raw, format)[0]; return parsed && "marker" in parsed ? parsed.marker ?? undefined : undefined; })(),
    collapsed: false,
    children: [],
    properties: [["id", id]] as [string, string][],
  };
  loadSingle({
    kind: "page",
    name: "Reference source",
    title: "Reference source",
    pre_block: null,
    format,
    blocks: [target],
  });
  vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([
    { page: "Reference source", kind: "page" as const, blocks: [target] },
  ]);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <AstBody raw={refText} />, host);
  return { host, dispose };
}

describe("block-reference task state rendering (GH #518)", () => {
  it.each(MARKERS.flatMap(state => (["md", "org"] as const).map(format => ({ state, format }))))(
    "renders the $state marker in a $format target",
    async ({ state, format }) => {
      const id = `51800000-0000-4000-8000-0000000000${String(MARKERS.indexOf(state)).padStart(2, "0")}`;
      const raw = `${state} buy milk`;
      const source = parseBody(raw, format)[0];
      expect(source && "marker" in source ? source.marker : null).toBe(state);
      const { host, dispose } = setupTaskTarget(id, raw, `Get ((${id}))`, format);
      try {
        await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe(`${state} buy milk`));
        const chip = host.querySelector(".block-ref .block-marker");
        expect(chip?.textContent?.trim()).toBe(state);
        expect(chip?.classList.contains(`marker-${state.toLowerCase()}`)).toBe(true);
      } finally {
        dispose();
      }
    },
  );

  it.each(["md", "org"] as const)("retains literal task words and inline formatting in %s targets", async format => {
    const id = "51800000-0000-4000-8000-0000000000b0";
    const emphasis = format === "org" ? "/milk/" : "*milk*";
    const { host, dispose } = setupTaskTarget(id, `TODO TODO buy ${emphasis}`, `((${id}))`, format);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe("TODO TODO buy milk"));
      expect(host.querySelector(".block-ref em")?.textContent?.trim()).toBe("milk");
    } finally {
      dispose();
    }
  });

  it.each([
    ["\n\nTODO buy milk", "TODO", "TODO buy milk"],
    ["\tWAIT buy milk", "WAIT", "WAIT buy milk"],
    ["TODO", "TODO", "TODO"],
    ["TODO \nmore detail", "TODO", "TODO more detail"],
    ["TODO TODO buy milk", "TODO", "TODO TODO buy milk"],
    ["TODO\nmore detail", null, "TODO"],
    ["TODO\tbuy milk", null, "TODO\tbuy milk"],
    ["TODO: buy milk", null, "TODO: buy milk"],
    ["todo buy milk", null, "todo buy milk"],
  ])("matches the source marker for %j", async (raw, marker, text) => {
    const id = "51800000-0000-4000-8000-0000000000ad";
    // The real parser supplies the source block's task state.
    const source = parseBody(raw!, "md")[0];
    expect(source && "marker" in source ? source.marker ?? null : null).toBe(marker);
    const { host, dispose } = setupTaskTarget(id, raw!, `((${id}))`);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe(text));
      expect(host.querySelector(".block-ref .block-marker")?.textContent ?? null).toBe(marker);
    } finally {
      dispose();
    }
  });

  it("refreshes unloaded task references after external state/content changes and deletion", async () => {
    const id = "51800000-0000-4000-8000-0000000000ae";
    const group = (raw: string) => ({
      page: "Unloaded source", kind: "page" as const,
      blocks: [{ id, raw, collapsed: false, children: [] }],
    });
    const resolve = vi.spyOn(backend(), "resolveBlocks")
      .mockResolvedValueOnce([group("WAIT buy milk")])
      .mockResolvedValueOnce([group("CANCELLED buy oat milk")])
      .mockResolvedValueOnce([null]);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AstBody raw={`((${id})) and ((${id}))`} />, host);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe("WAIT buy milk"));
      bumpDataRev();
      await vi.waitFor(() => expect([...host.querySelectorAll(".block-ref")].map(el => el.textContent?.trim()))
        .toEqual(["CANCELLED buy oat milk", "CANCELLED buy oat milk"]));
      expect(resolve).toHaveBeenCalledTimes(2);
      bumpDataRev();
      await vi.waitFor(() => expect(host.querySelectorAll(".block-ref-missing")).toHaveLength(2));
      expect(host.querySelectorAll(".block-marker")).toHaveLength(0);
    } finally {
      dispose();
    }
  });

  it("clicking the task chip navigates to the source; shift-click opens its sidebar", async () => {
    const id = "51800000-0000-4000-8000-0000000000af";
    const open = vi.spyOn(router, "openPageAtBlock").mockImplementation(() => {});
    const sidebar = vi.spyOn(ui, "openBlockInSidebar").mockImplementation(() => {});
    const { host, dispose } = setupTaskTarget(id, "TODO buy milk", `((${id}))`);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref-missing")).toBeNull());
      const chip = host.querySelector(".block-ref .block-marker")!;
      chip.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      expect(open).toHaveBeenCalledWith(expect.objectContaining({ name: "Reference source", block: id }));
      chip.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
      expect(sidebar).toHaveBeenCalledWith(expect.objectContaining({ page: "Reference source", uuid: id }));
      expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe("TODO buy milk");
    } finally {
      dispose();
    }
  });

  it("follows a task-state change on the source block", async () => {
    const id = "51800000-0000-4000-8000-0000000000aa";
    const { host, dispose } = setupTaskTarget(id, `TODO buy milk\nid:: ${id}`, `Get ((${id}))`);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref .block-marker")?.textContent?.trim()).toBe("TODO"));
      setRaw(id, `DONE buy milk\nid:: ${id}`);
      await vi.waitFor(() => expect(host.querySelector(".block-ref .block-marker")?.textContent?.trim()).toBe("DONE"));
      expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe("DONE buy milk");
      // Dropping the task state entirely removes the chip.
      setRaw(id, `buy milk\nid:: ${id}`);
      await vi.waitFor(() => expect(host.querySelector(".block-ref .block-marker")).toBeNull());
      expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe("buy milk");
    } finally {
      dispose();
    }
  });

  it("keeps an explicit alias label instead of the marker", async () => {
    const id = "51800000-0000-4000-8000-0000000000ab";
    const { host, dispose } = setupTaskTarget(id, `TODO buy milk\nid:: ${id}`, `[the groceries](((${id})))`);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe("the groceries"));
      expect(host.querySelector(".block-ref .block-marker")).toBeNull();
      setRaw(id, `DONE different groceries\nid:: ${id}`);
      expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe("the groceries");
      expect(host.querySelector(".block-ref .block-marker")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("renders no chip for an ordinary non-task reference", async () => {
    const id = "51800000-0000-4000-8000-0000000000ac";
    const { host, dispose } = setupTaskTarget(id, `just a note\nid:: ${id}`, `See ((${id}))`);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent?.trim()).toBe("just a note"));
      expect(host.querySelector(".block-ref .block-marker")).toBeNull();
    } finally {
      dispose();
    }
  });
});
