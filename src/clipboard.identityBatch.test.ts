// master 8c495c1ce (F3): the preserved-id checks of cut-paste and redo scan the
// loaded document ONCE for the whole candidate list, not once per id. The work
// counter is the number of times `doc.byId` is enumerated during the command.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { clearClipboardSlot, copyBlockOutline, peekClipboardSlot } from "./clipboard";
import { buildClipboardPayload, deleteBlock, loadFeed, pasteClipboardPayload, redo, resetStore, undo } from "./document";
import { doc, hasLoadedIdentityCollision, loadedIdentityCollisions } from "./document/model";
import { initParser } from "./render/parse";
import { setGraphMeta } from "./graphSession";
import type { BlockDto, PageDto } from "./types";

const HOST = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const gid = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;

beforeAll(() => initParser());
beforeEach(() => {
  vi.spyOn(backend(), "writeRich").mockResolvedValue();
  vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["saved-rev"] });
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
});
afterEach(() => {
  clearClipboardSlot();
  resetStore();
  setGraphMeta(null);
  vi.restoreAllMocks();
});

const block = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const page = (name: string, blocks: BlockDto[]): PageDto => ({ name, kind: "page", title: name, pre_block: null, blocks, format: "md" });

/** Calls that enumerate the loaded document map while `run` executes. */
async function enumerations(run: () => unknown | Promise<unknown>): Promise<number> {
  let calls = 0;
  const wrap = <K extends "keys" | "values" | "entries">(name: K) => {
    const original = Object[name] as (o: object) => unknown;
    vi.spyOn(Object, name).mockImplementation(((o: object) => {
      if (o === doc.byId) calls++;
      return original(o);
    }) as never);
  };
  wrap("keys"); wrap("values"); wrap("entries");
  try {
    await run();
  } finally {
    vi.mocked(Object.keys).mockRestore();
    vi.mocked(Object.values).mockRestore();
    vi.mocked(Object.entries).mockRestore();
  }
  return calls;
}

async function cutPasteSetup(count: number): Promise<string[]> {
  const ids = Array.from({ length: count }, (_, i) => gid(i + 1));
  loadFeed([
    page("Source", ids.map((id, i) => block(id, `source ${i}\nid:: ${id}`))),
    page("Loaded", Array.from({ length: 40 }, (_, i) => block(`loaded-${i}`, `loaded ${i}`))),
    page("Target", [block(HOST, "")]),
  ]);
  setGraphMeta({ root: "/graph" } as never);
  await copyBlockOutline("cut", "- source", buildClipboardPayload(ids)!);
  for (const id of ids) deleteBlock(id);
  return ids;
}

describe("preserved clipboard identity checks are one pass (master 8c495c1ce)", () => {
  it("cut-paste of 200 preserved ids enumerates the loaded document a constant number of times", async () => {
    const small = await cutPasteSetup(2);
    const smallCalls = await enumerations(() => pasteClipboardPayload(HOST, peekClipboardSlot()!));
    expect(small.every((id) => doc.byId[id]?.page === "Target")).toBe(true);
    resetStore(); clearClipboardSlot();

    const ids = await cutPasteSetup(200);
    const bigCalls = await enumerations(() => pasteClipboardPayload(HOST, peekClipboardSlot()!));

    expect(ids.every((id) => doc.byId[id]?.page === "Target")).toBe(true);
    expect(bigCalls).toBe(smallCalls);
    expect(bigCalls).toBeLessThanOrEqual(2);
  });

  it("redo of 200 preserved ids enumerates the loaded document a constant number of times", async () => {
    const ids = await cutPasteSetup(200);
    await pasteClipboardPayload(HOST, peekClipboardSlot()!);
    undo();

    const calls = await enumerations(() => redo());

    expect(ids.every((id) => doc.byId[id]?.page === "Target")).toBe(true);
    expect(calls).toBeLessThanOrEqual(2);
  });

  it("still sees a live key case-insensitively, a Markdown id:: line and an Org :id: line", () => {
    const upper = "ABCDEFAB-1111-4111-8111-111111111111";
    loadFeed([page("P", [
      block(upper.toLowerCase(), "keyed"),
      block("md", "x\nid:: 22222222-2222-4222-8222-222222222222"),
      block("org", "y\n:PROPERTIES:\n:id: 33333333-3333-4333-8333-333333333333\n:END:"),
    ])]);
    const free = "44444444-4444-4444-8444-444444444444";
    expect([...loadedIdentityCollisions([upper, "22222222-2222-4222-8222-222222222222", "33333333-3333-4333-8333-333333333333", free])])
      .toEqual([upper, "22222222-2222-4222-8222-222222222222", "33333333-3333-4333-8333-333333333333"]);
    expect(hasLoadedIdentityCollision([free])).toBe(false);
    expect(hasLoadedIdentityCollision([free, free.toUpperCase()])).toBe(true);
  });
});
