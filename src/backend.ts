// Backend abstraction. In the Tauri app we call Rust via `invoke`. In a plain
// browser (Vite dev / Playwright screenshots) we fall back to an in-memory mock
// seeded from a fixture graph, so the whole UI is exercisable without the shell.

import { readSavePlatformStep } from "./savePlatformStep";
import { markCommandSlow } from "./slowBackend";
import { timingNamesForCommand } from "./focusTiming";
import type { GraphVerificationProgress, GraphVerificationReport } from "./graphVerification";
import type {
  Diagnostic,
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
  DraftRecord,
} from "./types";
import type { GraphSourceFile, GraphFolderPickResult, ClipboardFileList, MediaCaptureResult, KnownGraph, InstalledPluginRecord, PluginRegistryCacheLoad, LoadGraphResult, CaptureGraphBindingResult, GraphAccessInspection } from "./backendTypes";
import { dbg } from "./debug";
import { assetFileName } from "./media";
import type { EditKinds } from "./editKind";
import { mockBackend } from "./mock";
import type { SheetExport, SheetInput, SheetScope } from "./sheet/staticExport";
import { isPublishedExport, publishedBackend } from "./publishedBackend";

import { nativeTineLinks, type NativeTineLinks } from "./nativeTineLinks";

export interface SavePageEntry {
  id: string;
  page: PageDto;
  baseRev: string | null;
  force: boolean;
  kinds: EditKinds;
}

/** Native publication delta, shared by save acknowledgements and watcher events.
 * Values are final counts for only the changed targets; zero clears a badge. */
export interface GraphAnswersChange {
  rev: string;
  inventoryChanged: boolean;
  blockRefCounts: Record<string, number>;
}

export type SavePagesResult =
  | { ok: string[]; changes?: GraphAnswersChange | null }
  | { failed: { index: number; family: string; diskRev?: string | null; undoFailed: string[]; publicationErrors?: string[]; unreadableOwner?: string; operation?: string; osError?: number } };

/** Adapt a one-page intent to the shared request while preserving its refusal.
 * Calls observed with its native answer delta before returning the revision;
 * the observer owns graph-binding validation. Cost follows save plus targets. */
export async function saveOnePage(api: Backend, entry: SavePageEntry, bindingGeneration?: number, observed?: (change: GraphAnswersChange | null | undefined) => void): Promise<string> {
  const result = await api.savePages([entry], bindingGeneration);
  if ("failed" in result) throw Object.assign(new Error(result.failed.family), { diskRev: result.failed.diskRev, platformStep: readSavePlatformStep(result.failed), unreadableOwner: result.failed.unreadableOwner });
  observed?.(result.changes);
  return result.ok[0];
}

// Encode asset bytes as one base64 string for the save_*/copy_image IPC. The old
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

export const CLIPBOARD_IMAGE_MAX_PIXELS = 32 * 1024 * 1024;
export const CLIPBOARD_IMAGE_MAX_RGBA_BYTES = 128 * 1024 * 1024;
export const ASSET_INGRESS_MAX_BYTES = 64 * 1024 * 1024;

type ClipboardImage = {
  size(): Promise<{ width: number; height: number }>;
  rgba(): Promise<Uint8Array>;
};

export async function clipboardImageToPng(img: ClipboardImage): Promise<Uint8Array | null> {
  // Dimensions are metadata: validate them before asking the native plugin to
  // materialize an attacker-controlled RGBA allocation on the WebView thread.
  const { width, height } = await img.size();
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width <= 0 || height <= 0) return null;
  const pixels = width * height;
  const rgbaBytes = pixels * 4;
  if (!Number.isSafeInteger(pixels) || pixels > CLIPBOARD_IMAGE_MAX_PIXELS
      || rgbaBytes > CLIPBOARD_IMAGE_MAX_RGBA_BYTES) {
    return null;
  }
  const rgba = await img.rgba();
  if (rgba.byteLength !== rgbaBytes) return null;
  const clamped = new Uint8ClampedArray(rgba.buffer as ArrayBuffer, rgba.byteOffset, rgba.byteLength);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.putImageData(new ImageData(clamped, width, height), 0, 0);
  const blob: Blob | null = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob || blob.size > ASSET_INGRESS_MAX_BYTES) return null;
  const encoded = await blob.arrayBuffer();
  return encoded.byteLength <= ASSET_INGRESS_MAX_BYTES ? new Uint8Array(encoded) : null;
}

