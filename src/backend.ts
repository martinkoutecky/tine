import { t } from "./i18n";
// Backend abstraction. In the Tauri app we call Rust via `invoke`. In a plain
// browser (Vite dev / Playwright screenshots) we fall back to an in-memory mock
// seeded from a fixture graph, so the whole UI is exercisable without the shell.

import { markCommandSlow } from "./slowBackend";
import { orderedLane } from "./orderedWrites";
import { REFRESH_READ_COMMANDS, trackRefreshRead } from "./refreshReads";
import { timingNamesForCommand } from "./focusTiming";
import type { GraphVerificationProgress, GraphVerificationReport } from "./graphVerification";
import type {
  ExecutionContext,
  ExplainEmptyResult,
  ParsedQuery,
  Query,
  QueryPrintDialect,
  QueryResult,
  QueryTextDialect,
  RegistrySnapshot,
  ViewSettings,
} from "./editor/queryIr";
import type {
  BacklinkFilterContext,
  BacklinkFilterTarget,
  AssetInfo,
  GuideCopyResult,
  GuidePage,
  Highlight,
  PageDto,
  PageKind,
  PageEntry,
  RefGroup,
  BlockPreview,
  TemplateDto,
  TrashStats,
  JournalConflict,
  SyncConflict,
  SyncConflictDiff,
  MergeDecision,
  ConflictInventory,
  MarkerConflictDiff,
  PrintOpts,
  PdfState,
  QueryExecution,
  QueryPageScope,
  QueryExportBatch,
  QueryExportSpec,
  QueryPublicationRequest,
  QueryPublicationPlan,
  PublicationReceipt,
} from "./types";
import type { GraphFolderPickResult, PreparedGraphFolder, ClipboardFileList, MediaCaptureResult, KnownGraph, InstalledPluginRecord, PluginRegistryCacheLoad, LoadGraphResult, CaptureGraphBindingResult, GraphAccessInspection, TrayStatus } from "./backendTypes";
import { dbg } from "./debug";
import type { EditKinds } from "./editKind";
import { mockBackend } from "./mock";
import type { SheetExport, SheetInput, SheetScope } from "./sheet/staticExport";
import { isPublishedExport, publishedBackend } from "./publishedBackend";

import { nativeTineLinks } from "./nativeTineLinks";
import { ASSET_INGRESS_MAX_BYTES, clipboardImageToPng } from "./clipboardImage";

// Encode asset bytes as one base64 string for save_* and Android image-copy
// IPC (Android cannot receive raw InvokeBody). The old
// `Array.from(bytes)` produced a JSON number[] — ~4-5x the payload + a multi-MB
// per-element parse and a giant throwaway array on the webview thread for every
// image paste / PDF crop. base64 is ~1.33x the byte size, a single string the
// backend decodes in one pass. Chunked so `String.fromCharCode(...)` never blows
// the argument-count limit on a multi-MB buffer.
function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const CHUNK = 0x8000; // 32K args/call — safely under the spread/apply limit
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

export type { Backend } from "./backendInterface";
import type { Backend } from "./backendInterface";


export type { DebugInfo, DiagnosticReport, DiagnosticFrontendKind, DiscardReason, DiagnosticFrontendFields, GpuEnv, BackupInfo, GraphChange, AssetChangedBatch, GraphConfigChange, CustomCssChange, TrashAssetOutcome, GraphAnswersChange, GraphFolderPickResult, PreparedGraphFolder, ClipboardAssetFile, ClipboardFileList, MediaCaptureResult, KnownGraph, InstalledPluginRecord, PluginRegistryCacheEnvelope, PluginRegistryCacheLoad, LoadGraphResult, CaptureGraphBindingResult, GraphAccessInspection } from "./backendTypes";
import type { DebugInfo, DiagnosticReport, DiagnosticFrontendKind, DiagnosticFrontendFields, GpuEnv, BackupInfo, GraphChange, AssetChangedBatch, GraphConfigChange, CustomCssChange, TrashAssetOutcome, GraphAnswersChange } from "./backendTypes";
import type { DiskToken, OwedPage, PageMail, PageOperation, PageRefusal, PublishedNeed } from "./document";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export { QueryPrintRefusedError, queryPrintRefusal } from "./queryPrintRefusal";
import { queryPrintRefusal } from "./queryPrintRefusal";

/** Commands whose timing would only describe the diagnostics channel. */
const DIAGNOSTIC_COMMANDS = new Set([
  "debug_info", "debug_log", "diagnostic_ipc_event", "diagnostic_frontend_event", "diagnostic_report", "clear_diagnostics",
  "save_diagnostic_report", "diagnostic_session_active", "diagnostic_timing_event",
]);
/** GH #623: the page-load commands whose every call is timed (and, apart, the calls
 *  made soon after a focus return). A closed list, so no argument or free text can
 *  become a timing name (I-5); the Rust side registers the same names. */
