import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { installQuickCaptureReceiver } from "./App";
import { backend } from "./backend";
import { loadSingle } from "./document/workingSet";
import { resetStore, pageByName } from "./document";
import { doc } from "./document/model";
import { answerOpensFromDocument } from "./document/host/documentHost.test.support";
import { journalTitle, appNow } from "./journal";
import { bindTestHost, submittedPages } from "./document/host/wiring.test.support";
import { mainPaneRouter, route } from "./router";

import { initParser } from "./render/parse";
beforeAll(() => initParser());

const h = vi.hoisted(() => ({ receive: null as null | ((event: { payload: unknown }) => Promise<void>), ack: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_: string, receive: typeof h.receive) => { h.receive = receive; return () => { h.receive = null; }; },
  emitTo: h.ack,
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: "main" }) }));

afterEach(() => { resetStore(); vi.restoreAllMocks(); h.ack.mockClear(); });

it.each(["Destination", ""])("L9-08: retries a landed capture without inserting twice (title %s)", async (title) => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(11);
  const name = title || journalTitle(appNow());
  const page = { name, kind: title ? "page" as const : "journal" as const, title: name, id: title ? "pages/Destination.md" : "journals/today.md", rev: "before", pre_block: null, blocks: [] };
  loadSingle(page);
  answerOpensFromDocument(await bindTestHost());
  vi.spyOn(backend(), "getPage").mockResolvedValue(page);
  const wait = vi.spyOn(backend(), "pageWait").mockResolvedValue(false);
  const dispose = await installQuickCaptureReceiver();
  const payload = { id: `retry-landed-${title}`, target: "main", bindingGeneration: 11, title, text: "- captured thought" };
  await h.receive!({ payload });
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: payload.id, ok: false, landed: true });
  wait.mockResolvedValue(true);
  await h.receive!({ payload });
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: payload.id, ok: true });
  expect(pageByName(name)!.roots.map(id => doc.byId[id].raw).filter(raw => raw === "captured thought")).toHaveLength(1);
  dispose();
});

it("F1: a reinstalled receiver does not insert a pending capture a second time", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(11);
  const name = "Reload destination";
  const page = { name, kind: "page" as const, title: name, id: "pages/Reload.md", rev: "before", pre_block: null, blocks: [] };
  loadSingle(page);
  answerOpensFromDocument(await bindTestHost());
  vi.spyOn(backend(), "getPage").mockResolvedValue(page);
  vi.spyOn(backend(), "pageWait").mockResolvedValue(false);
  const dispose = await installQuickCaptureReceiver();
  const payload = { id: "reload-pending", target: "main", bindingGeneration: 11, title: name, text: "- once" };
  await h.receive!({ payload });
  dispose();
  const disposeReload = await installQuickCaptureReceiver();
  await h.receive!({ payload });
  expect(pageByName(name)!.roots.map(id => doc.byId[id].raw).filter(raw => raw === "once")).toHaveLength(1);
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: payload.id, ok: false, landed: true, closedGraph: true });
  disposeReload();
});

it.each(["Destination", ""])("refuses graph A's delayed capture in graph B (title %s)", async (title) => {
  let generation = 11;
  vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
  const save = vi.spyOn(backend(), "pageSubmit");
  const get = vi.spyOn(backend(), "getPage").mockImplementation(async (name, kind) => ({ name, kind, title: name, id: `pages/${name}.md`, rev: "before", pre_block: null, blocks: [] }));
  const dispose = await installQuickCaptureReceiver();
  resetStore(); generation = 12;
  loadSingle({ name: "B", kind: "page", title: "B", id: "pages/B.md", pre_block: null, blocks: [] });
  await bindTestHost();
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
  const save = vi.spyOn(backend(), "pageSubmit");
  vi.spyOn(backend(), "getPage").mockResolvedValue({ name: "Destination", kind: "page", title: "Destination", id: "pages/Destination.md", rev: "before", pre_block: null, blocks: [] });
  loadSingle({ name: "A", kind: "page", title: "A", id: "pages/A.md", pre_block: null, blocks: [] });
  // Graph A's binding: its page-host session is the write's binding.
  const host = await bindTestHost();
  mainPaneRouter.openPage("A");
  const editingRoute = route();
  const dispose = await installQuickCaptureReceiver();
  const payload = { id: "current", target: "main", bindingGeneration: 11, title: "Destination", text: "- belongs to A" };
  await h.receive!({ payload });
  expect(save).toHaveBeenCalledOnce();
  expect(submittedPages(save)[0].blocks[0].raw).toBe("belongs to A");
  expect(save.mock.calls[0][0]).toBe(host.session);
  expect(route()).toEqual(editingRoute);
  resetStore(); generation = 12;
  await h.receive!({ payload });
  expect(save).toHaveBeenCalledOnce();
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: "current", ok: true });
  dispose();
});

