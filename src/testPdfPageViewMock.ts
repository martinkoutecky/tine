interface Listener {
  (event: unknown): void;
}

export class EventBus {
  private readonly listeners = new Map<string, Set<Listener>>();

  on(name: string, listener: Listener): void {
    const listeners = this.listeners.get(name) ?? new Set<Listener>();
    listeners.add(listener);
    this.listeners.set(name, listeners);
  }

  off(name: string, listener: Listener): void {
    this.listeners.get(name)?.delete(listener);
  }

  dispatch(name: string, event: unknown): void {
    for (const listener of this.listeners.get(name) ?? []) listener(event);
  }
}

interface MockPage {
  getViewport(options: { scale: number }): { width: number; height: number };
  getTextContent?(): Promise<{ items?: Array<{ str?: string }> }>;
  render(options: unknown): {
    promise: Promise<unknown>;
    cancel(): void;
    onContinue?: (continueRendering: () => void) => void;
  };
}

interface PageViewOptions {
  container: HTMLDivElement;
  eventBus: EventBus;
  id: number;
  scale: number;
  maxCanvasPixels: number;
  renderingQueue?: { hasViewer(): boolean; isHighestPriority(view: PDFPageView): boolean };
}

export class PDFPageView {
  readonly id: number;
  readonly renderingId: string;
  readonly div = document.createElement("div");
  renderingState = 0;
  resume: (() => void) | null = null;
  maxCanvasPixels: number;
  canvas: HTMLCanvasElement | null = null;
  textLayer: { div: HTMLDivElement } | null = null;
  scale: number;
  width = 0;
  height = 0;
  private page: MockPage | null = null;
  private renderTask: ReturnType<MockPage["render"]> | null = null;

  constructor(private readonly options: PageViewOptions) {
    this.id = options.id;
    this.renderingId = `page${options.id}`;
    this.scale = options.scale;
    this.maxCanvasPixels = options.maxCanvasPixels;
    this.div.className = "page";
    options.container.append(this.div);
  }

  setPdfPage(page: MockPage): void {
    this.page = page;
    this.updateViewport();
    this.reset();
  }

  update({ scale }: { scale?: number }): void {
    if (scale) this.scale = scale;
    this.updateViewport();
    this.renderTask?.cancel();
    this.renderTask = null;
    this.renderingState = 0;
  }

  async draw(): Promise<void> {
    if (!this.page) throw new Error("pdfPage is not loaded");
    this.renderingState = 1;
    const previous = this.canvas;
    const canvas = document.createElement("canvas");
    const requestedRatio = Math.min(window.devicePixelRatio || 1, 2);
    const ratio = Math.min(
      requestedRatio,
      16_384 / this.width,
      16_384 / this.height,
      Math.sqrt(this.maxCanvasPixels / (this.width * this.height)),
    );
    canvas.width = Math.max(1, Math.floor(this.width * ratio));
    canvas.height = Math.max(1, Math.floor(this.height * ratio));
    const wrapper = this.div.querySelector<HTMLDivElement>(".canvasWrapper")
      ?? this.div.appendChild(Object.assign(document.createElement("div"), { className: "canvasWrapper" }));
    if (!previous) wrapper.append(canvas);
    this.canvas = canvas;
    const task = this.renderTask = this.page.render({
      canvasContext: canvas.getContext("2d"),
      viewport: this.page.getViewport({ scale: this.scale * 4 / 3 }),
      transform: ratio !== 1 ? [ratio, 0, 0, ratio, 0, 0] : undefined,
    });
    task.onContinue = (continueRendering) => {
      if (this.options.renderingQueue && !this.options.renderingQueue.isHighestPriority(this)) {
        this.renderingState = 2;
        this.resume = () => {
          this.renderingState = 1;
          continueRendering();
        };
      } else {
        continueRendering();
      }
    };
    try {
      await task.promise;
    } catch (error) {
      canvas.width = canvas.height = 0;
      canvas.remove();
      if (this.canvas === canvas) this.canvas = previous;
      this.renderingState = 0;
      throw error;
    }
    if (this.renderTask !== task) return;
    this.renderTask = null;
    if (previous) {
      previous.replaceWith(canvas);
      previous.width = previous.height = 0;
    }
    const textLayer = document.createElement("div");
    textLayer.className = "textLayer";
    const text = await this.page.getTextContent?.();
    for (const item of text?.items ?? []) {
      const span = document.createElement("span");
      span.textContent = item.str ?? "";
      textLayer.append(span);
    }
    this.textLayer?.div.remove();
    this.textLayer = { div: textLayer };
    this.div.append(textLayer);
    this.renderingState = 3;
    this.options.eventBus.dispatch("pagerendered", { source: this });
    this.options.eventBus.dispatch("textlayerrendered", { source: this });
  }

  reset(): void {
    this.renderTask?.cancel();
    this.renderTask = null;
    this.resume = null;
    this.renderingState = 0;
    if (this.canvas) {
      this.canvas.width = this.canvas.height = 0;
      this.canvas.remove();
      this.canvas = null;
    }
    this.textLayer?.div.remove();
    this.textLayer = null;
  }

  destroy(): void {
    throw new Error("Tine must never call PDFPageView.destroy()");
  }

  private updateViewport(): void {
    if (!this.page) return;
    const viewport = this.page.getViewport({ scale: this.scale * 4 / 3 });
    this.width = viewport.width;
    this.height = viewport.height;
  }
}
