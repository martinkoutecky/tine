// A retried Quick Capture acknowledges true only when a save of Tine's own
// wrote a version of the page that held the captured blocks. Each test asserts
// the acknowledgement together with what the file holds.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { installQuickCaptureReceiver } from "./App";
import { backend } from "./backend";
import { installDiskHost } from "./diskHost.test.support";
import { deleteBlock, isConflicted, moveBlock, pageByName, pinPageWhileDrafting, resetStore, resolveConflict, setRaw } from "./document";
import { HostClient } from "./document/host/client";
import { bindHost, transferInProgress } from "./document/host/wiring";
import { bindTestHost } from "./document/host/wiring.test.support";
import { doc } from "./document/model";
import { loadSingle } from "./document/workingSet";
import { initParser } from "./render/parse";
import type { BlockDto, PageRead } from "./types";

type Listener = (event: { payload: unknown }) => Promise<void>;
const h = vi.hoisted(() => ({ receive: null as null | Listener, abandon: null as null | Listener, ack: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, receive: Listener) => {
    if (name === "quick-capture") h.receive = receive; else h.abandon = receive;
    return () => { if (name === "quick-capture") h.receive = null; else h.abandon = null; };
  },
  emitTo: h.ack,
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: "main" }) }));
beforeAll(() => initParser());
afterEach(() => { resetStore(); vi.restoreAllMocks(); h.ack.mockClear(); });

const UNSAVED = { ok: false, landed: true };
const UNPROVED = { ok: false, landed: true, unproved: true };
const acked = (id: string, ack: object) =>
  expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", { id, ...ack });
const block = (id: string, raw = id): BlockDto => ({ id, raw, collapsed: false, children: [] });
const file = (name: string, blocks: BlockDto[] = []): PageRead =>
  ({ id: `pages/${name}.md`, name, kind: "page", title: name, pre_block: null, rev: "r0", blocks });
const raws = (page: PageRead | undefined): string[] =>
  (page?.blocks ?? []).flatMap(function all(b: BlockDto): string[] { return [b.raw, ...b.children.flatMap(all)]; });

/** Graph files behind a disk host; `writable` says which paths a write reaches. */
async function graph(pages: PageRead[], options: { retryFailed?: boolean } = {}) {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(11);
  const files = new Map(pages.map((page) => [page.id, page]));
  const state = { writable: (_path: string) => false, revision: 0 };
  const host = await bindTestHost();
  for (const page of pages) loadSingle(structuredClone(page));
  const disk = installDiskHost(host, {
    read: (path) => structuredClone(files.get(path) ?? null),
    write: (path, dto) => {
      if (!state.writable(path)) throw new Error("disk full");
      const rev = `w${++state.revision}`;
      files.set(path, { ...structuredClone(dto), id: path, rev });
      return rev;
    },
  }, options);
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => structuredClone(files.get(`pages/${name}.md`) ?? null));
  const dispose = await installQuickCaptureReceiver();
  const capture = async (id: string, text: string, title = "Inbox") => {
    const payload = { id, target: "main", bindingGeneration: 11, title, text };
    await h.receive!({ payload });
    return payload;
  };
  return { files, state, host, disk, dispose, capture, on: (name: string) => raws(files.get(`pages/${name}.md`)) };
}
const rootOf = (name: string, raw: string) => pageByName(name)!.roots.find((id) => doc.byId[id].raw === raw)!;

