// Backend abstraction. In the Tauri app we call Rust via `invoke`. In a plain
// browser (Vite dev / Playwright screenshots) we fall back to an in-memory mock
// seeded from a fixture graph, so the whole UI is exercisable without the shell.

import type {
  AdvancedQueryResult,
  AssetInfo,
  GraphMeta,
  Highlight,
  PageDto,
  PageEntry,
  RefGroup,
  TemplateDto,
  TrashStats,
  JournalConflict,
} from "./types";
import { assetFileName } from "./media";
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
  /** Map of block uuid → number of blocks that reference it (the count badge). */
  getBlockRefCounts(): Promise<Record<string, number>>;
  /** Blocks that reference block `uuid`, grouped by page (the referrers panel). */
  getBlockReferrers(uuid: string): Promise<RefGroup[]>;
  deletePage(name: string, kind: "journal" | "page"): Promise<void>;
  /** Rename a page and update all [[refs]]/#tags across the graph. */
  renamePage(old: string, next: string): Promise<void>;
  publishHtml(): Promise<[string, number]>;
  runQuery(query: string): Promise<RefGroup[]>;
  /** Advanced (datalog-subset) query: maps the supported clauses onto the engine
   *  and reports what ran vs was ignored. `currentPage` resolves `:current-page`. */
  runAdvancedQuery(query: string, currentPage?: string): Promise<AdvancedQueryResult>;
  /** Property keys (each with their distinct values) for query-builder
   *  autocomplete. */
  queryFacets(): Promise<[string, string[]][]>;
  /** `alias::` → canonical page name pairs. */
  pageAliases(): Promise<[string, string][]>;
  /** `icon::` property for each named page that has one (page-name → icon). */
  pageIcons(names: string[]): Promise<Record<string, string>>;
  /** Persist favorited page names to config.edn `:favorites`. */
  setFavorites(names: string[]): Promise<void>;
  /** Persist the task workflow to config.edn `:preferred-workflow`. */
  setPreferredWorkflow(workflow: "now" | "todo"): Promise<void>;
  /** Persist the format new pages/journals are created in to config.edn
   *  `:preferred-format` ("md" | "org"). */
  setPreferredFormat(format: "md" | "org"): Promise<void>;
  /** Persist the journal display-title format to config.edn
   *  `:journal/page-title-format` (e.g. "MMM do, yyyy"). Display-only — does not
   *  rename journal files (`:journal/file-name-format` is separate). */
  setJournalTitleFormat(format: string): Promise<void>;
  /** Set (or clear, with null) the new-journal default template in config.edn
   *  `:default-templates {:journals "Name"}`. */
  setDefaultJournalTemplate(name: string | null): Promise<void>;
  /** Persist the first day of week to config.edn `:start-of-week` (Logseq
   *  convention: 0=Monday … 6=Sunday). */
  setStartOfWeek(n: number): Promise<void>;
  /** The graph's logseq/custom.css (empty string if none). */
  readCustomCss(): Promise<string>;
  /** Open an http(s)/mailto URL in the OS default app. */
  openExternal(url: string): Promise<void>;
  /** Open a graph asset (by its `assets/`-relative name) in the OS default app —
   *  e.g. a video/audio file in the system player. */
  openAsset(name: string): Promise<void>;
  /** Top-level `assets/` files no block references (orphans), for cleanup. */
  listOrphanAssets(): Promise<AssetInfo[]>;
  /** Move an orphaned asset to the recoverable trash. */
  trashAsset(name: string): Promise<void>;
  /** Count + total bytes of the recoverable asset trash (logseq/.tine-trash). */
  assetTrashStats(): Promise<TrashStats>;
  /** Permanently delete everything in the asset trash; returns files removed. */
  emptyAssetTrash(): Promise<number>;
  /** Journal days that resolve to >1 file (date-stem + title-named, or md/org
   *  twin) — for the user to reconcile. */
  listJournalConflicts(): Promise<JournalConflict[]>;
  /** Move one journal file (by exact filename) to the recoverable trash. */
  trashJournalFile(name: string): Promise<void>;
  /** Raw contents of one journal file (by exact filename), for inspecting a
   *  duplicate day's files before reconciling. */
  readJournalFile(name: string): Promise<string>;
  /** Load a page from a SPECIFIC file by its graph-root-relative path — reaches a
   *  duplicate-day stray that shares a (kind,name) with the canonical file (#21). */
  getPageByPath(path: string): Promise<PageDto | null>;
  /** Append the blocks of `src` (graph-root-relative path) onto `dst`, then trash
   *  `src` — fold a duplicate-day stray into the canonical day (#21). */
  mergePages(src: string, dst: string): Promise<void>;
  /** Move a stray file (graph-root-relative path) to a uniquely-named page so it
   *  stops colliding and becomes normally navigable (#21). */
  renameFileToPage(path: string, newName: string): Promise<void>;
  search(query: string, limit: number): Promise<RefGroup[]>;
  quickSwitch(query: string, limit: number): Promise<PageEntry[]>;
  listTemplates(): Promise<TemplateDto[]>;
  resolveBlock(uuid: string): Promise<RefGroup | null>;
  resolveBlocks(uuids: string[]): Promise<(RefGroup | null)[]>;
  readAsset(name: string): Promise<Uint8Array>;
  saveAsset(name: string, bytes: Uint8Array): Promise<string>;
  /** If the OS clipboard holds an image, save it to assets/ and return the
   *  filename; otherwise null. */
  pasteImage(): Promise<string | null>;
  /** Decode an image off the OS clipboard to PNG bytes WITHOUT saving (the
   *  caller seeds the render cache + writes to disk in the background, so the
   *  pasted image appears instantly). Null if the clipboard has no image. */
  readClipboardImage(): Promise<Uint8Array | null>;
  /** Copy a file (by absolute path) into assets/, returning the stored name.
   *  `name` (optional) is the desired stored filename (timestamped). */
  importAsset(path: string, name?: string): Promise<string>;
  /** Native yes/no confirmation dialog. Returns true if the user confirms.
   *  Uses the GTK dialog plugin, NOT window.confirm — the latter silently
   *  returns true without showing anything in this WebKitGTK build, which would
   *  bypass destructive-action and close-tab prompts. */
  confirm(message: string, title?: string): Promise<boolean>;
  /** Native folder picker (graph open). Null if cancelled / unsupported. */
  pickFolder(): Promise<string | null>;
  /** Native file picker (asset upload). Null if cancelled / unsupported. */
  pickFile(): Promise<string | null>;
  writeText(text: string): Promise<void>;
  /** Copy with text/plain (markdown) + text/html flavors; degrades to text/plain. */
  writeRich(text: string, html: string): Promise<void>;
  /** Write a PNG image (bytes) to the OS clipboard. Goes through the Rust
   *  clipboard plugin, not WebKitGTK's native "Copy Image" (which doesn't
   *  actually populate the clipboard, so paste yielded nothing). */
  copyImageToClipboard(bytes: Uint8Array): Promise<void>;
  readHighlights(pdf: string): Promise<Highlight[]>;
  writeHighlights(pdf: string, label: string, highlights: Highlight[], baseIds: string[]): Promise<void>;
  /** Save a cropped area-highlight PNG to OG's layout `assets/<key>/<page>_<id>_<stamp>.png`
   *  (non-dedup — the filename links the `.edn` `:image <stamp>` to the file).
   *  Returns the assets-relative path. */
  savePdfAreaImage(pdf: string, page: number, id: string, stamp: number, bytes: Uint8Array): Promise<string>;
  /** Subscribe to external file changes (file watcher). Returns an unsubscribe. */
  onGraphChanged(cb: (c: GraphChange) => void): Promise<() => void>;
  /** How many launch snapshots to keep. */
  getBackupKeep(): Promise<number>;
  setBackupKeep(keep: number): Promise<void>;
  /** Quick-capture Enter behaviour: true → Enter files; false → Enter = new block. */
  getCaptureEnterFiles(): Promise<boolean>;
  setCaptureEnterFiles(value: boolean): Promise<void>;
  /** `[[`/`#` autocomplete default: true → Enter links the first match; false
   *  (default, OG) → Enter creates a new page/tag unless an exact match exists. */
  getLinkFirstMatch(): Promise<boolean>;
  setLinkFirstMatch(value: boolean): Promise<void>;
  /** How the file-watcher detects external edits: "inotify" (default, no idle
   *  wakeups) or "poll" (3s scan, for filesystems where inotify is flaky). */
  getWatchMode(): Promise<string>;
  setWatchMode(mode: string): Promise<void>;
  /** Available snapshots for the current graph, newest first. */
  listBackups(): Promise<BackupInfo[]>;
  /** Restore a snapshot (overwrites journals/pages/config; snapshots current
   *  state first). Destructive — confirm before calling. */
  restoreBackup(stamp: string): Promise<void>;
  /** Load the persisted UI session JSON (open tabs / active tab / zoom), or null.
   *  Stored in a real file by the backend — WebKitGTK localStorage isn't durably
   *  persisted for this app. */
  loadSession(): Promise<string | null>;
  /** Persist the UI session JSON. */
  saveSession(data: string): Promise<void>;
  /** What the backend knows about the rendering path, for the CPU-rendering
   *  warning (see `gpu.ts`). A silent driver fallback is detected in the webview
   *  (WebGL renderer); this just supplies why/where context for the message. */
  gpuEnv(): Promise<GpuEnv>;
  /** Experimental smooth-scrolling preference (Lenis), app-level, default off. */
  getSmoothScroll(): Promise<boolean>;
  setSmoothScroll(value: boolean): Promise<void>;
  /** Generic device-local boolean preference (tine-settings.json); caller supplies
   *  the key + default. Used by the copy-behavior options. */
  getAppBool(key: string, fallback: boolean): Promise<boolean>;
  setAppBool(key: string, value: boolean): Promise<void>;
  /** Startup debug logging (TINE_DEBUG=1 / --debug): whether it's on and where the
   *  log file is, so the UI can forward errors + show the path. */
  debugInfo(): Promise<DebugInfo>;
  /** Forward a frontend milestone / error into the backend debug log. */
  debugLog(line: string): Promise<void>;
}

