import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { backend, OperationCancelledError, QueryNotReadyError, QueryUnavailableError } from "../backend";
import { initParser } from "../render/parse";
import type { BlockDto, QueryExportBatch } from "../types";
import type { ExportNode } from "../editor/exportText";
import { warmExportResolutions } from "./ExportModal";

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
});

describe("query clipboard/export hydration budget", () => {
  it("emits the authored TQL dialect for tine-query export", async () => {
    const native = vi.spyOn(backend(), "exportQuerySubtrees")
      .mockResolvedValue({ results: [], omitted_queries: 1 });
    const nodes: ExportNode[] = [{
      raw: "{{tine-query task = 'TODO'}}",
      format: "md",
      children: [],
    }];

    await warmExportResolutions(nodes, new Map());

    expect(native).toHaveBeenCalledTimes(1);
    expect(native.mock.calls[0][0]).toHaveLength(1);
    expect(native.mock.calls[0][0][0]).toMatchObject({
      query: "task = 'TODO'",
      advanced: false,
      simple_dialect: "tql",
    });
  });

  it("retries a pending query export and propagates terminal availability errors", async () => {
    const nodes: ExportNode[] = [{ raw: "{{query (task TODO)}}", format: "md", children: [] }];
    const native = vi.spyOn(backend(), "exportQuerySubtrees")
      .mockRejectedValueOnce(new QueryNotReadyError("recovering"))
      .mockResolvedValue({ results: [], omitted_queries: 1 });
    const warmed = new Map();
    await warmExportResolutions(nodes, warmed);
    expect(native).toHaveBeenCalledTimes(2);
    expect(warmed.size).toBe(1);
    const failure = new QueryUnavailableError("projection.failed", "Index rebuild failed.");
    native.mockRejectedValue(failure);
    await expect(warmExportResolutions(nodes, new Map())).rejects.toBe(failure);
    expect(native).toHaveBeenCalledTimes(3);
  });

  it("stops export expansion when its owner is cancelled", async () => {
    const controller = new AbortController();
    const native = vi.spyOn(backend(), "exportQuerySubtrees").mockImplementation(async () => {
      controller.abort();
      throw new QueryNotReadyError("indexing");
    });
    const nodes: ExportNode[] = [{ raw: "{{query (task TODO)}}", format: "md", children: [] }];
    const warmed = new Map();
    await expect(warmExportResolutions(nodes, warmed, undefined, {
      signal: controller.signal, isCurrent: () => true, onPending: () => {},
    })).rejects.toBeInstanceOf(OperationCancelledError);
    expect(native).toHaveBeenCalledTimes(1);
    expect(warmed.size).toBe(0);
  });

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
    const runQuery = vi.spyOn(backend(), "runQuery");
    const nodes: ExportNode[] = [{
      raw: "{{query (task TODO)}} and {{query (task DONE)}}",
      format: "md",
      children: [],
    }];
    const warmed = new Map<string, any>();

    await warmExportResolutions(nodes, warmed);

    expect(native).toHaveBeenCalledTimes(1);
    expect(native.mock.calls[0][0]).toEqual([
      { key: batch.results[0].key, query: "(task TODO)", advanced: false },
      { key: batch.results[1].key, query: "(task DONE)", advanced: false },
    ]);
    expect(getPage).not.toHaveBeenCalled();
    expect(runQuery).not.toHaveBeenCalled();
    expect(warmed.get(batch.results[0].key)?.truncation).toContain(
      "showing first 1 of 20000 results; 17 descendant blocks omitted",
    );
    expect(warmed.get(batch.results[1].key)?.nodes[0].children[0].raw).toBe("done");
  });
});
