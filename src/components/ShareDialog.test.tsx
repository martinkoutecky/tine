import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ShareDialog, SHARE_REVIEW_DEBOUNCE_MS } from "./ShareDialog";
import { closeQueryExport, queryExportRequest } from "../ui";
import { closeQuickShare, closeShareDialog, openShareDialog, queryExportAfter, quickShareDefinition, quickShareRequest, shareTarget } from "../shareState";
import { clearTransientLayersForTest } from "../transientLayers";
import { toasts, setToasts } from "../toasts";

const doc = vi.hoisted(() => ({
  appended: [] as [string, string, unknown][],
  properties: new Map<string, string>(),
  set: [] as [string, string, string | null][],
  writable: true,
}));
vi.mock("../document", async (original) => ({
  ...await original<typeof import("../document")>(),
  appendQueryBlock: vi.fn(async (page: string, query: string, properties: unknown) => { doc.appended.push([page, query, properties]); return true; }),
  blockProperty: (id: string, key: string) => doc.properties.get(`${id}\0${key}`) ?? null,
  blockWritable: () => doc.writable,
  setBlockProperty: (id: string, key: string, value: string | null) => { doc.set.push([id, key, value]); },
}));
vi.mock("../exportsPage", async (original) => ({
  ...await original<typeof import("../exportsPage")>(),
  ensureExportsPage: vi.fn(async () => "Exports"),
}));
vi.mock("../owned", async (orig) => ({ ...await orig<typeof import("../owned")>(), graphOwner: () => () => true }));
// The builder and the preview are exercised by their own tests; here they only show what they were given.
vi.mock("./QueryBuilder", () => ({ QueryBuilder: (props: { session: () => { query: { anchor: string } } | undefined; onChange: (next: unknown) => void }) =>
  <div data-testid="builder" data-anchor={props.session()?.query.anchor ?? "none"}>
    <button type="button" data-testid="edit" onClick={() => props.onChange({ query: { anchor: "page", filter: { kind: "true" }, source: { kind: "builder" } }, view: {} })}>edit</button>
  </div> }));
vi.mock("./QueryLivePreview", () => ({ QueryLivePreview: () => <div data-testid="preview" /> }));

const tick = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => { for (let i = 0; i < 6; i++) await tick(); };
let dispose: (() => void) | undefined;
const spyPlan = () => vi.spyOn(backend(), "publishQueryPlan");
let plan: ReturnType<typeof spyPlan>;

const planOf = (extra: Record<string, unknown> = {}) => ({ anchor: "page", rowCount: 1,
  pages: [{ name: "Project", path: "pages/Project.md", journal: false }], leftOut: [], selectsHostPage: false,
  folder: "project", fingerprint: "reviewed", path: "/graph/published-queries/project", exists: false, suggestedFolder: null, ...extra });

beforeEach(() => {
  doc.appended.length = 0; doc.properties.clear(); doc.set.length = 0; doc.writable = true;
  vi.spyOn(backend(), "printQuery").mockImplementation(async (query) =>
    query.anchor === "page" && query.filter.kind === "leaf" && query.filter.leaf.kind === "attr" && query.filter.leaf.value.kind === "text"
      ? `@page and name = '${query.filter.leaf.value.text}'` : "@page");
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (source: string) => {
    const named = /name = '([^']*)'/.exec(source);
    return { query: named
      ? { anchor: "page", filter: { kind: "leaf", leaf: { kind: "attr", attr: "name", op: "eq", value: { kind: "text", text: named[1] } } }, source: { kind: "tql", original: source } }
      : { anchor: "block", filter: { kind: "true" }, source: { kind: "og", original: source } }, view: {} } as never;
  });
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false as never);
  plan = spyPlan().mockResolvedValue(planOf() as never);
});
afterEach(() => {
  dispose?.(); dispose = undefined; document.body.innerHTML = "";
  closeShareDialog(); closeQuickShare(); closeQueryExport(); clearTransientLayersForTest(); setToasts([]); vi.restoreAllMocks();
});

async function open(target: Parameters<typeof openShareDialog>[0]) {
  const root = document.createElement("div"); document.body.append(root);
  openShareDialog(target);
  dispose = render(() => <ShareDialog target={shareTarget} />, root);
  await settle();
  return root;
}
const radios = (root: HTMLElement, name: string) => [...root.querySelectorAll<HTMLInputElement>(`input[name="${name}"]`)];
const exportButton = (root: HTMLElement) => root.querySelector<HTMLButtonElement>(".share-export")!;
const blockTarget = (macro: string, extra: Record<string, unknown> = {}) => ({
  macro, query: { argument: macro.replace(/^\{\{\S+ |\}\}$/g, ""), dialect: "macro_tql", properties: [["tine.view", "table"]],
    currentPage: "Notes", hostBlockId: "block-1", name: "My query", ...extra } as never });

