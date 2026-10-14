import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import * as router from "../router";
import { QueryMacroActions } from "./QueryMacroActions";

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });
it("opens a real Search workspace in the foreground with the printed dialect", async () => {
  const open = vi.spyOn(router, "openInNewTab").mockImplementation(() => {});
  const print = vi.spyOn(backend(), "printQuery").mockResolvedValue("@block and scheduled > today");
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryMacroActions editable results current={() => ({ query: { anchor: "block", filter: { kind: "true" }, diagnostics: [], source: { kind: "tql", original: "@block and scheduled > today" } }, view: { view: "table" } })}
    groups={() => []} pages={() => []} edit={() => {}} export={() => {}} />, root);
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
