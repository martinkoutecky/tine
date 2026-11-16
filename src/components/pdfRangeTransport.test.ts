// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { createPdfRangeTransport, PDF_RANGE_CHUNK_SIZE as CHUNK } from "./pdfRangeTransport";

vi.mock("pdfjs-dist", () => ({ PDFDataRangeTransport: class {
  constructor(public length: number) {}
  onDataRange = vi.fn();
} }));

async function flush() { for (let i = 0; i < 30; i++) await Promise.resolve(); }

describe("PDF asset ranges", () => {
  it("opens without reading and returns coalesced ranges through capped reads", async () => {
    const source = { length: CHUNK * 4, read: vi.fn(async (begin: number, end: number) => new Uint8Array(end - begin).fill(begin / CHUNK + 1)) };
    const { range } = createPdfRangeTransport(source, () => true);
    expect(source.read).not.toHaveBeenCalled();
    range.requestDataRange(CHUNK, CHUNK * 3 + 10);
    await flush();
    expect(source.read.mock.calls).toEqual([[CHUNK, CHUNK * 2], [CHUNK * 2, CHUNK * 3], [CHUNK * 3, CHUNK * 3 + 10]]);
    expect(range.onDataRange).toHaveBeenCalledOnce();
    const [begin, bytes] = vi.mocked(range.onDataRange).mock.calls[0];
    expect(begin).toBe(CHUNK); expect(bytes!.length).toBe(CHUNK * 2 + 10);
    expect([bytes![0], bytes![CHUNK], bytes![CHUNK * 2]]).toEqual([2, 3, 4]);
  });

  it("serializes IPC reads and discards late completion after teardown", async () => {
    let finish!: (bytes: Uint8Array) => void;
    const source = { length: CHUNK * 4, read: vi.fn(() => new Promise<Uint8Array>((resolve) => { finish = resolve; })) };
    const { range } = createPdfRangeTransport(source, () => true);
    range.requestDataRange(0, CHUNK); range.requestDataRange(CHUNK, CHUNK * 2);
    await flush(); expect(source.read).toHaveBeenCalledOnce();
    range.abort(); finish(new Uint8Array(CHUNK));
    await flush();
    expect(source.read).toHaveBeenCalledOnce(); expect(range.onDataRange).not.toHaveBeenCalled();
  });

  it("does not start queued reads after the graph binding retires", async () => {
    let current = true;
    const source = { length: CHUNK * 4, read: vi.fn(async () => { current = false; return new Uint8Array(CHUNK); }) };
    const { range } = createPdfRangeTransport(source, () => current);
    range.requestDataRange(0, CHUNK); range.requestDataRange(CHUNK, CHUNK * 2);
    await flush();
    expect(source.read).toHaveBeenCalledOnce(); expect(range.onDataRange).not.toHaveBeenCalled();
  });

  it.each(["rejected", "truncated", "invalid"])("reports a %s range once instead of hanging the load", async (kind) => {
    const source = { length: CHUNK * 4, read: vi.fn(async () => {
      if (kind === "rejected") throw new Error("asset changed");
      return new Uint8Array(1);
    }) };
    const { range, failure } = createPdfRangeTransport(source, () => true);
    range.requestDataRange(kind === "invalid" ? -1 : 0, CHUNK);
    await expect(failure).resolves.toContain("Couldn't read this PDF asset");
    range.requestDataRange(CHUNK, CHUNK * 2); await flush();
    expect(range.onDataRange).not.toHaveBeenCalled();
    expect(source.read).toHaveBeenCalledTimes(kind === "invalid" ? 0 : 1);
  });
});
