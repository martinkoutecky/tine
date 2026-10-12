import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { isDirty, node as docNode, pageByName, resetStore, setRaw, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { RefBlocks } from "../components/RefBlocks";
import * as router from "../router";
import type { BlockDto, PageRead } from "../types";
import { taskCheckboxState } from "../markers";

// GH #518 follow-up (Brikas, 2026-10-05): the task state renders inside a
// block reference, but its checkbox could not be ticked there. OG renders the
// referenced block's own `block-checkbox` inside the reference and checks /
// unchecks THAT block (og-1.0.0 frontend/components/block.cljs
// block-reference -> build-block-title -> block-checkbox).

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

const ID = "51800000-0000-4000-8000-0000000001a0";

function sourceBlock(raw: string): BlockDto {
  return { id: ID, raw: `${raw}\nid:: ${ID}`, collapsed: false, children: [], properties: [["id", ID]], marker: raw.split(" ", 1)[0] };
}

function sourcePage(raw: string): PageRead {
  return { id: "pages/Task source.md", kind: "page", name: "Task source", title: "Task source", pre_block: null, format: "md", blocks: [sourceBlock(raw)] } as PageRead;
}

function mount(view: () => any) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(view, host);
  return { host, dispose };
}

function press(el: Element) {
  el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
  el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
}

const markerOf = (raw: string | undefined) => raw?.split(" ", 1)[0];

describe("ticking a task inside a block reference (GH #518)", () => {
  it("toggles the loaded source block through its own edit path, without navigating", async () => {
    loadSingle(sourcePage("TODO buy milk"));
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page: "Task source", kind: "page", blocks: [sourceBlock("TODO buy milk")] }]);
    const open = vi.spyOn(router, "openPageAtBlock").mockImplementation(() => {});
    const { host, dispose } = mount(() => <AstBody raw={`Get ((${ID}))`} />);
    try {
      const box = await vi.waitFor(() => {
        const el = host.querySelector(".block-ref .block-task-checkbox");
        expect(el).not.toBeNull();
        return el!;
      });
      expect(box.getAttribute("aria-checked")).toBe("false");
      press(box);
      await vi.waitFor(() => expect(markerOf(docNode(ID).raw)).toBe("DONE"));
      expect(isDirty("Task source")).toBe(true);
      expect(open).not.toHaveBeenCalled();
      await vi.waitFor(() => expect(host.querySelector(".block-ref .block-task-checkbox")?.getAttribute("aria-checked")).toBe("true"));
      // Same undo as the source page's own checkbox.
      undo();
      expect(markerOf(docNode(ID).raw)).toBe("TODO");
      // And back: a checked box reopens the task.
      press(host.querySelector(".block-ref .block-task-checkbox")!);
      await vi.waitFor(() => expect(markerOf(docNode(ID).raw)).toBe("DONE"));
      // Unchecking reopens it with the workflow's open marker (OG uncheck).
      press(host.querySelector(".block-ref .block-task-checkbox")!);
      await vi.waitFor(() => expect(taskCheckboxState(markerOf(docNode(ID).raw))).toBe(false));
    } finally {
      dispose();
    }
  });

  it("loads an unloaded source page first, then toggles the real block", async () => {
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page: "Task source", kind: "page", blocks: [sourceBlock("LATER call Bob")] }]);
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(sourcePage("LATER call Bob"));
    const { host, dispose } = mount(() => <AstBody raw={`((${ID}))`} />);
    try {
      const box = await vi.waitFor(() => {
        const el = host.querySelector(".block-ref .block-task-checkbox");
        expect(el).not.toBeNull();
        return el!;
      });
      expect(pageByName("Task source")).toBeUndefined();
      press(box);
      await vi.waitFor(() => expect(markerOf(docNode(ID)?.raw)).toBe("DONE"));
      expect(getPage).toHaveBeenCalledWith("Task source", "page");
      expect(isDirty("Task source")).toBe(true);
    } finally {
      dispose();
    }
  });

  it("writes nothing when the source changed since the reference was drawn", async () => {
    loadSingle(sourcePage("TODO buy milk"));
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page: "Task source", kind: "page", blocks: [sourceBlock("TODO buy milk")] }]);
    // A reference row drawn from an older snapshot still shows the task open.
    const { host, dispose } = mount(() => <RefBlocks blocks={[sourceBlock("TODO buy milk")]} page="Task source" pageKind="page" />);
    try {
      setRaw(ID, `DONE buy milk\nid:: ${ID}`);
      const before = docNode(ID).raw;
      press(host.querySelector(".block-task-checkbox")!);
      await new Promise((r) => setTimeout(r, 20));
      expect(docNode(ID).raw).toBe(before);
    } finally {
      dispose();
    }
  });

  it("toggles from a reference row (unhydrated linked reference / peek)", async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(sourcePage("TODO buy milk"));
    const { host, dispose } = mount(() => <RefBlocks blocks={[sourceBlock("TODO buy milk")]} page="Task source" pageKind="page" />);
    try {
      const box = host.querySelector(".block-task-checkbox")!;
      expect(box.getAttribute("aria-checked")).toBe("false");
      press(box);
      await vi.waitFor(() => expect(markerOf(docNode(ID)?.raw)).toBe("DONE"));
      expect(isDirty("Task source")).toBe(true);
    } finally {
      dispose();
    }
  });

  it("an aliased reference shows its label only, with no checkbox", async () => {
    loadSingle(sourcePage("TODO buy milk"));
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page: "Task source", kind: "page", blocks: [sourceBlock("TODO buy milk")] }]);
    const { host, dispose } = mount(() => <AstBody raw={`[groceries](((${ID})))`} />);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")?.textContent).toBe("groceries"));
      expect(host.querySelector(".block-task-checkbox")).toBeNull();
    } finally {
      dispose();
    }
  });
});
