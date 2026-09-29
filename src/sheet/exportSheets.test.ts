import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { exportSheets } from "./exportSheets";

afterEach(() => vi.restoreAllMocks());

describe("exportSheets", () => {
  it("hands the publication scope to the input read, so a private row never reaches the evaluator", async () => {
    const read = vi.spyOn(backend(), "sheetExportInputs").mockResolvedValue([]);
    await exportSheets(undefined, { kind: "live", allPages: false });
    expect(read).toHaveBeenCalledWith(undefined, { kind: "live", allPages: false });
  });

  it("reads without a scope for print, which has no publication boundary", async () => {
    const read = vi.spyOn(backend(), "sheetExportInputs").mockResolvedValue([]);
    await exportSheets(["Page"]);
    expect(read).toHaveBeenCalledWith(["Page"], undefined);
  });
});