// T1, T2, T4: the first save meets a file changed outside; the user resolves.
it.each([
  ["Use disk", "disk", UNPROVED, 0],
  ["Use disk while a draft pins the page", "pinned", UNPROVED, 0],
  ["Keep mine", "mine", { ok: true }, 1],
] as const)("a capture whose first save conflicted, then %s: the retry answers from what Tine wrote", async (_label, choice, ack, copies) => {
  const g = await graph([file("Inbox")]);
  g.state.writable = () => true;
  const submit = backend().pageSubmit.bind(backend());
  g.disk.submit.mockImplementationOnce((...args) => {
    g.files.set("pages/Inbox.md", { ...file("Inbox", [block("external")]), rev: "outside" });
    return submit(...args);
  });
  let unpin: (() => void) | undefined;
  try {
    const payload = await g.capture(`conflict-${choice}`, "- only in scratch");
    expect(isConflicted("Inbox")).toBe(true);
    acked(payload.id, UNSAVED);
    if (choice === "pinned") unpin = pinPageWhileDrafting(() => "Inbox");
    expect(await resolveConflict("Inbox", choice === "mine" ? "mine" : "disk")).toBe(true);
    await h.receive!({ payload });
    acked(payload.id, ack);
    expect(g.on("Inbox").filter((raw) => raw === "only in scratch")).toHaveLength(copies);
    // A further retry inserts nothing and answers the same.
    await h.receive!({ payload });
    acked(payload.id, ack);
    expect(g.on("Inbox").filter((raw) => raw === "only in scratch")).toHaveLength(copies);
  } finally { unpin?.(); g.dispose(); }
});

// T5, T6: the write fails, then recovers.
it.each([
  ["as captured", false],
  ["after the user edited the captured block", true],
])("a capture whose first save failed is acknowledged once the disk takes it (%s)", async (_label, edit) => {
  const g = await graph([file("Inbox", [block("earlier")])], { retryFailed: true });
  try {
    const payload = await g.capture(`recovers-${edit}`, "- captured thought");
    acked(payload.id, UNSAVED);
    expect(g.on("Inbox")).toEqual(["earlier"]);
    if (edit) setRaw(rootOf("Inbox", "captured thought"), "captured thought, edited");
    g.state.writable = () => true;
    await h.receive!({ payload });
    acked(payload.id, { ok: true });
    expect(g.on("Inbox")).toEqual(["earlier", edit ? "captured thought, edited" : "captured thought"]);
  } finally { g.dispose(); }
});

// T7: no save ever held both captured blocks.
it("a capture of two blocks, one deleted before any save succeeded, is not acknowledged as saved", async () => {
  const g = await graph([file("Inbox")]);
  try {
    const payload = await g.capture("partial", "- first thought\n- second thought");
    acked(payload.id, UNSAVED);
    deleteBlock(rootOf("Inbox", "second thought"));
    g.state.writable = () => true;
    await h.receive!({ payload });
    acked(payload.id, UNPROVED);
    expect(g.on("Inbox")).toEqual(["first thought"]);
  } finally { g.dispose(); }
});

// T8: the captured block leaves the page before the page's save succeeds.
it("a capture moved to another page before its page saved is not acknowledged from the emptied page", async () => {
  const g = await graph([file("Inbox", [block("inbox-anchor", "anchor")]), file("Receiver", [block("receiver-anchor", "anchor")])]);
  try {
    const payload = await g.capture("moved", "- only in scratch");
    acked(payload.id, UNSAVED);
    await h.receive!({ payload });
    acked(payload.id, UNSAVED); // still failing: develop's answer, no `unproved`
    g.state.writable = () => true;
    expect(await moveBlock(rootOf("Inbox", "only in scratch"), null, 1, "Receiver")).toBe(true);
    await vi.waitFor(() => expect(transferInProgress()).toBe(false));
    await h.receive!({ payload });
    acked(payload.id, UNPROVED);
    expect(g.on("Inbox")).toEqual(["anchor"]);
  } finally { g.dispose(); }
});

// T12 (third design check): the window shows the block on the page again, but
// the text the host took for that page never held it.
it("a capture moved away and back is judged by the text that was sent, not by what the window shows", async () => {
  const g = await graph([file("Inbox", [block("Inbox-anchor", "anchor")]), file("Receiver", [block("Receiver-anchor", "anchor")])]);
  try {
    const payload = await g.capture("there-and-back", "- only in scratch");
    acked(payload.id, UNSAVED);
    const id = rootOf("Inbox", "only in scratch");
    g.state.writable = (path) => path === "pages/Inbox.md";
    expect(await moveBlock(id, null, 1, "Receiver")).toBe(true);
    await vi.waitFor(() => expect(transferInProgress()).toBe(false));
    expect(doc.byId[id].page).toBe("Receiver");
    g.disk.move.mockResolvedValueOnce({ reason: "failed", message: "draft unavailable" });
    setRaw("Inbox-anchor", "changed anchor");
    expect(await moveBlock(id, null, 1, "Inbox")).toBe(true);
    await vi.waitFor(() => expect(transferInProgress()).toBe(false));
    await h.receive!({ payload });
    acked(payload.id, UNPROVED);
    expect([...g.files.values()].flatMap(raws)).not.toContain("only in scratch");
  } finally { g.dispose(); }
});

