// @vitest-environment jsdom
import { renderEnglish } from "./i18n";
import { readFileSync } from "node:fs";
import { mainPaneRouter, route } from "./router";
import { setNavReuseTabs } from "./navSettings";
import { refreshPageIndex, resetPageIndex } from "./pageIndex";
import { pageIdentityKey } from "./pageIdentity";
// S1 (ADR 0073): shared items reach a journal only through the quick-capture
// writer, leave the inbox only after the write reached disk, are never lost,
// and are not appended twice across a crash between the write and the
// removal. The "review:" cases are the regressions of the independent review
// (tine-agents/specs/campaigns/2026-10-native-integrations/REVIEW-codex.md,
// findings 1-3), ported from its scratch repros.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { bindTestHost } from "./document/host/wiring.test.support";
import { installDiskHost, type FakeDisk } from "./diskHost.test.support";
import { resetStore, pageByName, node, appendToTodayJournal } from "./document";
import { loadSingle } from "./document/workingSet";
import { graphMeta, setGraphMeta } from "./graphSession";
import { setToasts, toasts } from "./toasts";
import { journalTitle, appNow } from "./journal";
import { initParser } from "./render/parse";
import { ingestShares, CAPTURED_TOAST, resetShareIngestForTests } from "./shareIngest";
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
/** The "disk" as the page host sees it: journal `name` lives at `journals/<name>.md`. */
const disk: FakeDisk = {
  read: (_key, name) => dto(name),
  write: (_key, page) => {
    if (saveFails) throw new Error("disk full");
    saves++;
    writeFile(page.name, page.blocks.map((block) => block.raw));
    return `r${revs.get(key(page.name))}`;
  },
};
/** Bind the window to a page host over `disk` (a fresh host: nothing held). */
async function bindHost() {
  installDiskHost(await bindTestHost(), disk, { retryFailed: true });
}
/** The process restarts: a fresh store reads the journal from disk. */
async function restart(root = graphMeta()!.root) {
  resetShareIngestForTests();
  resetStore();
  bindStore();
  bindGraph(root);
  await bindHost();
}

beforeEach(async () => {
  resetShareIngestForTests();
  bindStore();
  files = new Map(); revs = new Map(); saves = 0; imports = [];
  commitFails = false; saveFails = false;
  inbox = new Map();
  bindGraph("/g");
  const api = backend();
  api.tineLinks = { identity: vi.fn(), scanKnownGraphs: vi.fn(), take: vi.fn(async () => []), handoff: vi.fn(), subscribe: vi.fn(), inbox: fakeInbox() };
  vi.spyOn(api, "getPage").mockImplementation(async (name) => dto(name) as any);
  vi.spyOn(api, "resolvePage").mockImplementation(async (name) =>
    files.has(key(name)) ? { kind: "existing", id: `journals/${name}.md`, others: [] } : { kind: "absent", id: `journals/${name}.md` });
  vi.spyOn(api, "importAsset").mockImplementation(async (_path, name) => {
    imports.push(`${graphMeta()!.root}:${name}`);
    return `${name}`;
  });
  await bindHost();
  mainPaneRouter.resetTabsToJournals();
  vi.spyOn(api, "pageInventory").mockResolvedValue({ rev: "1", entries: [{
    key: pageIdentityKey(day()), name: day(), is_journal: true, day: 20261016,
    target: { kind: "existing", id: `journals/${day()}.md`, others: [] },
  }] });
  await refreshPageIndex();
  mainPaneRouter.openPage("Other");
});

afterEach(() => { resetStore(); resetPageIndex(); setGraphMeta(null); setToasts([]); delete backend().tineLinks; vi.restoreAllMocks(); });

function share(id: string, text: string, extra: Partial<ShareInboxItem> = {}) {
  inbox.set(id, { id, source: "android", created: Date.UTC(2026, 9, 10, 8, 30), text, resources: [], ...extra });
}

const shared = (text: string, raws = journal()) => raws.filter((raw) => raw.endsWith(`[[quick capture]]: ${text}`));
const errorToasts = () => toasts().filter((toast) => toast.kind === "error");

