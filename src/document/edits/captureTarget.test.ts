// A capture writes to the file its page admission chose. The page read that
// admits a capture's destination resolves the name through the graph's one
// resolver (any spelling of a page's name, an alias, a date spelling of a
// journal day) and loads that page under its own name. A capture to a name the
// user typed must land in that page; a computed name (today's journal, the
// exports page) keeps writing to the page of that exact name. Each case runs
// over the browser core (the graph's resolver) and a fake disk, and checks the
// files as the user would find them.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { initParser } from "../../render/parse";
import { backend } from "../../backend";
import { graphReads } from "../../graphForwards";
import { openMockGraph } from "../../mockGraph";
import { installDiskHost } from "../../diskHost.test.support";
import { bindTestHost } from "../host/wiring.test.support";
import { appNow, journalTitle } from "../../journal";
import { withToday } from "../convert";
import { appendQueryBlock, appendToJournalDay, appendToTodayJournal, captureToPage, installAliasDraftRouteHandler, loadFeed, pageByName, resetStore } from "../index";
import { doc } from "../model";
import { loadSingle } from "../workingSet";
import { captureDestination, QUICK_CAPTURE_PAGE } from "../../quickCaptureConfig";
import type { BlockDto, PageDto, PageRead } from "../../types";

beforeAll(() => initParser());

let disk: Map<string, PageDto & { rev: string }>;
let writes: string[];

/** Open `files` as the graph: the core answers every page read, the fake disk holds the files' pages. */
async function graph(files: Record<string, string>) {
  const core = openMockGraph(new Map(Object.entries(files)));
  const reads = graphReads(core.read);
  disk = new Map();
  writes = [];
  for (const path of Object.keys(files)) {
    const { page } = await core.current().page(path);
    if (page) disk.set(path, { ...structuredClone(page), rev: "r0" });
  }
  // The core decides which file a read finds; the page is that file as the disk holds it now.
  const current = (page: PageRead | null) => page && disk.has(page.id) ? structuredClone(disk.get(page.id)!) as PageRead : page;
  const api = backend();
  vi.spyOn(api, "getPage").mockImplementation(async (name, kind) => current(await reads.getPage(name, kind)));
  vi.spyOn(api, "resolvePage").mockImplementation(reads.resolvePage);
  vi.spyOn(api, "getPageByPath").mockImplementation(async (path, source) => current(await reads.getPageByPath(path, source)));
  // A bound graph's window has a page open (the app captures only after the graph loaded).
  expect(loadSingle({ name: "Elsewhere", kind: "page", title: "Elsewhere", id: "pages/Elsewhere.md", pre_block: null, blocks: [] })).toBeNull();
  let rev = 0;
  installDiskHost(await bindTestHost(), {
    read: (path) => disk.get(path) ?? null,
    write: (path, dto) => {
      writes.push(path);
      disk.set(path, { ...structuredClone(dto), rev: `r${++rev}` });
      return `r${rev}`;
    },
  });
}

const raws = (blocks: readonly BlockDto[]): string[] => blocks.flatMap((b) => [b.raw, ...raws(b.children)]);
/** Every block text of the file at `path` as it is on disk. */
const onDisk = (path: string) => raws(disk.get(path)?.blocks ?? []);

beforeEach(() => resetStore());
afterEach(() => { installAliasDraftRouteHandler(() => {}); vi.restoreAllMocks(); });

it("a capture to another spelling of a page's name lands in that page's file", async () => {
  await graph({ "pages/Inbox.md": "- earlier\n" });
  const names: string[] = [];
  expect(await captureToPage("inbox", "- typed lowercase", (name) => names.push(name))).toBe(true);
  expect(onDisk("pages/Inbox.md")).toEqual(["earlier", "typed lowercase"]);
  expect(writes).toEqual(["pages/Inbox.md"]);
  expect(names, "the capture reports the page it wrote").toEqual(["Inbox"]);
  expect(pageByName("inbox")).toBeUndefined();
});

// With journals off and no default page set, an untitled capture goes to the
// built-in page name, by the same route as a typed title.
it("an untitled capture with journals off lands in the page a link to the built-in name opens", async () => {
  await graph({ "pages/Quick Capture.md": "- earlier\n" });
  const destination = captureDestination("", { enableJournals: false });
  expect(destination).toEqual({ kind: "page", name: QUICK_CAPTURE_PAGE });
  const names: string[] = [];
  expect(await captureToPage(QUICK_CAPTURE_PAGE, "- untitled", (name) => names.push(name))).toBe(true);
  expect(onDisk("pages/Quick Capture.md")).toEqual(["earlier", "untitled"]);
  expect(writes).toEqual(["pages/Quick Capture.md"]);
  expect(names).toEqual(["Quick Capture"]);
});

