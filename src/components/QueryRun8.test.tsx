import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { produce } from "solid-js/store";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { applyGraphChange, blockProperty, resetStore } from "../document";
import { doc, setDoc } from "../document/model";
import { adoptResults, beginResultsChange, captureResultsTicket, installResultsPendingGate } from "../queryResultsEpoch";
import { resolveResultRow } from "./queryResultIdentity";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { DatePicker } from "./DatePicker";
import { ContextMenu } from "./ContextMenu";
import { closeDatePicker, datePicker, setContextMenu } from "../ui";
import { setToasts, toasts } from "../toasts";
import { createMembershipResource, groupsContainEdit } from "../resultMembership";
import type { BlockDto, RefGroup } from "../types";

const host = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? () => {} : host.actual?.[key as string],
  has: () => true,
}));
beforeAll(async () => { host.actual = await vi.importActual("../document/host/wiring"); await initParser(); });
const disposals: (() => void)[] = [];
afterEach(() => { disposals.splice(0).forEach(fn => fn()); resetStore(); closeDatePicker(); setContextMenu(null); setToasts([]); document.body.replaceChildren(); });
const dto = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const page = (name = "Source", roots = ["same"]) => ({ id: `pages/${name}.md`, name, kind: "page" as const, title: name, preBlock: null, roots, format: "md" as const, readOnly: false, guide: false });
async function fixture() {
  setDoc({ byId: {
    same: { id: "same", raw: "Selected\nowner:: Original\nstatus:: todo\ndue:: 2026-10-14\nSCHEDULED: <2026-10-14 Wed>\nDEADLINE: <2026-10-15 Thu>", collapsed: false, parent: null, page: "Source", children: [] },
    query: { id: "query", raw: "Query\ntine.fields:: owner=text;status=enum:todo,doing;due=date", collapsed: false, parent: null, page: "Source", children: [] },
  }, pages: [page("Source", ["same", "query"])], feed: ["Source"], loaded: true });
  await Promise.resolve();
  const groups: RefGroup[] = [{ page: "Source", kind: "page", path: "pages/Source.md", blocks: [dto("same", doc.byId.same.raw)] }];
  expect(adoptResults(groups, captureResultsTicket())).toBe(groups);
  return groups;
}
function mount(view: () => import("solid-js").JSX.Element) {
  const root = document.createElement("div"); document.body.append(root); disposals.push(render(view, root)); return root;
}
function cell(root: HTMLElement, field: string) { return root.querySelector(`[data-sheet-column-id="${field}"][data-block-id="same"]`)!; }
function key(input: HTMLInputElement, name: string) { input.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true })); }

it.each(["Done", "Clear"])("R8-1: an open planning picker refuses %s after source replacement and keeps its draft", async action => {
  const groups = await fixture();
  const root = mount(() => <><SheetTable ownerId="query" rowSource="query" groups={groups} queryDisplay={{ view: { columns: ["scheduled"] }, apply: () => {} }} /><DatePicker /></>);
  (cell(root, "scheduled").querySelector(".date-chip") as HTMLElement).click();
  expect(datePicker()).not.toBeNull();
  (root.querySelector('[data-day="20"]') as HTMLButtonElement).click();
  const finish = beginResultsChange();
  setDoc("byId", "same", { ...doc.byId.same, raw: "Replacement\nSCHEDULED: <2026-10-14 Wed>" });
  finish(); await Promise.resolve();
  const before = doc.byId.same.raw;
  [...root.querySelectorAll<HTMLButtonElement>(".dp-btn")].find(button => button.textContent === action)!.click();
  expect(doc.byId.same.raw).toBe(before);
  expect(datePicker()).not.toBeNull();
  expect(root.querySelector(".dp-cell.selected")?.getAttribute("data-day")).toBe("20");
  expect(toasts()).toHaveLength(1);
});