it("after an Android share, opens its journal and reveals the last captured block in the batch", async () => {
  share("a", "first capture");
  share("b", "last capture");
  await ingestShares();
  expect(route()).toMatchObject({ kind: "page", name: day(), pageKind: "journal" });
  const id = pageByName(day())!.roots.at(-1)!;
  expect(node(id).raw).toContain("last capture");
  const scroller = document.createElement("div");
  const block = document.createElement("div");
  block.className = "ls-block";
  block.dataset.blockId = id;
  block.scrollIntoView = vi.fn();
  scroller.append(block);
  document.body.append(scroller);
  mainPaneRouter.setScrollerElement(scroller);
  try {
    await vi.waitFor(() => expect(block.scrollIntoView).toHaveBeenCalled());
    expect(block.classList.contains("block-flash")).toBe(true);
  } finally { mainPaneRouter.setScrollerElement(null); scroller.remove(); }
});

it.each([true, false])("share navigation respects tab reuse = %s", async (reuse) => {
  setNavReuseTabs(reuse);
  mainPaneRouter.openPage(day(), "journal");
  const journalTab = mainPaneRouter.activeId();
  mainPaneRouter.openPageInNewTab("Other", "page", undefined, true);
  const sourceTab = mainPaneRouter.activeId();
  share("a", "navigate normally");
  await ingestShares();
  expect(route()).toMatchObject({ kind: "page", name: day(), pageKind: "journal" });
  expect(mainPaneRouter.activeId()).toBe(reuse ? journalTab : sourceTab);
  setNavReuseTabs(true);
});

it("kept items and their later retry do not interrupt the current page", async () => {
  share("a", "retry later");
  saveFails = true;
  await ingestShares();
  expect(route()).toMatchObject({ kind: "page", name: "Other" });
  saveFails = false;
  await ingestShares();
  expect(shared("retry later")).toHaveLength(1);
  expect(route()).toMatchObject({ kind: "page", name: "Other" });
});

it("iOS shares saved without bringing Tine forward preserve the current page", async () => {
  share("a", "background capture", { source: "ios" });
  await ingestShares();
  expect(journal().join()).toContain("background capture");
  expect(route()).toMatchObject({ kind: "page", name: "Other" });
});

it("does not navigate if the graph changes during the final acknowledgement", async () => {
  share("a", "old graph");
  backend().tineLinks!.inbox!.commit = async (id) => { inbox.delete(id); resetStore(); bindGraph("/B"); };
  await ingestShares();
  expect(route()).toMatchObject({ kind: "page", name: "Other" });
});

it("the mobile Guide explains the share landing and preserves in-app capture context", () => {
  const guide = readFileSync("crates/tine-core/src/templates/platforms-and-mobile.md", "utf8");
  for (const outcome of ["reveals the shared block", "last item in the batch", "Retries leave your current page open", "In-app quick capture keeps the page you are editing open"]) {
    expect(guide).toContain(outcome);
  }
});

it("appends a shared item at the bottom of today's journal, then removes it", async () => {
  writeFile(day(), ["earlier note"]);
  share("a", "from the share sheet");
  await ingestShares();
  expect(journal()[0]).toBe("earlier note");
  expect(journal().at(-1)).toMatch(/^\*\*\d\d:\d\d\*\* \[\[quick capture\]\]: from the share sheet$/);
  expect(inbox.size).toBe(0);
  expect(toasts().map((toast) => toast.message)).toContain(renderEnglish(CAPTURED_TOAST));
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
  expect(inbox.get("a")?.prepared).toMatchObject({ graph: "/g", day: day(), armed: { before: null }, written: true });
  await restart();
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
  expect(inbox.get("a")?.prepared?.armed).toEqual({ before: "r1" });
  expect(shared("twice")).toHaveLength(1);
  await restart();
  writeFile(day(), ["an edit elsewhere", ...journal()]);
  await ingestShares();
  expect(shared("twice")).toHaveLength(2);
  expect(inbox.size).toBe(0);
});

it("a written item is only acknowledged: a later edit or deletion of its block is the user's", async () => {
  share("a", "edited later");
  commitFails = true;
  await ingestShares();
  await restart();
  writeFile(day(), journal().map((raw) => `${raw} (edited)`));
  commitFails = false;
  const before = saves;
  await ingestShares();
  expect(saves).toBe(before);
  expect(journal()).toHaveLength(1);
  expect(inbox.size).toBe(0);
});

/** Make the `written` marker fail once: the process dies after the journal
 *  flush and before that marker reached disk. */
