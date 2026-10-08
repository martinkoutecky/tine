import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./components/Block";
import { doc } from "./document/model";
import { startEditing } from "./editorController";
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
afterEach(() => { vi.restoreAllMocks(); resetStore(); document.body.innerHTML = ""; });

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

it.each(["main", "pane:split", "sidebar:item"])("%s drops land beside the target without replacing its live edit", async surface => {
  loadSingle({ name: "Drop", title: "Drop", kind: "page", pre_block: null,
    blocks: [{ id: "drop-host", raw: "host", collapsed: false, children: [] }] });
  startEditing("drop-host", 4, null, surface);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <SurfaceContext.Provider value={surface}><Block id="drop-host" /></SurfaceContext.Provider>, root);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => root.querySelector("[data-block-id]") });
  let finish!: (name: string) => void;
  vi.spyOn(backend(), "importAsset").mockReturnValue(new Promise(r => { finish = r; }));
  const uninstall = await installFileDrop();
  try {
    const dropped = drag.listener!({ payload: { type: "drop", paths: ["/tmp/photo.jpg"], position: { x: 1, y: 1 } } });
    const ta = root.querySelector("textarea")!; ta.value = "host edited";
    ta.dispatchEvent(new Event("input", { bubbles: true }));
    expect(pageByName("Drop")!.roots).toHaveLength(1); // data precedes reference
    finish("saved.jpg"); await dropped;
    expect(doc.byId["drop-host"].raw).toBe("host edited");
    expect(pageByName("Drop")!.roots.map(id => doc.byId[id].raw)).toEqual(["host edited", "![](../assets/saved.jpg)"]);
    expect(ta.value).toBe("host edited");
  } finally { uninstall(); dispose(); }
});
