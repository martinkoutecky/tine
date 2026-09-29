import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { installQuickCaptureReceiver } from "./App";
import { backend } from "./backend";
import { loadSingle } from "./document/workingSet";
import { resetStore, pageByName } from "./document";

import { initParser } from "./render/parse";
beforeAll(() => initParser());

const h = vi.hoisted(() => ({ receive: null as null | ((event: { payload: unknown }) => Promise<void>), ack: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_: string, receive: typeof h.receive) => { h.receive = receive; return () => { h.receive = null; }; },
  emitTo: h.ack,
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: "main" }) }));

afterEach(() => { resetStore(); vi.restoreAllMocks(); h.ack.mockClear(); });

it.each(["Destination", ""])("refuses graph A's delayed capture in graph B (title %s)", async (title) => {
  let generation = 11;
  vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
  const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["saved"] });
  const get = vi.spyOn(backend(), "getPage").mockImplementation(async (name, kind) => ({ name, kind, title: name, id: `pages/${name}.md`, rev: "before", pre_block: null, blocks: [] }));
  const dispose = await installQuickCaptureReceiver();
  resetStore(); generation = 12;
  loadSingle({ name: "B", kind: "page", title: "B", id: "pages/B.md", pre_block: null, blocks: [] });
  await h.receive!({ payload: { id: "delayed", target: "main", bindingGeneration: 11, title, text: "- belongs to A" } });
  expect(save).not.toHaveBeenCalled();
  expect(get).not.toHaveBeenCalled();
  expect(pageByName("Destination")).toBeUndefined();
  expect(h.ack).toHaveBeenCalledWith("capture", "quick-capture-ack", { id: "delayed", ok: false });
  dispose();
});

it("saves one bound capture and acknowledges its retry without writing again", async () => {
  let generation = 11;
  vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
  const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["saved"] });
  vi.spyOn(backend(), "getPage").mockResolvedValue({ name: "Destination", kind: "page", title: "Destination", id: "pages/Destination.md", rev: "before", pre_block: null, blocks: [] });
  loadSingle({ name: "A", kind: "page", title: "A", id: "pages/A.md", pre_block: null, blocks: [] });
  const dispose = await installQuickCaptureReceiver();
  const payload = { id: "current", target: "main", bindingGeneration: 11, title: "Destination", text: "- belongs to A" };
  await h.receive!({ payload });
  expect(save).toHaveBeenCalledOnce();
  expect(save.mock.calls[0][0][0].page.blocks[0].raw).toBe("belongs to A");
  expect(save.mock.calls[0][1]).toBe(11);
  resetStore(); generation = 12;
  await h.receive!({ payload });
  expect(save).toHaveBeenCalledOnce();
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: "current", ok: true });
  dispose();
});
