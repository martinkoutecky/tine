import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { closeExportModal, exportModal } from "../ui";
import { QueryResultsSelection } from "./QueryResultsSelection";
import type { QueryHit } from "../types";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";

afterEach(() => { document.body.replaceChildren(); closeExportModal(); vi.restoreAllMocks(); });
it("toggles selection, keeps actions above rows and preserves page/block row content", () => {
  const hits: QueryHit[] = [
    { entity: "page", page: { name: "Project", path: "pages/Project.md", kind: "page", date_key: null }, display_text: "Project", evidence: [], score: 0 },
    { entity: "block", page: "Project", kind: "page", block: { id: "b", raw: "Task", collapsed: false, children: [] }, display_text: "Task", evidence: [] },
  ];
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryResultsSelection hits={hits} />, root);
  const button = (text: string) => [...root.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent === text)!;
  try {
    const toggle = button("Select results");
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    toggle.click();
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    const rows = root.querySelectorAll(".query-copy-row");
    expect(rows).toHaveLength(2);
    for (const text of ["Select all", "Select none", "Copy results", "Done"]) {
      expect(button(text).compareDocumentPosition(rows[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
    expect(rows[0].querySelector(".switcher-kind")?.textContent).toBe("page");
    expect(rows[1].querySelector(".switcher-kind")?.textContent).toBe("block");
    expect(rows[1].querySelector(".search-result-context")?.textContent).toBe("Project");
    toggle.click();
    expect(root.querySelector("input[type=checkbox]")).toBeNull();
    toggle.click(); button("Done").click();
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
  } finally { dispose(); }
});
it("copies only selected Search results under their page or date title", async () => {
  const block = { id: "selected", raw: "Selected", collapsed: false, children: [] };
  const hits: QueryHit[] = [
    { entity: "page", page: { name: "Skip", path: "pages/Skip.md", kind: "page", date_key: null }, display_text: "Skip", evidence: [], score: 0 },
    { entity: "block", page: "Oct 13th, 2026", kind: "journal", path: "journals/2026_10_13.md", block, display_text: "Selected", evidence: [] },
  ];
  adoptResults(hits, captureResultsTicket());
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue({ id: "journals/2026_10_13.md", name: "Oct 13th, 2026", title: "Oct 13th, 2026", kind: "journal", pre_block: null, blocks: [block] });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryResultsSelection hits={hits} />, root);
  try {
    root.querySelector<HTMLButtonElement>("button")!.click();
    root.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
    [...root.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Copy results")!.click();
    await vi.waitFor(() => expect(JSON.stringify(exportModal())).toContain("Oct 13th, 2026"));
    expect(JSON.stringify(exportModal())).not.toContain("Skip");
  } finally { dispose(); }
});

it("can exit a frozen selection after a live refresh removes every hit", () => {
  const hit: QueryHit = { entity: "page", page: { name: "Project", path: "pages/Project.md", kind: "page", date_key: null }, display_text: "Project", evidence: [], score: 0 };
  const [hits, setHits] = createSignal<QueryHit[]>([hit]);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryResultsSelection hits={hits()} />, root);
  try {
    const toggle = root.querySelector<HTMLButtonElement>("button")!;
    toggle.click();
    setHits([]);
    expect(root.querySelectorAll(".query-copy-row")).toHaveLength(1);
    expect(toggle.disabled).toBe(false);
    toggle.click();
    expect(root.querySelector(".query-copy-row")).toBeNull();
    expect(toggle.disabled).toBe(true);
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
  } finally { dispose(); }
});
