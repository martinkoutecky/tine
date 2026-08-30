// PDF.js exports PDFPageView but keeps PDFRenderingQueue private. This
// structural adapter implements the queue methods used by PDFPageView while a
// single coordinator owns render admission and backing-store accounting.

export const PDF_RENDERING_INITIAL = 0;
export const PDF_RENDERING_RUNNING = 1;
export const PDF_RENDERING_PAUSED = 2;
export const PDF_RENDERING_FINISHED = 3;

export interface PdfRenderableView {
  id: number;
  renderingId: string;
  renderingState: number;
  resume?: (() => void) | null;
  draw(): Promise<unknown>;
  reset(): void;
  maxCanvasPixels?: number;
  canvas?: HTMLCanvasElement | null;
}

export interface PdfVisibleEntry {
  id: number;
  view: PdfRenderableView;
}

export interface PdfVisiblePages {
  first: PdfVisibleEntry;
  last: PdfVisibleEntry;
  views: PdfVisibleEntry[];
  ids: Set<number>;
}

export interface PdfViewerQueueHost {
  forceRendering(visible?: PdfVisiblePages): boolean;
  getCachedPageViews(): Set<PdfRenderableView>;
}

export interface TinePdfQueueOptions {
  coordinator: PdfRenderCoordinator;
  /** Lower values win. The focused visible pane normally returns zero. */
  priority?: () => number;
  cachedViews?: () => Iterable<PdfRenderableView>;
}

interface ActiveRender {
  queue: TinePdfRenderingQueue;
  view: PdfRenderableView;
}

function canvasPixels(view: PdfRenderableView): number {
  const width = view.canvas?.width ?? 0;
  const height = view.canvas?.height ?? 0;
  return Number.isSafeInteger(width) && Number.isSafeInteger(height) ? width * height : 0;
}

export class PdfRenderCoordinator {
  private readonly queues = new Set<TinePdfRenderingQueue>();
  private readonly pending = new Set<TinePdfRenderingQueue>();
  private active: ActiveRender | null = null;
  private pumpQueued = false;
  private clock = 0;
  private readonly touched = new WeakMap<PdfRenderableView, number>();

  constructor(
    readonly pixelBudget: number,
    readonly perPagePixelLimit: number,
  ) {
    if (!Number.isSafeInteger(pixelBudget) || pixelBudget < 1) {
      throw new Error("PDF render pixel budget must be a positive safe integer");
    }
    if (!Number.isSafeInteger(perPagePixelLimit) || perPagePixelLimit < 1) {
      throw new Error("PDF per-page pixel limit must be a positive safe integer");
    }
  }

  register(queue: TinePdfRenderingQueue): void {
    this.queues.add(queue);
  }

  unregister(queue: TinePdfRenderingQueue): void {
    this.queues.delete(queue);
    this.pending.delete(queue);
    if (this.active?.queue === queue) this.active = null;
    this.schedulePump();
  }

  request(queue: TinePdfRenderingQueue): void {
    if (!this.queues.has(queue)) return;
    this.pending.add(queue);
    this.schedulePump();
  }

  isHighestPriority(queue: TinePdfRenderingQueue, view: PdfRenderableView): boolean {
    if (this.active?.queue !== queue || this.active.view !== view) return false;
    const preferred = this.preferredPending();
    if (!preferred || preferred.priority() >= queue.priority()) return true;

    // PDFPageView sets PAUSED and installs resume immediately after this false
    // return. Pump in a microtask so that state transition is observable.
    this.active = null;
    this.pending.add(queue);
    this.schedulePump();
    return false;
  }

  admit(queue: TinePdfRenderingQueue, view: PdfRenderableView): boolean {
    if (!this.queues.has(queue)) return false;
    if (this.active && (this.active.queue !== queue || this.active.view !== view)) {
      this.pending.add(queue);
      return false;
    }
    const preferred = this.preferredPending();
    if (preferred && preferred !== queue && preferred.priority() < queue.priority()) {
      this.pending.add(queue);
      return false;
    }
    this.pending.delete(queue);
    this.active = { queue, view };
    this.touch(view);
    view.maxCanvasPixels = this.availablePixels(view);
    return true;
  }

  complete(queue: TinePdfRenderingQueue, view: PdfRenderableView): void {
    this.touch(view);
    if (this.active?.queue === queue && this.active.view === view) this.active = null;
    this.enforcePixelBudget();
    // Ask the queue again so PDF.js can choose an adjacent page after all
    // visible pages have completed.
    this.request(queue);
  }

  retainedPixels(): number {
    let total = 0;
    for (const view of this.cachedViews()) total += canvasPixels(view);
    return total;
  }

  enforcePixelBudget(): void {
    let retained = this.retainedPixels();
    if (retained <= this.pixelBudget) return;

    const candidates = [...this.queues]
      .flatMap((queue) => [...queue.cachedViews()].map((view) => ({ queue, view })))
      .filter(({ queue, view }) => !queue.isVisible(view.id) && this.active?.view !== view)
      .sort((left, right) =>
        (this.touched.get(left.view) ?? 0) - (this.touched.get(right.view) ?? 0)
        || left.view.id - right.view.id
      );
    const seen = new Set<PdfRenderableView>();
    for (const { view } of candidates) {
      if (retained <= this.pixelBudget) break;
      if (seen.has(view)) continue;
      seen.add(view);
      const pixels = canvasPixels(view);
      if (!pixels) continue;
      // PDFPageView.destroy() also calls PDFPageProxy.cleanup(), which is not
      // view-owned when multiple views share a document session.
      view.reset();
      retained -= pixels;
    }
  }

