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
import { setGraphMeta } from "./graphSession";
import type { GraphMeta } from "./types";
import { toasts } from "./toasts";

import { initParser } from "./render/parse";
beforeAll(() => initParser());

const h = vi.hoisted(() => ({ receive: null as null | ((event: { payload: unknown }) => Promise<void>), ack: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_: string, receive: typeof h.receive) => { h.receive = receive; return () => { h.receive = null; }; },
  emitTo: h.ack,
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: "main" }) }));

afterEach(() => { setGraphMeta(null); resetStore(); vi.restoreAllMocks(); h.ack.mockClear(); });

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
  wait.mockResolvedValue([]);
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

/** GH #832 / #833: OG's `:quick-capture-options` and `:quick-capture-templates`. */
async function receiveConfigured(meta: Partial<GraphMeta>, payload: { title?: string; text: string }, pages: string[]) {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(11);
  setGraphMeta({ preferred_format: "md", ...meta } as GraphMeta);
  for (const name of pages) {
    loadSingle({ name, kind: "page", title: name, id: `pages/${name}.md`, rev: "before", pre_block: null, blocks: [] });
  }
  answerOpensFromDocument(await bindTestHost());
  vi.spyOn(backend(), "getPage").mockImplementation(async (name, kind) => ({ name, kind, title: name, id: `pages/${name}.md`, rev: "before", pre_block: null, blocks: [] }));
  vi.spyOn(backend(), "pageWait").mockResolvedValue([]);
  const dispose = await installQuickCaptureReceiver();
  await h.receive!({ payload: { id: `cfg-${Math.random()}`, target: "main", bindingGeneration: 11, ...payload } });
  dispose();
}
const rootsOf = (name: string) => pageByName(name)!.roots.map(id => doc.byId[id].raw);

it("GH #832: an untitled capture goes to the configured default page", async () => {
  await receiveConfigured({ quick_capture_default_page: "Inbox" }, { text: "- idea" }, ["Inbox"]);
  expect(rootsOf("Inbox")).toEqual(["idea"]);
  expect(pageByName(journalTitle(appNow()))).toBeUndefined();
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", expect.objectContaining({ ok: true }));
});

it("GH #832: a typed title beats the default page; insert-today? beats it for an untitled capture", async () => {
  await receiveConfigured({ quick_capture_default_page: "Inbox" }, { title: "Typed", text: "- one" }, ["Inbox", "Typed"]);
  expect(rootsOf("Typed")).toEqual(["one"]);
  expect(rootsOf("Inbox")).toEqual([]);
  resetStore();
  const today = journalTitle(appNow());
  vi.restoreAllMocks();
  await receiveConfigured({ quick_capture_default_page: "Inbox", quick_capture_insert_today: true }, { text: "- two" }, ["Inbox"]);
  expect(rootsOf(today)).toEqual(["two"]);
  expect(rootsOf("Inbox")).toEqual([]);
});

it("GH #833: the configured text template stamps the captured entry", async () => {
  await receiveConfigured({ quick_capture_default_page: "Inbox", quick_capture_template_text: "[{time}] {text}" }, { text: "- Follow up with client\n- second" }, ["Inbox"]);
  const [first, second] = rootsOf("Inbox");
  expect(first).toMatch(/^\[\d{2}:\d{2}\] Follow up with client$/);
  expect(second).toBe("second");
});

it("with no options a capture lands in today's journal, untemplated, and does not navigate", async () => {
  const today = journalTitle(appNow());
  mainPaneRouter.openPage("Elsewhere");
  const before = route();
  await receiveConfigured({}, { text: "- plain" }, []);
  expect(rootsOf(today)).toEqual(["plain"]);
  expect(route()).toEqual(before);
});

it("a capture saves when the system language is not a language tag (Linux LANG=C)", async () => {
  // WebKitGTK under LANG=C reports navigator.language "C", which Intl rejects.
  vi.spyOn(navigator, "language", "get").mockReturnValue("C");
  const today = journalTitle(appNow());
  await receiveConfigured({}, { text: "- under C" }, []);
  expect(rootsOf(today)).toEqual(["under C"]);
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", expect.objectContaining({ ok: true }));
});

it("GH #832: redirect-page? opens the page the capture landed on", async () => {
  mainPaneRouter.openPage("Elsewhere");
  await receiveConfigured({ quick_capture_default_page: "Inbox", quick_capture_redirect_page: true }, { text: "- go" }, ["Inbox"]);
  expect(route()).toMatchObject({ kind: "page", name: "Inbox" });
});

