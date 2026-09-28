import { afterEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { backend } from "../backend";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import type { Highlight } from "../types";
import { createPdfHighlightState, rebasePdfHighlights, unusedPdfCrops } from "./pdfHighlightState";

const rect = { left: 2, top: 4, width: 6, height: 8 };
function highlight(id: string, color = "yellow", image: number | null = null): Highlight {
  return { id, page: 1, position: { page: 1, bounding: rect, rects: [rect] }, color, text: "text", image };
}

describe("PDF highlight state", () => {
  afterEach(() => { vi.restoreAllMocks(); resetPdfOwnershipForTest(); });

  function state() {
    const owner = activatePdfOwnership("/test/pdf-graph");
    return createRoot(() => createPdfHighlightState({
      filename: "paper.pdf", label: "Paper", backendGeneration: 0, owner,
      prepare: async (items) => items,
    }));
  }

  async function conflictedState() {
    const current = state();
    current.addCrop("a", { page: 1, stamp: 42 });
    current.edit([highlight("a", "yellow", 42)]);
    vi.spyOn(backend(), "writeHighlights").mockRejectedValueOnce(new Error("conflict"));
    await current.persist();
    expect(current.conflict()).toBe(true);
    return current;
  }

  it("holds a disk decision while crop retirement is pending and ignores Keep mine", async () => {
    const current = await conflictedState();
    vi.spyOn(backend(), "readHighlights").mockResolvedValue([]);
    let finish!: () => void;
    vi.spyOn(backend(), "rollbackPdfAreaImage").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const choice = current.useDiskVersion();
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    await current.keepMine();
    expect(backend().writeHighlights).toHaveBeenCalledTimes(1);
    expect(current.decisionBusy()).toBe(true);
    finish();
    await choice;
    expect(current.conflict()).toBe(false);
  });

  it("holds Keep mine while its write is pending and ignores Use disk version", async () => {
    const current = await conflictedState();
    vi.spyOn(backend(), "readHighlights").mockResolvedValue([]);
    const rollback = vi.spyOn(backend(), "rollbackPdfAreaImage");
    let finish!: (items: Highlight[]) => void;
    vi.spyOn(backend(), "writeHighlights").mockImplementationOnce((_pdf, _label, items) =>
      new Promise((resolve) => { finish = () => resolve(items); }));
    const choice = current.keepMine();
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    await current.useDiskVersion();
    expect(rollback).not.toHaveBeenCalled();
    expect(current.decisionBusy()).toBe(true);
    finish([]);
    await choice;
  });

  it("re-reads disk after crop retirement before publishing Use disk version", async () => {
    const current = await conflictedState();
    const changed = highlight("disk-only");
    const read = vi.spyOn(backend(), "readHighlights").mockResolvedValueOnce([]).mockResolvedValueOnce([changed]);
    vi.spyOn(backend(), "rollbackPdfAreaImage").mockResolvedValue(undefined);
    await current.useDiskVersion();
    expect(read).toHaveBeenCalledTimes(2);
    expect(current.highlights()).toEqual([changed]);
  });

  it("keeps a failed post-save crop retirement pending until explicit retry", async () => {
    const current = state();
    current.load([highlight("a", "yellow", 42)]);
    current.addCrop("a", { page: 1, stamp: 42 });
    current.edit([]);
    vi.spyOn(backend(), "writeHighlights").mockResolvedValue([]);
    const rollback = vi.spyOn(backend(), "rollbackPdfAreaImage")
      .mockRejectedValueOnce(new Error("trash denied")).mockResolvedValue(undefined);
    expect(await current.persist()).toBe(false);
    expect(current.cleanupPending()).toBe(true);
    expect(current.unsaved()).toBe(true);
    expect(current.drainBlocked()).toBe(true);
    await current.retryCleanup();
    expect(rollback).toHaveBeenCalledTimes(2);
    expect(current.drainBlocked()).toBe(false);
  });

  it.each(["malformed EDN", "I/O failure"])("confirmed discard releases a %s conflict without writing disk", async (failure) => {
    const current = await conflictedState();
    const read = vi.spyOn(backend(), "readHighlights").mockRejectedValue(new Error(failure));
    await current.useDiskVersion();
    await current.keepMine();
    expect(current.drainBlocked()).toBe(true);
    const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    await current.discardMine();
    expect(confirm).toHaveBeenCalledOnce();
    expect(read).toHaveBeenCalledTimes(2);
    expect(current.drainBlocked()).toBe(false);
    expect(backend().writeHighlights).toHaveBeenCalledTimes(1);
  });

  it("keeps the conflict and local set when discard confirmation is denied", async () => {
    const current = await conflictedState();
    vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    await current.discardMine();
    expect(current.conflict()).toBe(true);
    expect(current.highlights()).toEqual([highlight("a", "yellow", 42)]);
    expect(current.drainBlocked()).toBe(true);
  });
  it("rebases changed fields and local deletion while preserving disk-only additions", () => {
    const a = highlight("a");
    const b = highlight("b");
    const diskOnly = highlight("disk");
    expect(rebasePdfHighlights([a, b], [{ ...a, color: "green" }], [
      { ...a, text: "disk text" }, { ...b, color: "blue" }, diskOnly,
    ])).toEqual([{ ...a, color: "green", text: "disk text" }, diskOnly]);
  });

  it("keeps a locally changed highlight when disk deleted its baseline", () => {
    const original = highlight("a");
    expect(rebasePdfHighlights([original], [{ ...original, color: "green" }], []))
      .toEqual([{ ...original, color: "green" }]);
    expect(rebasePdfHighlights([original], [original], [])).toEqual([]);
  });

  it("retires a crop only after neither committed nor optimistic highlights use it", () => {
    const crops = new Map([["a", { page: 1, stamp: 42 }]]);
    const area = highlight("a", "yellow", 42);
    expect(unusedPdfCrops(crops, [], [area])).toEqual([]);
    expect(unusedPdfCrops(crops, [area], [])).toEqual([]);
    expect(unusedPdfCrops(crops, [], [])).toEqual([["a", { page: 1, stamp: 42 }]]);
  });
});