it("FU2: an old-generation request with a native pending claim remains uncertain", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(12);
  await backend().captureReceipt("old-pending", "claim", 11);
  const save = vi.spyOn(backend(), "pageSubmit");
  const dispose = await installQuickCaptureReceiver();
  await h.receive!({ payload: { id: "old-pending", target: "main", bindingGeneration: 11, title: "A", text: "- once" } });
  expect(save).not.toHaveBeenCalled();
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", {
    id: "old-pending", ok: false, landed: true, closedGraph: true,
  });
  dispose();
});

it("FU2: a claim rejected during rebind without insertion has no previous-graph warning", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(11);
  const original = backend().captureReceipt.bind(backend());
  vi.spyOn(backend(), "captureReceipt").mockImplementation(async (id, action, generation) => {
    if (action === "claim") { resetStore(); throw new Error("stale binding"); }
    return original(id, action, generation);
  });
  const dispose = await installQuickCaptureReceiver();
  await h.receive!({ payload: { id: "never-claimed", target: "main", bindingGeneration: 11, text: "- unsent" } });
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: "never-claimed", ok: false });
  dispose();
});

it("FU2: an unavailable receipt lookup keeps identity for plain retry without alleging insertion", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(12);
  vi.spyOn(backend(), "captureReceipt").mockRejectedValue(new Error("transport unavailable"));
  const dispose = await installQuickCaptureReceiver();
  await h.receive!({ payload: { id: "unknown-claim", target: "main", bindingGeneration: 11, text: "- unconfirmed" } });
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: "unknown-claim", ok: false, landed: true });
  dispose();
});

it.each(["claim", "destination"])("A2: owner retiring during %s before insertion forgets the new claim without a previous-graph warning", async (during) => {
  let generation = 11;
  let statusAvailable = false;
  vi.spyOn(backend(), "graphBindingGeneration").mockImplementation(() => generation);
  const original = backend().captureReceipt.bind(backend());
  const receipts = vi.spyOn(backend(), "captureReceipt").mockImplementation(async (id, action, binding) => {
    if (action === "status" && !statusAvailable) throw new Error("old status lease is stale");
    const result = await original(id, action, binding);
    if (during === "claim" && action === "claim") { resetStore(); generation = 12; }
    return result;
  });
  const insert = vi.spyOn(backend(), "getPage").mockImplementation(async name => {
    resetStore(); generation = 12;
    return { name, kind: "page", title: name, id: `pages/${name}.md`, rev: "before", pre_block: null, blocks: [] };
  });
  const dispose = await installQuickCaptureReceiver();
  const payload = { id: `claimed-never-inserted-${during}`, target: "main", bindingGeneration: 11, title: "Destination", text: "- unsent" };
  await h.receive!({ payload });
  expect(insert).toHaveBeenCalledTimes(during === "claim" ? 0 : 1);
  expect(receipts).toHaveBeenCalledWith(payload.id, "forget", 11);
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: payload.id, ok: false });
  statusAvailable = true;
  await h.receive!({ payload });
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: payload.id, ok: false });
  dispose();
});

it("preserves the existing one-shot capture path for an idless delivery", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(11);
  const page = { name: "One shot", kind: "page" as const, title: "One shot", id: "pages/One shot.md", rev: "before", pre_block: null, blocks: [] };
  loadSingle(page);
  answerOpensFromDocument(await bindTestHost());
  vi.spyOn(backend(), "getPage").mockResolvedValue(page);
  const dispose = await installQuickCaptureReceiver();
  await h.receive!({ payload: { target: "main", bindingGeneration: 11, title: page.name, text: "- one shot" } });
  expect(pageByName(page.name)!.roots.map(id => doc.byId[id].raw)).toContain("one shot");
  expect(h.ack).not.toHaveBeenCalled();
  dispose();
});