it("offers three choices: this page, its neighbours (disabled, coming), and a query starting at this page", async () => {
  const root = await open({ page: "Project" });
  const [page, neighbours, query] = radios(root, "share-what");
  expect(page.checked, "a page opens as This page").toBe(true);
  expect(neighbours.disabled).toBe(true);
  expect(root.textContent).toContain("Coming with graph queries.");
  expect(query.checked).toBe(false);
  expect(root.querySelector('[data-testid="builder"]'), "no builder until a query is chosen").toBeNull();
  query.click(); await settle();
  expect(root.querySelector('[data-testid="builder"]')).not.toBeNull();
  expect(root.querySelector('[data-testid="preview"]')).not.toBeNull();
  // The query is seeded with the selector for this page, as the engine prints it.
  expect(root.querySelector('[data-testid="builder"]')!.getAttribute("data-anchor")).toBe("page");
  expect(radios(root, "share-where").map((r) => [r.value, r.disabled])).toEqual([["folder", false], ["link", true]]);
  expect(root.textContent).toContain("Coming soon");
});

it("reviews the exact selector for this page: pages, what is left out, the host-page note, and the off-by-default query-text box", async () => {
  plan.mockResolvedValue(planOf({
    leftOut: [{ name: "Diary", path: "pages/Diary.md", reason: "private" }, { name: "Exports", path: "pages/Exports.md", reason: "exportsPage" }],
    selectsHostPage: true,
  }) as never);
  const root = await open({ page: "Project" });
  expect(plan.mock.calls[0][0]).toMatchObject({ argument: "@page and name = 'Project'", dialect: "macro_tql", currentPage: "Project", name: "Project" });
  const text = root.textContent ?? "";
  expect(text).toContain("1 page will be shared:");
  expect(text).toContain("Project");
  expect(text).toContain("2 selected pages are not included");
  expect(text).toContain("Diary marked public:: false");
  expect(text).toContain("Exports the exports page is never exported");
  expect(text).toContain("This includes the page the query is written on.");
  const include = root.querySelector<HTMLInputElement>(".query-export-include-query input")!;
  expect(include.checked).toBe(false);
});

it("hands the reviewed request to the folder export, with the query text only when ticked, and saves the definition on the exports page", async () => {
  const root = await open({ page: "Project" });
  root.querySelector<HTMLInputElement>(".query-export-include-query input")!.click();
  exportButton(root).click(); await settle();
  expect(shareTarget(), "the share dialog is gone").toBeNull();
  expect(queryExportRequest()).toMatchObject({ argument: "@page and name = 'Project'", dialect: "macro_tql", includeQueryText: true, properties: [] });
  // The folder export reports the folder it wrote; only then is the definition saved.
  expect(doc.appended).toEqual([]);
  await queryExportAfter(queryExportRequest()!)!({ folder: "project-site" });
  expect(doc.appended).toEqual([["Exports", "{{tine-query @page and name = 'Project'}}", [["export", "project-site"]]]]);
  expect(toasts().map((toast) => String(JSON.stringify(toast)))).toEqual([expect.stringContaining("Saved the export on")]);
});

it("seeds a query block's own text, preselects the query, and puts the folder on that block", async () => {
  const root = await open(blockTarget("{{tine-query @block and #todo}}"));
  const [page, , query] = radios(root, "share-what");
  expect([page.checked, query.checked]).toEqual([false, true]);
  expect(plan.mock.calls[0][0]).toMatchObject({ hostBlockId: "block-1", properties: [["tine.view", "table"]], name: "My query" });
  exportButton(root).click(); await settle();
  expect(queryExportRequest()).toMatchObject({ hostBlockId: "block-1", currentPage: "Notes" });
  await queryExportAfter(queryExportRequest()!)!({ folder: "todo-site" });
  expect(doc.set).toEqual([["block-1", "export", "todo-site"]]);
  expect(doc.appended, "no second block is written").toEqual([]);
});

it("treats an edited query as a new definition on the exports page, not a change to the block it started from", async () => {
  const root = await open(blockTarget("{{tine-query @block and #todo}}"));
  root.querySelector<HTMLButtonElement>('[data-testid="edit"]')!.click(); await settle();
  await new Promise((r) => setTimeout(r, SHARE_REVIEW_DEBOUNCE_MS + 50)); await settle();
  exportButton(root).click(); await settle();
  expect(queryExportRequest()).toMatchObject({ argument: "@page", hostBlockId: null, properties: [] });
  await queryExportAfter(queryExportRequest()!)!({ folder: "edited" });
  expect(doc.set).toEqual([]);
  expect(doc.appended).toEqual([["Exports", "{{tine-query @page}}", [["export", "edited"]]]]);
});

