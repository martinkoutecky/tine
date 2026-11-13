import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ExportModal } from "./ExportModal";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { closeExportModal, openExportNodesModal } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import type { BlockDto } from "../types";

beforeAll(initParser);
afterEach(() => {
  closeExportModal();
  clearTransientLayersForTest();
  resetStore();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

it.each(["embed", "query", "tine-query"].flatMap(macro => [false, true].map(reply => ({ macro, reply }))))("$macro exports a detached thread block (reply=$reply) labelled, and omits it on request", async ({ macro, reply }) => {
  const uuid = "6a1b2c3d-0000-4000-8000-000000000009";
    const block: BlockDto & { comment: boolean } = {
      id: uuid, raw: reply ? "REPLYMARKER" : "COMMENTMARKER\ntine.quote:: Body",
      comment: true, collapsed: false, children: [],
    };
    const group = { page: "Source", kind: "page" as const, blocks: [block] };
    const query = macro === "tine-query" ? "task = 'TODO'" : "(task TODO)";
    const key = JSON.stringify([macro, [query]]);
    const previewBlock = vi.spyOn(backend(), "previewBlock").mockResolvedValue({ group, truncated: 0 });
    const exportQuery = vi.spyOn(backend(), "exportQuerySubtrees").mockResolvedValue({
      results: [{ key, groups: [group], shown: 1, total: 1, omitted_nodes: 0 }], omitted_queries: 0,
    });
    const getPage = vi.spyOn(backend(), "getPage");
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <ExportModal />, root);
    try {
      const raw = macro === "embed" ? `{{embed ((${uuid}))}}` : `{{${macro} ${query}}}`;
      openExportNodesModal([{ raw: "Body", children: [{ raw, children: [] }] }], 2);
      const preview = () => root.querySelector<HTMLTextAreaElement>(".export-preview")?.value ?? "";
      await vi.waitFor(() => expect(preview()).toContain(reply ? "REPLYMARKER" : "COMMENTMARKER"));
      expect(preview()).toContain("Comment");
      const checkbox = [...root.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')]
        .find(input => input.closest("label")?.textContent?.includes("Include comments"))!;
      expect(checkbox.checked).toBe(true);
      checkbox.click();
      await vi.waitFor(() => expect(preview()).not.toMatch(/COMMENTMARKER|REPLYMARKER/));
      expect(preview()).toContain("Body");
      expect(getPage).not.toHaveBeenCalled();
      expect(macro === "embed" ? previewBlock : exportQuery).toHaveBeenCalledTimes(1);
    } finally { closeExportModal(); dispose(); root.remove(); vi.restoreAllMocks(); }
});