it("R8-1: an open enum menu revalidates the result before writing", async () => {
  const groups = await fixture();
  const root = mount(() => <><SheetTable ownerId="query" rowSource="query" groups={groups} queryDisplay={{ view: { columns: ["status"] }, apply: () => {} }} /><ContextMenu /></>);
  (cell(root, "prop:status").querySelector(".sheet-tag-chip") as HTMLElement).click();
  beginResultsChange()();
  [...document.querySelectorAll<HTMLElement>(".ctx-item")].find(button => button.textContent?.trim() === "doing")!.click();
  expect(blockProperty("same", "status")).toBe("todo");
  expect(toasts()).toHaveLength(1);
});

it("R8-1: Board deadline popup guards the eventual write", async () => {
  const groups = await fixture();
  const root = mount(() => <><SheetBoard ownerId="query" rowSource="query" groupBy="state" groups={groups} /><DatePicker /></>);
  (root.querySelector('.sheet-board-card .date-chip.deadline') as HTMLElement).click();
  expect(datePicker()?.which).toBe("deadline");
  beginResultsChange()();
  [...root.querySelectorAll<HTMLButtonElement>(".dp-btn")].find(button => button.textContent === "Clear")!.click();
  expect(doc.byId.same.raw).toContain("DEADLINE:");
  expect(datePicker()).not.toBeNull();
});

it("R8-1: Table typed-date popup guards the immediate day-click write", async () => {
  const groups = await fixture();
  setDoc("byId", "same", "raw", "Selected\ndue:: 2026-10-14");
  groups[0].blocks[0].raw = doc.byId.same.raw;
  const root = mount(() => <><SheetTable ownerId="query" rowSource="query" groups={groups} queryDisplay={{ view: { columns: ["due"] }, apply: () => {} }} /><DatePicker /></>);
  (cell(root, "prop:due").querySelector(".date-chip") as HTMLElement).click();
  beginResultsChange()();
  (root.querySelector('[data-day="20"]') as HTMLButtonElement).click();
  expect(blockProperty("same", "due")).toBe("2026-10-14");
  expect(datePicker()).not.toBeNull();
});

it.each(["Enter", "Tab", "blur"])("R8-2: refused %s retains the typed draft across result replacement", async action => {
  const initial = await fixture(); const [groups, setGroups] = createSignal(initial);
  const root = mount(() => <SheetTable ownerId="query" rowSource="query" groups={groups()} queryDisplay={{ view: { columns: ["owner"] }, apply: () => {} }} />);
  cell(root, "prop:owner").dispatchEvent(new MouseEvent("dblclick", { bubbles: true, button: 0 }));
  const input = root.querySelector<HTMLInputElement>(".sheet-prop-input")!;
  input.value = "Martin"; input.dispatchEvent(new Event("input", { bubbles: true }));
  const finish = beginResultsChange();
  if (action === "blur") input.dispatchEvent(new FocusEvent("blur")); else key(input, action);
  expect(root.querySelector<HTMLInputElement>(".sheet-prop-input")?.value).toBe("Martin");
  expect(blockProperty("same", "owner")).toBe("Original");
  setDoc(produce(state => { delete state.byId.same; state.byId.same = { id: "same", raw: "Replacement\nowner:: Replacement", page: "Source", parent: null, children: [], collapsed: false }; }));
  finish(); await Promise.resolve();
  setGroups([]);
  expect(root.querySelector<HTMLInputElement>(".sheet-prop-input")?.value).toBe("Martin");
  key(root.querySelector<HTMLInputElement>(".sheet-prop-input")!, "Enter");
  expect(blockProperty("same", "owner")).toBe("Replacement");
  expect(root.querySelector<HTMLInputElement>(".sheet-prop-input")?.value).toBe("Martin");
});

it("R8-2: an unchanged owned source can commit its existing draft after unrelated settlement", async () => {
  const groups = await fixture();
  const root = mount(() => <SheetTable ownerId="query" rowSource="query" groups={groups} queryDisplay={{ view: { columns: ["owner"] }, apply: () => {} }} />);
  cell(root, "prop:owner").dispatchEvent(new MouseEvent("dblclick", { bubbles: true, button: 0 }));
  const input = root.querySelector<HTMLInputElement>(".sheet-prop-input")!;
  input.value = "Martin"; input.dispatchEvent(new Event("input", { bubbles: true }));
  beginResultsChange()(); key(input, "Enter");
  expect(blockProperty("same", "owner")).toBe("Martin");
  expect(root.querySelector(".sheet-prop-input")).toBeNull();
});