function crashBeforeWritten() {
  const apiInbox = backend().tineLinks!.inbox!;
  const prepare = apiInbox.prepare;
  apiInbox.prepare = async (id, prepared) => {
    if (prepared.written) { apiInbox.prepare = prepare; throw new Error("crash before the written marker"); }
    await prepare(id, prepared);
  };
}

it("the one duplicate window: a crash after the journal flush and before `written` appends again", async () => {
  share("a", "maybe twice");
  crashBeforeWritten();
  await ingestShares();
  expect(shared("maybe twice")).toHaveLength(1);
  expect(inbox.get("a")?.prepared?.written).toBe(false);
  await restart();
  await ingestShares();
  expect(shared("maybe twice")).toHaveLength(2);
  expect(inbox.size).toBe(0);
});

it("round2: acknowledgement rests on this process's own durable flush, never on what the live store shows", async () => {
  share("a", "keep original");
  crashBeforeWritten(); // the flush reached disk; only the marker failed
  await ingestShares();
  expect(shared("keep original")).toHaveLength(1);
  // An external editor then removes the block; its watcher notification has
  // not arrived, so the live store still shows it. No restart.
  writeFile(day(), []);
  const before = saves;
  await ingestShares();
  // The append did land (its own flush succeeded): the removal is the
  // editor's, and the item is acknowledged without writing again.
  expect(saves).toBe(before);
  expect(journal()).toHaveLength(0);
  expect(inbox.size).toBe(0);
  // In a fresh process with no `written` marker, the same item is appended
  // again, whatever the journal shows (the documented duplicate window). The
  // restart first lets the window read the editor's removal: under the page
  // host a write over a file changed since its Open conflicts instead.
  await restart();
  share("b", "fresh process");
  crashBeforeWritten();
  await ingestShares();
  await restart();
  await ingestShares();
  expect(shared("fresh process")).toHaveLength(2);
});

it("round2: an equal capture after arming must not consume a separate share", async () => {
  share("a", "two operations");
  const apiInbox = backend().tineLinks!.inbox!;
  const prepare = apiInbox.prepare;
  apiInbox.prepare = async (id, prepared) => {
    await prepare(id, prepared);
    if (prepared.armed) { apiInbox.prepare = prepare; throw new Error("killed after durable arm, before insert"); }
  };
  await ingestShares();
  expect(inbox.get("a")?.prepared?.armed).toEqual({ before: null });
  expect(journal()).toHaveLength(0);
  await restart();
  // A different writer's capture, byte-for-byte equal, after our arm.
  writeFile(day(), [inbox.get("a")!.prepared!.markdown!.slice(2)]);
  await ingestShares();
  expect(inbox.size).toBe(0);
  expect(shared("two operations")).toHaveLength(2);
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
  expect(inbox.get("a")?.prepared?.written).toBe(true);
  // A is safely on disk; only the inbox commit failed. Bind a new empty graph B.
  await restart("/B");
  setToasts([]);
  commitFails = false;
  const before = saves;
  await ingestShares();
  // `written`: only acknowledged, nothing is written into B.
  expect(saves).toBe(before);
  expect(journal()).toHaveLength(0);
  expect(inbox.size).toBe(0);
  expect(toasts().filter((toast) => toast.kind === "error")).toHaveLength(0);
  expect(route()).toMatchObject({ kind: "page", name: "Other" });
  await restart("/g");
  expect(shared("belongs in A")).toHaveLength(1);
});

it("review: an unwritten share bound to A waits silently in B and is appended once back in A", async () => {
  share("a", "belongs in A");
  saveFails = true;
  await ingestShares();
  await restart("/B");
  setToasts([]);
  saveFails = false;
  await ingestShares();
  await ingestShares();
  expect(journal()).toHaveLength(0);
  expect(inbox.has("a")).toBe(true);
  expect(toasts()).toHaveLength(0);
  expect(route()).toMatchObject({ kind: "page", name: "Other" });
  await restart("/g");
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
  await restart(); commitFails = false;
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
  await restart("/B");
  commitFails = false;
  await ingestShares();
  expect(imports).toHaveLength(1); // nothing imported into B
  await restart("/g");
  await ingestShares();
  expect(imports).toEqual([expect.stringMatching(/^\/g:/)]);
  expect(journal().filter((raw) => raw.includes("p.png"))).toHaveLength(1);
  expect(inbox.size).toBe(0);
});