  private availablePixels(incoming: PdfRenderableView): number {
    let retained = 0;
    for (const view of this.cachedViews()) {
      if (view !== incoming) retained += canvasPixels(view);
    }
    return Math.max(1, Math.min(this.perPagePixelLimit, this.pixelBudget - retained));
  }

  private *cachedViews(): Iterable<PdfRenderableView> {
    const seen = new Set<PdfRenderableView>();
    for (const queue of this.queues) {
      for (const view of queue.cachedViews()) {
        if (seen.has(view)) continue;
        seen.add(view);
        yield view;
      }
    }
  }

  private touch(view: PdfRenderableView): void {
    this.touched.set(view, ++this.clock);
  }

  private preferredPending(): TinePdfRenderingQueue | undefined {
    return [...this.pending].sort((left, right) =>
      left.priority() - right.priority() || left.order - right.order
    )[0];
  }

  private schedulePump(): void {
    if (this.pumpQueued) return;
    this.pumpQueued = true;
    queueMicrotask(() => {
      this.pumpQueued = false;
      this.pump();
    });
  }

  private pump(): void {
    if (this.active?.view.renderingState === PDF_RENDERING_RUNNING) return;
    if (this.active) this.active = null;
    // Skip exhausted queues in the same turn so a paused render behind them is
    // resumed without waiting for an unrelated future event.
    for (let queue = this.preferredPending(); queue; queue = this.preferredPending()) {
      this.pending.delete(queue);
      if (queue.renderNext()) return;
    }
  }
}

let nextQueueOrder = 1;

export class TinePdfRenderingQueue {
  readonly order = nextQueueOrder++;
  private viewer: PdfViewerQueueHost | null = null;
  private visible: PdfVisiblePages | null = null;
  private disposed = false;

  constructor(private readonly options: TinePdfQueueOptions) {
    options.coordinator.register(this);
  }

  setViewer(viewer: PdfViewerQueueHost): void {
    this.viewer = viewer;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.viewer = null;
    this.visible = null;
    this.options.coordinator.unregister(this);
  }

  hasViewer = (): boolean => this.viewer !== null;

  priority(): number {
    const value = this.options.priority?.() ?? 0;
    return Number.isFinite(value) ? value : Number.MAX_SAFE_INTEGER;
  }

  cachedViews(): Iterable<PdfRenderableView> {
    return this.options.cachedViews?.() ?? this.viewer?.getCachedPageViews() ?? [];
  }

  isVisible(pageId: number): boolean {
    return this.visible?.ids.has(pageId) ?? false;
  }

  isHighestPriority = (view: PdfRenderableView): boolean =>
    this.options.coordinator.isHighestPriority(this, view);

  renderHighestPriority = (visible: PdfVisiblePages): void => {
    this.visible = visible;
    this.options.coordinator.request(this);
  };

  renderNext(): boolean {
    return this.viewer?.forceRendering(this.visible ?? undefined) ?? false;
  }

  getHighestPriority(
    visible: PdfVisiblePages,
    views: PdfRenderableView[],
    scrolledDown: boolean,
    preRenderExtra = false,
  ): PdfRenderableView | null {
    const visibleViews = visible.views;
    if (!visibleViews.length) return null;
    for (const { view } of visibleViews) {
      if (!this.isViewFinished(view)) return view;
    }
    const firstId = visible.first.id;
    const lastId = visible.last.id;
    if (lastId - firstId + 1 > visibleViews.length) {
      for (let offset = 1; offset < lastId - firstId; offset += 1) {
        const id = scrolledDown ? firstId + offset : lastId - offset;
        if (visible.ids.has(id)) continue;
        const hole = views[id - 1];
        if (hole && !this.isViewFinished(hole)) return hole;
      }
    }
    let index = scrolledDown ? lastId : firstId - 2;
    let candidate = views[index];
    if (candidate && !this.isViewFinished(candidate)) return candidate;
    if (preRenderExtra) {
      index += scrolledDown ? 1 : -1;
      candidate = views[index];
      if (candidate && !this.isViewFinished(candidate)) return candidate;
    }
    return null;
  }

  isViewFinished(view: PdfRenderableView): boolean {
    return view.renderingState === PDF_RENDERING_FINISHED;
  }

  renderView = (view: PdfRenderableView): boolean => {
    if (this.disposed || this.isViewFinished(view)) return false;
    if (!this.options.coordinator.admit(this, view)) {
      this.options.coordinator.request(this);
      return true;
    }
    switch (view.renderingState) {
      case PDF_RENDERING_PAUSED:
        view.resume?.();
        break;
      case PDF_RENDERING_RUNNING:
        break;
      case PDF_RENDERING_INITIAL:
      default:
        void view.draw().catch((error: unknown) => {
          if ((error as { name?: string } | undefined)?.name !== "RenderingCancelledException") {
            console.error("PDF page render failed", error);
          }
        }).finally(() => this.options.coordinator.complete(this, view));
        break;
    }
    return true;
  };
}