// T13 (fourth design check): a listed false negative. A properties-only first
// block of an empty page is written as the page's preamble, which carries no
// block id, so no sent text "holds" it. The retry errs safe: text kept.
it.each([
  ["tags:: work", UNPROVED],
  ["plain capture", { ok: true }],
] as const)("a capture into a new page whose first save failed (%s)", async (raw, ack) => {
  const g = await graph([], { retryFailed: true });
  try {
    const payload = await g.capture(`new-page-${raw}`, `- ${raw}`);
    acked(payload.id, UNSAVED);
    expect(g.files.size).toBe(0);
    g.state.writable = () => true;
    await h.receive!({ payload });
    acked(payload.id, ack);
    const written = [...g.files.values()][0];
    expect(raw === "tags:: work" ? written.pre_block : written.blocks[0].raw).toBe(raw);
  } finally { g.dispose(); }
});

// A listed false negative (third design check): a capture inserted before the
// window's page client is bound has no watch.
it("a capture inserted before the page client bound is not acknowledged on retry", async () => {
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(11);
  vi.spyOn(backend(), "getPage").mockResolvedValue(null);
  let finishBind!: () => void;
  const wait = new Promise<void>((resolve) => { finishBind = resolve; });
  const bind = HostClient.prototype.bind;
  vi.spyOn(HostClient.prototype, "bind").mockImplementation(async function (this: HostClient) { await wait; await bind.call(this); });
  const binding = bindHost();
  const dispose = await installQuickCaptureReceiver();
  try {
    const payload = { id: "early", target: "main", bindingGeneration: 11, title: "Inbox", text: "- startup capture" };
    await h.receive!({ payload });
    acked(payload.id, UNSAVED);
    finishBind();
    await binding;
    await h.receive!({ payload });
    acked(payload.id, UNPROVED);
    expect(pageByName("Inbox")!.roots.map((id) => doc.byId[id].raw).filter((raw) => raw === "startup capture")).toHaveLength(1);
  } finally { dispose(); finishBind(); await binding; }
});

// The sender gave the capture up (Start a new capture): its watch is released,
// so later sends of the page no longer look for its blocks.
it("an abandoned capture's watch is released, and a watch for another window's id is not", async () => {
  const g = await graph([file("Inbox")]);
  g.state.writable = () => true;
  const submit = backend().pageSubmit.bind(backend());
  g.disk.submit.mockImplementationOnce((...args) => {
    g.files.set("pages/Inbox.md", { ...file("Inbox", [block("external")]), rev: "outside" });
    return submit(...args);
  });
  const watched = vi.spyOn(HostClient.prototype, "watchBlocks");
  try {
    const payload = await g.capture("abandoned", "- given up");
    acked(payload.id, UNSAVED);
    const watch = watched.mock.results.at(-1)!.value as { release(): void };
    const release = vi.spyOn(watch, "release");
    await h.abandon!({ payload: { id: payload.id, target: "elsewhere" } });
    expect(release).not.toHaveBeenCalled();
    await h.abandon!({ payload: { id: payload.id, target: "main" } });
    expect(release).toHaveBeenCalledTimes(1);
    // Nothing of the id is kept: a replay is a capture this window never inserted.
    expect(await resolveConflict("Inbox", "mine")).toBe(true);
    await h.receive!({ payload });
    expect(h.ack).toHaveBeenLastCalledWith("capture", "quick-capture-ack", expect.objectContaining({ id: payload.id, ok: false }));
    expect(g.on("Inbox").filter((raw) => raw === "given up")).toHaveLength(1);
  } finally { g.dispose(); }
});
