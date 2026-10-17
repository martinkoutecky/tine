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
