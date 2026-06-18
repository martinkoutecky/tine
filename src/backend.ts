// Backend abstraction. In the Tauri app we call Rust via `invoke`. In a plain
// browser (Vite dev / Playwright screenshots) we fall back to an in-memory mock
// seeded from a fixture graph, so the whole UI is exercisable without the shell.

import type { GraphMeta, Highlight, PageDto, PageEntry, RefGroup, TemplateDto } from "./types";
import { mockBackend } from "./mock";

export interface Backend {
  loadGraph(path: string): Promise<GraphMeta>;
  listPages(): Promise<PageEntry[]>;
  journalsDesc(limit: number, offset: number): Promise<PageDto[]>;
  /** Journal date-keys (yyyymmdd) whose page has real content. */
  journalContentDays(): Promise<number[]>;
  getPage(name: string, kind: "journal" | "page"): Promise<PageDto | null>;
  /** Save a page. `baseRev` is the file hash the editor loaded; the backend
   *  rejects with "conflict" if the file changed on disk since then (unless
   *  `force`). Returns the new on-disk rev to use as the next baseline. */
  savePage(page: PageDto, baseRev: string | null, force?: boolean): Promise<string>;
  getBacklinks(name: string): Promise<RefGroup[]>;
  getUnlinkedRefs(name: string): Promise<RefGroup[]>;
  deletePage(name: string, kind: "journal" | "page"): Promise<void>;
  /** Rename a page and update all [[refs]]/#tags across the graph. */
  renamePage(old: string, next: string): Promise<void>;
  publishHtml(): Promise<[string, number]>;
  runQuery(query: string): Promise<RefGroup[]>;
  /** Property keys (each with their distinct values) for query-builder
   *  autocomplete. */
  queryFacets(): Promise<[string, string[]][]>;
  /** `alias::` → canonical page name pairs. */
  pageAliases(): Promise<[string, string][]>;
  /** Persist favorited page names to config.edn `:favorites`. */
  setFavorites(names: string[]): Promise<void>;
  /** The graph's logseq/custom.css (empty string if none). */
  readCustomCss(): Promise<string>;
  /** Open an http(s)/mailto URL in the OS default app. */
  openExternal(url: string): Promise<void>;
  search(query: string, limit: number): Promise<RefGroup[]>;
  quickSwitch(query: string, limit: number): Promise<PageEntry[]>;
  listTemplates(): Promise<TemplateDto[]>;
  resolveBlock(uuid: string): Promise<RefGroup | null>;
  readAsset(name: string): Promise<Uint8Array>;
  saveAsset(name: string, bytes: Uint8Array): Promise<string>;
  /** If the OS clipboard holds an image, save it to assets/ and return the
   *  filename; otherwise null. */
  pasteImage(): Promise<string | null>;
  /** Copy a file (by absolute path) into assets/, returning the stored name. */
  importAsset(path: string): Promise<string>;
  /** Native folder picker (graph open). Null if cancelled / unsupported. */
  pickFolder(): Promise<string | null>;
  /** Native file picker (asset upload). Null if cancelled / unsupported. */
  pickFile(): Promise<string | null>;
  writeText(text: string): Promise<void>;
  readHighlights(pdf: string): Promise<Highlight[]>;
  writeHighlights(pdf: string, label: string, highlights: Highlight[]): Promise<void>;
  /** Subscribe to external file changes (file watcher). Returns an unsubscribe. */
  onGraphChanged(cb: (c: GraphChange) => void): Promise<() => void>;
  /** How many launch snapshots to keep. */
  getBackupKeep(): Promise<number>;
  setBackupKeep(keep: number): Promise<void>;
  /** Available snapshots for the current graph, newest first. */
  listBackups(): Promise<BackupInfo[]>;
  /** Restore a snapshot (overwrites journals/pages/config; snapshots current
   *  state first). Destructive — confirm before calling. */
  restoreBackup(stamp: string): Promise<void>;
}

export interface BackupInfo {
  /** `YYYY-MM-DD_HH-MM-SS` (UTC). */
  stamp: string;
  files: number;
}

export interface GraphChange {
  name: string;
  kind: "journal" | "page";
  removed: boolean;
}

export function isTauri(): boolean {
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
  journalContentDays() {
    return this.call<number[]>("journal_content_days");
  }
  getPage(name: string, kind: "journal" | "page") {
    return this.call<PageDto | null>("get_page", { name, kind });
  }
  savePage(page: PageDto, baseRev: string | null, force = false) {
    return this.call<string>("save_page", { page, baseRev, force });
  }
  getBacklinks(name: string) {
    return this.call<RefGroup[]>("get_backlinks", { name });
  }
  getUnlinkedRefs(name: string) {
    return this.call<RefGroup[]>("get_unlinked_refs", { name });
  }
  deletePage(name: string, kind: "journal" | "page") {
    return this.call<void>("delete_page", { name, kind });
  }
  renamePage(old: string, next: string) {
    return this.call<void>("rename_page", { old, new: next });
  }
  publishHtml() {
    return this.call<[string, number]>("publish_html");
  }
  runQuery(query: string) {
    return this.call<RefGroup[]>("run_query", { query });
  }
  queryFacets() {
    return this.call<[string, string[]][]>("query_facets");
  }
  pageAliases() {
    return this.call<[string, string][]>("page_aliases");
  }
  setFavorites(names: string[]) {
    return this.call<void>("set_favorites", { names });
  }
  readCustomCss() {
    return this.call<string>("read_custom_css");
  }
  openExternal(url: string) {
    return this.call<void>("open_external", { url });
  }
  search(query: string, limit: number) {
    return this.call<RefGroup[]>("search", { query, limit });
  }
  quickSwitch(query: string, limit: number) {
    return this.call<PageEntry[]>("quick_switch", { query, limit });
  }
  listTemplates() {
    return this.call<TemplateDto[]>("list_templates");
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
  importAsset(path: string) {
    return this.call<string>("import_asset", { path });
  }
  async pickFolder(): Promise<string | null> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const res = await open({ directory: true, multiple: false, title: "Open graph folder" });
    return typeof res === "string" ? res : null;
  }
  async pickFile(): Promise<string | null> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const res = await open({ multiple: false, title: "Choose a file" });
    return typeof res === "string" ? res : null;
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
  async onGraphChanged(cb: (c: GraphChange) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<GraphChange>("graph-changed", (e) => cb(e.payload));
  }
  getBackupKeep() {
    return this.call<number>("get_backup_keep");
  }
  setBackupKeep(keep: number) {
    return this.call<void>("set_backup_keep", { keep });
  }
  listBackups() {
    return this.call<BackupInfo[]>("list_backups");
  }
  restoreBackup(stamp: string) {
    return this.call<void>("restore_backup", { stamp });
  }
}

let _backend: Backend | null = null;

export function backend(): Backend {
  if (!_backend) {
    _backend = isTauri() ? new TauriBackend() : mockBackend();
  }
  return _backend;
}
