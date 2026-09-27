import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { pageByName, resetStore } from "./document";
import { loadSingle } from "./document/workingSet";
import { installFileDrop } from "./filedrop";

const drag = vi.hoisted(() => ({ listener: null as null | ((event: any) => Promise<void>) }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (listener: (event: any) => Promise<void>) => {
      drag.listener = listener;
      return () => { drag.listener = null; };
    },
  }),
}));

beforeAll(() => initParser());
afterEach(() => { vi.restoreAllMocks(); resetStore(); });

it("binds the second file of a drop to the generation captured at drop time", async () => {
  loadSingle({ name: "Drop", title: "Drop", kind: "page", pre_block: null,
    blocks: [{ id: "drop-host", raw: "host", collapsed: false, children: [] }] });
  expect(pageByName("Drop")?.roots).toContain("drop-host");
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => null });
  let generation = 1;
  vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
  let finishFirst!: (name: string) => void;
  const first = new Promise<string>((resolve) => { finishFirst = resolve; });
  const writes: number[] = [];
  vi.spyOn(backend(), "importAsset").mockImplementation(async (_path, _name, requested) => {
    const target = requested ?? generation; // old TauriBackend leased current graph
    writes.push(target);
    if (writes.length === 1) return first;
    if (target !== generation) throw new Error("stale-graph-binding");
    return "second.png";
  });
  const uninstall = await installFileDrop();
  try {
    const dropped = drag.listener!({ payload: { type: "drop", paths: ["/tmp/first.png", "/tmp/second.png"], position: { x: 1, y: 1 } } });
    expect(writes).toEqual([1]);
    generation = 2;
    finishFirst("first.png");
    await dropped;
    expect(writes).toEqual([1, 1]);
  } finally { uninstall(); }
});
