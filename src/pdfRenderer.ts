import { AnnotationMode, PixelsPerInch } from "pdfjs-dist";
import { EventBus, PDFPageView } from "pdfjs-dist/web/pdf_viewer.mjs";
import {
  PdfRenderCoordinator,
  TinePdfRenderingQueue,
  type PdfRenderableView,
  type PdfVisiblePages,
} from "./pdfRenderCoordinator";

export const TINE_PDF_LOADING_OPTIONS = Object.freeze({ disableAutoFetch: true } as const);

export interface PdfViewportLike {
  width: number;
  height: number;
  rotation: number;
  clone(options?: { scale?: number; rotation?: number }): PdfViewportLike;
  convertToPdfPoint(x: number, y: number): number[];
}

export interface PdfPageProxyLike {
  getViewport(options: { scale: number; rotation?: number }): PdfViewportLike;
}

export interface PdfDocumentProxyLike {
  getPage(pageNumber: number): Promise<PdfPageProxyLike>;
}

export interface DirectPdfPageView extends PdfRenderableView {
  div: HTMLDivElement;
  setPdfPage(page: PdfPageProxyLike): void;
  update(options: { scale?: number; rotation?: number; drawingDelay?: number }): void;
}

export interface DirectPdfPageViewOptions {
  container: HTMLDivElement;
  eventBus: EventBus;
  id: number;
  scale: number;
  defaultViewport: PdfViewportLike;
  renderingQueue: TinePdfRenderingQueue;
  textLayerMode: number;
  annotationMode: number;
  maxCanvasPixels: number;
}

export type DirectPdfPageViewFactory = (options: DirectPdfPageViewOptions) => DirectPdfPageView;

export interface PdfPageViewRendererOptions {
  document: PdfDocumentProxyLike;
  coordinator: PdfRenderCoordinator;
  priority?: () => number;
  eventBus?: EventBus;
  createPageView?: DirectPdfPageViewFactory;
}

interface PageRecord {
  token: symbol;
  displayScale: number;
  view: DirectPdfPageView | null;
}

const TEXT_LAYER_ENABLED = 1;

function defaultPageViewFactory(options: DirectPdfPageViewOptions): DirectPdfPageView {
  // PDFPageView's declaration names PDFRenderingQueue, but its runtime contract
  // is the structural five-method queue implemented by TinePdfRenderingQueue.
  return new PDFPageView({
    ...options,
    renderingQueue: options.renderingQueue as never,
    defaultViewport: options.defaultViewport as never,
  }) as unknown as DirectPdfPageView;
}

export function tineScaleToPdfPageViewScale(displayScale: number): number {
  if (!Number.isFinite(displayScale) || displayScale <= 0) {
    throw new Error("PDF display scale must be a positive finite number");
  }
  // PDFPageView multiplies its scale by PDF_TO_CSS_UNITS before asking the page
  // for a viewport. Tine's existing scale already describes that viewport.
  return displayScale / PixelsPerInch.PDF_TO_CSS_UNITS;
}

/**
 * Owns direct PDFPageView instances for one pane/view. Document/page proxies
 * remain session-owned; this object only resets view resources.
 */
export class PdfPageViewRenderer {
  private readonly eventBus: EventBus;
  private readonly createPageView: DirectPdfPageViewFactory;
  private readonly queue: TinePdfRenderingQueue;
  private readonly pages = new Map<number, PageRecord>();
  private visiblePageNumbers: number[] = [];
  private scrolledDown = true;
  private disposed = false;

  constructor(private readonly options: PdfPageViewRendererOptions) {
    this.eventBus = options.eventBus ?? new EventBus();
    this.createPageView = options.createPageView ?? defaultPageViewFactory;
    this.queue = new TinePdfRenderingQueue({
      coordinator: options.coordinator,
      priority: options.priority,
      cachedViews: () => this.cachedViews(),
    });
    // This must happen before PDFPageView construction. Otherwise PDF.js marks
    // the page view standalone and bypasses the injected rendering queue.
    this.queue.setViewer(this);
  }

