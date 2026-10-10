// S1 (ADR 0073): shared items reach a journal only through the quick-capture
// writer, leave the inbox only after the write reached disk, are never lost,
// and are not appended twice across a crash between the write and the
// removal. The "review:" cases are the regressions of the independent review
// (tine-agents/specs/campaigns/2026-10-native-integrations/REVIEW-codex.md,
// findings 1-3), ported from its scratch repros.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { backend, type SavePageEntry } from "./backend";
import { resetStore, pageByName, node, appendToTodayJournal } from "./document";
import { loadSingle } from "./document/workingSet";
import { graphMeta, setGraphMeta } from "./graphSession";
import { setToasts, toasts } from "./toasts";
import { journalTitle, appNow } from "./journal";
import { initParser } from "./render/parse";
import { ingestShares, CAPTURED_TOAST } from "./shareIngest";
import type { NativeShareInbox, ShareInboxItem } from "./nativeTineLinks";
import type { PageDto } from "./types";

beforeAll(initParser);

const day = () => journalTitle(appNow());
/** Files on "disk": root block bodies per graph root and journal day. */
let files: Map<string, string[]>;
let revs: Map<string, number>;
let inbox: Map<string, ShareInboxItem>;
let commitFails = false;
let saveFails = false;
let saves = 0;
let imports: string[] = [];

const key = (name: string, root = graphMeta()!.root) => `${root}|${name}`;
/** Today's journal (or `name`) in the bound graph, as stored. */
const journal = (name = day(), root?: string) => files.get(key(name, root)) ?? [];

function dto(name: string): PageDto | null {
  const raws = files.get(key(name));
  if (!raws) return null;
  const rev = revs.get(key(name)) ?? 0;
  return { name, kind: "journal", title: name, id: `journals/${name}.md`, rev: `r${rev}`, pre_block: null,
    blocks: raws.map((raw, i) => ({ id: `${name}-${rev}-${i}`, raw, collapsed: false, children: [] })) } as PageDto;
}

/** Write `raws` to journal `name` as an external editor would. */
function writeFile(name: string, raws: string[]) {
  files.set(key(name), raws);
  revs.set(key(name), (revs.get(key(name)) ?? 0) + 1);
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
const bindGraph = (root: string) =>
  setGraphMeta({ root, pages_dir: "pages", journals_dir: "journals", assets_dir: "assets", preferred_format: "md" } as any);
/** The process restarts: a fresh store reads the journal from disk. */
function restart(root = graphMeta()!.root) {
  resetStore();
  bindStore();
  bindGraph(root);
}

beforeEach(() => {
  bindStore();
  files = new Map(); revs = new Map(); saves = 0; imports = [];
  commitFails = false; saveFails = false;
  inbox = new Map();
  bindGraph("/g");
  const api = backend();
  api.tineLinks = { identity: vi.fn(), scanKnownGraphs: vi.fn(), take: vi.fn(async () => []), handoff: vi.fn(), subscribe: vi.fn(), inbox: fakeInbox() };
  vi.spyOn(api, "getPage").mockImplementation(async (name) => dto(name) as any);
  vi.spyOn(api, "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
    if (saveFails) throw new Error("disk full");
    saves++;
    for (const entry of entries) writeFile(entry.page.name, entry.page.blocks.map((block) => block.raw));
    return { ok: entries.map((entry) => `r${revs.get(key(entry.page.name))}`) };
  });
  vi.spyOn(api, "importAsset").mockImplementation(async (_path, name) => {
    imports.push(`${graphMeta()!.root}:${name}`);
    return `${name}`;
  });
});

afterEach(() => { resetStore(); setGraphMeta(null); setToasts([]); delete backend().tineLinks; vi.restoreAllMocks(); });

function share(id: string, text: string, extra: Partial<ShareInboxItem> = {}) {
  inbox.set(id, { id, source: "android", created: Date.UTC(2026, 9, 10, 8, 30), text, resources: [], ...extra });
}

const shared = (text: string, raws = journal()) => raws.filter((raw) => raw.endsWith(`[[quick capture]]: ${text}`));
const errorToasts = () => toasts().filter((toast) => toast.kind === "error");

it("appends a shared item at the bottom of today's journal, then removes it", async () => {
  writeFile(day(), ["earlier note"]);
  share("a", "from the share sheet");
  await ingestShares();
  expect(journal()[0]).toBe("earlier note");
  expect(journal().at(-1)).toMatch(/^\*\*\d\d:\d\d\*\* \[\[quick capture\]\]: from the share sheet$/);
  expect(inbox.size).toBe(0);
  expect(toasts().map((toast) => toast.message)).toContain(CAPTURED_TOAST);
});

it("keeps the item and shows an error when the write fails, and does not append it twice on retry", async () => {
  share("a", "keep me");
  saveFails = true;
  await ingestShares();
  expect(inbox.has("a")).toBe(true);
  expect(shared("keep me")).toHaveLength(0);
  expect(errorToasts().length).toBeGreaterThan(0);
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
  expect(inbox.get("a")?.prepared).toMatchObject({ graph: "/g", day: day(), armed: { before: null, matches: 0 } });
  restart();
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
  await ingestShares();
  expect(shared("same words")).toHaveLength(2);
  expect(inbox.size).toBe(0);
  expect(pageByName(day())!.roots.map((id) => node(id).raw).filter((raw) => raw.endsWith("same words"))).toHaveLength(2);
});

