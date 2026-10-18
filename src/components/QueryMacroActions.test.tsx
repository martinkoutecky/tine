import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import type { ParsedQuery } from "../editor/queryIr";
import { backend } from "../backend";
import * as router from "../router";
import { QueryMacroActions } from "./QueryMacroActions";

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });
it("opens a real Search workspace in the foreground with the printed dialect", async () => {
  const open = vi.spyOn(router, "openInNewTab").mockImplementation(() => {});
  const print = vi.spyOn(backend(), "printQuery").mockResolvedValue("@block and scheduled > today");
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacroActions editable results current={() => ({ query: { anchor: "block", filter: { kind: "true" }, diagnostics: [], source: { kind: "tql", original: "@block and scheduled > today" } }, view: { view: "table" } })}
    groups={() => []} pages={() => []} edit={() => {}} export={() => {}} share={() => {}} />, root);
  try {
    root.querySelector<HTMLElement>("summary")!.click();
    const button = await vi.waitFor(() => {
      const found = [...document.querySelectorAll<HTMLButtonElement>(".qc-menu button")].find(button => button.textContent === "Open in Search");
      expect(found).toBeDefined(); return found!;
    });
    button.click();
    await vi.waitFor(() => expect(open).toHaveBeenCalledWith(expect.objectContaining({ kind: "query", sourceKind: "dsl", source: "{{tine-query @block and scheduled > today}}", presentation: "table" }), true));
    expect(print.mock.calls[0][2]).toBe("tql_macro");
  } finally { dispose(); }
});

it("offers an always-present Edit button beside the menu that opens the filter editor, only when editable", () => {
  const edit = vi.fn();
  const root = document.createElement("div"); document.body.append(root);
  const [editable, setEditable] = createSignal(true);
  const dispose = render(() => <QueryMacroActions editable={editable()} results current={() => undefined}
    groups={() => []} pages={() => []} edit={edit} export={() => {}} share={() => {}} />, root);
  try {
    const button = root.querySelector<HTMLButtonElement>(".qc-edit")!;
    expect(button.getAttribute("aria-label")).toBe("Edit query");
    // beside the menu: directly before the disclosure, after Share
    expect(button.nextElementSibling).toBe(root.querySelector(".qc-actions"));
    button.click();
    expect(edit).toHaveBeenCalledWith("filter");
    setEditable(false);
    expect(root.querySelector(".qc-edit")).toBeNull();
  } finally { dispose(); }
});

it("Open in Search carries the block's own view: the list default, scoped views and displays, and Pages and blocks", async () => {
  const open = vi.spyOn(router, "openInNewTab").mockImplementation(() => {});
  vi.spyOn(backend(), "printQuery").mockResolvedValue("@block and task = 'TODO'");
  const root = document.createElement("div"); document.body.append(root);
  const [both, setBoth] = createSignal(true);
  const query = { anchor: "block" as const, filter: { kind: "true" as const }, diagnostics: [], source: { kind: "tql" as const, original: "@block and task = 'TODO'" } };
  const current = (parsed: Partial<ParsedQuery>): ParsedQuery => ({ query, view: { sort: [["priority", "asc"]] }, ...parsed });
  const [parsed, setParsed] = createSignal<ParsedQuery>(current({}));
  const dispose = render(() => <QueryMacroActions editable results current={parsed} both={both}
    groups={() => []} pages={() => []} edit={() => {}} export={() => {}} share={() => {}} />, root);
  const search = async () => {
    root.querySelector<HTMLElement>("summary")!.click();
    const button = await vi.waitFor(() => {
      const found = [...document.querySelectorAll<HTMLButtonElement>(".qc-menu button")].find(b => b.textContent === "Open in Search");
      expect(found).toBeDefined(); return found!;
    });
    button.click();
  };
  try {
    await search();
    await vi.waitFor(() => expect(open).toHaveBeenCalledTimes(1));
    // no `tine.view`: the block shows a list, and so must the Search tab (it used to open as "search")
    expect(open.mock.calls[0][0]).toMatchObject({ presentation: "list", resultKinds: "pages-and-blocks",
      blockDisplay: { sort: [["priority", "asc"]] }, pageDisplay: { sort: [["priority", "asc"]] } });
    expect(open.mock.calls[0][0]).not.toHaveProperty("pagePresentation");
    setBoth(false);
    setParsed(current({ view: { view: "table" }, page_presentation: "board", block_presentation: "list",
      page_display: { group_by: "prop:status" }, block_display: { columns: ["prop:owner"] } }));
    await search();
    await vi.waitFor(() => expect(open).toHaveBeenCalledTimes(2));
    const route = open.mock.calls[1][0] as Record<string, unknown>;
    expect(route).toMatchObject({ presentation: "table", pagePresentation: "board", blockPresentation: "list",
      pageDisplay: { group_by: "prop:status" }, blockDisplay: { columns: ["prop:owner"] } });
    expect(route).not.toHaveProperty("resultKinds");
  } finally { dispose(); }
});

it("shares from the always-reachable icon and from the menu, never gated on having results", async () => {
  const share = vi.fn();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacroActions editable={false} results={false} current={() => undefined}
    groups={() => []} pages={() => []} edit={() => {}} export={() => {}} share={share} />, root);
  try {
    root.querySelector<HTMLButtonElement>(".qc-share")!.click();
    expect(share).toHaveBeenCalledTimes(1);
    root.querySelector<HTMLElement>("summary")!.click();
    const item = await vi.waitFor(() => {
      const found = [...document.querySelectorAll<HTMLButtonElement>(".qc-menu button")].find((button) => button.textContent === "Share…");
      expect(found).toBeDefined(); return found!;
    });
    expect(item.disabled).toBe(false);
    item.click();
    expect(share).toHaveBeenCalledTimes(2);
  } finally { dispose(); }
});
