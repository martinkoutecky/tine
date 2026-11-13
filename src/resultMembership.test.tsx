import { afterEach, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { createMembershipResource, blocksContainEdit } from "./resultMembership";
import { startEditing, endEdit } from "./editorController";
import { bumpDataRev, bumpGraphEpoch } from "./graphSession";
import type { BlockDto } from "./types";

const save = vi.hoisted(() => ({ dirty: false, saving: false, pending: false, conflict: false, instance: 0, page: "Source" }));
vi.mock("./document", () => ({
  node: () => ({ page: save.page }),
  pageInstanceGeneration: () => save.instance,
  isDirty: () => save.dirty, isSaving: () => save.saving,
  isConflicted: () => save.conflict, pendingDataRevision: () => save.pending,
}));
const disposals: (() => void)[] = [];
afterEach(() => {
  disposals.splice(0).forEach((dispose) => dispose());
  endEdit("blur");
  Object.assign(save, { dirty: false, saving: false, pending: false, conflict: false, instance: 0, page: "Source" });
  document.body.innerHTML = "";
});
function mount(load: (id: string) => Promise<string[]>, identity = () => "Target") {
  const root = document.createElement("div"); document.body.append(root);
  disposals.push(render(() => {
    const [answer] = createMembershipResource(identity, identity, load, (rows, id) => rows.includes(id));
    return <div>{answer()?.join(",")}</div>;
  }, root));
  return root;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
const turn = () => new Promise((done) => setTimeout(done, 0));

it("I-20 / GH #659: a read started before editing cannot remove or move the editor", async () => {
  const pending = deferred<string[]>();
  const load = vi.fn().mockResolvedValueOnce(["first", "task", "last"]).mockReturnValueOnce(pending.promise).mockResolvedValue([]);
  const root = mount(load);
  await vi.waitFor(() => expect(root.textContent).toBe("first,task,last"));
  bumpDataRev(); await turn();
  startEditing("task"); pending.resolve(["last", "first"]); await turn();
  expect(root.textContent).toBe("first,task,last");
  endEdit("blur");
  await vi.waitFor(() => expect(root.textContent).toBe(""));
});

it("GH #659/#660 intentional OG difference: coalesce every open occurrence until the final saved revision", async () => {
  const load = vi.fn().mockResolvedValue(["task", "other"]);
  const main = mount(load); const sidebar = mount(load);
  await vi.waitFor(() => expect(sidebar.textContent).toBe("task,other"));
  startEditing("task"); load.mockResolvedValue(["other"]);
  bumpDataRev(); bumpDataRev(); await turn();
  save.dirty = true;
  endEdit("blur"); await turn();
  expect(main.textContent).toBe("task,other"); expect(sidebar.textContent).toBe("task,other");
  save.dirty = false; save.saving = true; bumpDataRev(); await turn();
  expect(load).toHaveBeenCalledTimes(2);
  save.saving = false; save.pending = true; bumpDataRev(); await turn();
  expect(load).toHaveBeenCalledTimes(2);
  save.pending = false; bumpDataRev();
  await vi.waitFor(() => expect(main.textContent).toBe("other"));
  expect(sidebar.textContent).toBe("other"); expect(load).toHaveBeenCalledTimes(4);
});

it("I-20: navigation retires retention and hides the old target during its read", async () => {
  const [identity, setIdentity] = createSignal("Target");
  const pending = deferred<string[]>();
  const load = vi.fn().mockResolvedValueOnce(["task"]).mockReturnValueOnce(pending.promise);
  const root = mount(load, identity);
  await vi.waitFor(() => expect(root.textContent).toBe("task"));
  startEditing("task"); setIdentity("Different"); await turn();
  expect(root.textContent).toBe("");
  pending.resolve(["different"]);
  await vi.waitFor(() => expect(root.textContent).toBe("different"));
});

it("I-20: a late old-graph read cannot enter the new graph", async () => {
  const pending = deferred<string[]>();
  const load = vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue(["new"]);
  const root = mount(load); bumpGraphEpoch();
  await vi.waitFor(() => expect(root.textContent).toBe("new"));
  pending.resolve(["old"]); await turn();
  expect(root.textContent).toBe("new");
});

it("lists that do not contain the editing block still follow save revisions", async () => {
  const load = vi.fn().mockResolvedValue(["other"]);
  const root = mount(load);
  await vi.waitFor(() => expect(root.textContent).toBe("other"));
  startEditing("task"); load.mockResolvedValue(["updated"]); bumpDataRev();
  await vi.waitFor(() => expect(root.textContent).toBe("updated"));
});

it("embedded descendant editing retains the containing answer", () => {
  const child: BlockDto = { id: "child", raw: "TODO child", collapsed: false, children: [] };
  expect(blocksContainEdit([{ ...child, id: "root", children: [child] }], "child")).toBe(true);
  expect(blocksContainEdit([{ ...child, id: "root", children: [] }], "child")).toBe(false);
});

it.each([["replacement", true], ["transfer", true], ["replacement", false], ["transfer", false]] as const)(
  "source %s retires a held answer (after local split: %s)", async (change, localSplit) => {
  const { beginResultsChange } = await import("./queryResultsEpoch");
  const load = vi.fn(async () => ["task"]);
  const root = document.createElement("div"); document.body.append(root);
  disposals.push(render(() => {
    const [answer] = createMembershipResource(() => "Target", () => "Target", load,
      (rows, id) => rows.includes(id), () => true, true);
    return <div>{answer()?.join(",")}</div>;
  }, root));
  await vi.waitFor(() => expect(root.textContent).toBe("task"));
  startEditing("task");
  load.mockResolvedValue(["replacement"]);
  if (localSplit) { beginResultsChange()(); await turn(); }
  expect(root.textContent).toBe("task");
  expect(load).toHaveBeenCalledTimes(1);
  if (change === "replacement") save.instance++;
  else save.page = "Other";
  beginResultsChange()();
  await vi.waitFor(() => expect(root.textContent).toBe("replacement"));
  expect(load).toHaveBeenCalledTimes(2);
});

it("D-10: a suppressed (collapsed) guarded read is not rerun by structural result epochs", async () => {
  const { beginResultsChange } = await import("./queryResultsEpoch");
  const [expanded, setExpanded] = createSignal(false);
  const load = vi.fn(async (_id: string): Promise<string[]> => ["task"]);
  const root = document.createElement("div"); document.body.append(root);
  disposals.push(render(() => {
    const [answer] = createMembershipResource(() => "Target", () => "Target", load,
      (rows, id) => rows.includes(id), expanded, true);
    return <div>{answer()?.join(",")}</div>;
  }, root));
  await vi.waitFor(() => expect(root.textContent).toBe("task"));
  for (let i = 0; i < 3; i++) { beginResultsChange()(); await turn(); }
  expect(load).toHaveBeenCalledTimes(1);
  setExpanded(true); await turn();
  beginResultsChange()(); await turn();
  expect(load).toHaveBeenCalledTimes(3);
});

it("GH #907: a precise reader reuses an unrelated epoch but never renews a coalesced stale answer's action ticket", async () => {
  const { beginResultsChange, resultTicket, resultsTicketCurrent } = await import("./queryResultsEpoch");
  const [revision, setRevision] = createSignal(0);
  const [current, setCurrent] = createSignal(true);
  const load = vi.fn(async () => ({ rows: [{ id: "task", raw: "TODO original" }] }));
  let answer!: ReturnType<typeof createMembershipResource<string, Awaited<ReturnType<typeof load>>>>[0];
  const root = document.createElement("div"); document.body.append(root);
  disposals.push(render(() => {
    [answer] = createMembershipResource(() => "Target", () => "Target", load,
      () => false, () => true, true, revision, { current });
    return <div>{answer()?.rows[0].raw}</div>;
  }, root));
  await turn();
  const original = answer()!;
  expect(resultsTicketCurrent(resultTicket(original.rows[0]))).toBe(true);
  beginResultsChange()(); await turn();
  expect(load).toHaveBeenCalledTimes(1);
  expect(answer()).toBe(original);
  expect(resultsTicketCurrent(resultTicket(original.rows[0]))).toBe(true);
  setCurrent(false);
  beginResultsChange()(); await turn();
  expect(load).toHaveBeenCalledTimes(1);
  expect(resultsTicketCurrent(resultTicket(original.rows[0]))).toBe(false);
  setCurrent(true); setRevision(1); await turn();
  expect(load).toHaveBeenCalledTimes(2);
  expect(resultsTicketCurrent(resultTicket(answer()!.rows[0]))).toBe(true);
});