it("R8-2: a mounted query keeps its Table draft through an epoch and reruns on cancellation", async () => {
  const groups = await fixture(); const load = vi.fn(async () => groups);
  const root = mount(() => {
    const [answer] = createMembershipResource(() => "question", () => "scope", load, groupsContainEdit, () => true, true);
    return <SheetTable ownerId="query" rowSource="query" groups={answer() ?? []} queryDisplay={{ view: { columns: ["owner"] }, apply: () => {} }} />;
  });
  await vi.waitFor(() => expect(cell(root, "prop:owner")).not.toBeNull());
  cell(root, "prop:owner").dispatchEvent(new MouseEvent("dblclick", { bubbles: true, button: 0 }));
  const input = root.querySelector<HTMLInputElement>(".sheet-prop-input")!;
  input.value = "Martin"; input.dispatchEvent(new Event("input", { bubbles: true }));
  load.mockResolvedValue([]); beginResultsChange()();
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(root.querySelector<HTMLInputElement>(".sheet-prop-input")?.value).toBe("Martin");
  expect(load).toHaveBeenCalledTimes(1);
  key(input, "Escape");
  await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2));
  expect(root.querySelector(".sheet-prop-input")).toBeNull();
});

it("R8-2: a writer refusal on a now read-only page keeps the draft", async () => {
  const groups = await fixture();
  const root = mount(() => <SheetTable ownerId="query" rowSource="query" groups={groups} queryDisplay={{ view: { columns: ["owner"] }, apply: () => {} }} />);
  cell(root, "prop:owner").dispatchEvent(new MouseEvent("dblclick", { bubbles: true, button: 0 }));
  const input = root.querySelector<HTMLInputElement>(".sheet-prop-input")!;
  input.value = "Martin"; input.dispatchEvent(new Event("input", { bubbles: true }));
  setDoc("pages", 0, "readOnly", true); key(input, "Enter");
  expect(blockProperty("same", "owner")).toBe("Original");
  expect(root.querySelector<HTMLInputElement>(".sheet-prop-input")?.value).toBe("Martin");
});

it.each(["shared", "missing"])("R8-3: fresh authored result %s resolves its unique remapped node within the source page", async locator => {
  await fixture();
  setDoc("pages", [page("A", ["shared"]), page("B", ["dup~B"])]);
  setDoc("byId", "shared", { id: "shared", raw: "A\nid:: shared", page: "A", parent: null, children: [], collapsed: false });
  setDoc("byId", "dup~B", { id: "dup~B", raw: "B\nid:: shared", page: "B", parent: null, children: [], collapsed: false });
  await Promise.resolve();
  const hit = dto(locator, "B\nid:: shared"); adoptResults(hit, captureResultsTicket());
  expect(resolveResultRow({ id: locator, page: "B", kind: "page", dto: hit })?.id).toBe("dup~B");
});

it("R8-3: stale ID-less locators and ambiguous authored IDs still refuse", async () => {
  const groups = await fixture();
  beginResultsChange()();
  expect(resolveResultRow({ id: "same", page: "Source", dto: groups[0].blocks[0] })).toBeNull();
  setDoc("byId", "same", "raw", "First\nid:: shared");
  setDoc("byId", "query", "raw", "Second\nid:: shared");
  await Promise.resolve();
  const hit = dto("missing", "Selected\nid:: shared"); adoptResults(hit, captureResultsTicket());
  expect(resolveResultRow({ id: "missing", page: "Source", dto: hit })).toBeNull();
});