export interface Backend {
  graphBindingGeneration(): number;
  inspectGraphAccess(path: string): Promise<GraphAccessInspection>;
  approveExternalAssets(graphRoot: string, assetsPath: string): Promise<void>;
  loadGraph(path: string): Promise<LoadGraphResult>;
  openGraphWindow(path: string): Promise<LoadGraphResult>;
  startupGraphPath(): Promise<string | null>;
  tineLinks?: NativeTineLinks;
  captureTarget(): Promise<string>;
  /** Lease the graph selected for this Quick Capture show before issuing
   * graph-scoped reads from its independent WebView. */
  bindCaptureGraph(): Promise<void>;
  listKnownGraphs(): Promise<KnownGraph[]>;
  forgetKnownGraph(path: string): Promise<void>;
  /** Reveal a remembered graph's folder in the desktop file manager. */
  revealKnownGraph(path: string): Promise<void>;
  appPlatform(): Promise<"android" | "ios" | "desktop">;
  /** Immutable, app-local plugin packages. Installation stores bytes but never
   * executes them; enabling is an explicit second step after host validation. */
  listInstalledPlugins(): Promise<InstalledPluginRecord[]>;
  installPlugin(manifestJson: string, wasm: Uint8Array): Promise<InstalledPluginRecord>;
  /** Remove one immutable local plugin package. This never touches graph data. */
  uninstallPlugin(id: string, version: string): Promise<void>;
  readPluginEntry(id: string, version: string): Promise<Uint8Array>;
  setPluginEnabled(id: string, version: string, enabled: boolean): Promise<void>;
  verifyPluginRegistry(indexJson: string, signatureB64: string): Promise<void>;
  loadPluginRegistryCache(): Promise<PluginRegistryCacheLoad>;
  storePluginRegistryCache(
    indexJson: string,
    signature: string
  ): Promise<void>;
  /** Keep Android's edge-to-edge status/navigation icon appearance readable
   *  against Tine's explicit in-app theme. Other platforms are a no-op. */
  setSystemBarAppearance(dark: boolean): Promise<void>;
  defaultGraphParent(): Promise<string>;
  /** Quit the app. On Linux this first SIGKILLs WebKitGTK's helper subprocesses so
   *  they don't dump a SIGABRT core on exit (GH #28 — GL driver atexit double-free);
   *  the caller MUST have flushed pending edits first. Does not resolve — the
   *  process exits. */
  quit(): Promise<void>;
  closeGraphWindow(): Promise<void>;
  /** Toggle the WebView developer tools (WebKit Web Inspector) for theme/CSS
   *  debugging. No-op on a build without devtools compiled in. */
  openDevtools(): Promise<void>;
  /** Scaffold a brand-new demo graph (onboarding "create new graph"); returns
   *  the created graph's root path to then `loadGraph`. Creates the graph in
   *  `dir` if empty, else in a fresh `tine-demo` subfolder. */
  createGraph(dir: string): Promise<string>;
  /** The whole name inventory (physical pages/journals, aliases, reference-only
   *  names), each with the backend's resolved target. Cached only by
   *  `pageIndex.ts` holds the full inventory; `store.ts` separately indexes
   *  loaded pages in its working set. */
  pageInventory(): Promise<import("./types").PageInventory>;
  journalFeedPage(limit: number, beforeDay: number | null): Promise<import("./types").JournalFeedPage>;
  /** Journal date-keys (yyyymmdd) whose page has real content. */
  journalContentDays(): Promise<number[]>;
  getPage(name: string, kind: "journal" | "page"): Promise<import("./types").PageRead | null>;
  resolvePage(name: string, kind: "journal" | "page"): Promise<import("./types").ResolvedPage>;
  /** Raw source text of every md/org file in the open graph (+journals when
   *  asked), for the "Help improve Tine" diff panel. Read-only, local. */
  graphSourceFiles(includeJournals: boolean): Promise<GraphSourceFile[]>;
  /** Native backend: require a current graph binding, prepare bases (force
   * reads current UTF-8 bytes), and save ordered entries in one guarded
   * transaction. Success strings are file revisions; failure paths are
   * graph-relative. Transaction refusals resolve as failed; binding/invoke
   * failures reject. An empty request resolves as failed. The mock returns
   * "mock-rev" per entry without saving or validating. */
  savePages(entries: SavePageEntry[], bindingGeneration?: number): Promise<SavePagesResult>;
  /** Bundled read-only Guide pages, compiled from the same templates as the demo graph. */
  guidePages(): Promise<GuidePage[]>;
  /** Copy the bundled Guide into the real graph under `tine-guide/`. */
  copyGuideIntoGraph(title: string, kind: "replace-page"): Promise<GuideCopyResult>;
  /** Persist the graph-local one-time Guide announcement flag. */
  setGuideAnnounced(announced: boolean): Promise<void>;
  getBacklinks(name: string): Promise<RefGroup[]>;
  /** Parser-owned visible-subtree/facet index for only the roots in an open
   *  Linked References filter. Ordinary backlink DTOs stay shallow. */
  getBacklinkFilterContext(name: string, targets: BacklinkFilterTarget[]): Promise<BacklinkFilterContext>;
  getUnlinkedRefs(name: string): Promise<RefGroup[]>;
  /** True once the background whole-graph warm has built derived graph-open caches. */
  warmDone(): Promise<boolean>;
  /** Map of block uuid → number of blocks that reference it (the count badge). */
  getBlockRefCounts(): Promise<Record<string, number>>;
  /** Blocks that reference block `uuid`, grouped by page (the referrers panel). */
  getBlockReferrers(uuid: string): Promise<RefGroup[]>;
  deletePage(name: string, kind: "journal" | "page", expectedPath?: string): Promise<void>;
  /** Rename a page and update all [[refs]]/#tags across the graph. `mergeInto`
   *  is the confirmed path of the one page `next` reaches (its file or alias
   *  owner); the backend merges into it in one transaction and refuses if that
   *  changed. Without it, a `next` another page already has is refused.
   *  `unsavedPaths`: page files whose edits could not be saved; the rename
   *  refuses, writing nothing, if it would move or rewrite one (GH #535). */
  renamePage(old: string, next: string, kind: "rename-page", expectedPath?: string, mergeInto?: string, unsavedPaths?: string[]): Promise<import("./types").RenameDone>;
  publishHtml(): Promise<[string, number]>;
  /** Resolve a query's complete owner pages without writing. O(graph query +
   * selected source bytes); the fingerprint binds the reviewed selection. */
  publishQueryPlan(request: QueryPublicationRequest): Promise<QueryPublicationPlan>;
  /** Recheck sources and commit a graph query leaf. Replace preserves/reports
   * prior output; create refuses collisions. Asset-budget refusal is typed as
   * {kind: "assetBudget", message}; missing assets are receipt warnings. */
  publishQuery(request: QueryPublicationRequest, fingerprint: string, sheets: SheetExport[]): Promise<PublicationReceipt>;
  /** Publish a whole-graph read-only app plus static fallback under a picked
   * folder. `allPages` explicitly includes private pages; default public only. */
  publishLive(destination: string, name: string, allPages: boolean, sheets: SheetExport[]): Promise<PublicationReceipt>;
  /** The `tine.view` sheet blocks of the named pages (every page when omitted)
   * with the data the sheet evaluator needs to compute each for a static export.
   * The `sheets` argument of publishLive/publishQuery/pagePrintHtml answers it
   * (see sheet/exportSheets.ts); without it a sheet exports as its plain outline. */
  sheetExportInputs(pages?: string[], scope?: SheetScope): Promise<SheetInput[]>;
  /** Render one page to a self-contained HTML document (assets inlined, no
   *  sidebar) for the print-to-PDF export, with the dialog's options. Rejects if
   *  the page doesn't exist. */
  pagePrintHtml(name: string, opts: PrintOpts, sheets: SheetExport[]): Promise<string>;
  /** Resolve all Copy / Export query macros under one cumulative native budget. */
  exportQuerySubtrees(specs: QueryExportSpec[]): Promise<QueryExportBatch>;
  /** Parse text and host `tine.*` properties through the Rust query engine.
   *  Syntax errors resolve as raw nodes with diagnostics. Cost: waits for graph
   *  load; first registry use is O(pages + blocks), then O(text). Rejects an
   *  over-64-KiB UTF-8 source, excessive nesting, or a missing/stale/failed graph. */
  parseQuery(text: string, dialect: QueryTextDialect, blockProperties?: [string, string][]): Promise<ParsedQuery>;
  /** Print a macro argument (`og`, `tql_macro`, `advanced_macro`) or TQL pane text.
   *  Pure. `preserveForm` keeps authored text/options, but refuses builder or
   *  wrong-dialect sources. OG prints one sort and sample; other view fields
   *  need `tine.*` properties. TQL ignores view. Rejects with
   *  {@link QueryPrintRefusedError} for inexpressible or macro-unsafe text. */
  printQuery(query: Query, view: ViewSettings, dialect: QueryPrintDialect, preserveForm?: boolean): Promise<string>;
  /** Precondition for OG printing, not a guarantee: macro safety can still
   *  refuse. Pure O(IR); rejects on transport failure. */
  queryOgExpressible(query: Query, view: ViewSettings): Promise<boolean>;
  /** One row per normalized property key, at most eight top values. Built once
   *  per generation, O(pages + blocks); waits for graph load. Rejects a missing,
   *  stale, closed or failed graph. */
  queryRegistry(): Promise<RegistrySnapshot>;
  /** Evaluate in memory, O(pages + blocks) cold, memoized per snapshot. Invalid
   *  input resolves with diagnostics and zero rows. An answer over 20,000 rows
   *  or 32 MiB, or over the statistics budget, rejects; no truncated answer is
   *  returned. Context binds only advanced `:current-page`; OG/TQL ignore it. */
  queryRun(query: Query, view: ViewSettings, context?: ExecutionContext): Promise<QueryResult>;
  /** On-demand empty explanation: roughly two full evaluations per conjunct,
   *  O(conjuncts × (pages + blocks)), without result rows or memoization. Ignores
   *  view; no `result-too-large` refusal. Waits for graph load. */
  queryExplainEmpty(query: Query, view: ViewSettings, context?: ExecutionContext): Promise<ExplainEmptyResult>;
  /** Property keys (each with their distinct values) for query-builder
   *  autocomplete. */
  queryFacets(autocomplete?: boolean): Promise<[string, string[]][]>;
  /** `icon::` property for each named page that has one (page-name → icon). */
  pageIcons(names: string[]): Promise<Record<string, string>>;
  /** Persist favorited page names to config.edn `:favorites` and, when given,
   *  the arrangement page to `:tine/favorites-page`, in one config write. */
  setFavorites(names: string[], page?: string | null): Promise<void>;
  /** Persist (or clear) `:default-home {:page "…"}` through the graph config writer. */
  setDefaultHome(name: string | null): Promise<void>;
  /** Persist the task workflow to config.edn `:preferred-workflow`. */
  setPreferredWorkflow(workflow: "now" | "todo"): Promise<void>;
  /** Persist `:feature/enable-timetracking?` (default on when absent). */
  setTimetrackingEnabled(enabled: boolean): Promise<void>;
  /** Persist `:ui/show-brackets?` (default on when absent). */
  setShowBrackets(enabled: boolean): Promise<void>;
  /** Persist document-mode Enter's structural escape hatch to config.edn. */
  setDocModeEnterForNewBlock(enabled: boolean): Promise<void>;
  /** Persist `:editor/logical-outdenting?` (default off when absent). */
  setLogicalOutdenting(enabled: boolean): Promise<void>;
  /** Persist the format new pages/journals are created in to config.edn
   *  `:preferred-format` ("md" | "org"). */
  setPreferredFormat(format: "md" | "org"): Promise<void>;
  /** Persist the journal display-title format to config.edn
   *  `:journal/page-title-format` (e.g. "MMM do, yyyy"). Display-only — does not
   *  rename journal files (`:journal/file-name-format` is separate). */
  setJournalTitleFormat(format: string, kinds: EditKinds): Promise<void>;
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
  // Graph binding: every asset/PDF method that takes `bindingGeneration` needs
  // a positive safe-integer generation for this window's current graph. The frontend
  // rejects missing/invalid generations with missing-graph-binding; after IPC, a
  // missing graph rejects with `no graph loaded for window …`, and a stale generation
  // against a bound graph rejects with stale-graph-binding. Pre-IPC null and size-limit
  // paths may return or reject first.
  /** Open an existing regular assets-relative file in the desktop OS default app.
   * In-area symlinks may resolve; invalid/escaped paths, missing files, I/O, and
   * unsupported mobile handoff reject. Cost O(path components). */
  openAsset(name: string, bindingGeneration: number): Promise<void>;
  openPageFile(name: string, kind: "page" | "journal", path: string | undefined, reveal: boolean): Promise<void>;
  /** Launch a desktop editor for an existing assets-relative file. Blank command
   * uses the OS opener; otherwise argv is parsed without a shell, replacing `{}`
   * with the path or appending it. Resolves on spawn, before editing completes.
   * Invalid/missing assets, command, spawn, or platform errors reject.
   * Cost O(path components + command bytes). */
  editAssetExternal(name: string, command: string, bindingGeneration: number): Promise<void>;
  /** Best-effort autodetect of an installed editor's launch command (probes disk,
   *  never executes). Returns a command template or "" if not found. */
  detectMediaEditor(id: string): Promise<string>;
  /** Top-level `assets/` files no block references (orphans), for cleanup. */
  listOrphanAssets(): Promise<AssetInfo[]>;
  /** Trash one top-level asset, whether referenced or not. Reads the whole file
   * per attempt and retries revision conflicts up to four times. Missing assets
   * and exhausted conflicts reject. Cost O(file bytes) per attempt. */
  trashAsset(name: string, bindingGeneration: number): Promise<void>;
  /** Count + total bytes of the recoverable asset trash (logseq/.tine-trash). */
  assetTrashStats(): Promise<TrashStats>;
  /** Permanently purge asset trash and return completed entry count. A failure
   * can follow partial deletion; its error reports entries and bytes removed.
   * Cost O(asset trash entries + bytes removed). */
  emptyAssetTrash(bindingGeneration: number): Promise<number>;
  /** Journal days that resolve to >1 file (date-stem + title-named, or md/org
   *  twin) — for the user to reconcile. */
  listJournalConflicts(): Promise<JournalConflict[]>;
  /** Move one journal file (by exact filename) to the recoverable trash. */
  trashJournalFile(name: string, kind: "delete-page"): Promise<void>;
  /** Title-named journal files and the date names they would get. Opening a
   *  graph only proposes these renames (master e6f9b6e1ceae). */
  listJournalFilenameMigrations(): Promise<import("./types").JournalFilenameMigration[]>;
  /** Apply the proposed renames after a snapshot of the graph. */
  /** Renames only these confirmed proposals; stale ones come back as skipped. */
  applyJournalFilenameMigrations(migrations: import("./types").JournalFilenameMigration[]): Promise<import("./types").JournalMigrationResult>;
  /** Raw contents of one journal file (by exact filename), for inspecting a
   *  duplicate day's files before reconciling. */
  readJournalFile(name: string): Promise<string>;
  /** Load a page from a SPECIFIC file by its graph-root-relative path — reaches a
   *  duplicate-day stray that shares a (kind,name) with the canonical file (#21). */
  getPageByPath(path: string): Promise<import("./types").PageRead | null>;
  /** Append the blocks of `src` (graph-root-relative path) onto `dst`, then trash
   *  `src` — fold a duplicate-day stray into the canonical day (#21). */
  mergePages(src: string, dst: string, kinds: EditKinds): Promise<void>;
  /** Move a stray file (graph-root-relative path) to a uniquely-named page so it
   *  stops colliding and becomes normally navigable (#21). */
  renameFileToPage(path: string, newName: string, kind: "rename-page"): Promise<void>;
  /** Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
   *  user to review + merge instead of them showing as garbage pages. */
  listSyncConflicts(): Promise<SyncConflict[]>;
  /** Block-level diff of a conflict copy against its winner (graph-root-relative
   *  paths). Read-only; null if a path is invalid or the file is gone. */
  syncConflictDiff(winner: string, conflict: string): Promise<SyncConflictDiff | null>;
  /** Merge a conflict copy into its winner per the user's per-row decisions
   *  (row id → mine/theirs/both), via the normal save path, then trash the copy.
   *  `baseRev` guards against the winner changing under the merge (throws
   *  "conflict" if it did). `preChoice`: "mine" | "theirs" | "union". */
  resolveSyncConflict(
    winner: string,
    conflict: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    conflictRev: string,
    kinds: EditKinds,
    preChoice?: "mine" | "theirs" | "union",
    mergeBaseRev?: string
  ): Promise<void>;
  /** Discard a conflict copy without merging (move it to the recoverable trash). */
  trashSyncConflict(conflict: string, kind: "delete-page"): Promise<void>;
  /** Two-way diff of a duplicate journal day's canonical file against one
   *  stray; null for a cross-format pair, which cannot be folded. Read-only. */
  duplicateJournalDiff(canonical: string, stray: string): Promise<SyncConflictDiff | null>;
  /** Fold one stray of a duplicate journal day into the day's canonical file
   *  per the reviewed decisions and trash the stray (recoverable). Throws
   *  "conflict" when either file changed since the review. */
  resolveDuplicateJournalDay(
    canonical: string,
    stray: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    strayRev: string,
    kinds: EditKinds,
    preChoice?: "mine" | "theirs" | "union"
  ): Promise<void>;
  /** The conflict listings and the derived queue, one answer; never stored.
   *  Cost: the first call per graph walks every page file (O(graph text
   *  bytes)); later calls answer from the backend's change-fed queue. */
  conflictInventory(): Promise<ConflictInventory>;
  /** A marker-bearing page's own sides as a block diff (3-way when the markers
   *  carry a common ancestor). Read-only; null when it carries no markers. */
  vcsMarkerConflictDiff(path: string): Promise<MarkerConflictDiff | null>;
  /** Rewrite a marker-bearing page per the user's decisions, guarded by the
   *  file's `baseRev` ("conflict" if it changed); the pre-resolution bytes are
   *  first staged in the recoverable trash. */
  resolveVcsMarkerConflict(
    path: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    kinds: EditKinds,
    preChoice?: "mine" | "theirs" | "union"
  ): Promise<void>;
  /** Review an editor draft whose save was refused against the file as it is
   *  now (og 8e); 3-way when the Concord ledger retains the draft's `baseRev`.
   *  Read-only; `conflict_rev` is the disk revision shown, or "absent". */
  liveConflictDiff(path: string, page: PageDto, baseRev: string | null): Promise<SyncConflictDiff>;
  /** Write the reviewed live resolution in one guarded transaction at
   *  `conflictRev` ("conflict" if the disk or the reviewed ledger base moved).
   *  Returns the written page with its new revision. */
  resolveLiveConflict(path: string, page: PageDto, baseRev: string | null, conflictRev: string,
    mergeBaseRev: string | undefined, decisions: Record<string, MergeDecision>,
    preChoice: "mine" | "theirs" | "union"): Promise<PageDto>;
  /** Subscribe to the backend's `conflicts-changed` event (the derived
   *  conflict queue changed). Returns an unlisten fn. */
  onConflictsChanged(cb: () => void): Promise<() => void>;
  search(query: string, limit: number, lane?: string): Promise<RefGroup[]>;
  /** One Rust-authoritative graph scan for bounded page and block hits. Page
   * membership defaults to names/aliases; content and both scan block text.
   * Each Display view sorts before its section limit, then sample caps that
   * limit; omitted views keep relevance order. Cost O(graph text) off the UI
   * thread, plus O(matches log matches) for authored sorts. Native errors reject. */
  runGraphSearch(
    source: string,
    pageLimit: number,
    blockLimit: number,
    lane?: string,
    explain?: boolean,
    scope?: QueryPageScope,
    pageMatchScope?: import("./editor/queryIr").FriendlyPageMatchScope,
    views?: { page: ViewSettings; block: ViewSettings }
  ): Promise<QueryExecution>;
  quickSwitch(query: string, limit: number): Promise<PageEntry[]>;
  /** Capture-only page/tag completion capability. It is intentionally not the
   * general graph command route used by the main WebView. */
  captureQuickSwitch(query: string, limit: number): Promise<PageEntry[]>;
  listTemplates(): Promise<TemplateDto[]>;
  resolveBlock(uuid: string): Promise<RefGroup | null>;
  resolveBlocks(uuids: string[]): Promise<(RefGroup | null)[]>;
  /** Explicit bounded subtree; ordinary resolution is intentionally shallow. */
  previewBlock(uuid: string, maxNodes: number): Promise<BlockPreview | null>;
  readAsset(name: string, maxBytes?: number): Promise<Uint8Array>;
  /** Native range-aware URL for audio/video. Unlike `readAsset`, this never
   *  copies the whole media file through IPC. */
  streamAsset(name: string): Promise<string>;
  /** Read an image from an absolute path OUTSIDE the graph (raw-HTML `<img>` the
   *  user opted into via Settings). Rejects when the opt-in is off or the path
   *  isn't a permitted image. */
  readLocalImage(path: string): Promise<Uint8Array>;
  /** Save up to 64 MiB under a unique top-level asset name, without overwriting.
   * Return the chosen assets-relative name. Invalid names and writes reject.
   * Cost O(bytes + collision candidates). */
  saveAsset(name: string, bytes: Uint8Array, bindingGeneration: number): Promise<string>;
  /** Read the OS clipboard image, convert to PNG, and save under a unique name.
   * Return null if clipboard access/conversion yields no image; save failures
   * reject. A saved image returns its assets-relative name. Cost O(image bytes +
   * collision candidates). */
  pasteImage(bindingGeneration: number): Promise<string | null>;
  /** Decode an image off the OS clipboard to PNG bytes WITHOUT saving (the
   *  caller seeds the render cache + writes to disk in the background, so the
   *  pasted image appears instantly). Null if the clipboard has no image. */
  readClipboardImage(): Promise<Uint8Array | null>;
  /** Stream a device file into assets under a nonempty explicit top-level name,
   * or its source basename. Collisions choose a unique name; return that name.
   * No source-byte cap. Bad source/name or write failures reject.
   * Cost O(source bytes + collision candidates). */
  importAsset(path: string, name: string | undefined, bindingGeneration: number): Promise<string>;
  /** Import an app-cache tine_photo_*.jpg (64 MiB max) or tine_memo_*.m4a
   * (32 MiB max) capability into a unique asset. Reject empty/invalid sources,
   * bad names, size limits, and writes. After commit, attempt temp removal;
   * cleanup failure does not reject. Cost O(source bytes + collision candidates).
   * `graphRoot` names the graph the capture was started in (og H1b); Rust refuses one that is not a known graph. */
  importNativeCapture(path: string, name: string, bindingGeneration: number, graphRoot?: string): Promise<string>;
  /** Paths explicitly copied in the OS file manager. Empty when the clipboard
   *  has no native file-list flavor or the platform cannot expose one. */
  clipboardFiles(): Promise<ClipboardFileList>;
  /** Read a dropped UTF-8 text file by absolute path. Used only for explicit
   *  local file drops such as CSV/TSV import. */
  readTextFile(path: string): Promise<string>;
  /** Native yes/no confirmation dialog. Returns true if the user confirms.
   *  Uses the GTK dialog plugin, NOT window.confirm — the latter silently
   *  returns true without showing anything in this WebKitGTK build, which would
   *  bypass destructive-action and close-tab prompts. */
  confirm(message: string, title?: string): Promise<boolean>;
  /** Native folder picker (graph open). Null if cancelled / unsupported.
   *  `title` overrides the dialog title (e.g. for "create new graph"). */
  pickFolder(title?: string): Promise<string | null>;
  /** Android native graph-folder picker. Returns a real filesystem path when
   *  picked; never a content URI. */
  pickGraphFolder(): Promise<GraphFolderPickResult>;
  /** Native file picker (asset upload). Null if cancelled / unsupported. */
  pickFile(): Promise<string | null>;
  /** Android: take a photo with the camera (or pick an existing image) → base64
   *  bytes + ext. `status: "cancelled"` if dismissed. */
  capturePhoto(): Promise<MediaCaptureResult>;
  /** Android: start a voice-memo recording (prompts for mic permission on first
   *  use). `status: "recording"` on success. */
  startRecording(): Promise<MediaCaptureResult>;
  /** Android: stop the active recording → native cache-file path + ext. */
  stopRecording(): Promise<MediaCaptureResult>;
  /** Android: discard an in-progress recording without inserting anything. */
  cancelRecording(): Promise<MediaCaptureResult>;
  writeText(text: string): Promise<void>;
  /** Copy with text/plain (markdown) + text/html flavors; degrades to text/plain. */
  writeRich(text: string, html: string): Promise<void>;
  /** Write a PNG image (bytes) to the OS clipboard. Goes through the Rust
   *  clipboard plugin, not WebKitGTK's native "Copy Image" (which doesn't
   *  actually populate the clipboard, so paste yielded nothing). */
  copyImageToClipboard(bytes: Uint8Array): Promise<void>;
  readHighlights(pdf: string): Promise<Highlight[]>;
  /** Read PDF highlights and view state without creating, rewriting or moving
   * graph files. Prefer the OG-key sidecar; only when absent, consult legacy
   * unless another PDF owns its key. An unavailable asset listing permits legacy
   * lookup. Missing files return empty state; malformed nonblank EDN and sidecar
   * read failures reject. A stale graph binding rejects; label does not affect
   * this read.
   * Annotation pages are created by writeHighlights on annotation actions.
   * Cost O(asset entries + sidecar bytes), with no graph refresh. */
  openPdf(pdf: string, label: string, bindingGeneration: number): Promise<PdfState>;
  /** Native backend: merge caller changes by highlight ID against the current
   * sidecar. Changed color, text and image values win locally; unchanged values
   * follow disk. Page and position form one geometry value: changing either
   * locally selects both local values. Unchanged highlights follow disk,
   * including deletion, and disk-only additions survive. An edit versus an
   * external deletion, or a deletion versus an external edit, rejects before
   * either artifact is written.
   * Return the committed set for the next baseline only on success.
   * A blank sidecar or valid top-level EDN map is accepted; malformed nonblank
   * EDN rejects. Malformed highlight entries within a valid map are skipped,
   * and duplicate IDs are not rejected. Sidecar and annotation
   * page use one guarded transaction. A failure with incomplete undo or
   * publication can leave disk uncertain: retain edits and inspect disk.
   * Crop/legacy trash moves after commit are best effort and do not reject.
   * Invalid or stale bindings reject. Cost O(asset entries + sidecar + page +
   * deleted crop bytes + deleted crops × sidecar bytes) per retry, plus up to
   * O(P) refresh if the page is absent.
   * The mock stores caller values directly without merge, files, or these failures. */
  writeHighlights(pdf: string, label: string, highlights: Highlight[], baseHighlights: Highlight[], kind: "replace-page", bindingGeneration: number): Promise<Highlight[]>;
  /** Update page and scale while preserving other sidecar fields; retry and merge
   * concurrent changes up to four attempts. Invalid state/sidecar, I/O, or
   * exhausted conflicts reject. Cost O(asset entries + sidecar) per attempt. */
  writePdfViewState(pdf: string, page: number, scale: number, bindingGeneration: number): Promise<void>;
  /** Save a cropped area-highlight PNG to OG's layout `assets/<key>/<page>_<id>_<stamp>.png`
   *  (non-dedup — the filename links the `.edn` `:image <stamp>` to the file).
   *  Returns the assets-relative path; a repeated save replaces that crop.
   *  Rejects payloads over 64 MiB before IPC and Store failures. Each of up to
   *  four attempts reads the existing crop; cost O(existing + input bytes). */
  savePdfAreaImage(pdf: string, page: number, id: string, stamp: number, bytes: Uint8Array, bindingGeneration: number): Promise<string>;
  /** Trash a crop only when the current primary sidecar does not reference its
   * ID/stamp. A whitespace-only sidecar rewrite and crop trash share one
   * revision-guarded transaction.
   * Cost O(sidecar + crop bytes) per attempt, up to four attempts. Missing or
   * malformed sidecar, referenced/missing crop, I/O, and conflicts reject. */
  rollbackPdfAreaImage(pdf: string, page: number, id: string, stamp: number, bindingGeneration: number): Promise<void>;
  /** Subscribe to external file changes (file watcher). Returns an unsubscribe. */
  onGraphChanged(cb: (c: GraphChange) => void): Promise<() => void>;
  /** Watcher freshness (family 10), native only: a checkout-sized batch as one
   *  event, a refused/restored OS watch, and a focus rescan (one full stat
   *  diff) whose returned sequence completes after its page events. */
  onGraphChangedBulk?(cb: (bulk: { changes: GraphChange[]; binding_generation?: number; answers?: GraphAnswersChange | null }) => void): Promise<() => void>;
  onGraphWatchStatus?(cb: (status: { refused: boolean; message: string; binding_generation?: number }) => void): Promise<() => void>;
  onGraphRescanComplete?(cb: (sequence: number) => void): Promise<() => void>;
  /** `rebuild` (Settings only) ignores every stamp and re-parses every file. */
  rescanGraphNow?(rebuild?: boolean): Promise<number>;
  /** Subscribe to graph assets changed by an outside actor (editor, Syncthing,
   *  another window): cache observation only, never page or config state. */
  onAssetChanged(cb: (batch: AssetChangedBatch) => void): Promise<() => void>;
  /** Subscribe to effective config.edn changes for this window. The event
   * carries a fresh graph meta snapshot after the store reloaded the file. */
  onGraphConfigChanged(cb: (change: GraphConfigChange) => void): Promise<() => void>;
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
  /** Restore a snapshot (graph text at original paths, config, and sidecars; snapshots current
   *  state first). Destructive — confirm before calling. */
  restoreBackup(stamp: string, kind: "replace-page"): Promise<void>;
  /** Load the persisted UI session JSON (open tabs / active tab / zoom), or null.
   *  Stored atomically in a backend file so structured session state is independent
   *  of a particular WebView/origin and can be shared across windows. */
  loadSession(): Promise<string | null>;
  /** Persist the UI session JSON. */
  saveSession(data: string): Promise<void>;
  /** This graph's crash-surviving draft records (og ADR 0061). A corrupt store
   *  loads empty; absent where drafts cannot be kept (published export). */
  loadDrafts?(): Promise<DraftRecord[]>;
  /** Replace one draft record; refused past the store's bound. `graphRoot`
   *  names another graph's store (a graph switch keeping the old graph's edit). */
  storeDraft?(record: DraftRecord, graphRoot?: string): Promise<void>;
  /** Remove one draft record by id; a missing id is not an error. */
  retireDraft?(id: string): Promise<void>;
  /** Load the current graph's device-local named-workspace registry JSON. */
  loadWorkspaces(): Promise<string>;
  /** Replace the registry atomically. A failed post-rename directory sync reports
   * a visible, unsynced publication. */
  saveWorkspaces(data: string): Promise<"durable" | "published-unsynced">;
  /** What the backend knows about the rendering path, for the CPU-rendering
   *  warning (see `gpu.ts`). A silent driver fallback is detected in the webview
   *  (WebGL renderer); this just supplies why/where context for the message. */
  gpuEnv(): Promise<GpuEnv>;
  /** The fallback app-data folder iff this launch had to relocate an unwritable
   *  one (desktop Linux), delivered once; `null` otherwise. */
  takeDataHomeFallbackNotice(): Promise<string | null>;
  /** Experimental smooth-scrolling preference (Lenis), app-level, default off. */
  getSmoothScroll(): Promise<boolean>;
  setSmoothScroll(value: boolean): Promise<void>;
  /** Generic device-local boolean preference (tine-settings.json); caller supplies
   *  the key + default. Used by the copy-behavior options. */
  getAppBool(key: string, fallback: boolean): Promise<boolean>;
  setAppBool(key: string, value: boolean): Promise<void>;
  /** GH #623: should the Windows Defender hint show for the open graph? Always
   *  `{show:false}` off Windows. Per-graph dismissal lives in the backend. */
  defenderHint(): Promise<{ show: boolean }>;
  dismissDefenderHint(): Promise<void>;
  /** Runs the UAC-elevated `Add-MpPreference` for the open graph's folder; only
   *  ever called from the user's click on the hint. */
  addDefenderExclusion(): Promise<
    { outcome: "added" } | { outcome: "declined" } | { outcome: "failed"; code: number | null; message: string }
  >;
  /** Generic device-local STRING preference (tine-settings.json); caller supplies
   *  the key + default. Used by the asset-filename format template. */
  getAppString(key: string, fallback: string): Promise<string>;
  setAppString(key: string, value: string): Promise<void>;
  /** Push the spellcheck prefs onto the native webview(s) live (no restart):
   *  `enabled` toggles WebKitGTK's checker; `languages` (locale codes, empty ⇒ OS
   *  locale) sets the dictionaries checked simultaneously. */
  applySpellcheck(enabled: boolean, languages: string[]): Promise<void>;
  /** Locale codes of the spell-check dictionaries installed on this machine, so
   *  the UI can offer them instead of asking the user to type codes. */
  listSpellcheckDictionaries(): Promise<string[]>;
  /** Startup debug logging (TINE_DEBUG=1 / --debug): whether it's on and where the
   *  log file is, so the UI can forward errors + show the path. */
  debugInfo(): Promise<DebugInfo>;
  /** Forward a frontend milestone / error into the backend debug log. */
  debugLog(line: string): Promise<void>;
  /** The privacy-safe diagnostic report of this run (GH #343): fixed-shape
   *  events only. Build commit/time that are not a hex commit and an ISO
   *  timestamp are dropped by the backend. Never contains graph content. */
  diagnosticReport(buildCommit: string, buildTime: string): Promise<DiagnosticReport>;
  /** Build the report and save it where the user picks (desktop save
   *  dialog); `false` when cancelled. Mobile rejects: use Copy report. */
  saveDiagnosticReport(buildCommit: string, buildTime: string): Promise<boolean>;
  /** Drop every recorded diagnostic event of this run and the previous one. */
  clearDiagnostics(): Promise<void>;
  /** Exact-byte manifest of the open graph's Markdown/Org files; rejects with `{ kind: "cancelled" }` after a cancel. */
  createGraphVerification(operationId: string): Promise<GraphVerificationReport>;
  cancelGraphVerification(operationId: string): Promise<void>;
  /** Save a report where the user picks (desktop); `false` when cancelled. */
  saveGraphVerificationReport(text: string): Promise<boolean>;
  onGraphVerificationProgress(cb: (progress: GraphVerificationProgress) => void): Promise<() => void>;
  /** Mobile only (GH #426): whether the recorded session counts as live, so an
   *  OS reap of a hidden app is not reported as an unclean exit. */
  diagnosticSessionActive(active: boolean): Promise<void>;
  /** Record one fixed-kind frontend event. The backend drops the event when a
   *  token is outside its closed vocabulary; fields carry no free text. */
  diagnosticFrontendEvent(kind: DiagnosticFrontendKind, fields?: DiagnosticFrontendFields): Promise<void>;
  /** Count one duration under a registered timing name (focus phases, page-load
   *  commands; see focusTiming.ts). Numbers only; native builds only. */
  diagnosticTimingEvent?(name: string, elapsedMs: number): Promise<void>;
  /** The backend's current UTC offset and sample instant: the app's calendar
   * authority (see `appNow` in journal.ts, GH #607). */
  localClock(): Promise<{ offset_minutes: number; unix_ms: number }>;
  /** The CPU architecture of this binary (`x86`, `x86_64`, `aarch64`, …). */
  appArchitecture(): Promise<string>;
  /** The last 64 external-change latency receipts, oldest first: counts and
   *  milliseconds only, no path. O(64). Backs the devtools helper
   *  `window.__tineWatcherLatency()` (GH #337). */
  watcherLatencyRecent(): Promise<unknown[]>;
}

