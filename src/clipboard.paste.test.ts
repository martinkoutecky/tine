import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import {
  clearClipboardSlot,
  copyBlockOutline,
  peekClipboardSlot,
  type ClipboardBlock,
  type ClipboardPayloadData,
} from "./clipboard";
import { buildClipboardPayload, deleteBlock, ensurePageLoaded, flushPage, insertOutlineAfter, loadFeed, markDirty, pageByName, pasteClipboardPayload, sanitizeOutlineIdsForPaste, redo, resetStore, setRaw, toggleUndoRedoMode, undo } from "./document";
import { forgetPage, reloadPage } from "./document/workingSet";
import { historyPageOnlyMode } from "./document/history";
import { loadSingle } from "./document/workingSet";
import { doc, setDoc } from "./document/model";
import { startEditing } from "./editorController";
import { initParser } from "./render/parse";
import type { BlockDto, Format, PageDto } from "./types";
import { graphEpoch, setGraphEpoch, setGraphMeta } from "./graphSession";
import { setGraphTransitioning } from "./ui";
import { setToasts, toasts } from "./toasts";
import { parseOutline } from "./editor/outline";

const HOST = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const ID1 = "11111111-1111-4111-8111-111111111111";
const ID2 = "22222222-2222-4222-8222-222222222222";
const bulkId = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;

beforeAll(() => initParser());

function block(id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto {
  return { id, raw, collapsed, children };
}

function page(
  name: string,
  blocks: BlockDto[],
  options: { format?: Format; id?: string } = {},
): PageDto & { id?: string } {
  return {
    name,
    kind: "page",
    title: name,
    pre_block: null,
    blocks,
    format: options.format ?? "md",
    ...(options.id ? { id: options.id } : {}),
  };
}

function seed(pages: (PageDto & { id?: string })[]): void {
  loadFeed(pages);
  setGraphMeta({ root: "/graph" } as any);
}

function roots(name: string): string[] {
  return [...pageByName(name)!.roots];
}

async function record(
  op: "copy" | "cut",
  text: string,
  payload: ClipboardPayloadData,
): Promise<void> {
  await copyBlockOutline(op, text, payload);
}

async function paste(target = HOST): Promise<string | null> {
  const slot = peekClipboardSlot();
  expect(slot).not.toBeNull();
  return pasteClipboardPayload(target, slot!);
}

beforeEach(() => {
  vi.spyOn(backend(), "writeRich").mockResolvedValue();
  vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["saved-rev"] });
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
});

afterEach(() => {
  if (historyPageOnlyMode()) toggleUndoRedoMode();
  clearClipboardSlot();
  resetStore();
  setGraphMeta(null);
  setGraphTransitioning(false);
  setGraphEpoch(0);
  setToasts([]);
  vi.restoreAllMocks();
});