it("a capture to a date spelling of a journal day lands in that day's journal file", async () => {
  await graph({ "journals/2026_11_02.md": "- morning\n" });
  expect(await captureToPage("2026-11-02", "- by date")).toBe(true);
  expect(onDisk("journals/2026_11_02.md")).toEqual(["morning", "by date"]);
  expect(writes).toEqual(["journals/2026_11_02.md"]);
});

it("a capture to an alias lands in the page that declares it", async () => {
  await graph({ "pages/Inbox.md": "alias:: In Tray\n\n- earlier\n" });
  expect(await captureToPage("In Tray", "- via alias")).toBe(true);
  expect(onDisk("pages/Inbox.md")).toEqual(["earlier", "via alias"]);
  expect(writes).toEqual(["pages/Inbox.md"]);
});

it("a capture to another spelling of a page holding only properties lands in that page's file", async () => {
  await graph({ "pages/Inbox.md": "tags:: tray\n" });
  expect(await captureToPage("inbox", "- first entry")).toBe(true);
  expect(onDisk("pages/Inbox.md")).toEqual(["first entry"]);
  expect(disk.get("pages/Inbox.md")!.pre_block).toContain("tags:: tray");
  expect(writes).toEqual(["pages/Inbox.md"]);
});

it("a capture to the page's own spelling lands in its file", async () => {
  await graph({ "pages/Inbox.md": "- earlier\n" });
  expect(await captureToPage("Inbox", "- exact")).toBe(true);
  expect(onDisk("pages/Inbox.md")).toEqual(["earlier", "exact"]);
});

it("a quick-share definition is not written when another file claims the exports page's name", async () => {
  // The exports page is named by its marker row (`exports`); the resolver ranks the
  // unmarked `Exports.md` first. Writing there would hide the definition from the listing.
  await graph({ "pages/exports.md": "tine/exports:: true\n\n- {{tine-query (property tine.export)}}\n", "pages/Exports.md": "- unrelated\n" });
  const before = structuredClone([...disk.entries()]);
  expect(await appendQueryBlock("exports", "{{tine-query @page and name = 'A'}}", [["tine.export", "a-site"]])).toBe(false);
  expect(writes).toEqual([]);
  expect([...disk.entries()]).toEqual(before);
  expect(JSON.stringify(doc.byId)).not.toContain("a-site");
});

const today = () => journalTitle(appNow());
const aliasOfToday = () => ({ "pages/Meeting.md": `alias:: [[${today()}]]\n\n- agenda\n` });

it("an untitled capture with today's journal shown goes to the page whose alias is today, through the alias landing", async () => {
  await graph(aliasOfToday());
  installAliasDraftRouteHandler(() => {});
  expect(loadFeed(withToday([]))).toBe("published");
  await appendToTodayJournal("- note for today");
  // The landing moves the whole draft, the feed's empty block for today included.
  await vi.waitFor(() => expect(onDisk("pages/Meeting.md")).toEqual(["agenda", "", "note for today"]));
  expect([...disk.keys()]).toEqual(["pages/Meeting.md"]);
});

it("an untitled capture or a share into today is refused, text kept, when only an alias declares today and its journal is not shown", async () => {
  await graph(aliasOfToday());
  const before = structuredClone(disk.get("pages/Meeting.md"));
  expect(await appendToTodayJournal("- kept in the window")).toBe(false);
  const arm = vi.fn(async () => true);
  expect(await appendToJournalDay(today(), "- kept in the inbox", arm)).toBe(false);
  expect(arm).not.toHaveBeenCalled();
  expect(writes).toEqual([]);
  expect(disk.get("pages/Meeting.md")).toEqual(before);
  expect(JSON.stringify(doc.byId)).not.toMatch(/kept in the/);
});

// Setup check: the fake disk reads what the core parsed.
it("reads the graph's files through the core", async () => {
  await graph({ "pages/Inbox.md": "- earlier\n" });
  const page = await backend().getPage("inbox", "page") as PageRead;
  expect(page.name).toBe("Inbox");
  expect(page.id).toBe("pages/Inbox.md");
});