it("re-opening a definition shows the same selection: a page's selector opens as This page, and already-exported shows", async () => {
  doc.properties.set("block-1\0export", "project-site");
  const root = await open(blockTarget("{{tine-query @page and name = 'Project'}}"));
  const [page, , query] = radios(root, "share-what");
  expect([page.checked, query.checked]).toEqual([true, false]);
  expect(root.textContent).toContain("Shares only Project.");
  expect(root.textContent).toContain("Already exported to project-site.");
  expect(root.textContent).toContain("records the folder on this query block");
  // Unchanged, so the destination still goes onto the definition block itself.
  exportButton(root).click(); await settle();
  await queryExportAfter(queryExportRequest()!)!({ folder: "project-site-2" });
  expect(doc.set).toEqual([["block-1", "export", "project-site-2"]]);
});

it("refuses to export a selection with no pages, and shows the plan's refusal", async () => {
  plan.mockResolvedValue(planOf({ pages: [] }) as never);
  let root = await open({ page: "Project" });
  expect(exportButton(root).disabled).toBe(true);
  dispose!(); document.body.innerHTML = ""; closeShareDialog();
  plan.mockRejectedValue(new Error("every selected page is left out"));
  root = await open({ page: "Project" });
  expect(root.querySelector('[role="alert"]')?.textContent).toContain("every selected page is left out");
  expect(exportButton(root).disabled).toBe(true);
});

it("tells the user when the folder was written but the definition could not be saved", async () => {
  const { appendQueryBlock } = await import("../document");
  vi.mocked(appendQueryBlock).mockResolvedValueOnce(false);
  const root = await open({ page: "Project" });
  exportButton(root).click(); await settle();
  await queryExportAfter(queryExportRequest()!)!({ folder: "project-site" });
  expect(toasts().map((toast) => JSON.stringify(toast)).join("\n")).toContain("saving its definition failed");
});

it("closes on Escape through the transient layer and traps Tab inside the dialog", async () => {
  const root = await open({ page: "Project" });
  const dialog = root.querySelector<HTMLElement>('[role="dialog"]')!;
  expect(dialog.getAttribute("aria-modal")).toBe("true");
  const { dismissTopTransient } = await import("../transientLayers");
  expect(dismissTopTransient("escape")).toBe(true);
  expect(shareTarget()).toBeNull();
});

// Quick share (DESIGN §3.9): Link is offered only where quick share is on, and hands the reviewed request to the
// quick share dialog. Nothing is written to the graph before the link exists: the quick share dialog records the
// created share (`tine.share::`, D-20261018-Q1) where the share dialog tells it to.
it("offers Link only where quick share is on, and hands the reviewed request to the quick share dialog", async () => {
  vi.spyOn(backend(), "quickShareList").mockResolvedValue({ enabled: true, account: null, shares: [], error: null });
  const root = await open({ page: "Project" });
  const link = radios(root, "share-where").find((radio) => radio.value === "link")!;
  expect(link.disabled).toBe(false);
  expect(root.textContent).toContain("Uploads an encrypted copy to a secret gist in your GitHub account");
  link.click(); await settle();
  expect(exportButton(root).textContent).toBe("Create link…");
  root.querySelector<HTMLInputElement>(".query-export-include-query input")!.click();
  exportButton(root).click(); await settle();
  expect(shareTarget(), "the share dialog is gone").toBeNull();
  expect(quickShareRequest()).toMatchObject({ argument: "@page and name = 'Project'", dialect: "macro_tql", includeQueryText: true, properties: [] });
  expect(queryExportRequest(), "no folder export").toBeNull();
  expect(doc.appended, "no definition is written before the link exists").toEqual([]);
  expect(doc.set).toEqual([]);
  expect(quickShareDefinition(quickShareRequest()!), "This page: a new definition on the exports page")
    .toEqual({ hostBlockId: null, macro: "{{tine-query @page and name = 'Project'}}" });
});

it("from an unchanged query block, the created link is recorded on that block; an edited query is a new definition", async () => {
  vi.spyOn(backend(), "quickShareList").mockResolvedValue({ enabled: true, account: null, shares: [], error: null });
  let root = await open(blockTarget("{{tine-query @block and #todo}}"));
  radios(root, "share-where").find((radio) => radio.value === "link")!.click(); await settle();
  expect(root.textContent).toContain("Tine records it on this query block (tine.share:: property)");
  exportButton(root).click(); await settle();
  expect(quickShareDefinition(quickShareRequest()!)).toEqual({ hostBlockId: "block-1", macro: "{{tine-query @block and #todo}}" });
  dispose!(); dispose = undefined; closeQuickShare(); document.body.innerHTML = "";

  root = await open(blockTarget("{{tine-query @block and #todo}}"));
  radios(root, "share-what")[0].click(); await settle(); // This page: not the block's own query any more
  await new Promise((r) => setTimeout(r, SHARE_REVIEW_DEBOUNCE_MS + 50)); await settle();
  radios(root, "share-where").find((radio) => radio.value === "link")!.click(); await settle();
  expect(root.textContent).toContain("Tine lists it on the Exports page");
  exportButton(root).click(); await settle();
  expect(quickShareDefinition(quickShareRequest()!)).toEqual({ hostBlockId: null, macro: "{{tine-query @page and name = 'Notes'}}" });
});
