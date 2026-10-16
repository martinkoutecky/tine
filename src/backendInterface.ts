/** Backend API shared by native, mock and published implementations.
 * Re-exported by backend.ts as the public entry point. */
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
import type { GraphFolderPickResult, PreparedGraphFolder, ClipboardFileList, MediaCaptureResult, KnownGraph, InstalledPluginRecord, PluginRegistryCacheLoad, LoadGraphResult, GraphAccessInspection, TrayStatus } from "./backendTypes";
import type { EditKinds } from "./editKind";
import type { SheetExport, SheetInput, SheetScope } from "./sheet/staticExport";
import type { NativeTineLinks } from "./nativeTineLinks";
import type { DebugInfo, DiagnosticReport, DiagnosticFrontendKind, DiagnosticFrontendFields, GpuEnv, BackupInfo, GraphChange, AssetChangedBatch, GraphConfigChange, CustomCssChange, TrashAssetOutcome, GraphAnswersChange } from "./backendTypes";
import type { DiskToken, OwedPage, PageMail, PageOperation, PageRefusal, PublishedNeed } from "./document";

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
  /** Host-internal Android bar background (opaque #rrggbb) and icon contrast.
   * Missing/invalid background uses the mode fallback. Other platforms no-op. */
  setSystemBarAppearance(dark: boolean, background?: string): Promise<void>;
  defaultGraphParent(): Promise<string>;
  closeGraphWindow(): Promise<void>;
  /** Desktop tray (GH #625, src/tray.ts): sync the icon to the `tray_*` device settings. */
  trayApply(): Promise<TrayStatus>;
  /** Toggle the WebView developer tools (WebKit Web Inspector) for theme/CSS
   *  debugging. No-op on a build without devtools compiled in. */
  openDevtools(): Promise<void>;
  /** Scaffold a brand-new demo graph (onboarding "create new graph"); returns
   *  the created graph's root path to then `loadGraph`. Creates the graph in
   *  the named child of `dir`, reusing only an empty directory. */
  createGraph(dir: string, name: string): Promise<string | { root: string; display_root: string }>;
  /** First unused notes, notes-2, … child name, supplied by Rust. */
  suggestGraphName(dir: string): Promise<string>;
  /** The whole name inventory (physical pages/journals, aliases, reference-only
   *  names), each with the backend's resolved target. Cached only by
   *  `pageIndex.ts` holds the full inventory; `store.ts` separately indexes
   *  loaded pages in its working set. */
  pageInventory(): Promise<import("./types").PageInventory>;
  /** Last-modified time of every page file, read once when the All pages view
   *  opens: times the store already holds (no stat, nothing persisted). */
  pageModified(): Promise<import("./types").PageModified>;
  pageBacklinkCounts(): Promise<[string, number][]>;
  journalFeedPage(limit: number, beforeDay: number | null): Promise<import("./types").JournalFeedPage>;
  /** Journal date-keys (yyyymmdd) whose page has real content. */
  journalContentDays(): Promise<number[]>;
  getPage(name: string, kind: "journal" | "page"): Promise<import("./types").PageRead | null>;
  resolvePage(name: string, kind: "journal" | "page"): Promise<import("./types").ResolvedPage>;
  /** The page host (STEP3-DESIGN §3; `src-tauri/src/page_commands.rs`): the only
   * page persistence of a window. Only `src/document/host/wiring.ts` calls these
   * (I-1). Mutating commands carry the window's session and a request id and go
   * through the ordered lane (src/orderedWrites.ts); a resolved refusal means
   * "not admitted". Answers and pushes arrive as `onPageMail`. The mock is a
   * single-window in-memory host that takes every request. */
  pageWindowReloaded(): Promise<{ session: number; nextId: number }>;
  /** A null `path` (no file yet) is resolved with `resolve_page` inside the
   * ordered lane, so the open keeps its place among the window's requests; an
   * alias name is refused client side (`alias`) without a command. */
  pageOpen(session: number, id: number, page: { name: string; kind: "journal" | "page"; path: string | null }):
    Promise<{ key: string; baselineEntry: boolean } | PageRefusal>;
  pageSubmit(session: number, id: number, key: string, dto: PageDto, version: number, resolve: DiskToken | null,
    kinds: EditKinds): Promise<PageRefusal | null>;
  pageMove(session: number, id: number, source: [string, PageDto, number], receiver: [string, PageDto, number],
    kinds: EditKinds): Promise<PageRefusal | null>;
  pageDiscard(session: number, id: number, key: string): Promise<PageRefusal | null>;
  pageClose(session: number, id: number, key: string): Promise<PageRefusal | null>;
  /** Delete page `name` through the host. The backend checks its identity
   * (twins, a stale `expectedPath`, an external removal) and binds the host's
   * deletion to the bytes it read (GH #620, Q-P2b-5); a failed check rejects. */
  pageDelete(session: number, name: string, kind: PageKind, expectedPath?: string): Promise<PageOperation>;
  /** One bounded wait (≤ 5 s) for these needs to publish; null at the bound. */
  pageWait(session: number, needs: readonly PublishedNeed[], boundMs: number): Promise<boolean | null>;
  pageSaveNow(session: number, keys: readonly string[]): Promise<void>;
  pageOwed(session: number, paths: readonly string[] | null): Promise<OwedPage[] | null>;
  /** Retry crash-draft storage after the host reported it unavailable. */
  pageDraftsRetry(): Promise<void>;
  onPageMail(cb: (mail: PageMail) => void): Promise<() => void>;
  /** The graph being opened overlaps one that is still saving its pages (F6). */
  onGraphOpenWaiting(cb: (saving: string) => void): Promise<() => void>;
  /** The graph being opened waits for the command line's owner (`tine serve`)
   * to finish and hand it over; each call carries serve's changed progress. */
  onGraphOpenOwnerWaiting(cb: (progress: string) => void): Promise<() => void>;
  /** The v1 crash-draft file of this graph when one exists (D-1: left untouched
   * as a backup); read-only, never its content. */
  legacyDraftsFile(): Promise<string | null>;
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
  /** Open an http(s)/mailto/`file:` URL in the OS default app. Any other
   *  scheme (GH #503) opens only when `confirmedScheme` names it: pass it only
   *  after the user confirmed a prompt naming the scheme and the full target
   *  (`openOutboundLink`); the native side refuses it otherwise. */
  openExternal(url: string, confirmedScheme?: string): Promise<void>;
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
  /** Open an existing graph-relative file or folder (a resolved relative link,
   * GH #457) in the desktop OS default app. A path that leaves the graph folder,
   * including through a symlink, a missing target, and mobile reject. Read-only
   * OS handoff. Cost O(path components). */
  openGraphFile(path: string, bindingGeneration: number): Promise<void>;
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
  /** Move one top-level asset to the recoverable trash unless the published
   * graph still references it: that resolves `"referenced"` with the file kept
   * (GH #623; the caller reports it, never matching error text). Reads the whole
   * file per attempt and retries revision conflicts up to four times. Missing
   * assets and exhausted conflicts reject. Cost O(file bytes + graph references)
   * per attempt. */
  trashAsset(name: string, bindingGeneration: number): Promise<TrashAssetOutcome>;
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
  /** Merge a reviewed live conflict without writing (read-only): the page the
   *  window then submits with `resolve` = the reviewed disk token (S7).
   *  "conflict" when the disk or the reviewed ledger base moved. */
  mergeLiveConflict(path: string, page: PageDto, conflictRev: string, mergeBaseRev: string | undefined,
    decisions: Record<string, MergeDecision>, preChoice: "mine" | "theirs" | "union"): Promise<PageDto>;
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
    views?: { page: ViewSettings; block: ViewSettings },
    transliteration?: boolean
  ): Promise<QueryExecution>;
  quickSwitch(query: string, limit: number, editor?: { pageName: string; blockId: string; originalRaw: string; session?: string }, transliteration?: boolean): Promise<PageEntry[]>;
  /** Capture-only page/tag completion capability. It is intentionally not the
   * general graph command route used by the main WebView. */
  captureQuickSwitch(query: string, limit: number, transliteration?: boolean): Promise<PageEntry[]>;
  listTemplates(): Promise<TemplateDto[]>;
  resolveBlock(uuid: string): Promise<RefGroup | null>;
  resolveBlocks(uuids: string[]): Promise<(RefGroup | null)[]>;
  /** Explicit bounded subtree; ordinary resolution is intentionally shallow. */
  previewBlock(uuid: string, maxNodes: number): Promise<BlockPreview | null>;
  readAsset(name: string, maxBytes: number): Promise<Uint8Array>;
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
  pickFolder(title?: string, defaultPath?: string): Promise<string | null>;
  revealExport(path: string): Promise<void>;
  readCssFile(global: boolean): Promise<{ text: string; revision: string | null }>;
  saveCssFile(global: boolean, text: string, revision: string | null): Promise<string>;
  /** Android native graph-folder picker. Returns a real filesystem path when
   *  picked; never a content URI. */
  pickGraphFolder(): Promise<GraphFolderPickResult>;
  /** iOS only: make an app-container (local/iCloud) graph readable before Rust enumerates it. */
  prepareGraphFolder(path: string): Promise<PreparedGraphFolder>;
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
  copyAssetFileToClipboard(name: string): Promise<boolean>;
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
  /** An outside edit of `logseq/custom.css` (editor, Syncthing, another
   *  window): re-read and re-apply it without reopening the graph. Own writes
   *  never announce. */
  onCustomCssChanged(cb: (change: CustomCssChange) => void): Promise<() => void>;
  /** Settings › Theme "Edit custom.css" (desktop): create `logseq/custom.css`
   *  through the store's audited create when missing, never rewriting an
   *  existing one, then open it in the system editor. */
  editCustomCss(): Promise<void>;
  /** How many launch snapshots to keep. */
  getBackupKeep(): Promise<number>;
  setBackupKeep(keep: number): Promise<void>;
  /** Quick-capture Enter behaviour: true → Enter files; false → Enter = new block. */
  getCaptureEnterFiles(): Promise<boolean>;
  setCaptureEnterFiles(value: boolean): Promise<void>;
  /** `[[`/`#` autocomplete default: true → Enter links the first match; false
   *  (default, OG) → Enter creates a new page/tag unless an exact match exists. */
  getLinkFirstMatch(): Promise<boolean>;
  /** How the file-watcher detects external edits: "inotify" (default, no idle
   *  wakeups) or "poll" (3s scan, for filesystems where inotify is flaky). */
  getWatchMode(): Promise<string>;
  setWatchMode(mode: string): Promise<void>;
  /** Available snapshots for the current graph, newest first. */
  listBackups(): Promise<BackupInfo[]>;
  /** Restore a snapshot (graph text at original paths, config, and sidecars; snapshots current
   *  state first). Destructive — confirm before calling. `consumedLastId`: the
   *  last page-host answer the window consumed. `reloaded` is the session of the
   *  host relaunched after the restore, on success and on a partial failure
   *  alike (the window rebinds either way, S8); null when the stop refused (the
   *  host keeps running) or without a host. `error` says why it failed. */
  restoreBackup(stamp: string, kind: "replace-page", consumedLastId: number):
    Promise<{ error: string | null; reloaded: { session: number; nextId: number } | null }>;
  /** Load the persisted UI session JSON (open tabs / active tab / zoom), or null.
   *  Stored atomically in a backend file so structured session state is independent
   *  of a particular WebView/origin and can be shared across windows. */
  loadSession(): Promise<string | null>;
  /** Persist the UI session JSON. */
  saveSession(data: string): Promise<void>;
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
  /** The UI locale the frontend resolved (tine#107): native shell text (the
   *  tray) follows it, since native code never negotiates or formats messages. */
  setUiLocale(locale: string): Promise<void>;
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