const TIMED_COMMANDS: ReadonlySet<string> = new Set([
  "get_page", "get_page_by_path", "get_backlinks", "journal_feed_page", "page_inventory", "search",
]);
/** A command still running after this long is recorded as `slow`. */
const SLOW_IPC_MS = 500;

class TauriBackend implements Backend {
  private invoke!: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
  private convertFileSrc!: (path: string, protocol?: string) => string;
  private ready: Promise<void>;
  private bindingGeneration = 0;
  private ipcDiagnosticsUnavailable = false;
  private readonly ordered = orderedLane();
  /** Issue order of lane searches. The native handlers reach their lane in
   *  scheduler order, so a keystroke's older search can land after the newer
   *  and cancel it; the lane keeps the highest order and refuses an older one.
   *  Seeded from the clock so a reloaded WebView never restarts below a
   *  request the native side still holds. */
  private laneSeq = Date.now() * 1000;

  constructor() {
    this.ready = import("@tauri-apps/api/core").then((m) => {
      this.invoke = m.invoke;
      this.convertFileSrc = m.convertFileSrc;
    });
  }

  graphBindingGeneration() { return this.bindingGeneration; }

  /** R3: write commands keep their call order (src/orderedWrites.ts). A
   *  refresh read is counted until it settles (src/refreshReads.ts). */
  private call<T>(cmd: string, args?: Record<string, unknown>, bindingGeneration = this.bindingGeneration): Promise<T> {
    const issued = this.ordered(cmd, () => this.issue<T>(cmd, args, bindingGeneration));
    return REFRESH_READ_COMMANDS.has(cmd) ? trackRefreshRead(issued) : issued;
  }

  private async issue<T>(cmd: string, args: Record<string, unknown> | undefined, bindingGeneration: number): Promise<T> {
    await this.ready;
    const leasedArgs = bindingGeneration
      ? { ...(args ?? {}), bindingGeneration }
      : args;
    if (DIAGNOSTIC_COMMANDS.has(cmd)) return this.invoke<T>(cmd, leasedArgs);
    const started = performance.now();
    let slow = false;
    let settleSlow: (() => void) | undefined;
    const slowTimer = setTimeout(() => {
      slow = true;
      settleSlow = markCommandSlow(started);
      this.reportIpcPhase(cmd, "slow", started);
    }, SLOW_IPC_MS);
    try {
      const result = await this.invoke<T>(cmd, leasedArgs);
      if (slow) this.reportIpcPhase(cmd, "completed", started);
      this.reportCommandTiming(cmd, started);
      return result;
    } catch (error) {
      this.reportIpcPhase(cmd, "failed", started);
      dbg(`command ${cmd} failed: ${String(error)}`); // opt-in --debug log only (GH #594)
      throw error;
    } finally {
      clearTimeout(slowTimer);
      settleSlow?.();
    }
  }

  /** GH #343: tell the flight recorder a command was slow, completed after
   *  being slow, or failed — its registered name and duration only. A failed
   *  report stops further reports for this run (the recorder is unavailable). */
  private reportIpcPhase(command: string, phase: "slow" | "completed" | "failed", started: number) {
    if (this.ipcDiagnosticsUnavailable) return;
    const elapsedMs = Math.max(0, Math.round(performance.now() - started));
    void this.invoke<void>("diagnostic_ipc_event", { command, phase, elapsedMs }).catch(() => {
      this.ipcDiagnosticsUnavailable = true;
    });
  }

  /** GH #623: count every call of a page-load command (not only slow ones),
   *  and apart those made soon after a focus return. Numbers only. */
  private reportCommandTiming(cmd: string, started: number) {
    if (!TIMED_COMMANDS.has(cmd)) return;
    const names = timingNamesForCommand(cmd);
    for (const name of names) this.diagnosticTimingEvent(name, performance.now() - started);
  }

  diagnosticTimingEvent(name: string, elapsedMs: number): Promise<void> {
    if (this.ipcDiagnosticsUnavailable) return Promise.resolve();
    return this.invoke<void>("diagnostic_timing_event", { name, elapsedMs: Math.max(0, Math.round(elapsedMs)) }).catch(() => {
      this.ipcDiagnosticsUnavailable = true;
    });
  }

  private assetCall<T>(cmd: string, args: Record<string, unknown> | undefined, bindingGeneration: number): Promise<T> {
    if (!Number.isSafeInteger(bindingGeneration) || bindingGeneration <= 0)
      return Promise.reject(new Error("missing-graph-binding"));
    return this.call<T>(cmd, args, bindingGeneration);
  }