export type { DebugInfo, DiagnosticReport, DiagnosticFrontendKind, DiscardReason, DiagnosticFrontendFields, GpuEnv, BackupInfo, GraphChange, AssetChangedBatch, GraphConfigChange, GraphSourceFile, GraphFolderPickResult, ClipboardAssetFile, ClipboardFileList, MediaCaptureResult, KnownGraph, InstalledPluginRecord, PluginRegistryCacheEnvelope, PluginRegistryCacheLoad, LoadGraphResult, CaptureGraphBindingResult, GraphAccessInspection } from "./backendTypes";
import type { DebugInfo, DiagnosticReport, DiagnosticFrontendKind, DiagnosticFrontendFields, GpuEnv, BackupInfo, GraphChange, AssetChangedBatch, GraphConfigChange } from "./backendTypes";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** `query_print` refused this IR in the requested dialect. `isNotApplicable` is
 *  the expected "OG cannot say this" answer the save path turns into
 *  `{{tine-query}}`; every other refusal is surfaced, never swallowed (I-9). */
export class QueryPrintRefusedError extends Error {
  constructor(readonly reasonCode: string, readonly diagnostic: Diagnostic | null) {
    super(diagnostic?.message ?? `The query could not be printed (reason code: ${reasonCode}).`);
    this.name = "QueryPrintRefusedError";
  }
  get isNotApplicable(): boolean {
    return this.reasonCode === "not_applicable";
  }
}

