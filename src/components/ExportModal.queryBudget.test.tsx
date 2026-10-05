import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import type { BlockDto, QueryExportBatch } from "../types";
import type { ExportNode } from "../editor/exportText";
import { warmExportResolutions } from "./ExportModal";
import { setToasts, toasts } from "../toasts";
import { bumpGraphEpoch } from "../graphSession";

const shallow = (id: string): BlockDto => ({
  id,
  raw: id,
  collapsed: false,
  children: [],
});

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  setToasts([]);
});

describe("query clipboard/export hydration budget", () => {
  it("resolves multiple query macros in one native batch without loading source pages", async () => {
    const batch: QueryExportBatch = {
      results: [
        {
          key: JSON.stringify(["query", ["(task TODO)"]]),
          groups: [{ page: "Tasks", kind: "page", blocks: [shallow("todo")] }],
          shown: 1,
          total: 20_000,
          omitted_nodes: 17,
        },
        {
          key: JSON.stringify(["query", ["(task DONE)"]]),
          groups: [{ page: "Done", kind: "page", blocks: [shallow("done")] }],
          shown: 1,
          total: 1,
          omitted_nodes: 0,
        },
      ],
      omitted_queries: 0,
    };
    const native = vi.spyOn(backend(), "exportQuerySubtrees").mockResolvedValue(batch);
    const getPage = vi.spyOn(backend(), "getPage");
    const nodes: ExportNode[] = [{
      raw: "{{query (task TODO)}} and {{query (task DONE)}}",
      format: "md",
      children: [],
    }];
    const warmed = new Map<string, any>();

    await warmExportResolutions(nodes, warmed);

    expect(native).toHaveBeenCalledTimes(1);
    expect(native.mock.calls[0][0]).toEqual([
      { key: batch.results[0].key, query: "(task TODO)" },
      { key: batch.results[1].key, query: "(task DONE)" },
    ]);
    expect(getPage).not.toHaveBeenCalled();
    expect(warmed.get(batch.results[0].key)?.truncation).toContain(
      "showing first 1 of 20000 results; 17 descendant blocks omitted",
    );
    expect(warmed.get(batch.results[1].key)?.nodes[0].children[0].raw).toBe("done");
  });
});

describe("I-9: export pre-warm failures are shown, not swallowed", () => {
  const queryNodes: ExportNode[] = [{ raw: "{{query (task TODO)}}", format: "md", children: [] }];
  it("shows a sticky error when the bounded query batch is rejected, and keeps the literal macro", async () => {
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
    vi.spyOn(backend(), "exportQuerySubtrees").mockRejectedValue(new Error("io:Broken"));
    const warmed = new Map<string, any>();
    await warmExportResolutions(queryNodes, warmed);
    expect(warmed.size).toBe(0);
    expect(toasts().some((t) => t.kind === "error" && t.sticky)).toBe(true);
  });
  it("shows a sticky error when an embed preview is rejected", async () => {
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
    vi.spyOn(backend(), "previewBlock").mockRejectedValue(new Error("io:Broken"));
    const embed: ExportNode[] = [{ raw: "{{embed ((6a1b2c3d-0000-4000-8000-000000000009))}}", format: "md", children: [] }];
    await warmExportResolutions(embed, new Map());
    expect(toasts().some((t) => t.kind === "error")).toBe(true);
  });
  it("stays quiet when the graph changed before the failure landed", async () => {
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
    let fail!: (error: Error) => void;
    vi.spyOn(backend(), "exportQuerySubtrees").mockImplementation(() => new Promise((_, reject) => { fail = reject; }));
    const pending = warmExportResolutions(queryNodes, new Map());
    await vi.waitFor(() => expect(fail).toBeTypeOf("function"));
    bumpGraphEpoch();
    fail(new Error("old graph"));
    await pending;
    expect(toasts()).toEqual([]);
  });
});