describe("clipboard payload insertion and identity validation", () => {
  it("keeps the in-memory key of a cut block without id::", async () => {
    seed([page("Source", [block("plain-key", "plain")]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload(["plain-key"])!;
    await record("cut", "- plain", payload);
    deleteBlock("plain-key");
    await paste();
    expect(backend().savePages).toHaveBeenCalledTimes(1);
    expect(roots("Target")).toEqual(["plain-key"]);
  });

  it("replaces an empty host that only literal code mentions by id (C5 L11 paste.ts:128)", async () => {
    seed([page("Source", [block("plain-key", "plain")]),
      page("Target", [block(HOST, ""), block("sample", `example\n\`\`\`\n((${HOST}))\n\`\`\``)])]);
    await record("copy", "- plain", buildClipboardPayload(["plain-key"])!);
    await paste();
    expect(roots("Target")).not.toContain(HOST);
    expect(roots("Target").map((id) => doc.byId[id].raw)).toEqual(["plain", expect.stringContaining("example")]);
  });

  it("keeps an empty host that live text references by id", async () => {
    seed([page("Source", [block("plain-key", "plain")]),
      page("Target", [block(HOST, ""), block("ref", `see ((${HOST.toUpperCase()}))`)])]);
    await record("copy", "- plain", buildClipboardPayload(["plain-key"])!);
    await paste();
    expect(roots("Target")).toContain(HOST);
    expect(roots("Target")).toContain("ref");
  });

  it("warns when a cut must be pasted as a copy", async () => {
    seed([page("Source", [block(ID1, `source\nid:: ${ID1}`)]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    vi.mocked(backend().savePages).mockRejectedValueOnce(new Error("disk full"));
    await paste();
    expect(toasts().some((toast) => /references.*will not follow/i.test(toast.message))).toBe(true);
  });

  it("strips a live id from plain outline paste and keeps an unused id", () => {
    seed([page("Target", [block(HOST, `host\nid:: ${ID1}`)])]);
    insertOutlineAfter(HOST, parseOutline(`- duplicate\n  id:: ${ID1}\n- new\n  id:: ${ID2}`));
    const raws = roots("Target").slice(1).map((id) => doc.byId[id].raw);
    expect(raws).toEqual(["duplicate", `new\nid:: ${ID2}`]);
  });
  it("keeps an inserted authored id when only literal code mentions it", () => {
    seed([page("Target", [block(HOST, `host\n\`\`\`\nid:: ${ID1}\n\`\`\``)])]);
    insertOutlineAfter(HOST, [{ raw: `new\nid:: ${ID1}`, children: [] }]);
    const added = roots("Target").slice(1).map((id) => doc.byId[id].raw);
    expect(added).toEqual([`new\nid:: ${ID1}`]);
  });
  it("checks off-screen IDs before ordinary outline insertion", async () => {
    seed([page("Target", [block(HOST, "host")])]);
    vi.mocked(backend().resolveBlocks).mockResolvedValueOnce([
      { page: "Offscreen", kind: "page", blocks: [] }, null,
    ]);
    const nodes = parseOutline(`- duplicate\n  id:: ${ID1}\n- new\n  id:: ${ID2}`);
    const clean = await sanitizeOutlineIdsForPaste(HOST, nodes);
    expect(clean?.map((node) => node.raw)).toEqual(["duplicate", `new\nid:: ${ID2}`]);
    expect(backend().resolveBlocks).toHaveBeenCalledWith([ID1, ID2]);
  });
  it("bounds ordinary paste ID lookup and preserves uncertain IDs only when lookup succeeds", async () => {
    seed([page("Target", [block(HOST, "host")])]);
    const nodes = Array.from({ length: 129 }, (_, n) => ({ raw: `item\nid:: ${bulkId(n)}`, children: [] }));
    const clean = await sanitizeOutlineIdsForPaste(HOST, nodes);
    expect(clean).toHaveLength(129);
    expect(vi.mocked(backend().resolveBlocks).mock.calls.map(([ids]) => ids.length)).toEqual([128, 1]);
  });
  it("strips all ordinary-paste IDs when a later lookup chunk fails", async () => {
    seed([page("Target", [block(HOST, "host")])]);
    const nodes = Array.from({ length: 129 }, (_, n) => ({ raw: `item\nid:: ${bulkId(n)}`, children: [] }));
    vi.mocked(backend().resolveBlocks).mockImplementationOnce(async (ids) => ids.map(() => null))
      .mockRejectedValueOnce(new Error("lookup failed"));
    const clean = await sanitizeOutlineIdsForPaste(HOST, nodes);
    expect(clean?.every((node) => node.raw === "item")).toBe(true);
    expect(vi.mocked(backend().resolveBlocks).mock.calls.map(([ids]) => ids.length)).toEqual([128, 1]);
  });

  it("bounds cut-paste ID lookup before preserving identities", async () => {
    const blocks = Array.from({ length: 129 }, (_, n) => block(bulkId(n), `item ${n}\nid:: ${bulkId(n)}`));
    seed([page("Source", blocks), page("Target", [block(HOST, "")])]);
    await record("cut", "bulk", buildClipboardPayload(blocks.map((item) => item.id))!);
    blocks.forEach((item) => deleteBlock(item.id));
    await paste();
    expect(vi.mocked(backend().resolveBlocks).mock.calls.map(([ids]) => ids.length)).toEqual([128, 1]);
    expect(roots("Target")).toEqual(blocks.map((item) => item.id));
  });
  it("clears a rejected cut slot without clearing a newer clipboard generation", async () => {
    let rejectFirst!: (error: Error) => void;
    vi.mocked(backend().writeRich).mockImplementationOnce(() => new Promise((_, reject) => { rejectFirst = reject; }));
    const payload: ClipboardPayloadData = { blocks: [], sourcePages: [] };
    const first = copyBlockOutline("cut", "- old", payload);
    await copyBlockOutline("copy", "- new", payload);
    rejectFirst(new Error("clipboard denied"));
    await expect(first).rejects.toThrow("clipboard denied");
    expect(peekClipboardSlot()?.text).toBe("- new");
  });
  it("retires an immediate cut before the debounce and preserves identity", async () => {
    seed([page("Paste", [
      block(ID1, `source\nid:: ${ID1}`),
      block(HOST, ""),
    ])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);

    await paste();

    expect(backend().savePages).toHaveBeenCalledTimes(1);
    const retired = vi.mocked(backend().savePages).mock.calls[0][0][0].page;
    expect(retired.blocks.some((candidate) => candidate.id === ID1)).toBe(false);
    expect(doc.byId[ID1]?.raw).toBe(`source\nid:: ${ID1}`);
    expect(roots("Paste")).toEqual([ID1]);
  });

  it("retires every page in a multi-page cut before preserving all ids", async () => {
    seed([
      page("One", [block(ID1, `one\nid:: ${ID1}`)], { id: "pages/one.md" }),
      page("Two", [block(ID2, `two\nid:: ${ID2}`)], { id: "pages/two.md" }),
      page("Target", [block(HOST, "")], { id: "pages/target.md" }),
    ]);
    const payload = buildClipboardPayload([ID1, ID2])!;
    await record("cut", "- one\n- two", payload);
    deleteBlock(ID1);
    deleteBlock(ID2);

    await paste();

    expect(vi.mocked(backend().savePages).mock.calls.map(([entries]) => entries[0].page.name).sort()).toEqual(["One", "Two"]);
    expect(roots("Target")).toEqual([ID1, ID2]);
  });

  it("strips identity when an unsaved raw acquires the cut id during validation", async () => {
    seed([
      page("Source", [block(ID1, `source\nid:: ${ID1}`)]),
      page("Collision", [block("collision", "other")]),
      page("Target", [block(HOST, "")]),
    ]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    let release!: (value: null[]) => void;
    vi.mocked(backend().resolveBlocks).mockReturnValue(new Promise((resolve) => { release = resolve; }));

    const pending = paste();
    await vi.waitFor(() => expect(backend().resolveBlocks).toHaveBeenCalled());
    setRaw("collision", `collision\nid:: ${ID1}`);
    release([null]);
    await pending;
    await flushPage("Collision");
    await flushPage("Target");

    const ownsIdentity = (raw: string) => new RegExp(`^id::\\s*${ID1}$`, "im").test(raw);
    expect(Object.values(doc.byId).filter((node) => ownsIdentity(node.raw))).toHaveLength(1);
    const persistedOwners = vi.mocked(backend().savePages).mock.calls.flatMap(([entries]) => {
      const visit = (candidate: BlockDto): BlockDto[] => [candidate, ...candidate.children.flatMap(visit)];
      return entries[0].page.blocks.flatMap(visit).filter((candidate) => ownsIdentity(candidate.raw));
    });
    expect(persistedOwners).toHaveLength(1);
  });

  it.each([
    ["duplicate in graph", [ID1], async () => {
      vi.mocked(backend().resolveBlocks).mockResolvedValue([{ page: "Elsewhere", kind: "page", blocks: [] }]);
    }],
    ["resolveBlocks error", [ID1], async () => {
      vi.mocked(backend().resolveBlocks).mockRejectedValue(new Error("offline"));
    }],
  ])("strips every id on %s", async (_label, expectedIds, arrange) => {
    seed([page("Source", [block(ID1, `source\nid:: ${ID1}`)]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    await arrange();

    await paste();

    for (const id of expectedIds) expect(doc.byId[id]).toBeUndefined();
    expect(doc.byId[roots("Target")[0]].raw).toBe("source");
  });

  it("strips all ids for duplicate-within-payload, malformed, and cross-graph grants", async () => {
    const cases: Array<{ raw: string; child?: ClipboardBlock; graph?: string }> = [
      { raw: `root\nid:: ${ID1}`, child: { raw: `child\nid:: ${ID1}`, sourceFormat: "md", children: [] } },
      { raw: `root\nid:: ${ID1}`, child: { raw: `child\nid:: ${ID1.toUpperCase()}`, sourceFormat: "md", children: [] } },
      { raw: "root\nid:: definitely-not-a-uuid" },
      { raw: `root\nid:: ${ID1}`, graph: "/another-graph" },
    ];
    for (const [index, sample] of cases.entries()) {
      resetStore();
      clearClipboardSlot();
      seed([page("Source", [block(ID1, `old\nid:: ${ID1}`)]), page("Target", [block(HOST, "")])]);
      const payload = buildClipboardPayload([ID1])!;
      payload.blocks = [{ raw: sample.raw, sourceFormat: "md", children: sample.child ? [sample.child] : [] }];
      await record("cut", `- root ${index}`, payload);
      if (sample.graph) (peekClipboardSlot() as any).graph = sample.graph;
      deleteBlock(ID1);

      await paste();

      expect(doc.byId[ID1]).toBeUndefined();
      expect(roots("Target")).toHaveLength(1);
    }
  });

  it("fails closed on one-source flush failure, eviction, and same-name path rebind", async () => {
    const run = async (arrange: () => void | Promise<void>) => {
      resetStore();
      clearClipboardSlot();
      seed([
        page("Source", [block(ID1, `source\nid:: ${ID1}`)], { id: "pages/source.md" }),
        page("Target", [block(HOST, "")]),
      ]);
      const payload = buildClipboardPayload([ID1])!;
      await record("cut", "- source", payload);
      deleteBlock(ID1);
      await arrange();
      await paste();
      expect(doc.byId[ID1]).toBeUndefined();
    };

    vi.mocked(backend().savePages).mockRejectedValueOnce(new Error("disk full"));
    await run(() => {});
    await run(() => forgetPage("Source"));
    await run(() => {
      forgetPage("Source");
      reloadPage(page("Source", [block("replacement", "replacement")], { id: "pages/rebound.md" }));
    });
  });

  it("strips all ids after the actual working-set cap evicts the cut source", async () => {
    loadSingle(page("Target", [block(HOST, "")]));
    ensurePageLoaded(page("Source", [
      block(ID1, `source\nid:: ${ID1}`, [block(ID2, `child\nid:: ${ID2}`)]),
    ]));
    setGraphMeta({ root: "/graph" } as any);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source\n\t- child", payload);
    deleteBlock(ID1);
    expect(await flushPage("Source")).toBe(true);

    for (let i = 0; i < 79; i++) {
      ensurePageLoaded(page(`Eviction ${i}`, [block(`eviction-${i}`, String(i))]));
    }
    expect(pageByName("Source")).toBeUndefined();

    await paste();

    expect(doc.byId[ID1]).toBeUndefined();
    expect(doc.byId[ID2]).toBeUndefined();
    const pastedRoot = doc.byId[roots("Target")[0]];
    expect(pastedRoot.raw).toBe("source");
    expect(doc.byId[pastedRoot.children[0]].raw).toBe("child");
  });

  it("strips the whole multi-page payload when only one source flush fails", async () => {
    resetStore();
    clearClipboardSlot();
    seed([
      page("One", [block(ID1, `one\nid:: ${ID1}`)]),
      page("Two", [block(ID2, `two\nid:: ${ID2}`)]),
      page("Target", [block(HOST, "")]),
    ]);
    const payload = buildClipboardPayload([ID1, ID2])!;
    await record("cut", "- one\n- two", payload);
    deleteBlock(ID1);
    deleteBlock(ID2);
    vi.mocked(backend().savePages).mockImplementation(async (entries) => { const { id: _id, page: dto } = entries[0];
      if (dto.name === "Two") throw new Error("disk full");
      return { ok: ["saved-rev"] };
    });

    await paste();

    expect(doc.byId[ID1]).toBeUndefined();
    expect(doc.byId[ID2]).toBeUndefined();
    expect(roots("Target").every((id) => id !== ID1 && id !== ID2)).toBe(true);
  });

  it("fails retirement when a source is rebound while its save is in flight", async () => {
    seed([
      page("Source", [block(ID1, `source\nid:: ${ID1}`)], { id: "pages/source.md" }),
      page("Target", [block(HOST, "")]),
    ]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    let finishSave!: (result: { ok: string[] }) => void;
    vi.mocked(backend().savePages).mockReturnValue(new Promise((resolve) => { finishSave = resolve; }));

    const pending = paste();
    await vi.waitFor(() => expect(backend().savePages).toHaveBeenCalled());
    reloadPage(page("Source", [block("replacement", "replacement")], { id: "pages/rebound.md" }));
    finishSave({ ok: ["stale-rev"] });

    await pending;
    expect(doc.byId[ID1]).toBeUndefined();
    expect(doc.byId[roots("Target")[0]].raw).toBe("source");
  });

  it("consumes a cut grant up front and makes a second paste structural-only", async () => {
    seed([page("Source", [block(ID1, `source\nid:: ${ID1}`)]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    vi.mocked(backend().resolveBlocks).mockRejectedValueOnce(new Error("first validation fails"));

    await paste();
    expect(peekClipboardSlot()?.op).toBe("copy");
    const first = roots("Target")[0];
    expect(first).not.toBe(ID1);

    await paste(first);
    expect(doc.byId[ID1]).toBeUndefined();
    expect(roots("Target")).toHaveLength(2);
  });

  it("strips identity when undo restores the cut originals before paste", async () => {
    seed([page("Source", [block(ID1, `source\nid:: ${ID1}`)]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    undo();
    vi.mocked(backend().resolveBlocks).mockResolvedValue([{ page: "Source", kind: "page", blocks: [] }]);

    await paste();

    expect(doc.byId[ID1]?.page).toBe("Source");
    const pasted = roots("Target")[0];
    expect(pasted).not.toBe(ID1);
    expect(doc.byId[pasted].raw).toBe("source");
  });

  it("copy-paste strips every id while preserving exact structure, raws, and collapse", async () => {
    seed([page("Target", [block(HOST, "")])]);
    await record("copy", "- root", {
      blocks: [{
        raw: `root line\nsecond line\ncollapsed:: true\nid:: ${ID1}`,
        sourceFormat: "md",
        children: [{ raw: `child\ncustom:: value\nid:: ${ID2}`, sourceFormat: "md", children: [] }],
      }],
      sourcePages: [],
    });

    await paste();

    const root = doc.byId[roots("Target")[0]];
    const child = doc.byId[root.children[0]];
    expect(root.raw).toBe("root line\nsecond line\ncollapsed:: true");
    expect(root.collapsed).toBe(true);
    expect(child.raw).toBe("child\ncustom:: value");
    expect(doc.byId[ID1]).toBeUndefined();
    expect(doc.byId[ID2]).toBeUndefined();
  });

  it("keeps a same-format preserved raw byte-exact, including blank lines and trailing spaces", async () => {
    const exact = `first line  \n\nsecond line\t\nid:: ${ID1}`;
    seed([page("Source", [block(ID1, exact)]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- first line", payload);
    deleteBlock(ID1);

    await paste();

    expect(doc.byId[ID1].raw).toBe(exact);
    undo();
    expect(roots("Target")).toEqual([HOST]);
  });

  it("does not replace an empty host carrying identity or an incoming reference", async () => {
    seed([page("Target", [
      block(HOST, `id:: ${HOST}`),
      block("referrer", `((${ID2}))`),
      block(ID2, ""),
    ])]);
    await record("copy", "- payload", {
      blocks: [{ raw: "payload", sourceFormat: "md", children: [] }],
      sourcePages: [],
    });
    await paste(HOST);
    expect(roots("Target")[0]).toBe(HOST);
    expect(doc.byId[HOST].raw).toBe(`id:: ${HOST}`);

    await paste(ID2);
    expect(doc.byId[ID2]).toBeDefined();
    expect(roots("Target")).toContain(ID2);
  });

  it.each([
    ["md", "org", `body\nalpha:: one\nid:: ${ID1}\ncollapsed:: true`, `body\n:PROPERTIES:\n:alpha: one\n:id: ${ID1}\n:collapsed: true\n:END:`],
    ["org", "md", `body\n:PROPERTIES:\n:alpha: one\n:id: ${ID1}\n:collapsed: true\n:END:`, `body\nalpha:: one\nid:: ${ID1}\ncollapsed:: true`],
  ] as const)("translates ordered properties for %s → %s while preserving cut identity", async (sourceFormat, targetFormat, raw, expected) => {
    seed([
      page("Source", [block(ID1, raw)], { format: sourceFormat }),
      page("Target", [block(HOST, "")], { format: targetFormat }),
    ]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- body", payload);
    deleteBlock(ID1);

    await paste();

    expect(doc.byId[ID1]?.raw).toBe(expected);
    expect(doc.byId[ID1]?.collapsed).toBe(true);
  });

  it.each(["md", "org"] as const)("preserves parser-accepted keys and duplicate order when copying from %s", async (sourceFormat) => {
    const raw = sourceFormat === "md"
      ? "body\nklíč:: hodnota\n@custom:: first  \nklíč:: second"
      : "body\n:PROPERTIES:\n:klíč: hodnota\n:@custom: first  \n:klíč: second\n:END:";
    const targetFormat = sourceFormat === "md" ? "org" : "md";
    seed([page("Source", [block(ID1, raw)], { format: sourceFormat }), page("Target", [block(HOST, "")], { format: targetFormat })]);
    await record("copy", "body", buildClipboardPayload([ID1])!);
    const inserted = await paste();
    expect(doc.byId[inserted!].raw).toBe(targetFormat === "org"
      ? "body\n:PROPERTIES:\n:klíč: hodnota\n:@custom: first  \n:klíč: second\n:END:"
      : "body\nklíč:: hodnota\n@custom:: first  \nklíč:: second");
  });

  it("aborts entirely when graph authority changes during validation", async () => {
    seed([page("Source", [block(ID1, `source\nid:: ${ID1}`)]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    let release!: (value: (null)[]) => void;
    vi.mocked(backend().resolveBlocks).mockReturnValue(new Promise((resolve) => { release = resolve; }));

    const pending = paste();
    await vi.waitFor(() => expect(backend().resolveBlocks).toHaveBeenCalled());
    setGraphEpoch(graphEpoch() + 1);
    release([null]);

    await expect(pending).resolves.toBeNull();
    expect(roots("Target")).toEqual([HOST]);
    expect(peekClipboardSlot()?.op).toBe("copy");
  });

  it("aborts when the target page instance reloads during validation", async () => {
    seed([page("Source", [block(ID1, `source\nid:: ${ID1}`)]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    let release!: (value: (null)[]) => void;
    vi.mocked(backend().resolveBlocks).mockReturnValue(new Promise((resolve) => { release = resolve; }));

    const pending = paste();
    await vi.waitFor(() => expect(backend().resolveBlocks).toHaveBeenCalled());
    reloadPage(page("Target", [block(HOST, "reloaded")]))
    release([null]);

    await expect(pending).resolves.toBeNull();
    expect(roots("Target")).toEqual([HOST]);
    expect(doc.byId[HOST].raw).toBe("reloaded");
  });

  it("repeats the source clean-state check in the final no-await section", async () => {
    seed([page("Source", [block(ID1, `source\nid:: ${ID1}`)]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    let release!: (value: (null)[]) => void;
    vi.mocked(backend().resolveBlocks).mockReturnValue(new Promise((resolve) => { release = resolve; }));

    const pending = paste();
    await vi.waitFor(() => expect(backend().resolveBlocks).toHaveBeenCalled());
    markDirty("Source", "save-block");
    release([null]);

    await pending;
    expect(doc.byId[ID1]).toBeUndefined();
    expect(doc.byId[roots("Target")[0]].raw).toBe("source");
  });

  it("rechecks the live doc after backend absence validation and strips on an in-app conflict", async () => {
    seed([page("Source", [block(ID1, `source\nid:: ${ID1}`)]), page("Target", [block(HOST, "")])]);
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    let release!: (value: (null)[]) => void;
    vi.mocked(backend().resolveBlocks).mockReturnValue(new Promise((resolve) => { release = resolve; }));

    const pending = paste();
    await vi.waitFor(() => expect(backend().resolveBlocks).toHaveBeenCalled());
    setDoc("byId", ID1, {
      id: ID1,
      raw: `collision\nid:: ${ID1}`,
      collapsed: false,
      parent: null,
      page: "Target",
      children: [],
    });
    setDoc("pages", (candidate) => candidate.name === "Target", "roots", (ids) => [...ids, ID1]);
    release([null]);

    await pending;
    expect(doc.byId[ID1].raw).toContain("collision");
    const structural = roots("Target").find((id) => id !== ID1)!;
    expect(structural).not.toBe(HOST);
    expect(doc.byId[structural].raw).toBe("source");
  });
});

describe("identity-tagged redo", () => {
  async function preservedPaste(): Promise<void> {
    seed([
      page("Source", [block(ID1, `source\nid:: ${ID1}`)]),
      page("Target", [block(HOST, "")]),
      page("Other", [block("other", "old")]),
    ]);
    setRaw("other", "new");
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    await paste();
    expect(doc.byId[ID1]).toBeDefined();
  }

  function introduceCollision(): void {
    setDoc("byId", ID1, {
      id: ID1,
      raw: `collision\nid:: ${ID1}`,
      collapsed: false,
      parent: null,
      page: "Target",
      children: [],
    });
    setDoc("pages", (page) => page.name === "Target", "roots", (ids) => [...ids, ID1]);
  }

  it("refuses redo without changing content and clears the full redo stack", async () => {
    await preservedPaste();
    undo();
    expect(doc.byId[ID1]).toBeUndefined();
    introduceCollision();
    const before = JSON.stringify(doc);

    redo();

    expect(JSON.stringify(doc)).toBe(before);
    expect(toasts().at(-1)?.message).toBe("Redo skipped: a block with the same id now exists");
    setDoc("pages", (page) => page.name === "Target", "roots", (ids) => ids.filter((id) => id !== ID1));
    setDoc("byId", ID1, undefined!);
    redo();
    expect(doc.byId[ID1]).toBeUndefined();
    expect(doc.byId.other.raw).toBe("new");
    expect(toggleUndoRedoMode()).toBe("Page only");
    startEditing("other", 0);
    undo();
    expect(doc.byId.other.raw).toBe("old");
  });

  it("retains the identity tag through a successful undo/redo round-trip", async () => {
    await preservedPaste();
    undo();
    redo();
    expect(doc.byId[ID1]).toBeDefined();
    undo();
    introduceCollision();

    redo();

    expect(doc.byId[ID1].raw).toContain("collision");
    expect(toasts().at(-1)?.message).toContain("Redo skipped");
  });

  it("clears mixed page-only redo entries when the tagged prerequisite is selected from the middle", async () => {
    seed([
      page("Source", [block(ID1, `source\nid:: ${ID1}`)]),
      page("Target", [block(HOST, "")]),
      page("Other", [block("other", "old")]),
    ]);
    setRaw("other", "new");
    const payload = buildClipboardPayload([ID1])!;
    await record("cut", "- source", payload);
    deleteBlock(ID1);
    await paste();
    undo(); // tagged paste → redo[0]
    undo(); // cut delete
    undo(); // Other raw entry → newest redo, tagged entry is now interior
    introduceCollision();
    expect(toggleUndoRedoMode()).toBe("Page only");
    startEditing(HOST, 0);

    redo();

    expect(toasts().at(-1)?.message).toContain("Redo skipped");
    setDoc("pages", (page) => page.name === "Target", "roots", (ids) => ids.filter((id) => id !== ID1));
    setDoc("byId", ID1, undefined!);
    redo();
    expect(doc.byId[ID1]).toBeUndefined();
    expect(doc.byId.other.raw).toBe("old");
  });
});


describe("OG-P11B literal outline insertion", () => {
  it.each([
    "#+BEGIN_SRC js\n- literal\n\n  id:: literal\n#+END_SRC",
    "  #+BEGIN_EXAMPLE\n- literal\n\n  kept spaces\n  #+END_EXAMPLE",
    "```text\n- literal\n\n```",
    "~~~~text\n- literal\n\n~~~~",
  ])("inserts a parser-owned literal intact: %s", (source) => {
    seed([page("Target", [block(HOST, "host")])]);
    insertOutlineAfter(HOST, parseOutline(source));
    const inserted = roots("Target").slice(1);
    expect(inserted.map(id => doc.byId[id].raw)).toEqual([source]);
    expect(inserted.map(id => doc.byId[id].children)).toEqual([[]]);
  });
  it("keeps a bullet's literal continuation and following ordered item", () => {
    seed([page("Target", [block(HOST, "host")])]);
    const literal = "  ```js\n  - literal\n\n    kept spaces\n  ```";
    insertOutlineAfter(HOST, parseOutline(`- parent\n${literal}\n2) second`));
    const inserted = roots("Target").slice(1);
    expect(inserted.map(id => doc.byId[id].raw)).toEqual([`parent\n${literal}`, "second"]);
    expect(inserted.map(id => doc.byId[id].children)).toEqual([[], []]);
  });
});
