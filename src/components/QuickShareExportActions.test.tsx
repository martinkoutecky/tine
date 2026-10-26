import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { blockProperty, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { QueryMacroActions } from "./QueryMacroActions";
import * as definitions from "../quickShare/definitions";
import { invalidateBinding } from "../binding";
import { setToasts, toasts } from "../toasts";
import { setSettingsOpen } from "../ui";

const GIST = "1234567890abcdef1234567890abcdef";
const MACRO = "{{tine-query @page and name = 'Project'}}";
let dispose: (() => void) | undefined;
beforeAll(initParser);
afterEach(() => { dispose?.(); dispose = undefined; resetStore(); vi.restoreAllMocks(); setToasts([]); setSettingsOpen(false); document.body.replaceChildren(); });

async function mount(keyHere = true, value = `octo/${GIST}`, matches = true) {
  loadSingle({ name: "Exports", title: "Exports", kind: "page", pre_block: "tine/exports:: true\npublic:: false\n",
    blocks: [{ id: "definition", raw: `${MACRO}\n  tine.share:: ${value}`, properties: [["tine.share", value]], collapsed: false, children: [] }] });
  vi.spyOn(backend(), "quickShareList").mockResolvedValue({ enabled: true, error: null, account: { login: "octo", accountId: 42 },
    shares: keyHere ? [{ shareId: "record", state: "live", login: "octo", gistId: GIST, title: "Project", pages: 1,
      createdAt: "2026-10-25T00:00:00Z", accountMatches: matches }] : [] });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <QueryMacroActions hostBlockId="definition" current={() => undefined} groups={() => []} pages={() => []}
    editable results edit={() => {}} export={() => {}} share={() => {}} />, root);
  await new Promise(resolve => setTimeout(resolve, 0));
  return root;
}

it.each([true, false])("an export definition offers Delete and confirms before using the existing native delete (key here: %s)", async keyHere => {
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(false);
  const remove = vi.spyOn(backend(), "quickShareDelete").mockResolvedValue("deleted");
  const unlist = vi.spyOn(definitions, "removeShareProperty").mockResolvedValue(["Exports"]);
  const root = await mount(keyHere);
  const button = root.querySelector<HTMLButtonElement>(".quick-share-export-delete");
  expect(button, "the same query action bar is rendered for definitions listed by the Exports query").not.toBeNull();
  button!.click();
  await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
  expect(remove).not.toHaveBeenCalled();
  await vi.waitFor(() => expect(button!.disabled).toBe(false));
  confirm.mockResolvedValue(true);
  button!.click();
  await vi.waitFor(() => expect(remove).toHaveBeenCalledWith(keyHere ? { shareId: "record" } : { gistId: GIST }));
  await vi.waitFor(() => expect(unlist).toHaveBeenCalledWith(GIST, expect.any(Function)));
  expect(blockProperty("definition", "tine.share")).toBe(`octo/${GIST}`); // mocked graph cleanup
});

it("a deleted-on-GitHub share keeps its marker and key and reports that it is gone", async () => {
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  vi.spyOn(backend(), "quickShareDelete").mockResolvedValue("gone");
  const unlist = vi.spyOn(definitions, "removeShareProperty").mockResolvedValue([]);
  const root = await mount();
  root.querySelector<HTMLButtonElement>(".quick-share-export-delete")!.click();
  await vi.waitFor(() => expect(root.textContent).toContain("No longer on GitHub"));
  expect(unlist).not.toHaveBeenCalled();
});

it("pending markers and shares of another connected account do not offer Delete", async () => {
  let root = await mount(true, "pending/Rec0rd-id_0123456789x1");
  expect(root.querySelector(".quick-share-export-delete")).toBeNull();
  dispose!(); resetStore();
  root = await mount(true, `octo/${GIST}`, false);
  expect(root.querySelector<HTMLButtonElement>(".quick-share-export-delete")?.disabled).toBe(true);
});

it.each(["close", "switch"])("Delete finishing after %s keeps cleanup bound to the original graph", async leave => {
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  let finish!: (value: "deleted") => void;
  const remove = vi.spyOn(backend(), "quickShareDelete").mockReturnValue(new Promise(resolve => { finish = resolve; }));
  const unlist = vi.spyOn(definitions, "removeShareProperty").mockResolvedValue([]);
  const root = await mount();
  root.querySelector<HTMLButtonElement>(".quick-share-export-delete")!.click();
  await vi.waitFor(() => expect(remove).toHaveBeenCalled());
  if (leave === "close") { dispose!(); dispose = undefined; } else invalidateBinding();
  finish("deleted");
  await new Promise(resolve => setTimeout(resolve, 0));
  if (leave === "close") expect(unlist).toHaveBeenCalledWith(GIST, expect.any(Function));
  else expect(unlist).not.toHaveBeenCalled();
});

it("a refused delete keeps its graph marker and reports the cause", async () => {
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  vi.spyOn(backend(), "quickShareDelete").mockRejectedValue({ kind: "github", message: "No access" });
  const unlist = vi.spyOn(definitions, "removeShareProperty").mockResolvedValue([]);
  const root = await mount(false);
  root.querySelector<HTMLButtonElement>(".quick-share-export-delete")!.click();
  await vi.waitFor(() => expect(root.textContent).toContain("GitHub refused: No access"));
  expect(unlist).not.toHaveBeenCalled();
  expect(blockProperty("definition", "tine.share")).toBe(`octo/${GIST}`);
});

it("a graph cleanup failure stays visible even if the query result disappears", async () => {
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  vi.spyOn(backend(), "quickShareDelete").mockResolvedValue("deleted");
  vi.spyOn(definitions, "removeShareProperty").mockRejectedValue(new Error("disk full"));
  const root = await mount();
  root.querySelector<HTMLButtonElement>(".quick-share-export-delete")!.click();
  await vi.waitFor(() => expect(toasts().some(toast => toast.kind === "error" && toast.message.includes("disk full"))).toBe(true));
});

it("returning from Settings refreshes an export's account and key availability", async () => {
  const root = await mount();
  const list = vi.mocked(backend().quickShareList);
  list.mockResolvedValue({ enabled: true, account: null, shares: [], error: null });
  setSettingsOpen(true); setSettingsOpen(false);
  await vi.waitFor(() => expect(root.querySelector<HTMLButtonElement>(".quick-share-export-delete")!.disabled).toBe(true));
  list.mockResolvedValue({ enabled: true, account: { login: "octo", accountId: 42 }, shares: [], error: null });
  setSettingsOpen(true); setSettingsOpen(false);
  await vi.waitFor(() => expect(root.querySelector<HTMLButtonElement>(".quick-share-export-delete")!.disabled).toBe(false));
});

it("two clicks while confirmation is pending open one confirmation and send no delete", async () => {
  const confirm = vi.spyOn(backend(), "confirm").mockReturnValue(new Promise(() => {}));
  const remove = vi.spyOn(backend(), "quickShareDelete").mockResolvedValue("deleted");
  const root = await mount();
  const button = root.querySelector<HTMLButtonElement>(".quick-share-export-delete")!;
  button.click(); button.click();
  expect(confirm).toHaveBeenCalledTimes(1);
  expect(remove).not.toHaveBeenCalled();
});
