import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { closeExportModal, openExportNodesModal } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { ExportModal } from "./ExportModal";

afterEach(() => { closeExportModal(); clearTransientLayersForTest(); document.body.innerHTML = ""; localStorage.clear(); resetStore(); vi.restoreAllMocks(); });

it("L10-04: exporting more references than inactive cache capacity preserves every resolved target", async () => {
  const ids = Array.from({ length: 600 }, (_, i) => `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`);
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(id => ({ page: "P", kind: "page", blocks: [{ id, raw: `target-${id.slice(-12)}`, collapsed: false, children: [] }] })));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <ExportModal />, root);
  try {
    openExportNodesModal(ids.map(id => ({ raw: `((${id}))`, children: [] })), ids.length);
    const copy = () => [...root.querySelectorAll("button")].find(button => button.textContent?.trim() === "Copy");
    await vi.waitFor(() => expect(copy()?.disabled).toBe(false));
    const preview = root.querySelector<HTMLTextAreaElement>(".export-preview")!.value;
    for (const id of ids) expect(preview).toContain(`target-${id.slice(-12)}`);
    expect(preview).not.toContain("((");
  } finally { dispose(); }
});
