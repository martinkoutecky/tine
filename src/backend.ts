// Backend abstraction. In the Tauri app we call Rust via `invoke`. In a plain
// browser (Vite dev / Playwright screenshots) we fall back to an in-memory mock
// seeded from a fixture graph, so the whole UI is exercisable without the shell.

import type { GraphMeta, Highlight, PageDto, PageEntry, RefGroup } from "./types";
import { mockBackend } from "./mock";

export interface Backend {
  loadGraph(path: string): Promise<GraphMeta>;
  listPages(): Promise<PageEntry[]>;
  journalsDesc(limit: number, offset: number): Promise<PageDto[]>;
  getPage(name: string, kind: "journal" | "page"): Promise<PageDto | null>;
  savePage(page: PageDto): Promise<void>;
  getBacklinks(name: string): Promise<RefGroup[]>;
  runQuery(query: string): Promise<RefGroup[]>;
  search(query: string, limit: number): Promise<RefGroup[]>;
  quickSwitch(query: string, limit: number): Promise<PageEntry[]>;
  resolveBlock(uuid: string): Promise<RefGroup | null>;
  readAsset(name: string): Promise<Uint8Array>;
  saveAsset(name: string, bytes: Uint8Array): Promise<string>;
  /** If the OS clipboard holds an image, save it to assets/ and return the
   *  filename; otherwise null. */
  pasteImage(): Promise<string | null>;
  writeText(text: string): Promise<void>;
  readHighlights(pdf: string): Promise<Highlight[]>;
  writeHighlights(pdf: string, label: string, highlights: Highlight[]): Promise<void>;
}

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

class TauriBackend implements Backend {
  private invoke!: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
  private ready: Promise<void>;

  constructor() {
    this.ready = import("@tauri-apps/api/core").then((m) => {
      this.invoke = m.invoke;
    });
  }

  private async call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    await this.ready;
    return this.invoke<T>(cmd, args);
  }

  loadGraph(path: string) {
    return this.call<GraphMeta>("load_graph", { path });
  }
  listPages() {
    return this.call<PageEntry[]>("list_pages");
  }
  journalsDesc(limit: number, offset: number) {
    return this.call<PageDto[]>("journals_desc", { limit, offset });
  }
  getPage(name: string, kind: "journal" | "page") {
    return this.call<PageDto | null>("get_page", { name, kind });
  }
  savePage(page: PageDto) {
    return this.call<void>("save_page", { page });
  }
  getBacklinks(name: string) {
    return this.call<RefGroup[]>("get_backlinks", { name });
  }
  runQuery(query: string) {
    return this.call<RefGroup[]>("run_query", { query });
  }
  search(query: string, limit: number) {
    return this.call<RefGroup[]>("search", { query, limit });
  }
  quickSwitch(query: string, limit: number) {
    return this.call<PageEntry[]>("quick_switch", { query, limit });
  }
  resolveBlock(uuid: string) {
    return this.call<RefGroup | null>("resolve_block", { uuid });
  }
  async readAsset(name: string) {
    const bytes = await this.call<number[]>("read_asset", { name });
    return new Uint8Array(bytes);
  }
  saveAsset(name: string, bytes: Uint8Array) {
    return this.call<string>("save_asset", { name, bytes: Array.from(bytes) });
  }
  async pasteImage(): Promise<string | null> {
    try {
      const { readImage } = await import("@tauri-apps/plugin-clipboard-manager");
      const img = await readImage();
      const rgba = await img.rgba();
      const { width, height } = await img.size();
      if (!width || !height || !rgba.length) return null;
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const ctx = canvas.getContext("2d");
      if (!ctx) return null;
      ctx.putImageData(new ImageData(new Uint8ClampedArray(rgba), width, height), 0, 0);
      const blob: Blob | null = await new Promise((res) => canvas.toBlob(res, "image/png"));
      if (!blob) return null;
      const bytes = new Uint8Array(await blob.arrayBuffer());
      return await this.saveAsset(`image_${Date.now()}.png`, bytes);
    } catch {
      return null; // no image in clipboard, or plugin unavailable
    }
  }
  async writeText(text: string): Promise<void> {
    try {
      const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
      await writeText(text);
    } catch {
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        // ignore
      }
    }
  }
  readHighlights(pdf: string) {
    return this.call<Highlight[]>("read_highlights", { pdf });
  }
  writeHighlights(pdf: string, label: string, highlights: Highlight[]) {
    return this.call<void>("write_highlights", { pdf, label, highlights });
  }
}

let _backend: Backend | null = null;

export function backend(): Backend {
  if (!_backend) {
    _backend = isTauri() ? new TauriBackend() : mockBackend();
  }
  return _backend;
}
