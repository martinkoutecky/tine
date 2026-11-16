import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { bumpGraphEpoch } from "../graphSession";
import { closeQueryExport, openQueryExport, queryExportRequest } from "../ui";
import { QueryExportDialog } from "./QueryExportDialog";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  closeQueryExport();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

it.each(["same-root reopen", "graph switch", "display epoch change"])(
  "closes the reviewed export on %s rather than showing an old plan (GH #543 R14-08)",
  async (change) => {
    const plan = vi.spyOn(backend(), "publishQueryPlan").mockResolvedValue({
      anchor: "page", rowCount: 1,
      pages: [{ name: "Public", path: "pages/Public.md", journal: false }],
      leftOut: [], selectsHostPage: false, folder: "tasks", fingerprint: "reviewed",
      path: "/graph/published-queries/tasks", exists: false, suggestedFolder: null,
    });
    const root = document.createElement("div");
    document.body.append(root);
    // Use the same accessor as WindowOverlays, without mocking ui: a constant
    // accessor would bypass the graph-scoped state that owns the dialog.
    dispose = render(() => <QueryExportDialog request={queryExportRequest} />, root);
    openQueryExport({ argument: "(task TODO)", dialect: "macro_query", properties: [], name: "Tasks" });
    await vi.waitFor(() => expect(root.textContent).toContain("Public"));
    expect(root.querySelector('[role="dialog"]')).not.toBeNull();
    expect(plan).toHaveBeenCalledTimes(1);

    // Both actual graph transitions reset the store before publishing a new
    // binding; even an epoch-only repaint invalidates the held request.
    if (change === "display epoch change") bumpGraphEpoch();
    else resetStore();

    await vi.waitFor(() => expect(root.querySelector('[role="dialog"]')).toBeNull());
    expect(queryExportRequest()).toBeNull();
  },
);