export interface DebugInfo {
  enabled: boolean;
  path: string;
}

/** Backend-visible rendering-environment facts (Linux-relevant; all false on
 *  macOS/Windows where the env vars don't exist). */
export interface GpuEnv {
  /** GPU compositing is off because an env var disabled it (TINE_GPU=0 or
   *  WEBKIT_DISABLE_DMABUF_RENDERER / WEBKIT_DISABLE_COMPOSITING_MODE). */
  software_forced: boolean;
  /** Running from an AppImage (`$APPIMAGE` set) — its bundled GL stack is the
   *  usual culprit for a silent CPU fallback; steer the user to the deb/rpm. */
  appimage: boolean;
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
  getBlockRefCounts() {
    return this.call<Record<string, number>>("block_ref_counts", {});
  }
  getBlockReferrers(uuid: string) {
    return this.call<RefGroup[]>("block_referrers", { uuid });
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
  runAdvancedQuery(query: string, currentPage?: string) {
    return this.call<AdvancedQueryResult>("run_advanced_query", { query, currentPage });
  }
  queryFacets() {
    return this.call<[string, string[]][]>("query_facets");
  }
  pageAliases() {
    return this.call<[string, string][]>("page_aliases");
  }
  pageIcons(names: string[]) {
    return this.call<Record<string, string>>("page_icons", { names });
  }
  setFavorites(names: string[]) {
    return this.call<void>("set_favorites", { names });
  }
  setPreferredWorkflow(workflow: "now" | "todo") {
    return this.call<void>("set_preferred_workflow", { workflow });
  }
  setPreferredFormat(format: "md" | "org") {
    return this.call<void>("set_preferred_format", { format });
  }
  setJournalTitleFormat(format: string) {
    return this.call<void>("set_journal_title_format", { format });
  }
  setDefaultJournalTemplate(name: string | null) {
    return this.call<void>("set_default_journal_template", { name });
  }
  setStartOfWeek(n: number) {
    return this.call<void>("set_start_of_week", { n });
  }
  readCustomCss() {
    return this.call<string>("read_custom_css");
  }
  openExternal(url: string) {
    return this.call<void>("open_external", { url });
  }
  openAsset(name: string) {
    return this.call<void>("open_asset", { name });
  }
  listOrphanAssets() {
    return this.call<AssetInfo[]>("list_orphan_assets");
  }
  trashAsset(name: string) {
    return this.call<void>("trash_asset", { name });
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
  resolveBlocks(uuids: string[]) {
    return this.call<(RefGroup | null)[]>("resolve_blocks", { uuids });
  }
  async readAsset(name: string) {
    // read_asset now returns raw bytes (tauri::ipc::Response) → an ArrayBuffer,
    // not a JSON number[] — far cheaper for large PDFs/images.
    const buf = await this.call<ArrayBuffer>("read_asset", { name });
    return new Uint8Array(buf);
  }
  saveAsset(name: string, bytes: Uint8Array) {
    return this.call<string>("save_asset", { name, bytes: Array.from(bytes) });
  }
  async readClipboardImage(): Promise<Uint8Array | null> {
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
      return new Uint8Array(await blob.arrayBuffer());
    } catch {
      return null; // no image in clipboard, or plugin unavailable
    }
  }
  async pasteImage(): Promise<string | null> {
    const bytes = await this.readClipboardImage();
    if (!bytes) return null;
    return await this.saveAsset(assetFileName(), bytes);
  }
  assetTrashStats() {
    return this.call<TrashStats>("asset_trash_stats");
  }
  emptyAssetTrash() {
    return this.call<number>("empty_asset_trash");
  }
  listJournalConflicts() {
    return this.call<JournalConflict[]>("list_journal_conflicts");
  }
  trashJournalFile(name: string) {
    return this.call<void>("trash_journal_file", { name });
  }
  readJournalFile(name: string) {
    return this.call<string>("read_journal_file", { name });
  }
  getPageByPath(path: string) {
    return this.call<PageDto | null>("get_page_by_path", { path });
  }
  mergePages(src: string, dst: string) {
    return this.call<void>("merge_pages", { src, dst });
  }
  renameFileToPage(path: string, newName: string) {
    return this.call<void>("rename_file_to_page", { path, newName });
  }
  importAsset(path: string, name?: string) {
    return this.call<string>("import_asset", { path, name });
  }
  async confirm(message: string, title?: string): Promise<boolean> {
    const { ask } = await import("@tauri-apps/plugin-dialog");
    return ask(message, { title: title ?? "Tine", kind: "warning" });
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
  async writeRich(text: string, html: string): Promise<void> {
    // Multi-flavor (text/plain + text/html) ONLY where the async Clipboard API +
    // ClipboardItem exist in a secure context — so a WebKitGTK build lacking them
    // safely uses the reliable text/plain path below (never a regression). Any
    // failure also falls through to text/plain.
    try {
      if (
        typeof window !== "undefined" &&
        window.isSecureContext &&
        typeof ClipboardItem !== "undefined" &&
        navigator.clipboard?.write
      ) {
        await navigator.clipboard.write([
          new ClipboardItem({
            "text/plain": new Blob([text], { type: "text/plain" }),
            "text/html": new Blob([html], { type: "text/html" }),
          }),
        ]);
        return;
      }
    } catch {
      // fall through to the reliable text/plain path
    }
    await this.writeText(text);
  }
  copyImageToClipboard(bytes: Uint8Array): Promise<void> {
    return this.call<void>("copy_image_to_clipboard", { bytes: Array.from(bytes) });
  }
  readHighlights(pdf: string) {
    return this.call<Highlight[]>("read_highlights", { pdf });
  }
  writeHighlights(pdf: string, label: string, highlights: Highlight[], baseIds: string[]) {
    return this.call<void>("write_highlights", { pdf, label, highlights, baseIds });
  }
  savePdfAreaImage(pdf: string, page: number, id: string, stamp: number, bytes: Uint8Array) {
    return this.call<string>("save_pdf_area_image", {
      pdf,
      page,
      id,
      stamp,
      bytes: Array.from(bytes),
    });
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
  getCaptureEnterFiles() {
    return this.call<boolean>("get_capture_enter_files");
  }
  setCaptureEnterFiles(value: boolean) {
    return this.call<void>("set_capture_enter_files", { value });
  }
  getLinkFirstMatch() {
    return this.call<boolean>("get_link_first_match");
  }
  setLinkFirstMatch(value: boolean) {
    return this.call<void>("set_link_first_match", { value });
  }
  getWatchMode() {
    return this.call<string>("get_watch_mode");
  }
  setWatchMode(mode: string) {
    return this.call<void>("set_watch_mode", { mode });
  }
  listBackups() {
    return this.call<BackupInfo[]>("list_backups");
  }
  restoreBackup(stamp: string) {
    return this.call<void>("restore_backup", { stamp });
  }
  loadSession() {
    return this.call<string | null>("load_session");
  }
  saveSession(data: string) {
    return this.call<void>("save_session", { data });
  }
  gpuEnv() {
    return this.call<GpuEnv>("gpu_env");
  }
  debugInfo() {
    return this.call<DebugInfo>("debug_info");
  }
  debugLog(line: string) {
    return this.call<void>("debug_log", { line });
  }
  getSmoothScroll() {
    return this.call<boolean>("get_smooth_scroll");
  }
  getAppBool(key: string, fallback: boolean) {
    return this.call<boolean>("get_app_bool", { key, default: fallback });
  }
  setAppBool(key: string, value: boolean) {
    return this.call<void>("set_app_bool", { key, value });
  }
  setSmoothScroll(value: boolean) {
    return this.call<void>("set_smooth_scroll", { value });
  }
}

let _backend: Backend | null = null;

export function backend(): Backend {
  if (!_backend) {
    _backend = isTauri() ? new TauriBackend() : mockBackend();
  }
  return _backend;
}