it("R8-4: one external notification starts one settled reread", async () => {
  await fixture(); const load = vi.fn(async () => [dto("same", "Selected")]);
  mount(() => { const [answer] = createMembershipResource(() => "question", () => "scope", load, () => false, () => true, true); return <span>{answer()?.length}</span>; });
  await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1));
  await applyGraphChange({ name: "Unloaded", kind: "page", created: false, removed: false });
  await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2));
});

it("R8-4: clean host work blocks actions but reuses the current answer on settlement", async () => {
  await fixture(); const [busy, setBusy] = createSignal(false); installResultsPendingGate(busy);
  const load = vi.fn(async () => [dto("same", "Selected")]);
  const root = mount(() => { const [answer] = createMembershipResource(() => "question", () => "scope", load, () => false, () => true, true); return <span>{answer()?.length}</span>; });
  await vi.waitFor(() => expect(root.textContent).toBe("1"));
  const hit = (await load.mock.results[0].value)[0];
  const row = { id: hit.id, page: "Source", dto: hit };
  expect(resolveResultRow(row)?.id).toBe("same");
  setBusy(true); expect(captureResultsTicket().settled).toBe(false);
  expect(resolveResultRow(row)).toBeNull();
  setBusy(false); await new Promise(resolve => setTimeout(resolve, 0));
  expect(load).toHaveBeenCalledTimes(1);
  expect(root.textContent).toBe("1");
  expect(resolveResultRow(row)?.id).toBe("same");
});

it("own save work delays reruns but leaves result actions available; a held change still blocks them", async () => {
  await fixture();
  const [saving, setSaving] = createSignal(false); installResultsPendingGate(saving, "own-work");
  const [held, setHeld] = createSignal(false); installResultsPendingGate(held);
  const load = vi.fn(async () => [dto("same", "Selected")]);
  const root = mount(() => { const [answer] = createMembershipResource(() => "question", () => "scope", load, () => false, () => true, true); return <span>{answer()?.length}</span>; });
  await vi.waitFor(() => expect(root.textContent).toBe("1"));
  const hit = (await load.mock.results[0].value)[0];
  const row = { id: hit.id, page: "Source", dto: hit };
  setSaving(true);
  expect(captureResultsTicket().settled).toBe(false);
  expect(resolveResultRow(row)?.id).toBe("same");
  setHeld(true);
  expect(resolveResultRow(row)).toBeNull();
  setHeld(false); setSaving(false);
  expect(resolveResultRow(row)?.id).toBe("same");
});

it("an open Table draft commits during the user's own save work, not during a held change", async () => {
  const groups = await fixture();
  const [saving, setSaving] = createSignal(false); installResultsPendingGate(saving, "own-work");
  const [held, setHeld] = createSignal(false); installResultsPendingGate(held);
  const root = mount(() => <SheetTable ownerId="query" rowSource="query" groups={groups} queryDisplay={{ view: { columns: ["owner"] }, apply: () => {} }} />);
  cell(root, "prop:owner").dispatchEvent(new MouseEvent("dblclick", { bubbles: true, button: 0 }));
  const input = root.querySelector<HTMLInputElement>(".sheet-prop-input")!;
  input.value = "Martin"; input.dispatchEvent(new Event("input", { bubbles: true }));
  setHeld(true); key(input, "Enter");
  expect(blockProperty("same", "owner")).toBe("Original");
  setHeld(false); setSaving(true); key(input, "Enter");
  expect(blockProperty("same", "owner")).toBe("Martin");
  setSaving(false);
});

it("an authored-ID result does not resolve around a held change", async () => {
  await fixture();
  const [held, setHeld] = createSignal(false); installResultsPendingGate(held);
  setDoc("byId", "same", "raw", "Selected\nid:: same-authored"); await Promise.resolve();
  const hit = dto("missing", "Selected\nid:: same-authored"); adoptResults(hit, captureResultsTicket());
  const row = { id: "missing", page: "Source", dto: hit };
  expect(resolveResultRow(row)?.id).toBe("same");
  setHeld(true);
  expect(resolveResultRow(row)).toBeNull();
  setHeld(false);
  expect(resolveResultRow(row)?.id).toBe("same");
});
