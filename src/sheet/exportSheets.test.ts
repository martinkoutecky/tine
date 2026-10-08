import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { setToasts, toasts } from "../toasts";
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

  it("never rejects, but a failed read is a sticky error naming the consequence", async () => {
    setToasts([]);
    vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
    vi.spyOn(backend(), "sheetExportInputs").mockRejectedValue(new Error("unreadable"));
    await expect(exportSheets(["Page"])).resolves.toEqual([]);
    expect(toasts().filter((t) => t.kind === "error" && t.sticky && t.message.includes("plain outlines"))).toHaveLength(1);
  });
});
