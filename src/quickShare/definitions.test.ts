// A quick share in the graph (D-20261018-Q1): creating a share writes `tine.share:: <login>/<gist_id>` (no key)
// onto the query block it started from, or a definition on the exports page; deleting or forgetting it removes
// the property. Asserted on the bytes that reach the (fake) disk through the document layer's normal save.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { clearSeededFacets } from "../render/facets";
import { backend } from "../backend";
import { flushAll, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { bindTestHost } from "../document/host/wiring.test.support";
import { installDiskHost } from "../diskHost.test.support";
import type { BlockDto, PageDto, PageRead } from "../types";
import { removeShareProperty, saveShareDefinition, shareValue } from "./definitions";

vi.mock("../exportsPage", async (original) => ({
  ...await original<typeof import("../exportsPage")>(),
  ensureExportsPage: vi.fn(async () => "Exports"),
}));

let counter = 0;
// As the backend ships them: a block's `key:: value` lines as its properties (a page read seeds facets from these).
const blk = (raw: string, children: BlockDto[] = []): BlockDto => ({
  id: `q${counter++}`, raw, collapsed: false, children,
  properties: raw.split("\n").slice(1).map((line) => /^([^\s:]+):: (.*)$/.exec(line)).filter((m) => !!m).map((m) => [m![1], m![2]] as [string, string]),
});
const page = (name: string, blocks: BlockDto[], format: "md" | "org" = "md", preBlock: string | null = null): PageRead => ({
  name, kind: "page", title: name, id: `pages/${name}.${format}`, rev: "r0", format, pre_block: preBlock, blocks,
} as PageRead);
const EXPORTS = () => page("Exports", [blk("{{query (or (property export) (property tine.share))}}")], "md", "tine/exports:: true\npublic:: false");

async function bindDisk(initial: PageRead[]) {
  const files = new Map<string, PageRead>(initial.map((dto) => [dto.id, structuredClone(dto)]));
  let rev = 0;
  const host = await bindTestHost();
  installDiskHost(host, {
    read: (path) => files.get(path) ?? null,
    write: (path, dto) => { files.set(path, { ...structuredClone(dto), id: path, rev: `rev${++rev}` }); return `rev${rev}`; },
  });
  vi.spyOn(backend(), "getPage").mockImplementation(async (name: string) => structuredClone([...files.values()].find((dto) => dto.name === name) ?? null));
  const raws = (path: string): string[] => {
    const out: string[] = [];
    const walk = (blocks: BlockDto[]) => { for (const b of blocks) { out.push(b.raw); walk(b.children); } };
    walk(files.get(path)!.blocks);
    return out;
  };
  return { files, raws };
}

function load(dto: PageRead): PageDto {
  loadSingle(structuredClone(dto));
  clearSeededFacets();
  return dto;
}
const idOf = (name: string, raw: string) => Object.values(doc.byId).find((n) => n.page === name && n.raw === raw)!.id;

/** The engine's `(property tine.share)` answer: the given pages' blocks as they are on disk. */
function answerShareQuery(disk: { files: Map<string, PageRead> }, pages: string[]) {
  vi.spyOn(backend(), "parseQuery").mockResolvedValue({ query: { anchor: "block" }, view: {} } as never);
  vi.spyOn(backend(), "queryRun").mockImplementation(async () => ({
    anchor: "block", total: 0, exceeded: false, report: { supported: true },
    groups: pages.map((name) => {
      const dto = [...disk.files.values()].find((d) => d.name === name)!;
      const flat: BlockDto[] = [];
      const walk = (blocks: BlockDto[]) => { for (const b of blocks) { flat.push(b); walk(b.children); } };
      walk(dto.blocks);
      return { page: name, kind: "page", blocks: flat.filter((b) => b.raw.includes("tine.share::")) };
    }),
  }) as never);
}

beforeAll(() => initParser());
beforeEach(() => { counter = 0; resetStore(); });
afterEach(() => { vi.restoreAllMocks(); });

describe("saveShareDefinition", () => {
  it("the value is login/gist and never carries a key", () => {
    expect(shareValue("octo-cat", "0123456789abcdef0123")).toBe("octo-cat/0123456789abcdef0123");
  });

  it("from This page or an edited query: appends a definition with tine.share:: to the exports page and saves it", async () => {
    const disk = await bindDisk([EXPORTS()]);
    load(EXPORTS());
    expect(await saveShareDefinition({ hostBlockId: null, macro: "{{tine-query @page and name = 'Project'}}" }, "octo/abc123")).toBe("Exports");
    expect(await flushAll()).toBe(true);
    expect(disk.raws("pages/Exports.md")).toEqual([
      "{{query (or (property export) (property tine.share))}}",
      "{{tine-query @page and name = 'Project'}}\ntine.share:: octo/abc123",
    ]);
  });

  it("from an unchanged query block: puts tine.share:: on that block (beside an export:: it has)", async () => {
    const notes = page("Notes", [blk("intro"), blk("{{query (task TODO)}}\nexport:: site")]);
    const disk = await bindDisk([notes, EXPORTS()]);
    load(notes);
    const host = idOf("Notes", "{{query (task TODO)}}\nexport:: site");
    expect(await saveShareDefinition({ hostBlockId: host, macro: "{{query (task TODO)}}" }, "octo/abc123")).toBe("Notes");
    expect(disk.raws("pages/Notes.md")).toEqual(["intro", "{{query (task TODO)}}\nexport:: site\ntine.share:: octo/abc123"]);
    expect(disk.raws("pages/Exports.md")).toHaveLength(1);
  });

  it("a block that already names another share keeps it; the new share goes to the exports page", async () => {
    const notes = page("Notes", [blk("{{query (task TODO)}}\ntine.share:: octo/old")]);
    const disk = await bindDisk([notes, EXPORTS()]);
    load(notes);
    load(EXPORTS());
    const host = idOf("Notes", "{{query (task TODO)}}\ntine.share:: octo/old");
    expect(await saveShareDefinition({ hostBlockId: host, macro: "{{query (task TODO)}}" }, "octo/new")).toBe("Exports");
    expect(await flushAll()).toBe(true);
    expect(disk.raws("pages/Notes.md")).toEqual(["{{query (task TODO)}}\ntine.share:: octo/old"]);
    expect(disk.raws("pages/Exports.md")[1]).toBe("{{query (task TODO)}}\ntine.share:: octo/new");
  });

  it("spells the property for an Org exports page", async () => {
    const org = page("Exports", [blk("{{query (or (property export) (property tine.share))}}")], "org", "#+tine/exports: true\n#+public: false");
    const disk = await bindDisk([org]);
    load(org);
    await saveShareDefinition({ hostBlockId: null, macro: "{{tine-query @page and name = 'A'}}" }, "octo/abc");
    expect(await flushAll()).toBe(true);
    const added = disk.raws("pages/Exports.org")[1];
    expect(added).toContain("{{tine-query @page and name = 'A'}}");
    expect(added).toMatch(/:tine\.share: octo\/abc/);
  });
});

describe("removeShareProperty", () => {
  it("removes exactly that share's property wherever it is, loading pages that are not open, and leaves the rest", async () => {
    const exportsPage = page("Exports", [
      blk("{{query (or (property export) (property tine.share))}}"),
      blk("{{tine-query @page and name = 'A'}}\ntine.share:: octo/abc"),
      blk("{{tine-query @page and name = 'B'}}\ntine.share:: octo/abcd"),
    ], "md", "tine/exports:: true\npublic:: false");
    const notes = page("Notes", [blk("parent", [blk("{{query (task TODO)}}\nexport:: site\ntine.share:: octo/abc")])]);
    const disk = await bindDisk([exportsPage, notes]);
    answerShareQuery(disk, ["Exports", "Notes"]);
    // Neither page is open: both are loaded into the working set first.
    expect(doc.pages).toHaveLength(0);
    expect((await removeShareProperty("octo/abc")).sort()).toEqual(["Exports", "Notes"]);
    expect(await flushAll()).toBe(true);
    expect(disk.raws("pages/Exports.md")).toEqual([
      "{{query (or (property export) (property tine.share))}}",
      "{{tine-query @page and name = 'A'}}",
      "{{tine-query @page and name = 'B'}}\ntine.share:: octo/abcd",
    ]);
    expect(disk.raws("pages/Notes.md")).toEqual(["parent", "{{query (task TODO)}}\nexport:: site"]);
  });

  it("a share no block names changes nothing", async () => {
    const disk = await bindDisk([EXPORTS()]);
    answerShareQuery(disk, []);
    expect(await removeShareProperty("octo/none")).toEqual([]);
    expect(disk.files.get("pages/Exports.md")!.rev).toBe("r0");
  });
});
