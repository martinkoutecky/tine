import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { closeExportModal, exportModal } from "../ui";
import { QueryResultsSelection } from "./QueryResultsSelection";
import type { QueryHit } from "../types";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";

afterEach(() => { document.body.replaceChildren(); closeExportModal(); vi.restoreAllMocks(); });
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
