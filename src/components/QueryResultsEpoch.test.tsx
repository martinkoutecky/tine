import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { applyGraphChange, blockProperty, insertEmptyChildBlock, resetStore } from "../document";
import { doc, setDoc } from "../document/model";
import { backend } from "../backend";
import { adoptResults, beginResultsChange, captureResultsTicket, resultsSettled, resultsTicketCurrent } from "../queryResultsEpoch";
import { QueryResultTask } from "./QueryResultTask";
import { LiveRefGroup } from "./LiveRefGroup";
import { SheetTable } from "./SheetTable";
import { collapseQueryResults } from "./QueryCollapse";
import { openQueryResultsCopy } from "./queryResultsCopy";
import { closeExportModal, exportModal } from "../ui";
import { setToasts, toasts } from "../toasts";
import type { BlockDto, RefGroup } from "../types";

beforeAll(initParser);
afterEach(() => { resetStore(); closeExportModal(); setToasts([]); vi.restoreAllMocks(); document.body.replaceChildren(); });
const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, marker: raw.startsWith("TODO ") ? "TODO" : undefined, children, collapsed: false });
const group = (blocks: BlockDto[]): RefGroup => ({ page: "Source", kind: "page", path: "pages/Source.md", blocks });
async function load(raw = "TODO Selected", second = false) {
  setDoc({ byId: {
    same: { id: "same", raw, collapsed: false, parent: null, page: "Source", children: ["child"] },
    child: { id: "child", raw: "Child", collapsed: false, parent: "same", page: "Source", children: [] },
    ...(second ? { other: { id: "other", raw: "Other parent", collapsed: false, parent: null, page: "Source", children: ["last"] },
      last: { id: "last", raw: "Last child", collapsed: false, parent: "other", page: "Source", children: [] } } : {}),
  }, pages: [{ id: "pages/Source.md", name: "Source", kind: "page", title: "Source", preBlock: null,
    roots: second ? ["same", "other"] : ["same"], format: "md", readOnly: false, guide: false }], feed: ["Source"], loaded: true });
  await Promise.resolve();
}
function adopt<T>(answer: T): T { expect(adoptResults(answer, captureResultsTicket())).toBe(answer); return answer; }

it("refuses a task click when an external notification arrives during source hydration (R1)", async () => {
  const hit = adopt(block("same", "TODO Selected"));
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => {
    await applyGraphChange({ name: "Unloaded", kind: "page", created: false, removed: false });
    return { id: "pages/Source.md", name: "Source", title: "Source", kind: "page", pre_block: null,
      blocks: [block("same", "TODO Unrelated replacement")] };
  });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryResultTask row={{ id: hit.id, dto: hit, page: "Source", kind: "page", path: "pages/Source.md" }} />, root);
  try {
    root.querySelector('[role="checkbox"]')!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    await vi.waitFor(() => expect(toasts()).toHaveLength(1));
    expect(doc.byId.same.raw).toBe("TODO Unrelated replacement");
  } finally { dispose(); }
});

it("refuses a click after a local structural insert, even when its text still matches", async () => {
  await load();
  const hit = adopt(block("same", "TODO Selected"));
  insertEmptyChildBlock("same", 0);
  const { toggleTaskAtSource } = await import("./SourceTaskCheckbox");
  expect(await toggleTaskAtSource({ uuid: "same", page: "Source", pageKind: "page", path: "pages/Source.md" }, false,
    { id: hit.id, dto: hit, page: "Source", kind: "page", path: "pages/Source.md" })).toBe("stale");
  expect(doc.byId.same.raw).toBe("TODO Selected");
  expect(resultsSettled()).toBe(false); // unpublished local input is not a fresh result boundary
});

it("refuses equal-text replacements in bulk fold and copy (R2)", async () => {
  await load("Repeated heading");
  const groups = adopt([group([block("same", "Repeated heading", [block("old-child", "Selected subtree")])])]);
  await applyGraphChange({ name: "Unloaded", kind: "page", created: false, removed: false });
  await collapseQueryResults(groups, true);
  expect(blockProperty("same", "collapsed")).toBeNull();
  await openQueryResultsCopy(groups);
  expect(exportModal()).toBeNull();
});

it("refuses the whole copy if an external change arrives after an earlier group was read", async () => {
  await load();
  const groups = adopt([group([block("same", "TODO Selected")]), { page: "Later", kind: "page" as const, path: "pages/Later.md", blocks: [block("later", "Later")] }]);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => {
    await applyGraphChange({ name: "Unloaded", kind: "page", created: false, removed: false });
    return { id: "pages/Later.md", name: "Later", title: "Later", kind: "page", pre_block: null, blocks: [block("later", "Later")] };
  });
  await openQueryResultsCopy(groups);
  expect(exportModal()).toBeNull();
  expect(toasts()).toHaveLength(1);
});

it("renders stale List and Table rows from their result snapshot (R3)", async () => {
  await load("TODO [#B] Replacement");
  const hit = block("same", "TODO Selected");
  setDoc("byId", "query", { id: "query", raw: "{{query (task TODO)}}\ntine.columns:: priority", collapsed: false, parent: null, page: "Source", children: [] });
  await Promise.resolve();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><LiveRefGroup {...group([hit])} surface="query" eager />
    <SheetTable ownerId="query" rowSource="query" groups={[group([hit])]} queryDisplay={{ view: { columns: ["priority"] }, apply: () => {} }} /></>, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("Selected"));
    expect(root.textContent).not.toContain("Replacement");
    expect(doc.byId.same.raw).toBe("TODO [#B] Replacement");
    expect(root.querySelector(".sheet-field-cell .block-priority")).toBeNull();
  } finally { dispose(); }
});

