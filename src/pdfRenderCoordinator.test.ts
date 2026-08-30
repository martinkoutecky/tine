import { describe, expect, it, vi } from "vitest";
import {
  PDF_RENDERING_FINISHED,
  PDF_RENDERING_INITIAL,
  PDF_RENDERING_PAUSED,
  PDF_RENDERING_RUNNING,
  PdfRenderCoordinator,
  TinePdfRenderingQueue,
  type PdfRenderableView,
  type PdfVisiblePages,
} from "./pdfRenderCoordinator";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

function fakeCanvas(width: number, height: number): HTMLCanvasElement {
  return { width, height } as HTMLCanvasElement;
}

function fakeView(id: number, pixels = 0): PdfRenderableView & {
  reset: ReturnType<typeof vi.fn>;
  destroy: ReturnType<typeof vi.fn>;
} {
  const completion = deferred();
  return {
    id,
    renderingId: `page${id}`,
    renderingState: PDF_RENDERING_INITIAL,
    canvas: pixels ? fakeCanvas(pixels, 1) : null,
    reset: vi.fn(function (this: PdfRenderableView) {
      if (this.canvas) this.canvas.width = this.canvas.height = 0;
      this.renderingState = PDF_RENDERING_INITIAL;
    }),
    destroy: vi.fn(),
    draw: vi.fn(function (this: PdfRenderableView) {
      this.renderingState = PDF_RENDERING_RUNNING;
      return completion.promise.then(() => { this.renderingState = PDF_RENDERING_FINISHED; });
    }),
    resume: vi.fn(function (this: PdfRenderableView) {
      this.renderingState = PDF_RENDERING_RUNNING;
    }),
    __completion: completion,
  } as PdfRenderableView & {
    reset: ReturnType<typeof vi.fn>;
    destroy: ReturnType<typeof vi.fn>;
  };
}

function visible(...views: PdfRenderableView[]): PdfVisiblePages {
  const entries = views.map((view) => ({ id: view.id, view }));
  return {
    first: entries[0],
    last: entries.at(-1)!,
    views: entries,
    ids: new Set(entries.map(({ id }) => id)),
  };
}

async function turns(count = 3): Promise<void> {
  for (let index = 0; index < count; index += 1) await Promise.resolve();
}

describe("Tine PDF render coordination", () => {
  it("injects the remaining measured pixel allowance before rendering", async () => {
    const coordinator = new PdfRenderCoordinator(1_000, 800);
    const existing = fakeView(1, 350);
    existing.renderingState = PDF_RENDERING_FINISHED;
    const incoming = fakeView(2);
    const queue = new TinePdfRenderingQueue({
      coordinator,
      cachedViews: () => [existing, incoming],
    });
    queue.setViewer({
      getCachedPageViews: () => new Set([existing, incoming]),
      forceRendering: () => queue.renderView(incoming),
    });

    queue.renderHighestPriority(visible(incoming));
    await turns();

    expect(incoming.maxCanvasPixels).toBe(650);
    expect(incoming.draw).toHaveBeenCalledOnce();
    queue.dispose();
  });

  it("pauses background continuation, runs focused work, then resumes", async () => {
    const coordinator = new PdfRenderCoordinator(2_000, 1_000);
    const low = fakeView(1);
    const high = fakeView(2);
    const lowQueue = new TinePdfRenderingQueue({ coordinator, priority: () => 10, cachedViews: () => [low] });
    const highQueue = new TinePdfRenderingQueue({ coordinator, priority: () => 0, cachedViews: () => [high] });
    lowQueue.setViewer({ getCachedPageViews: () => new Set([low]), forceRendering: () => lowQueue.renderView(low) });
    highQueue.setViewer({ getCachedPageViews: () => new Set([high]), forceRendering: () => highQueue.renderView(high) });

    lowQueue.renderHighestPriority(visible(low));
    await turns();
    expect(low.renderingState).toBe(PDF_RENDERING_RUNNING);

    highQueue.renderHighestPriority(visible(high));
    expect(lowQueue.isHighestPriority(low)).toBe(false);
    low.renderingState = PDF_RENDERING_PAUSED;
    await turns();
    expect(high.draw).toHaveBeenCalledOnce();

    (high as unknown as { __completion: ReturnType<typeof deferred> }).__completion.resolve();
    await turns(5);
    expect(low.resume).toHaveBeenCalledOnce();

    lowQueue.dispose();
    highQueue.dispose();
  });

  it("resets cold non-visible canvases and never destroys shared page proxies", () => {
    const coordinator = new PdfRenderCoordinator(1_000, 800);
    const cold = fakeView(1, 700);
    const visibleView = fakeView(2, 700);
    cold.renderingState = visibleView.renderingState = PDF_RENDERING_FINISHED;
    const queue = new TinePdfRenderingQueue({ coordinator, cachedViews: () => [cold, visibleView] });
    queue.setViewer({ getCachedPageViews: () => new Set([cold, visibleView]), forceRendering: () => false });
    queue.renderHighestPriority(visible(visibleView));

    coordinator.enforcePixelBudget();

    expect(cold.reset).toHaveBeenCalledOnce();
    expect(cold.destroy).not.toHaveBeenCalled();
    expect(visibleView.reset).not.toHaveBeenCalled();
    expect(coordinator.retainedPixels()).toBe(700);
    queue.dispose();
  });
});
