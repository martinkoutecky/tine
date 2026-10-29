import { afterEach, expect, it, vi } from "vitest";
import type { PDFPageProxy } from "pdfjs-dist";
import { releasePdfPage, renderPdfPage } from "./pdfRenderTask";

afterEach(() => { vi.useRealTimers(); });

// Promise.withResolvers needs Node 22; local gates run Node 20.
type Job = { promise: Promise<void>; resolve: () => void; reject: (error: unknown) => void };
function deferred(): Job {
  let resolve!: () => void, reject!: (error: unknown) => void;
  const promise = new Promise<void>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function fixture(bytes = 14_641_011) {
  const images = new Map<string, unknown>([["image", { dataLen: bytes }]]);
  const jobs: Job[] = [];
  const cleanup = vi.fn(() => { images.clear(); return true; });
  const page = { objs: images, cleanup, render: vi.fn(() => {
    const job = deferred(); jobs.push(job);
    return { promise: job.promise, cancel: vi.fn() };
  }) } as unknown as PDFPageProxy;
  return { page, images, jobs, cleanup };
}

it("releases a scan's decoded buffers after the same five-second grace as pdf.js 4", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const task = renderPdfPage(f.page, {} as never);
  f.jobs[0].resolve(); await task.promise;
  await vi.advanceTimersByTimeAsync(4999);
  expect(f.images.size).toBe(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(f.images.size).toBe(0);
  expect(f.cleanup).toHaveBeenCalledTimes(1);
});

it("keeps small decoded images and the original task API", async () => {
  vi.useFakeTimers();
  const f = fixture(10_000_000);
  const task = renderPdfPage(f.page, {} as never);
  expect(task.cancel).toBeTypeOf("function");
  f.jobs[0].resolve(); await task.promise;
  await vi.advanceTimersByTimeAsync(20_000);
  expect(f.images.size).toBe(1);
  expect(f.cleanup).not.toHaveBeenCalled();
});

it.each([
  { data: new Uint8Array(10_000_001) },
  { bitmap: { width: 2501, height: 1000 } },
])("recognizes current decoded-image payloads without the old dataLen hint", async image => {
  vi.useFakeTimers();
  const f = fixture(); f.images.set("image", image);
  const task = renderPdfPage(f.page, {} as never);
  f.jobs[0].resolve(); await task.promise;
  await vi.advanceTimersByTimeAsync(5000);
  expect(f.images.size).toBe(0);
});

it("shares one grace period across overlapping split and tile renders", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const first = renderPdfPage(f.page, {} as never);
  const second = renderPdfPage(f.page, {} as never);
  f.jobs[0].resolve(); await first.promise;
  await vi.advanceTimersByTimeAsync(6000);
  expect(f.cleanup).not.toHaveBeenCalled();
  f.jobs[1].resolve(); await second.promise;
  await vi.advanceTimersByTimeAsync(4000);
  const third = renderPdfPage(f.page, {} as never);
  await vi.advanceTimersByTimeAsync(6000);
  expect(f.cleanup).not.toHaveBeenCalled();
  f.jobs[2].resolve(); await third.promise;
  await vi.advanceTimersByTimeAsync(5000);
  expect(f.cleanup).toHaveBeenCalledTimes(1);
});

it("cancels delayed cleanup on the last release, including a later cancelled completion", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const first = renderPdfPage(f.page, {} as never);
  f.jobs[0].resolve(); await first.promise;
  releasePdfPage(f.page);
  await vi.advanceTimersByTimeAsync(6000);
  expect(f.cleanup).toHaveBeenCalledTimes(1);
  const next = renderPdfPage(f.page, {} as never);
  f.images.set("image", { data: new Uint8Array(10_000_001) });
  releasePdfPage(f.page);
  f.jobs[1].reject(new Error("cancelled"));
  await expect(next.promise).rejects.toThrow("cancelled");
  await vi.advanceTimersByTimeAsync(6000);
  expect(f.cleanup).toHaveBeenCalledTimes(2);
});