// Salvaged from batch/cm-yearless-day-key af71de05b (review 43900f5ea R2-1,
// R2-2), without its year-less day handle: a capture written into a day
// before midnight redirects to the day it was written into, whenever its
// flush, acknowledgement or retry completes, never to a today recomputed at
// another moment; an idless delivery that landed is a success.
const captured = () => toasts().map((toast) => toast.message).filter((message) => message.startsWith("Captured"));
function beforeMidnight() {
  let now = new Date(2026, 10, 2, 23, 59).getTime();
  vi.spyOn(Date, "now").mockImplementation(() => now);
  const day = journalTitle(appNow());
  return { day, midnight: () => { now = new Date(2026, 10, 3, 0, 1).getTime(); } };
}
async function bindRedirecting(flushed: () => false | [] = () => []) {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(11);
  setGraphMeta({ preferred_format: "md", quick_capture_redirect_page: true } as GraphMeta);
  answerOpensFromDocument(await bindTestHost());
  vi.spyOn(backend(), "getPage").mockImplementation(async (name, kind) => ({ name, kind, title: name, id: `journals/${name}.md`, rev: "before", pre_block: null, blocks: [] }));
  vi.spyOn(backend(), "pageWait").mockImplementation(async () => flushed());
  mainPaneRouter.openPage("Elsewhere");
  return installQuickCaptureReceiver();
}

it.each(["acknowledgement", "flush", "idless flush"])("a capture whose %s completes after midnight redirects to the day it was written into", async (when) => {
  const clock = beforeMidnight();
  const receipts = backend().captureReceipt.bind(backend());
  vi.spyOn(backend(), "captureReceipt").mockImplementation(async (id, action, generation) => {
    if (action === "saved" && when === "acknowledgement") clock.midnight();
    return receipts(id, action, generation);
  });
  const dispose = await bindRedirecting(() => { if (when !== "acknowledgement") clock.midnight(); return []; });
  const id = when === "idless flush" ? {} : { id: `midnight-${when}` };
  await h.receive!({ payload: { ...id, target: "main", bindingGeneration: 11, text: "- late" } });
  dispose();
  expect(clock.day).toBe("Nov 2nd, 2026");
  expect(rootsOf(clock.day)).toEqual(["late"]);
  expect(route()).toMatchObject({ kind: "page", name: clock.day, pageKind: "journal" });
  // Review HOLD 4 F4: it names that day; "today's journal" is now another.
  expect(captured().at(-1)).toBe("Captured to “Nov 2nd, 2026”");
});

it("a capture retried the next day redirects to the day it was written into", async () => {
  const clock = beforeMidnight();
  let flushed = false;
  const dispose = await bindRedirecting(() => flushed && []);
  const payload = { id: "retry-next-day", target: "main", bindingGeneration: 11, text: "- late" };
  await h.receive!({ payload });
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: payload.id, ok: false, landed: true });
  clock.midnight(); flushed = true;
  await h.receive!({ payload });
  dispose();
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: payload.id, ok: true });
  expect(rootsOf(clock.day)).toEqual(["late"]);
  expect(route()).toMatchObject({ kind: "page", name: clock.day, pageKind: "journal" });
  expect(captured().at(-1)).toBe("Captured to “Nov 2nd, 2026”");
});

// Review HOLD 7 F7: a retry flushes what an earlier delivery inserted. Its
// redirect and success text name the page that insertion wrote into, with
// that page's kind, never the destination the retry's settings now give.
it.each([
  { from: "Inbox", to: undefined },
  { from: undefined, to: "Inbox" },
])("a retry after the default destination moves from $from to $to names the page it was written into", async ({ from, to }) => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(11);
  const settings = (page: string | undefined) => ({ preferred_format: "md", quick_capture_redirect_page: true, quick_capture_default_page: page } as GraphMeta);
  setGraphMeta(settings(from));
  answerOpensFromDocument(await bindTestHost());
  vi.spyOn(backend(), "getPage").mockImplementation(async (name, kind) => ({ name, kind, title: name, id: kind === "journal" ? `journals/${name}.md` : `pages/${name}.md`, rev: "before", pre_block: null, blocks: [] }));
  let flushed = false;
  vi.spyOn(backend(), "pageWait").mockImplementation(async () => flushed && []);
  mainPaneRouter.openPage("Elsewhere");
  const dispose = await installQuickCaptureReceiver();
  const payload = { id: `retry-moved-${from ?? "journal"}`, target: "main", bindingGeneration: 11, text: "- moved" };
  await h.receive!({ payload });
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: payload.id, ok: false, landed: true });
  setGraphMeta(settings(to));
  flushed = true;
  const before = toasts().length;
  await h.receive!({ payload });
  dispose();
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id: payload.id, ok: true });
  const day = journalTitle(appNow());
  const written = from ?? day;
  expect(rootsOf(written)).toEqual(["moved"]);
  expect(route()).toMatchObject({ kind: "page", name: written, pageKind: from ? "page" : "journal" });
  expect(toasts().slice(before).map((toast) => toast.message)).toEqual([from ? `Captured to “${from}”` : "Captured to today's journal"]);
});