/** Decode og's `query-print-refused:<reason>:<diagnostic JSON>` envelope. */
export function queryPrintRefusal(error: unknown): QueryPrintRefusedError | null {
  const text = error instanceof Error ? error.message : String(error);
  const match = /^query-print-refused:([a-z_]+):([\s\S]*)$/.exec(text);
  if (!match) return null;
  let diagnostic: Diagnostic | null = null;
  try {
    const value = JSON.parse(match[2]) as Record<string, unknown> | null;
    if (value && typeof value["message"] === "string" && typeof value["kind"] === "string") diagnostic = value as unknown as Diagnostic;
  } catch {
    diagnostic = null; // a malformed detail still refuses; the reason code carries it
  }
  return new QueryPrintRefusedError(match[1], diagnostic);
}

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

  constructor() {
    this.ready = import("@tauri-apps/api/core").then((m) => {
      this.invoke = m.invoke;
      this.convertFileSrc = m.convertFileSrc;
    });
  }

  graphBindingGeneration() { return this.bindingGeneration; }

  private async call<T>(cmd: string, args?: Record<string, unknown>, bindingGeneration = this.bindingGeneration): Promise<T> {
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
  setSystemBarAppearance(dark: boolean) {
    return this.call<void>("set_system_bar_appearance", { dark });
  }
  quit() {
    return this.call<void>("tine_quit");
  }
  closeGraphWindow() {
    return this.call<void>("close_graph_window");
  }
  openDevtools() {
    return this.call<void>("tine_open_devtools");
  }
  defaultGraphParent() {
    return this.call<string>("default_graph_parent");
  }
  createGraph(dir: string) {
    return this.call<string>("create_graph", { dir });
  }
  pageInventory() {
    return this.call<import("./types").PageInventory>("page_inventory");
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
  graphSourceFiles(includeJournals: boolean) {
    return this.call<GraphSourceFile[]>("graph_source_files", { includeJournals });
  }
  savePages(entries: SavePageEntry[], bindingGeneration = this.bindingGeneration) {
    return this.call<SavePagesResult>("save_pages", { entries }, bindingGeneration);
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
  deletePage(name: string, kind: "journal" | "page", expectedPath?: string) {
    return this.call<void>("delete_page", { name, kind, expectedPath });
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
  openExternal(url: string) {
    return this.call<void>("open_external", { url });
  }
  openAsset(name: string, bindingGeneration: number) {
    return this.assetCall<void>("open_asset", { name }, bindingGeneration);
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
    return this.assetCall<void>("trash_asset", { name }, bindingGeneration);
  }
  search(query: string, limit: number, lane?: string) {
    return this.call<RefGroup[]>("search", { query, limit, lane });
  }
  async runGraphSearch(source: string, pageLimit: number, blockLimit: number, lane = "graph-search", explain = false, scope?: QueryPageScope, pageMatchScope?: import("./editor/queryIr").FriendlyPageMatchScope, views?: { page: ViewSettings; block: ViewSettings }) {
    const execution = await this.call<QueryExecution>("run_graph_search", { source, pageLimit, blockLimit, lane, explain, scope: scope ?? null, pageMatchScope: pageMatchScope ?? null, pageView: views?.page ?? null, blockView: views?.block ?? null });
    return {
      ...execution,
      has_more: execution.has_more ?? { pages: false, blocks: false },
    };
  }
  quickSwitch(query: string, limit: number) {
    return this.call<PageEntry[]>("quick_switch", { query, limit });
  }
  captureQuickSwitch(query: string, limit: number) {
    return this.call<PageEntry[]>("capture_quick_switch", { query, limit });
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
  async readAsset(name: string, maxBytes?: number) {
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
      const { readImage } = await import("@tauri-apps/plugin-clipboard-manager");
      const img = await readImage();
      return await clipboardImageToPng(img);
    } catch {
      return null; // no image in clipboard, or plugin unavailable
    }
  }
  async pasteImage(bindingGeneration: number): Promise<string | null> {
    const bytes = await this.readClipboardImage();
    if (!bytes) return null;
    return await this.saveAsset(assetFileName(), bytes, bindingGeneration);
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
  resolveLiveConflict(path: string, page: PageDto, baseRev: string | null, conflictRev: string,
    mergeBaseRev: string | undefined, decisions: Record<string, MergeDecision>, preChoice: "mine" | "theirs" | "union") {
    return this.call<PageDto>("resolve_live_conflict", { path, page, baseRev, conflictRev, mergeBaseRev, decisions, preChoice });
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
    return ask(message, { title: title ?? "Tine", kind: "warning" });
  }
  async pickFolder(title?: string): Promise<string | null> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const res = await open({ directory: true, multiple: false, title: title ?? "Open graph folder" });
    return typeof res === "string" ? res : null;
  }
  pickGraphFolder(): Promise<GraphFolderPickResult> {
    return this.call<GraphFolderPickResult>("pick_graph_folder");
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
    const res = await open({ multiple: false, title: "Choose a file" });
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
  copyImageToClipboard(bytes: Uint8Array): Promise<void> {
    if (bytes.byteLength > ASSET_INGRESS_MAX_BYTES) return Promise.reject(new Error("image exceeds 64 MiB clipboard limit"));
    return this.call<void>("copy_image_to_clipboard", { bytesB64: bytesToBase64(bytes) });
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
  writePdfViewState(pdf: string, page: number, scale: number, bindingGeneration: number) {
    return this.assetCall<void>("write_pdf_view_state", { pdf, page, scale }, bindingGeneration);
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
  loadDrafts() {
    return this.call<DraftRecord[]>("load_drafts");
  }
  storeDraft(record: DraftRecord, graphRoot?: string) {
    return this.call<void>("store_draft", graphRoot === undefined ? { record } : { record, graphRoot });
  }
  retireDraft(id: string) {
    return this.call<void>("retire_draft", { id });
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
