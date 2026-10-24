import { afterEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { backend } from "../backend";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import type { Highlight } from "../types";
import {
  acquirePdfHighlightState, createPdfHighlightState, pdfHighlightSessionCount, rebasePdfHighlights, unusedPdfCrops,
} from "./pdfHighlightState";

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

  it("gives every view of one PDF the same state and frees it with the last view", () => {
    const owner = activatePdfOwnership("/test/pdf-graph");
    const options = { filename: "paper.pdf", label: "Paper", backendGeneration: 0, owner };
    const a = acquirePdfHighlightState(options), b = acquirePdfHighlightState(options);
    const other = acquirePdfHighlightState({ ...options, filename: "other.pdf" });
    expect(b.state).toBe(a.state);
    expect(other.state).not.toBe(a.state);
    expect(a.state.loadInitial([highlight("a")])).toBe(true);
    expect(b.state.loadInitial([])).toBe(false); // a joining view keeps what its sibling holds
    expect(b.state.highlights().map((h) => h.id)).toEqual(["a"]);
    a.release();
    a.release(); // idempotent
    const again = acquirePdfHighlightState(options);
    expect(again.state).toBe(b.state);
    other.release();
    expect(pdfHighlightSessionCount()).toBe(1);
    // A new graph binding never borrows the old binding's state.
    const next = acquirePdfHighlightState({ ...options, owner: activatePdfOwnership("/test/pdf-graph") });
    expect(next.state).not.toBe(b.state);
    for (const lease of [b, next]) lease.release();
    again.release();
    expect(pdfHighlightSessionCount()).toBe(0);
  });

  it("writes once when several views flush together, through the latest live view's geometry", async () => {
    const current = state();
    const write = vi.spyOn(backend(), "writeHighlights").mockImplementation(async (_pdf, _label, items) => items);
    const fromA = vi.fn(async (items: Highlight[]) => items.map((h) => ({ ...h, text: "A" })));
    const fromB = vi.fn(async (items: Highlight[]) => items.map((h) => ({ ...h, text: "B" })));
    current.attachPrepare(fromA);
    const detachB = current.attachPrepare(fromB);
    current.edit([highlight("a")]);
    expect(await Promise.all([current.flush(), current.flush()])).toEqual([true, true]);
    expect(write).toHaveBeenCalledTimes(1);
    expect(write.mock.calls[0][2][0].text).toBe("B");
    detachB();
    current.edit([highlight("a", "green")]);
    expect(await current.flush()).toBe(true);
    expect(write).toHaveBeenCalledTimes(2);
    expect(write.mock.calls[1][2][0].text).toBe("A");
    expect(await current.flush()).toBe(true); // nothing unsaved: no write
    expect(write).toHaveBeenCalledTimes(2);
  });

  it("re-checks a blocked drain on every flush", async () => {
    const current = await conflictedState();
    expect(await current.flush()).toBe(false);
    current.load([]); // the conflict is decided
    expect(await current.flush()).toBe(true); // a once-refused drain must not stay refused
  });

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
