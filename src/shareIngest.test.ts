// S1 (ADR 0073): shared items reach today's journal only through the quick
// capture writer, leave the inbox only after the write reached disk, and are
// never appended twice across a crash between the write and the removal.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { backend, type SavePageEntry } from "./backend";
import { resetStore, pageByName, node } from "./document";
import { loadSingle } from "./document/workingSet";
import { setGraphMeta } from "./graphSession";
import { setToasts, toasts } from "./toasts";
import { journalTitle, appNow } from "./journal";
import { initParser } from "./render/parse";
import { ingestShares, CAPTURED_TOAST } from "./shareIngest";
import type { NativeShareInbox, ShareInboxItem } from "./nativeTineLinks";
import type { PageDto } from "./types";

beforeAll(initParser);

const day = () => journalTitle(appNow());
/** The journal file as "disk" holds it: root block bodies. */
let disk: string[] = [];
let inbox: Map<string, ShareInboxItem>;
let commitFails = false;
let saveFails = false;
let saves = 0;

function journalDto(): PageDto {
  return { name: day(), kind: "journal", title: day(), id: "journals/today.md", rev: `r${saves}`, pre_block: null,
    blocks: disk.map((raw, i) => ({ id: `b${saves}-${i}`, raw, collapsed: false, children: [] })) } as PageDto;
}

function fakeInbox(): NativeShareInbox {
  return {
    list: async () => ({ items: [...inbox.values()].map((item) => structuredClone(item)), rejected: 0 }),
    prepare: async (id, prepared) => { inbox.get(id)!.prepared = structuredClone(prepared); },
    commit: async (id) => { if (commitFails) throw new Error("killed before removal"); inbox.delete(id); },
    subscribe: async () => () => {},
  };
}

/** A bound graph's store (the app ingests only after the graph loaded). */
const bindStore = () => loadSingle({ name: "Other", kind: "page", title: "Other", id: "pages/Other.md", pre_block: null, blocks: [] });

beforeEach(() => {
  bindStore();
  disk = []; saves = 0; commitFails = false; saveFails = false;
  inbox = new Map();
  setGraphMeta({ root: "/g", pages_dir: "pages", journals_dir: "journals", assets_dir: "assets", preferred_format: "md" } as any);
  const api = backend();
  api.tineLinks = { identity: vi.fn(), scanKnownGraphs: vi.fn(), take: vi.fn(async () => []), handoff: vi.fn(), subscribe: vi.fn(), inbox: fakeInbox() };
  vi.spyOn(api, "getPage").mockImplementation(async (name) => (name === day() && disk.length ? journalDto() as any : null));
  vi.spyOn(api, "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
    if (saveFails) throw new Error("disk full");
    saves++;
    disk = entries[0].page.blocks.map((block) => block.raw);
    return { ok: [`r${saves}`] };
  });
});

afterEach(() => { resetStore(); setGraphMeta(null); setToasts([]); delete backend().tineLinks; vi.restoreAllMocks(); });

function share(id: string, text: string) {
  inbox.set(id, { id, created: Date.UTC(2026, 9, 10, 8, 30), text, resources: [] });
}

const shared = (text: string) => disk.filter((raw) => raw.endsWith(`[[quick capture]]: ${text}`));

it("appends a shared item at the bottom of today's journal, then removes it", async () => {
  disk = ["earlier note"];
  share("a", "from the share sheet");
  await ingestShares();
  expect(disk[0]).toBe("earlier note");
  expect(disk.at(-1)).toMatch(/^\*\*\d\d:\d\d\*\* \[\[quick capture\]\]: from the share sheet$/);
  expect(inbox.size).toBe(0);
  expect(toasts().map((toast) => toast.message)).toContain(CAPTURED_TOAST);
});

it("keeps the item and shows an error when the write fails, and does not append it twice on retry", async () => {
  share("a", "keep me");
  saveFails = true;
  await ingestShares();
  expect(inbox.has("a")).toBe(true);
  expect(shared("keep me")).toHaveLength(0);
  expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
  saveFails = false;
  await ingestShares();
  expect(shared("keep me")).toHaveLength(1);
  expect(inbox.size).toBe(0);
});

it("a crash between the journal write and the removal does not duplicate the item", async () => {
  share("a", "exactly once");
  commitFails = true; // the process dies after the write reached disk
  await expect(ingestShares()).resolves.toBeUndefined();
  expect(shared("exactly once")).toHaveLength(1);
  expect(inbox.get("a")?.prepared?.baseline).toBe(0);
  // Restart: a fresh store reads the journal from disk.
  resetStore();
  bindStore();
  commitFails = false;
  const before = saves;
  await ingestShares();
  expect(saves).toBe(before);
  expect(shared("exactly once")).toHaveLength(1);
  expect(inbox.size).toBe(0);
});

it("an identical block already on the journal is not mistaken for the item", async () => {
  share("a", "same words");
  await ingestShares();
  share("b", "same words");
  inbox.get("b")!.created = inbox.get("a")?.created ?? Date.UTC(2026, 9, 10, 8, 30);
  await ingestShares();
  expect(shared("same words")).toHaveLength(2);
  expect(inbox.size).toBe(0);
  expect(pageByName(day())!.roots.map((id) => node(id).raw).filter((raw) => raw.endsWith("same words"))).toHaveLength(2);
});
