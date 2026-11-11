import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { backend } from "../backend";
import { bumpGraphEpoch } from "../graphSession";
import { closeSwitcher, openSwitcher } from "../ui";
import { resetStore } from "../document";
import type { QueryExecution } from "../types";

afterEach(() => { closeSwitcher(); resetStore(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

const execution = (names: string[]): QueryExecution => ({
  hits: names.map((name) => ({
    entity: "page" as const, page: { name, kind: "page" as const, path: `pages/${name}.md`, date_key: null },
    display_text: name, evidence: [], score: 1500, match_class: "exact" as const,
  })),
  diagnostics: [], explanation: { branches: [] }, has_more: { pages: false, blocks: false }, cancelled: false,
});

function typeIntoSwitcher(root: HTMLElement, text: string) {
  const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
  input.value = text;
  input.dispatchEvent(new InputEvent("input", { bubbles: true }));
}

const rows = (root: HTMLElement) => [...root.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")]
  .map((node) => node.textContent?.trim());

// The graph is not warm until its first complete view exists. A reply the open
// graph's display state has already moved past (the config and title format the
// window paints from land while the search is outstanding) must not stay on
// screen as the final answer: "Create page: X" for a page that exists.
it("offers the page once a search answer the window outgrew is asked again, without retyping", async () => {
  let reply!: (answer: QueryExecution) => void;
  const search = vi.spyOn(backend(), "runGraphSearch")
    .mockImplementationOnce(() => new Promise<QueryExecution>((resolve) => { reply = resolve; }))
    .mockResolvedValue(execution(["page_A"]));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QuickSwitcher />, root);
  try {
    openSwitcher({ mode: "pages" });
    typeIntoSwitcher(root, "page_A");
    await vi.waitFor(() => expect(search).toHaveBeenCalledTimes(1));
    // The window repaints (graph epoch) while the first answer is outstanding,
    // then the answer, computed before the graph could name its pages, arrives.
    bumpGraphEpoch();
    reply(execution([]));
    await vi.waitFor(() => expect(rows(root)).toContain("page_A"));
    expect(root.querySelector('#switcher-results [role="status"]')).toBeNull();
    expect(search).toHaveBeenCalledTimes(2);
    expect(rows(root)).not.toContain("Create page: page_A");
  } finally { dispose(); }
});

it("asks once per query when nothing retires the answer", async () => {
  const search = vi.spyOn(backend(), "runGraphSearch").mockResolvedValue(execution(["page_A"]));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QuickSwitcher />, root);
  try {
    openSwitcher({ mode: "pages" });
    typeIntoSwitcher(root, "page_A");
    await vi.waitFor(() => expect(rows(root)).toContain("page_A"));
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(search).toHaveBeenCalledTimes(1);
  } finally { dispose(); }
});