it("a crash after arming but before the append, then an edit of the journal, still appends the item (loss-free)", async () => {
  writeFile(day(), ["**08:30** [[quick capture]]: twice"]); // an identical earlier capture
  share("a", "twice");
  const apiInbox = backend().tineLinks!.inbox!;
  const prepare = apiInbox.prepare;
  apiInbox.prepare = async (id, prepared) => {
    await prepare(id, prepared);
    if (prepared.armed) { apiInbox.prepare = prepare; throw new Error("crash after arming, before the append"); }
  };
  await ingestShares();
  expect(inbox.get("a")?.prepared?.armed).toEqual({ before: "r1", matches: 1 });
  expect(shared("twice")).toHaveLength(1);
  restart();
  writeFile(day(), ["an edit elsewhere", ...journal()]);
  await ingestShares();
  expect(shared("twice")).toHaveLength(2);
  expect(inbox.size).toBe(0);
});

it("a landed item the user edited afterwards is appended again: a duplicate, never a loss", async () => {
  share("a", "edited later");
  commitFails = true;
  await ingestShares();
  restart();
  writeFile(day(), journal().map((raw) => `${raw} (edited)`));
  commitFails = false;
  await ingestShares();
  expect(journal()).toHaveLength(2);
  expect(shared("edited later")).toHaveLength(1);
  expect(inbox.size).toBe(0);
});

it("review: a different writer's identical capture must not consume a pending share", async () => {
  share("a", "same words");
  const apiInbox = backend().tineLinks!.inbox!;
  const prepare = apiInbox.prepare;
  let inject = true;
  apiInbox.prepare = async (id, prepared) => {
    await prepare(id, prepared);
    // After the share is shaped and before its append: another capture of
    // the same words lands, then the process dies.
    if (inject && prepared.markdown && !prepared.armed) {
      inject = false;
      await appendToTodayJournal(prepared.markdown);
      throw new Error("crash before this share's append");
    }
  };
  await ingestShares();
  expect(inbox.has("a")).toBe(true);
  await ingestShares();
  expect(shared("same words")).toHaveLength(2);
});

it("review: a committed share awaiting deletion must not replay into a switched graph", async () => {
  share("a", "belongs in A");
  commitFails = true;
  await ingestShares();
  expect(shared("belongs in A")).toHaveLength(1);
  // A is safely on disk; only the inbox commit failed. Bind a new empty graph B.
  restart("/B");
  setToasts([]);
  commitFails = false;
  await ingestShares();
  await ingestShares();
  expect(journal()).toHaveLength(0);
  expect(inbox.has("a")).toBe(true);
  expect(toasts()).toHaveLength(0);
  // Back in A, the landed append is recognised and acknowledged once.
  restart("/g");
  await ingestShares();
  expect(shared("belongs in A")).toHaveLength(1);
  expect(inbox.size).toBe(0);
});

it("review: crossing midnight between prepare and append must not duplicate on recovery", async () => {
  let now = new Date(2026, 9, 10, 23, 59, 59, 900).getTime();
  vi.spyOn(Date, "now").mockImplementation(() => now);
  const yesterday = day();
  share("a", "midnight share");
  const apiInbox = backend().tineLinks!.inbox!;
  const prepare = apiInbox.prepare;
  let rollover = true;
  apiInbox.prepare = async (id, prepared) => {
    await prepare(id, prepared);
    if (rollover) { rollover = false; now += 200; }
  };
  commitFails = true;
  await ingestShares();
  expect(day()).not.toBe(yesterday);
  // The append went into the frozen day, not into the new "today".
  expect(shared("midnight share", journal(yesterday))).toHaveLength(1);
  expect(journal()).toHaveLength(0);
  restart(); commitFails = false;
  await ingestShares();
  expect(shared("midnight share", journal(yesterday))).toHaveLength(1);
  expect(journal()).toHaveLength(0);
  expect(inbox.size).toBe(0);
});

it("the {date} template value is the frozen day the item lands in", async () => {
  bindGraph("/g");
  setGraphMeta({ ...graphMeta()!, quick_capture_template_text: "{date}: {text}" } as any);
  share("a", "dated");
  await ingestShares();
  expect(journal()).toEqual([`${day()}: dated`]);
});

it("files are imported into the recorded graph once, and a retry reuses them", async () => {
  share("a", "", { text: null, resources: [{ path: "/inbox/a/p.png", name: "p.png", type: "image/png" }] });
  commitFails = true;
  await ingestShares();
  expect(imports).toHaveLength(1);
  expect(inbox.get("a")?.prepared?.assets).toHaveLength(1);
  restart("/B");
  commitFails = false;
  await ingestShares();
  expect(imports).toHaveLength(1); // nothing imported into B
  restart("/g");
  await ingestShares();
  expect(imports).toEqual([expect.stringMatching(/^\/g:/)]);
  expect(journal().filter((raw) => raw.includes("p.png"))).toHaveLength(1);
  expect(inbox.size).toBe(0);
});