it("validates every bulk fold target before the first source write (R4)", async () => {
  await load("Parent\nlogseq.order-list-type:: number\nother:: v", true);
  const groups = adopt([group([block("same", doc.byId.same.raw), block("other", "Other parent")])]);
  await collapseQueryResults(groups, true);
  expect(blockProperty("same", "collapsed")).toBe("true");
  expect(blockProperty("other", "collapsed")).toBe("true");
  expect(toasts()).toHaveLength(0);
});

it("discards reads started before external hydration and during pending hydration", () => {
  const before = captureResultsTicket();
  const finish = beginResultsChange();
  const during = captureResultsTicket();
  expect(adoptResults({}, before)).toBeUndefined();
  expect(adoptResults({}, during)).toBeUndefined();
  finish();
  expect(resultsTicketCurrent(before)).toBe(false);
  expect(resultsTicketCurrent(during)).toBe(false);
  expect(adoptResults({}, captureResultsTicket())).toBeDefined();
});

// L7s6-05: reading more source pages than the working set keeps must not make its own results "changed".
it("folds results spanning more source pages than the working set keeps", async () => {
  const pages = Array.from({ length: 120 }, (_, i) => `S${i}`);
  setDoc({ byId: {}, pages: [], feed: [], loaded: true });
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async (path: string) => {
    const name = /pages\/(.*)\.md/.exec(path)![1];
    return { id: path, name, title: name, kind: "page", format: "md", read_only: false, pre_block: null,
      blocks: [block(`${name}-a`, `Row ${name}`, [block(`${name}-b`, "Child")])] } as never;
  });
  const groups = adopt(pages.map((name) => ({ page: name, kind: "page" as const, path: `pages/${name}.md`, blocks: [block(`${name}-a`, `Row ${name}`, [block(`${name}-b`, "Child")])] })));
  expect(await collapseQueryResults(groups, true)).not.toBeNull();
  expect(toasts()).toEqual([]);
  expect(blockProperty("S0-a", "collapsed")).toBe("true");
  expect(blockProperty("S119-a", "collapsed")).toBe("true");
});

// L7s6-05 review: only a load evicts, so a large fold must evict when it ends.
it("a fold over many source pages leaves the working set at its cap", async () => {
  const pages = Array.from({ length: 120 }, (_, i) => `L${i}`);
  setDoc({ byId: {}, pages: [], feed: [], loaded: true });
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async (path: string) => {
    const name = /pages\/(.*)\.md/.exec(path)![1];
    return { id: path, name, title: name, kind: "page", format: "md", read_only: false, pre_block: null, blocks: [block(`${name}-a`, `Row ${name}`)] } as never;
  });
  const groups = adopt(pages.map((name) => ({ page: name, kind: "page" as const, path: `pages/${name}.md`, blocks: [block(`${name}-a`, `Row ${name}`)] })));
  expect(await collapseQueryResults(groups, true)).not.toBeNull();
  expect(doc.pages.length).toBeLessThanOrEqual(80);
});

// L7s6-05 review: a fold's hold must not read as unsaved work. A clean slot held
// by another file of the same name is replaced, as any load replaces it; a
// positional row then reads as changed, an id:: row folds.
it("folds a result whose page name a different clean file holds", async () => {
  const uuid = "11111111-1111-4111-8111-111111111111";
  const raw = `Row\nid:: ${uuid}`;
  setDoc({ byId: { canon: { id: "canon", raw: "Canonical", collapsed: false, parent: null, page: "Day", children: [] } },
    pages: [{ id: "journals/day.md", name: "Day", kind: "page", title: "Day", preBlock: null, roots: ["canon"], format: "md", readOnly: false, guide: false }],
    feed: [], loaded: true });
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async (path: string) => ({ id: path, name: "Day", title: "Day", kind: "page", format: "md", read_only: false, pre_block: null,
    blocks: [block("disk", raw, [block("disk-kid", "Child")])] }) as never);
  const groups = adopt([{ page: "Day", kind: "page" as const, path: "journals/stray.md", blocks: [block("stale-id", raw, [block("stale-kid", "Child")])] }]);
  expect(await collapseQueryResults(groups, true)).not.toBeNull();
  expect(toasts()).toEqual([]);
  expect(doc.pages[0].id).toBe("journals/stray.md");
  expect(blockProperty(doc.pages[0].roots[0], "collapsed")).toBe("true");
});

it("two results of one page name in different files do not raise an unsaved-work refusal", async () => {
  setDoc({ byId: {}, pages: [], feed: [], loaded: true });
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async (path: string) => {
    const id = path.includes("stray") ? "stray" : "canon";
    return { id: path, name: "Day", title: "Day", kind: "page", format: "md", read_only: false, pre_block: null, blocks: [block(id, "Row", [block(`${id}-kid`, "Child")])] } as never;
  });
  const groups = adopt(["canon", "stray"].map((id) => ({ page: "Day", kind: "page" as const, path: `journals/${id}.md`, blocks: [block(id, "Row", [block(`${id}-kid`, "Child")])] })));
  await collapseQueryResults(groups, true);
  expect(toasts().map((toast) => toast.message).join(" ")).not.toMatch(/unsaved/i);
});