  async mountPage(
    pageNumber: number,
    container: HTMLDivElement,
    displayScale: number,
  ): Promise<DirectPdfPageView | null> {
    this.assertActive();
    if (!Number.isSafeInteger(pageNumber) || pageNumber < 1) {
      throw new Error("PDF page number must be a positive safe integer");
    }
    tineScaleToPdfPageViewScale(displayScale);

    this.unmountPage(pageNumber);
    const record: PageRecord = {
      token: Symbol(`pdf-page-${pageNumber}`),
      displayScale,
      view: null,
    };
    this.pages.set(pageNumber, record);

    const page = await this.options.document.getPage(pageNumber);
    if (this.disposed || this.pages.get(pageNumber)?.token !== record.token) return null;

    const view = this.createPageView({
      container,
      eventBus: this.eventBus,
      id: pageNumber,
      scale: tineScaleToPdfPageViewScale(record.displayScale),
      defaultViewport: page.getViewport({ scale: 1 }),
      renderingQueue: this.queue,
      textLayerMode: TEXT_LAYER_ENABLED,
      annotationMode: AnnotationMode.DISABLE,
      maxCanvasPixels: this.options.coordinator.perPagePixelLimit,
    });
    // A factory is synchronous today, but retain the token check at the last
    // mutation point so a future wrapper cannot attach to a replaced slot.
    if (this.disposed || this.pages.get(pageNumber)?.token !== record.token) {
      view.reset();
      view.div.remove();
      return null;
    }
    view.setPdfPage(page);
    record.view = view;
    if (this.visiblePageNumbers.includes(pageNumber)) this.requestVisibleRendering();
    return view;
  }

  unmountPage(pageNumber: number): void {
    const record = this.pages.get(pageNumber);
    if (!record) return;
    this.pages.delete(pageNumber);
    if (record.view) {
      record.view.reset();
      record.view.div.remove();
    }
  }

  updateScale(pageNumber: number, displayScale: number, drawingDelay = -1): void {
    const record = this.pages.get(pageNumber);
    if (!record) return;
    const pageViewScale = tineScaleToPdfPageViewScale(displayScale);
    record.displayScale = displayScale;
    record.view?.update({
      scale: pageViewScale,
      drawingDelay,
    });
    if (record.view && this.visiblePageNumbers.includes(pageNumber)) {
      this.requestVisibleRendering();
    }
  }

  setVisiblePages(pageNumbers: Iterable<number>, scrolledDown: boolean): void {
    this.assertActive();
    this.visiblePageNumbers = [...new Set(pageNumbers)].sort((left, right) => left - right);
    this.scrolledDown = scrolledDown;
    this.requestVisibleRendering();
  }

  getPageView(pageNumber: number): DirectPdfPageView | null {
    return this.pages.get(pageNumber)?.view ?? null;
  }

  getCachedPageViews(): Set<PdfRenderableView> {
    return new Set(this.cachedViews());
  }

  forceRendering(visible?: PdfVisiblePages): boolean {
    if (this.disposed) return false;
    const current = visible ?? this.buildVisiblePages();
    if (!current) return false;
    const byPageNumber: PdfRenderableView[] = [];
    for (const [pageNumber, record] of this.pages) {
      if (record.view) byPageNumber[pageNumber - 1] = record.view;
    }
    const next = this.queue.getHighestPriority(
      current,
      byPageNumber,
      this.scrolledDown,
    );
    return next ? this.queue.renderView(next) : false;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const pageNumber of [...this.pages.keys()]) this.unmountPage(pageNumber);
    this.visiblePageNumbers = [];
    this.queue.dispose();
  }

  private cachedViews(): DirectPdfPageView[] {
    const views: DirectPdfPageView[] = [];
    for (const record of this.pages.values()) {
      if (record.view) views.push(record.view);
    }
    return views;
  }

  private requestVisibleRendering(): void {
    const visible = this.buildVisiblePages();
    if (visible) this.queue.renderHighestPriority(visible);
  }

  private buildVisiblePages(): PdfVisiblePages | null {
    const entries = this.visiblePageNumbers.flatMap((pageNumber) => {
      const view = this.pages.get(pageNumber)?.view;
      return view ? [{ id: pageNumber, view }] : [];
    });
    if (!entries.length) return null;
    return {
      first: entries[0],
      last: entries.at(-1)!,
      views: entries,
      ids: new Set(entries.map(({ id }) => id)),
    };
  }

  private assertActive(): void {
    if (this.disposed) throw new Error("PDF page renderer has been disposed");
  }
}