  async loadGraph(path: string) {
    const result = await this.call<LoadGraphResult>("load_graph", { path });
    if (result.kind !== "focused_existing") this.bindingGeneration = result.binding_generation;
    return result;
  }
  inspectGraphAccess(path: string) {
    return this.call<GraphAccessInspection>("inspect_graph_access", { path });
  }
  approveExternalAssets(graphRoot: string, assetsPath: string) {
    return this.call<void>("approve_external_assets", { graphRoot, assetsPath });
  }
  openGraphWindow(path: string) {
    return this.call<LoadGraphResult>("open_graph_window", { path });
  }
  readonly tineLinks = nativeTineLinks((cmd, args) => this.call(cmd, args), (cb) => this.on<void>("tine-link-pending", cb));
  startupGraphPath() {
    return this.call<string | null>("startup_graph_path");
  }
  captureTarget() {
    return this.call<string>("capture_target");
  }
  async bindCaptureGraph() {
    const result = await this.call<CaptureGraphBindingResult>("capture_graph_binding");
    this.bindingGeneration = result.binding_generation;
  }
  listKnownGraphs() {
    return this.call<KnownGraph[]>("list_known_graphs");
  }
  forgetKnownGraph(path: string) {
    return this.call<void>("forget_known_graph", { path });
  }
  revealKnownGraph(path: string) {
    return this.call<void>("reveal_known_graph", { path });
  }
  appPlatform() {
    return this.call<"android" | "ios" | "desktop">("app_platform");
  }
  listInstalledPlugins() {
    return this.call<InstalledPluginRecord[]>("list_installed_plugins");
  }
  installPlugin(manifestJson: string, wasm: Uint8Array) {
    return this.call<InstalledPluginRecord>("install_plugin", {
      manifestJson,
      wasmB64: bytesToBase64(wasm),
    });
  }
  uninstallPlugin(id: string, version: string) {
    return this.call<void>("uninstall_plugin", { id, version });
  }
  async readPluginEntry(id: string, version: string) {
    const buffer = await this.call<ArrayBuffer>("read_plugin_entry", { id, version });
    return new Uint8Array(buffer);
  }
  setPluginEnabled(id: string, version: string, enabled: boolean) {
    return this.call<void>("set_plugin_enabled", { id, version, enabled });
  }
  verifyPluginRegistry(indexJson: string, signatureB64: string) {
    return this.call<void>("verify_plugin_registry", { indexJson, signatureB64 });
  }
  loadPluginRegistryCache() {
    return this.call<PluginRegistryCacheLoad>("load_plugin_registry_cache");
  }
  storePluginRegistryCache(
    indexJson: string,
    signature: string
  ) {
    return this.call<void>("store_plugin_registry_cache", {
      indexJson,
      signature,
    });
  }
  setSystemBarAppearance(dark: boolean, background?: string) {
    return this.call<void>("set_system_bar_appearance", { dark, background });
  }
  closeGraphWindow() {
    return this.call<void>("close_graph_window");
  }
  trayApply() {
    return this.call<TrayStatus>("tray_apply");
  }
  openDevtools() {
    return this.call<void>("tine_open_devtools");
  }
  defaultGraphParent() {
    return this.call<string>("default_graph_parent");
  }
  createGraph(dir: string, name: string) {
    return this.call<{ root: string; display_root: string }>("create_graph", { dir, name });
  }
  suggestGraphName(dir: string) { return this.call<string>("suggest_graph_name", { dir }); }
  pageInventory() {
    return this.call<import("./types").PageInventory>("page_inventory");
  }
  pageModified() {
    return this.call<import("./types").PageModified>("page_modified");
  }
  pageBacklinkCounts() {
    return this.call<[string, number][]>("page_backlink_counts");
  }
  journalFeedPage(limit: number, beforeDay: number | null) {
    return this.call<import("./types").JournalFeedPage>("journal_feed_page", { limit, beforeDay });
  }
  journalContentDays() {
    return this.call<number[]>("journal_content_days");
  }
  getPage(name: string, kind: "journal" | "page") {
    return this.call<import("./types").PageRead | null>("get_page", { name, kind });
  }
  resolvePage(name: string, kind: "journal" | "page") {
    return this.call<import("./types").ResolvedPage>("resolve_page", { name, kind });
  }
  pageWindowReloaded() {
    return this.call<{ session: number; nextId: number }>("page_window_reloaded");
  }
  pageOpen(session: number, id: number, page: { name: string; kind: "journal" | "page"; path: string | null }) {
    const generation = this.bindingGeneration;
    type Reply = { key: string; baselineEntry: boolean } | PageRefusal;
    // PERF-FIXES D3: opens issued back to back (a carry-over's sources) share
    // one `page_open` call, still in id order and in their lane place.
    return this.ordered.joined(`page_open:${session}:${generation}`, { id, page }, async (items): Promise<Reply[]> => {
      const paths = await Promise.all(items.map(async ({ page }): Promise<string | PageRefusal> => {
        if (page.path !== null) return page.path;
        const resolved = await this.issue<import("./types").ResolvedPage>("resolve_page", { name: page.name, kind: page.kind }, generation);
        return resolved.kind === "alias" ? { reason: "alias", owners: resolved.owners } : resolved.id;
      }));
      const sent = items.flatMap(({ id, page }, i) => {
        const path = paths[i];
        return typeof path === "string" ? [{ id, path, name: page.name }] : [];
      });
      const answers = sent.length ? await this.issue<Reply[]>("page_open", { session, pages: sent }, generation) : [];
      let next = 0;
      return paths.map((path) => typeof path === "string" ? answers[next++] : path);
    });
  }
  pageSubmit(session: number, id: number, key: string, dto: PageDto, version: number, resolve: DiskToken | null, kinds: EditKinds) {
    return this.call<PageRefusal | null>("page_submit", { session, id, key, dto, version, resolve, kinds });
  }
  pageMove(session: number, id: number, source: [string, PageDto, number], receiver: [string, PageDto, number], kinds: EditKinds) {
    return this.call<PageRefusal | null>("page_move", { session, id, source, receiver, kinds });
  }
  pageDiscard(session: number, id: number, key: string) {
    return this.call<PageRefusal | null>("page_discard", { session, id, key });
  }
  pageClose(session: number, id: number, key: string) {
    return this.call<PageRefusal | null>("page_close", { session, id, key });
  }
  pageDelete(session: number, name: string, kind: PageKind, expectedPath?: string) {
    return this.call<PageOperation>("page_delete", { session, name, kind, expectedPath });
  }
  pageWait(session: number, needs: readonly PublishedNeed[], boundMs: number) {
    return this.call<boolean | null>("page_wait", { session, needs, boundMs });
  }
  pageSaveNow(session: number, keys: readonly string[]) {
    return this.call<void>("page_save_now", { session, keys });
  }
  pageOwed(session: number, paths: readonly string[] | null) {
    return this.call<OwedPage[] | null>("page_owed", { session, paths });
  }
  pageDraftsRetry() {
    return this.call<void>("page_drafts_retry");
  }
  onPageMail(cb: (mail: PageMail) => void) { return this.on("page-mail", cb); }
  onGraphOpenWaiting(cb: (saving: string) => void) { return this.on("graph-open-waiting", cb); }
  onGraphOpenOwnerWaiting(cb: (progress: string) => void) { return this.on("graph-open-owner-waiting", cb); }
  legacyDraftsFile() {
    return this.call<string | null>("legacy_drafts_file");
  }
  guidePages() {
    return this.call<GuidePage[]>("guide_pages");
  }
  copyGuideIntoGraph(title: string) {
    return this.call<GuideCopyResult>("copy_guide_into_graph", { title });
  }
  setGuideAnnounced(announced: boolean) {
    return this.call<void>("set_guide_announced", { announced });
  }
  getBacklinks(name: string) {
    return this.call<RefGroup[]>("get_backlinks", { name });
  }
  getBacklinkFilterContext(name: string, targets: BacklinkFilterTarget[]) {
    return this.call<BacklinkFilterContext>("get_backlink_filter_context", { name, targets });
  }
  getUnlinkedRefs(name: string) {
    return this.call<RefGroup[]>("get_unlinked_refs", { name });
  }
  warmDone() {
    return this.call<boolean>("warm_done");
  }
  getBlockRefCounts() {
    return this.call<Record<string, number>>("block_ref_counts", {});
  }
  getBlockReferrers(uuid: string) {
    return this.call<RefGroup[]>("block_referrers", { uuid });
  }
  renamePage(old: string, next: string, _kind: "rename-page", expectedPath?: string, mergeInto?: string, unsavedPaths?: string[]) {
    return this.call<import("./types").RenameDone>("rename_page", { old, new: next, expectedPath, mergeInto, unsavedPaths });
  }
  publishHtml() {
    return this.call<[string, number]>("publish_html");
  }
  publishQueryPlan(request: QueryPublicationRequest) {
    return this.call<QueryPublicationPlan>("publish_query_plan", { request });
  }
  publishQuery(request: QueryPublicationRequest, fingerprint: string, sheets: SheetExport[]) {
    return this.call<PublicationReceipt>("publish_query", { request, fingerprint, sheets });
  }
  publishLive(destination: string, name: string, allPages: boolean, sheets: SheetExport[]) {
    return this.call<PublicationReceipt>("publish_live", { destination, name, allPages, sheets });
  }
  sheetExportInputs(pages?: string[], scope?: SheetScope) {
    return this.call<SheetInput[]>("sheet_export_inputs", { pages: pages ?? null, scope: scope ?? null });
  }
  pagePrintHtml(name: string, opts: PrintOpts, sheets: SheetExport[]) {
    return this.call<string>("page_print_html", { name, opts, sheets });
  }
  exportQuerySubtrees(specs: QueryExportSpec[]) {
    return this.call<QueryExportBatch>("export_query_subtrees", { specs });
  }
  parseQuery(text: string, dialect: QueryTextDialect, blockProperties?: [string, string][]) {
    return this.call<ParsedQuery>("query_parse", { text, dialect, blockProperties });
  }
  printQuery(query: Query, view: ViewSettings, dialect: QueryPrintDialect, preserveForm = false) {
    return this.call<string>("query_print", { query, view, dialect, preserveForm }).catch((error: unknown) => {
      throw queryPrintRefusal(error) ?? error;
    });
  }
  queryOgExpressible(query: Query, view: ViewSettings) {
    return this.call<boolean>("query_og_expressible", { query, view });
  }
  queryRegistry() {
    return this.call<RegistrySnapshot>("query_registry");
  }
  queryRun(query: Query, view: ViewSettings, context?: ExecutionContext) {
    return this.call<QueryResult>("query_run", { query, view, context });
  }
  queryExplainEmpty(query: Query, view: ViewSettings, context?: ExecutionContext) {
    return this.call<ExplainEmptyResult>("query_explain_empty", { query, view, context });
  }
  queryFacets(autocomplete = false) {
    return this.call<[string, string[]][]>(
      "query_facets",
      autocomplete ? { autocomplete: true } : undefined,
    );
  }
  pageIcons(names: string[]) {
    return this.call<Record<string, string>>("page_icons", { names });
  }
  setFavorites(names: string[], page: string | null = null) {
    return this.call<void>("set_favorites", { names, page });
  }
  setDefaultHome(name: string | null) {
    return this.call<void>("set_default_home", { name });
  }
  setPreferredWorkflow(workflow: "now" | "todo") {
    return this.call<void>("set_preferred_workflow", { workflow });
  }
  setTimetrackingEnabled(enabled: boolean) {
    return this.call<void>("set_timetracking_enabled", { enabled });
  }
  setShowBrackets(enabled: boolean) {
    return this.call<void>("set_show_brackets", { enabled });
  }
  setDocModeEnterForNewBlock(enabled: boolean) {
    return this.call<void>("set_doc_mode_enter_for_new_block", { enabled });
  }
  setLogicalOutdenting(enabled: boolean) {
    return this.call<void>("set_logical_outdenting", { enabled });
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
  openExternal(url: string, confirmedScheme?: string) {
    return this.call<void>("open_external", { url, confirmedScheme: confirmedScheme ?? null });
  }
  openAsset(name: string, bindingGeneration: number) {
    return this.assetCall<void>("open_asset", { name }, bindingGeneration);
  }
  openGraphFile(path: string, bindingGeneration: number) {
    return this.assetCall<void>("open_graph_file", { path }, bindingGeneration);
  }
  openPageFile(name: string, kind: "page" | "journal", path: string | undefined, reveal: boolean) {
    return this.call<void>("open_page_file", { name, kind, path: path || null, reveal });
  }
  editAssetExternal(name: string, command: string, bindingGeneration: number) {
    return this.assetCall<void>("edit_asset_external", { name, command }, bindingGeneration);
  }
  detectMediaEditor(id: string) {
    return this.call<string>("detect_media_editor", { id });
  }
  listOrphanAssets() {
    return this.call<AssetInfo[]>("list_orphan_assets");
  }
  trashAsset(name: string, bindingGeneration: number) {
    return this.assetCall<TrashAssetOutcome>("trash_asset", { name }, bindingGeneration);
  }
  search(query: string, limit: number, lane?: string) {
    return this.call<RefGroup[]>("search", { query, limit, lane, laneSeq: ++this.laneSeq });
  }
  async runGraphSearch(source: string, pageLimit: number, blockLimit: number, lane = "graph-search", explain = false, scope?: QueryPageScope, pageMatchScope?: import("./editor/queryIr").FriendlyPageMatchScope, views?: { page: ViewSettings; block: ViewSettings }, transliteration = false) {
    const execution = await this.call<QueryExecution>("run_graph_search", { source, pageLimit, blockLimit, lane, laneSeq: ++this.laneSeq, explain, scope: scope ?? null, pageMatchScope: pageMatchScope ?? null, pageView: views?.page ?? null, blockView: views?.block ?? null, transliteration });
    return {
      ...execution,
      has_more: execution.has_more ?? { pages: false, blocks: false },
    };
  }
  quickSwitch(query: string, limit: number, editor?: { pageName: string; blockId: string; originalRaw: string; session?: string }, transliteration = false) {
    return this.call<PageEntry[]>("quick_switch", { query, limit, editorPageName: editor?.pageName, editorBlockId: editor?.blockId, editorOriginalRaw: editor?.originalRaw, editorSession: editor?.session, transliteration });
  }
  captureQuickSwitch(query: string, limit: number, transliteration = false) {
    return this.call<PageEntry[]>("capture_quick_switch", { query, limit, transliteration });
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
  previewBlock(uuid: string, maxNodes: number) {
    return this.call<BlockPreview | null>("preview_block", { uuid, maxNodes });
  }
  async readAsset(name: string, maxBytes: number) {
    // read_asset now returns raw bytes (tauri::ipc::Response) → an ArrayBuffer,
    // not a JSON number[] — far cheaper for large PDFs/images.
    const buf = await this.call<ArrayBuffer>("read_asset", { name, maxBytes });
    return new Uint8Array(buf);
  }
  async streamAsset(name: string) {
    const path = await this.call<string>("stream_asset_path", { name });
    return this.convertFileSrc(path, "tine-media");
  }
  async readLocalImage(path: string) {
    const buf = await this.call<ArrayBuffer>("read_local_image", { path });
    return new Uint8Array(buf);
  }
  saveAsset(name: string, bytes: Uint8Array, bindingGeneration: number) {
    if (bytes.byteLength > ASSET_INGRESS_MAX_BYTES) return Promise.reject(new Error("asset exceeds 64 MiB ingress limit"));
    return this.assetCall<string>("save_asset", { name, bytesB64: bytesToBase64(bytes) }, bindingGeneration);
  }
  async readClipboardImage(): Promise<Uint8Array | null> {
    try {
      if (await this.appPlatform() === "android") {
        const buf = await this.call<ArrayBuffer>("read_android_clipboard_image");
        const bytes = new Uint8Array(buf);
        return bytes.length ? bytes : null;
      }
      const { readImage } = await import("@tauri-apps/plugin-clipboard-manager");
      const img = await readImage();
      return await clipboardImageToPng(img);
    } catch {
      return null; // no image in clipboard, or plugin unavailable
    }
  }
  assetTrashStats() {
    return this.call<TrashStats>("asset_trash_stats");
  }
  emptyAssetTrash(bindingGeneration: number) {
    return this.assetCall<number>("empty_asset_trash", undefined, bindingGeneration);
  }
  listJournalConflicts() {
    return this.call<JournalConflict[]>("list_journal_conflicts");
  }
  listJournalFilenameMigrations() {
    return this.call<import("./types").JournalFilenameMigration[]>("list_journal_filename_migrations");
  }
  applyJournalFilenameMigrations(migrations: import("./types").JournalFilenameMigration[]) {
    return this.call<import("./types").JournalMigrationResult>("apply_journal_filename_migrations", { migrations });
  }
  trashJournalFile(name: string) {
    return this.call<void>("trash_journal_file", { name });
  }
  readJournalFile(name: string) {
    return this.call<string>("read_journal_file", { name });
  }
  getPageByPath(path: string) {
    return this.call<import("./types").PageRead | null>("get_page_by_path", { path });
  }
  mergePages(src: string, dst: string) {
    return this.call<void>("merge_pages", { src, dst });
  }
  renameFileToPage(path: string, newName: string) {
    return this.call<void>("rename_file_to_page", { path, newName });
  }
  listSyncConflicts() {
    return this.call<SyncConflict[]>("list_sync_conflicts");
  }
  syncConflictDiff(winner: string, conflict: string) {
    return this.call<SyncConflictDiff | null>("sync_conflict_diff", { winner, conflict });
  }
  resolveSyncConflict(
    winner: string,
    conflict: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    conflictRev: string,
    _kinds: EditKinds,
    preChoice?: "mine" | "theirs" | "union",
    mergeBaseRev?: string
  ) {
    return this.call<void>("resolve_sync_conflict", {
      winner,
      conflict,
      decisions,
      baseRev,
      conflictRev,
      mergeBaseRev: mergeBaseRev ?? null,
      preChoice: preChoice ?? "union",
    });
  }
  duplicateJournalDiff(canonical: string, stray: string) {
    return this.call<SyncConflictDiff | null>("duplicate_journal_diff", { canonical, stray });
  }
  resolveDuplicateJournalDay(
    canonical: string,
    stray: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    strayRev: string,
    _kinds: EditKinds,
    preChoice?: "mine" | "theirs" | "union"
  ) {
    return this.call<void>("resolve_duplicate_journal_day", {
      canonical, stray, decisions, baseRev, strayRev, preChoice: preChoice ?? "union",
    });
  }
  trashSyncConflict(conflict: string) {
    return this.call<void>("trash_sync_conflict", { conflict });
  }
  conflictInventory() {
    return this.call<ConflictInventory>("conflict_inventory");
  }
  vcsMarkerConflictDiff(path: string) {
    return this.call<MarkerConflictDiff | null>("vcs_marker_conflict_diff", { path });
  }
  resolveVcsMarkerConflict(
    path: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    _kinds: EditKinds,
    preChoice?: "mine" | "theirs" | "union"
  ) {
    return this.call<void>("resolve_vcs_marker_conflict", { path, decisions, baseRev, preChoice: preChoice ?? "union" });
  }
  liveConflictDiff(path: string, page: PageDto, baseRev: string | null) {
    return this.call<SyncConflictDiff>("live_conflict_diff", { path, page, baseRev });
  }
  mergeLiveConflict(path: string, page: PageDto, conflictRev: string, mergeBaseRev: string | undefined,
    decisions: Record<string, MergeDecision>, preChoice: "mine" | "theirs" | "union") {
    return this.call<PageDto>("merge_live_conflict", { path, page, conflictRev, mergeBaseRev, decisions, preChoice });
  }
  async onConflictsChanged(cb: () => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen("conflicts-changed", () => cb());
  }
  importAsset(path: string, name: string | undefined, bindingGeneration: number) {
    return this.assetCall<string>("import_asset", { path, name }, bindingGeneration);
  }
  importNativeCapture(path: string, name: string, bindingGeneration: number, graphRoot?: string) {
    return this.assetCall<string>("import_native_capture", { path, name, graphRoot }, bindingGeneration);
  }
  clipboardFiles() {
    return this.call<ClipboardFileList>("clipboard_files");
  }
  readTextFile(path: string) {
    return this.call<string>("read_text_file", { path });
  }
  async confirm(message: string, title?: string): Promise<boolean> {
    const { ask } = await import("@tauri-apps/plugin-dialog");
    return ask(message, { title: title ?? t("backend-backend-tine"), kind: "warning" });
  }
  async pickFolder(title?: string, defaultPath?: string): Promise<string | null> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const res = await open({ directory: true, multiple: false, title: title ?? t("backend-backend-open-graph-folder"), defaultPath });
    return typeof res === "string" ? res : null;
  }
  revealExport(path: string) { return this.call<void>("reveal_export", { path }); }
  readCssFile(global: boolean) { return this.call<{ text: string; revision: string | null }>("read_css_file", { global }); }
  saveCssFile(global: boolean, text: string, revision: string | null) { return this.call<string>("save_css_file", { global, text, revision }); }
  pickGraphFolder(): Promise<GraphFolderPickResult> {
    return this.call<GraphFolderPickResult>("pick_graph_folder");
  }
  prepareGraphFolder(path: string): Promise<PreparedGraphFolder> {
    return this.call<PreparedGraphFolder>("prepare_graph_folder", { path });
  }
  capturePhoto(): Promise<MediaCaptureResult> {
    return this.call<MediaCaptureResult>("capture_photo");
  }
  startRecording(): Promise<MediaCaptureResult> {
    return this.call<MediaCaptureResult>("start_recording");
  }
  stopRecording(): Promise<MediaCaptureResult> {
    return this.call<MediaCaptureResult>("stop_recording");
  }
  cancelRecording(): Promise<MediaCaptureResult> {
    return this.call<MediaCaptureResult>("cancel_recording");
  }
  async pickFile(): Promise<string | null> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const res = await open({ multiple: false, title: t("backend-backend-choose-a-file") });
    return typeof res === "string" ? res : null;
  }
  async writeText(text: string): Promise<void> {
    try {
      const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
      await writeText(text);
    } catch {
      await navigator.clipboard.writeText(text);
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
  async copyAssetFileToClipboard(name: string): Promise<boolean> {
    const generation = this.bindingGeneration;
    const platform = await this.appPlatform();
    if (platform === "android" || platform === "ios") return false;
    await this.assetCall<void>("copy_asset_file_to_clipboard", { name }, generation);
    return true;
  }
  async copyImageToClipboard(bytes: Uint8Array): Promise<void> {
    if (bytes.byteLength > ASSET_INGRESS_MAX_BYTES) throw new Error("image exceeds 64 MiB clipboard limit");
    // Binary InvokeBody avoids base64, JSON and a large UI-thread string build.
    await this.ready;
    const { invoke } = await import("@tauri-apps/api/core");
    if (await this.appPlatform() === "android") {
      return invoke<void>("copy_image_to_clipboard", { bytesB64: bytesToBase64(bytes) });
    }
    return invoke<void>("copy_image_to_clipboard", bytes);
  }
  readHighlights(pdf: string) {
    return this.call<Highlight[]>("read_highlights", { pdf });
  }
  openPdf(pdf: string, label: string, bindingGeneration: number) {
    return this.assetCall<PdfState>("open_pdf", { pdf, label }, bindingGeneration);
  }
  writeHighlights(pdf: string, label: string, highlights: Highlight[], baseHighlights: Highlight[], _kind: "replace-page", bindingGeneration: number) {
    return this.assetCall<Highlight[]>("write_highlights", { pdf, label, highlights, baseHighlights }, bindingGeneration);
  }
  savePdfAreaImage(pdf: string, page: number, id: string, stamp: number, bytes: Uint8Array, bindingGeneration: number) {
    if (bytes.byteLength > ASSET_INGRESS_MAX_BYTES) return Promise.reject(new Error("PDF area image exceeds 64 MiB ingress limit"));
    return this.assetCall<string>("save_pdf_area_image", {
      pdf,
      page,
      id,
      stamp,
      bytesB64: bytesToBase64(bytes),
    }, bindingGeneration);
  }
  rollbackPdfAreaImage(pdf: string, page: number, id: string, stamp: number, bindingGeneration: number) {
    return this.assetCall<void>("rollback_pdf_area_image", { pdf, page, id, stamp }, bindingGeneration);
  }
  private async on<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
    return (await import("@tauri-apps/api/event")).listen<T>(event, (e) => cb(e.payload));
  }
  onGraphChanged(cb: (c: GraphChange) => void) { return this.on("graph-changed", cb); }
  onGraphChangedBulk(cb: (bulk: { changes: GraphChange[]; binding_generation?: number; answers?: GraphAnswersChange | null }) => void) { return this.on("graph-changed-bulk", cb); }
  async onGraphWatchStatus(cb: (status: { refused: boolean; message: string; binding_generation?: number }) => void) {
    const [a, b] = await Promise.all([true, false].map((refused) => this.on<{ message: string }>(`graph-watch-${refused ? "refused" : "restored"}`, (p) => cb({ ...p, refused }))));
    return () => { a(); b(); };
  }
  onGraphRescanComplete(cb: (sequence: number) => void) { return this.on("graph-rescan-complete", cb); }
  rescanGraphNow(rebuild?: boolean) { return this.call<number>("rescan_graph_now", rebuild ? { rebuild: true } : undefined); }
  onAssetChanged(cb: (batch: AssetChangedBatch) => void) { return this.on("asset-changed", cb); }
  onGraphConfigChanged(cb: (change: GraphConfigChange) => void) { return this.on("graph-config-changed", cb); }
  onCustomCssChanged(cb: (change: CustomCssChange) => void) { return this.on("graph-custom-css-changed", cb); }
  editCustomCss() { return this.call<void>("edit_custom_css"); }
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
  getWatchMode() {
    return this.call<string>("get_watch_mode");
  }
  setWatchMode(mode: string) {
    return this.call<void>("set_watch_mode", { mode });
  }
  listBackups() {
    return this.call<BackupInfo[]>("list_backups");
  }
  restoreBackup(stamp: string, _kind: "replace-page", consumedLastId: number) {
    return this.call<{ error: string | null; reloaded: { session: number; nextId: number } | null }>(
      "restore_backup", { stamp, consumedLastId });
  }
  loadSession() {
    return this.call<string | null>("load_session");
  }
  saveSession(data: string) {
    return this.call<void>("save_session", { data });
  }
  loadWorkspaces() {
    return this.call<string>("load_workspaces");
  }
  saveWorkspaces(data: string) {
    return this.call<"durable" | "published-unsynced">("save_workspaces", { data });
  }
  gpuEnv() {
    return this.call<GpuEnv>("gpu_env");
  }
  takeDataHomeFallbackNotice() {
    return this.call<string | null>("take_data_home_fallback_notice");
  }
  debugInfo() {
    return this.call<DebugInfo>("debug_info");
  }
  debugLog(line: string) {
    return this.call<void>("debug_log", { line });
  }
  diagnosticReport(buildCommit: string, buildTime: string) {
    return this.call<DiagnosticReport>("diagnostic_report", { buildCommit, buildTime });
  }
  saveDiagnosticReport(buildCommit: string, buildTime: string) {
    return this.call<boolean>("save_diagnostic_report", { buildCommit, buildTime });
  }
  clearDiagnostics() {
    return this.call<void>("clear_diagnostics");
  }
  createGraphVerification(operationId: string) { return this.call<GraphVerificationReport>("create_graph_verification", { operationId }); }
  cancelGraphVerification(operationId: string) { return this.call<void>("cancel_graph_verification", { operationId }); }
  saveGraphVerificationReport(text: string) { return this.call<boolean>("save_graph_verification_report", { text }); }
  onGraphVerificationProgress(cb: (progress: GraphVerificationProgress) => void) { return this.on("graph-verification-progress", cb); }
  diagnosticSessionActive(active: boolean) {
    return this.call<void>("diagnostic_session_active", { active });
  }
  diagnosticFrontendEvent(kind: DiagnosticFrontendKind, fields: DiagnosticFrontendFields = {}) {
    return this.call<void>("diagnostic_frontend_event", { kind, ...fields });
  }
  localClock() {
    return this.call<{ offset_minutes: number; unix_ms: number }>("local_clock");
  }
  appArchitecture() {
    return this.call<string>("app_architecture");
  }
  watcherLatencyRecent() {
    return this.call<unknown[]>("watcher_latency_recent");
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
  defenderHint() {
    return this.call<{ show: boolean }>("defender_hint");
  }
  dismissDefenderHint() {
    return this.call<void>("dismiss_defender_hint");
  }
  addDefenderExclusion() {
    return this.call<Awaited<ReturnType<Backend["addDefenderExclusion"]>>>("add_defender_exclusion");
  }
  getAppString(key: string, fallback: string) {
    return this.call<string>("get_app_string", { key, default: fallback });
  }
  setAppString(key: string, value: string) {
    return this.call<void>("set_app_string", { key, value });
  }
  setUiLocale(locale: string) {
    return this.call<void>("set_ui_locale", { locale });
  }
  applySpellcheck(enabled: boolean, languages: string[]) {
    return this.call<void>("apply_spellcheck", { enabled, languages });
  }
  listSpellcheckDictionaries() {
    return this.call<string[]>("list_spellcheck_dictionaries", {});
  }
  setSmoothScroll(value: boolean) {
    return this.call<void>("set_smooth_scroll", { value });
  }
}

let _backend: Backend | null = null;

export function backend(): Backend {
  if (!_backend) {
    _backend = isTauri() ? new TauriBackend() : isPublishedExport() ? publishedBackend() : mockBackend();
  }
  return _backend;
}

/** OG-visible graph property keys/values for the block editor. Kept separate
 * from query-builder facets even though both share the registered IPC command. */
export function autocompleteFacets(): Promise<[string, string[]][]> {
  return backend().queryFacets(true);
}
