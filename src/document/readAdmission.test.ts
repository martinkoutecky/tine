import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { bindingOwner } from "../owned";
import { doc, pageByName } from "./model";
import { admitRead } from "./readAdmission";
import { ensurePageLoaded, loadFeed, loadSingle, resetStore, installPageContent } from "./workingSet";
import { baseRevFor } from "./host/wiring";
import type { PageDto } from "../types";
import { flushPage, hostHolds } from "./host/wiring";
import { setRaw } from "./edits/blocks";
import { bindTestHost } from "./host/wiring.test.support";
import { answerOpensFromDocument } from "./host/documentHost.test.support";
import { invalidatePageRead } from "./readAdmission";
import { forgetPage } from "./workingSet";
import { pageIdentityKey } from "../pageIdentity";
import { applyGraphChangesBulk } from "./external";

beforeAll(() => initParser());
afterEach(() => { resetStore(); vi.restoreAllMocks(); });
const page = (text: string): PageDto & { id: string } => ({ name: "Fresh", title: "Fresh", kind: "page", id: "pages/Fresh.md",
  rev: text, pre_block: null, blocks: [{ id: "fresh-block", raw: text, collapsed: false, children: [] }] });
const shown = () => pageByName("Fresh")?.roots.map(id => doc.byId[id].raw);
const loaders = {
  satellite: (dto: PageDto) => ensurePageLoaded(dto),
  single: (dto: PageDto) => loadSingle(dto),
  feed: (dto: PageDto) => loadFeed([dto]),
  retained: (dto: PageDto) => loadFeed([dto], { preserveExisting: true }),
};

function asynchronousInstallationIsRejected() {
  // @ts-expect-error admission and installation must share one synchronous turn
  return admitRead(bindingOwner(), async () => page("A"), dto => [dto.name], async dto => ensurePageLoaded(dto));
}
void asynchronousInstallationIsRejected;

for (const [name, load] of Object.entries(loaders)) it(`${name}: a delayed A read cannot replace saved B after release`, async () => {
  ensurePageLoaded(page("A"));
  const host = await bindTestHost();
  answerOpensFromDocument(host);
  let deliver!: (dto: PageDto) => void;
  const read = vi.fn<() => Promise<PageDto>>()
    .mockImplementationOnce(() => new Promise(resolve => { deliver = resolve; }))
    .mockResolvedValue(page("B"));
  const request = admitRead(bindingOwner(), read, dto => [dto.name], load);
  setRaw("fresh-block", "B", { timetracking: false });
  expect(await flushPage("Fresh")).toBe(true);
  await vi.waitFor(() => expect(hostHolds("Fresh")).toBe(false));
  deliver(page("A"));
  await request;
  expect(shown()).toEqual(["B"]);
  expect(baseRevFor("Fresh")).toBe("B");
  expect(read).toHaveBeenCalledTimes(2);
});

it("drops a second invalidated read without a retry loop", async () => {
  ensurePageLoaded(page("B"));
  const read = vi.fn(async () => { invalidatePageRead("Fresh"); return page("A"); });
  expect(await admitRead(bindingOwner(), read, dto => [dto.name], ensurePageLoaded)).toEqual({ kind: "stale" });
  expect(shown()).toEqual(["B"]);
  expect(read).toHaveBeenCalledTimes(2);
});

it("does not reread or install after the surface retires", async () => {
  ensurePageLoaded(page("A"));
  let live = true;
  const read = vi.fn(async () => { live = false; return page("B"); });
  expect(await admitRead(bindingOwner(() => live), read, dto => [dto.name], ensurePageLoaded)).toEqual({ kind: "stale" });
  expect(shown()).toEqual(["A"]);
  expect(read).toHaveBeenCalledOnce();
});

it("rechecks at a queued commit and after retirement of a loaded page", async () => {
  ensurePageLoaded(page("A"));
  let commit!: () => void;
  const read = vi.fn(async () => page("B"));
  let queued = 0;
  const result = admitRead(bindingOwner(), read, dto => [dto.name], ensurePageLoaded, run => {
    if (queued++) return Promise.resolve(run());
    return new Promise(resolve => { commit = () => resolve(run()); });
  });
  await vi.waitFor(() => expect(commit).toBeDefined());
  forgetPage("Fresh");
  commit();
  await result;
  expect(shown()).toEqual(["B"]);
  expect(read).toHaveBeenCalledTimes(2);
});

it("an unrelated page publication does not cause an extra read", async () => {
  const read = vi.fn(async () => { invalidatePageRead("Elsewhere"); return page("A"); });
  await admitRead(bindingOwner(), read, dto => [dto.name], ensurePageLoaded);
  expect(shown()).toEqual(["A"]);
  expect(read).toHaveBeenCalledOnce();
});

it("admits an external return from A to B to A without ordering content hashes", async () => {
  ensurePageLoaded(page("A"));
  installPageContent("Fresh", page("B"));
  await admitRead(bindingOwner(), async () => page("A"), dto => [dto.name], ensurePageLoaded);
  expect(shown()).toEqual(["A"]);
  expect(baseRevFor("Fresh")).toBe("A");
});

it("uses a publication's native key without normalizing its boundary slashes again", async () => {
  const name = "//Fresh//";
  let deliver!: (dto: PageDto) => void;
  const read = vi.fn<() => Promise<PageDto>>()
    .mockImplementationOnce(() => new Promise(resolve => { deliver = resolve; }))
    .mockResolvedValue({ ...page("B"), name });
  const request = admitRead(bindingOwner(), read, dto => [dto.name], ensurePageLoaded);
  await applyGraphChangesBulk({ changes: [], answers: { rev: "4001", inventoryChanged: false, blockRefCounts: {},
    summary: { rev: "4001", pages: [pageIdentityKey(name)], blocks: [], linksTo: [], props: {}, inventoryChanged: false, full: false } } });
  deliver({ ...page("A"), name });
  await request;
  expect(doc.byId[pageByName(name)!.roots[0]].raw).toBe("B");
  expect(read).toHaveBeenCalledTimes(2);
});
