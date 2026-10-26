import { describe, expect, it, vi } from "vitest";
import type * as pdfjs from "pdfjs-dist";
import { acquirePdfDocument, sharedPdfDocumentCount, type OpenPdfDocument } from "./pdfDocumentShare";

function fakeDoc() {
  return { destroy: vi.fn(async () => {}) } as unknown as pdfjs.PDFDocumentProxy & { destroy: ReturnType<typeof vi.fn> };
}

describe("shared PDF document", () => {
  it("loads once for every view and destroys the document with the last view", async () => {
    const doc = fakeDoc();
    const open = vi.fn<OpenPdfDocument>(async (adopt) => { adopt.document(doc); return { kind: "ok", doc }; });
    const a = acquirePdfDocument("k", open), b = acquirePdfDocument("k", open);
    expect(await a.loaded).toEqual({ kind: "ok", doc });
    expect(await b.loaded).toEqual({ kind: "ok", doc });
    expect(open).toHaveBeenCalledTimes(1);
    a.release();
    a.release(); // idempotent
    expect(doc.destroy).not.toHaveBeenCalled();
    b.release();
    expect(doc.destroy).toHaveBeenCalledTimes(1);
    expect(sharedPdfDocumentCount()).toBe(0);
  });

  it("forgets a failed load so the next view tries again", async () => {
    const doc = fakeDoc();
    const open = vi.fn<OpenPdfDocument>()
      .mockResolvedValueOnce({ kind: "error", message: "busy" })
      .mockImplementationOnce(async (adopt) => { adopt.document(doc); return { kind: "ok", doc }; });
    const a = acquirePdfDocument("k", open);
    expect((await a.loaded).kind).toBe("error");
    const b = acquirePdfDocument("k", open);
    expect(await b.loaded).toEqual({ kind: "ok", doc });
    a.release(); b.release();
    expect(sharedPdfDocumentCount()).toBe(0);
  });

  it("cancels a parse still under way when the last view leaves, without a second destroy", async () => {
    const doc = fakeDoc();
    let resolve!: () => void;
    const task = { destroy: vi.fn(async () => {}) };
    const a = acquirePdfDocument("k", async (adopt) => {
      adopt.task(task);
      await new Promise<void>((r) => { resolve = r; });
      adopt.document(doc);
      return { kind: "ok", doc };
    });
    await vi.waitFor(() => expect(resolve).toBeTypeOf("function"));
    a.release();
    expect(task.destroy).toHaveBeenCalledTimes(1);
    expect(sharedPdfDocumentCount()).toBe(0);
    resolve();
    await a.loaded;
    expect(doc.destroy).not.toHaveBeenCalled(); // the destroyed task already retired it
  });

  it("destroys a loading task created after every view left while the bytes were read", async () => {
    let bytesRead!: () => void;
    const task = { destroy: vi.fn(async () => {}) };
    const a = acquirePdfDocument("k", async (adopt) => {
      await new Promise<void>((r) => { bytesRead = r; });
      adopt.task(task);
      return { kind: "error", message: "destroyed" };
    });
    await vi.waitFor(() => expect(bytesRead).toBeTypeOf("function"));
    a.release();
    bytesRead();
    await a.loaded;
    expect(task.destroy).toHaveBeenCalledTimes(1);
  });

  it("destroys the loading task of a parse that failed, once, and retries on the next view", async () => {
    const task = { destroy: vi.fn(async () => {}) };
    const doc = fakeDoc();
    const open = vi.fn<OpenPdfDocument>()
      .mockImplementationOnce(async (adopt) => { adopt.task(task); return { kind: "error", message: "Invalid PDF structure" }; })
      .mockImplementationOnce(async (adopt) => { adopt.document(doc); return { kind: "ok", doc }; });
    const a = acquirePdfDocument("k", open);
    expect((await a.loaded).kind).toBe("error");
    expect(task.destroy).toHaveBeenCalledTimes(1);
    a.release();
    expect(task.destroy).toHaveBeenCalledTimes(1);
    const b = acquirePdfDocument("k", open);
    expect(await b.loaded).toEqual({ kind: "ok", doc });
    b.release();
  });

  it("destroys an adopted loading task when the opener throws", async () => {
    const task = { destroy: vi.fn(async () => {}) };
    const a = acquirePdfDocument("k", async (adopt) => { adopt.task(task); throw new Error("worker died"); });
    expect(await a.loaded).toEqual({ kind: "error", message: "Error: worker died" });
    a.release();
    expect(task.destroy).toHaveBeenCalledTimes(1);
    expect(sharedPdfDocumentCount()).toBe(0);
  });
});
