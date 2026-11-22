/** Shared read-only backend for folder exports and generation-2 quick shares.
 * Graph reads use the native wire adapters over one memory graph. The browser
 * owns publication identity, session state, assets and handoff policy.
 */
import type { Backend, LoadGraphResult } from "./backend";
import type { ConfirmOptions } from "./backendInterface";
import { loadBrowserCore } from "./browserCore";
import { setQuickSharePrefix } from "./publishedPermalink";
import type { QuickShareListing } from "./quickShare/commands";
import { checkBrowserHandoff } from "./linkSchemePolicy";
import type { BrowserGraph } from "./render/wasm-core/lsdoc_wasm";
import type { PageRead, PageReadSource, RefGroup } from "./types";

/** `<meta name="tine-published" content="snapshot.json">` in the exported shell. */
export const PUBLISHED_META_NAME = "tine-published";
export const PUBLISHED_SNAPSHOT_SCHEMA = 2;
export const QUICK_SHARE_SNAPSHOT = "quick-share:2";
export const PUBLISHED_READ_ONLY_MESSAGE = "This is a read-only published export.";

export interface PublishedSnapshot {
  schema: 2;
  name: string;
  exported_at: string;
  files: { path: string; text: string }[];
}

/** The snapshot's relative URL when this document is a published export, else null. */
export function publishedSnapshotUrl(doc: Document | undefined = globalThis.document): string | null {
  const meta = doc?.querySelector(`meta[name="${PUBLISHED_META_NAME}"]`);
  const content = meta?.getAttribute("content")?.trim();
  return content ? content : null;
}

/** Whether this document is a published export (the one presentation flag). */
export function isPublishedExport(): boolean {
  return publishedSnapshotUrl() !== null;
}

/** Share mode has a stricter media policy than an author's own published site. */
export function isQuickShareViewer(): boolean {
  return publishedSnapshotUrl() === QUICK_SHARE_SNAPSHOT;
}

let publication: ReturnType<typeof openPublishedGraph> | undefined;

/** Largest snapshot document a viewer will read (I-22). Far above any real export;
 *  a larger body is a wrong or hostile file and refuses visibly. */
export const PUBLISHED_SNAPSHOT_MAX_BYTES = 512 * 1024 * 1024;

/** Read a response body of at most `max` bytes: a declared length over the cap refuses
 *  before any byte is read, and a body that outgrows it (no or wrong length) is cancelled
 *  as soon as the running total passes the cap. O(max) memory at worst. */
export async function readBounded(response: Response, max: number, label: string): Promise<Uint8Array> {
  const tooLarge = () => new Error(`${label}: larger than ${max} bytes`);
  const declared = Number(response.headers?.get("content-length") ?? "");
  if (Number.isFinite(declared) && declared > max) throw tooLarge();
  const reader = response.body?.getReader();
  if (!reader) {
    const whole = new Uint8Array(await response.arrayBuffer());
    if (whole.byteLength > max) throw tooLarge();
    return whole;
  }
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > max) {
      // The oversize refusal is the answer; a failed cancel is attached, not dropped.
      try {
        await reader.cancel();
      } catch (cancelError) {
        throw new Error(tooLarge().message, { cause: cancelError });
      }
      throw tooLarge();
    }
    chunks.push(value);
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) {
    out.set(chunk, at);
    at += chunk.byteLength;
  }
  return out;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Admit the flat source snapshot. The core checks graph paths and reads the files. */
export function validateSnapshot(value: unknown): PublishedSnapshot {
  if (!record(value) || value.schema !== PUBLISHED_SNAPSHOT_SCHEMA) {
    throw new Error(`Expected snapshot schema ${PUBLISHED_SNAPSHOT_SCHEMA}`);
  }
  if (typeof value.name !== "string" || typeof value.exported_at !== "string" || !Array.isArray(value.files)
    || !value.files.every((file) => record(file) && typeof file.path === "string" && typeof file.text === "string")) {
    throw new Error("Invalid publication snapshot");
  }
  return { schema: value.schema, name: value.name, exported_at: value.exported_at, files: value.files };
}

async function fetchPublishedSnapshot(url: string): Promise<PublishedSnapshot> {
  if (url === QUICK_SHARE_SNAPSHOT) {
    const { loadQuickShare } = await import("./quickShare/quickShare");
    const share = await loadQuickShare(globalThis.location.pathname, globalThis.location.hash);
    setQuickSharePrefix(share.link.prefix);
    return share.snapshot;
  }
  const response = await fetch(url, { cache: "no-store" });
  if (!response.ok) throw new Error(`snapshot ${url}: HTTP ${response.status}`);
  const bytes = await readBounded(response, PUBLISHED_SNAPSHOT_MAX_BYTES, `snapshot ${url}`);
  return validateSnapshot(JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)));
}

async function openPublishedGraph(url: string) {
  const [{ name, files }, core] = await Promise.all([fetchPublishedSnapshot(url), loadBrowserCore()]);
  const paths: string[] = [];
  const contents: Uint8Array[] = [];
  const encoder = new TextEncoder();
  for (const file of files) {
    paths.push(file.path);
    contents.push(encoder.encode(file.text));
  }
  return { name, graph: new core.BrowserGraph(paths, contents) };
}

/** Open one memory graph per publication. Startup and backend readers share it.
 * The source JSON and transfer buffers are released once the graph opens.
 */
export function loadPublishedGraph(url = publishedSnapshotUrl() ?? "snapshot.json") {
  return (publication ??= openPublishedGraph(url));
}

// ---- the backend --------------------------------------------------------------

/** Methods declared optional on `Backend`; a published export leaves them
 *  absent so the browser fallbacks in `backend.ts` take over. */
export const OPTIONAL_METHODS = new Set([
  "pdfAssetSource", // Native asset IPC only; published PDFs use the browser byte loader.
  "readExternalPdf", // Outside files require an explicit desktop host link open.
  "ownerProtectionStatus",
  "retryOwnerProtection",
  // Device-local graph relocation/recovery has no namespace in a published site.
  "orphanDrafts",
  "recoverOrphanDrafts",
  "deleteOldBackups",
  "loadDrafts",
  "storeDraft",
  "retireDraft",
  // GH #181: external Tine links need the native URL handler; a published guide has none.
  "tineLinks",
  // CLI slice 2: only a native window answers its graph owner's flush requests.
  "onOwnerFlushRequest",
  "onOwnerPageRenamed",
  "ownerFlushAnswer",
  // GX4 S2/S4: change subscriptions. A snapshot never changes, so readers fall
  // back to the publication key; refusing them blanked every query and board.
  "registerUnlinkedReference",
  "registerQuerySubscription",
]);

/** Build the browser publication backend. Its graph is opened once for all reads. */
export function publishedBackend(load = loadPublishedGraph): Backend {
  const appBools = new Map<string, boolean>();
  const appStrings = new Map<string, string>();
  let session: string | null = null;
  let workspaces = JSON.stringify({ version: 1, activeId: "published", workspaces: [{
    id: "published", name: "Published", blob: {
      tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0,
      leftSidebar: true, rightSidebar: false, rightSidebarItems: [],
      favoritesSectionExpanded: true, recentSectionExpanded: true,
      layout: { kind: "pane", paneId: "main", tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 },
      focusedPaneId: "main", recentPages: [],
    },
  }] });
  let notices = '{"dismissed":[]}';
  let smoothScroll = false;
  let activationSeq = 0;
  const unsubscribed = async () => () => {};

  async function read<T>(answer: (graph: BrowserGraph) => string): Promise<T> {
    return JSON.parse(answer((await load()).graph));
  }

  /** `../assets/<name>` — or null when the authored name would leave the
   *  export's own `assets/` folder. The rule is the static exporter's
   *  (`AssetSink::asset_relative` in `publish.rs`), applied to the name after
   *  its `assets/` prefix: a `?` query or `#` fragment is dropped, a remote
   *  reference, a backslash, an absolute path, a leading `.` step or a `..`
   *  step is refused, and empty and interior `.` steps collapse the way
   *  `Path::components` collapses them — so every name the export copied is
   *  answered under the path it was copied to, and nothing else is asked for.
   *  A colon inside a file name (`x:y.png`, `data:plot.png`) is a name. */
  const assetUrl = (name: string): string | null => {
    const bare = name.split(/[?#]/)[0] ?? "";
    if (bare.includes("://") || bare.includes("\\") || bare.startsWith("/")) return null;
    const steps = bare.split("/").filter((segment) => segment !== "");
    if (steps.length === 0 || steps[0] === "." || steps.includes("..")) return null;
    const segments = steps.filter((segment) => segment !== ".");
    if (segments.length === 0) return null;
    return `../assets/${segments.map(encodeURIComponent).join("/")}`;
  };

  const answered = {
    // ---- graph identity ----
    async loadGraph(): Promise<LoadGraphResult> {
      const { name, graph } = await load();
      return {
        kind: "loaded",
        binding_generation: 1,
        meta: { ...JSON.parse(graph.graph_meta_json()), root: name, display_root: name },
        config_problem: JSON.parse(graph.config_problem_json()),
      };
    },
    async inspectGraphAccess(path) {
      return { graph_root: path || (await load()).name, external_assets_path: null, approved: true };
    },
    async startupGraphPath() {
      return (await load()).name;
    },
    pageInventory: () => read(graph => graph.page_inventory_json()),
    pageModified: () => read(graph => graph.page_modified_json()),
    pageBacklinkCounts: () => read(graph => graph.page_backlink_counts_json()),
    resolvePage: (name, kind) => read(graph => graph.resolve_page_json(name, kind === "journal")),
    pageAliasGroup: name => read(graph => graph.page_alias_group_json(name)),
    getPage: (name, kind) => read(graph => graph.get_page_json(name, kind === "journal")),
    async getPageByPath(path, source) {
      const answer = await read<{ page: PageRead | null; source: PageReadSource | null }>(
        graph => graph.get_page_by_path_json(path),
      );
      source?.(answer.source);
      return answer.page;
    },
    journalFeedPage: (limit, before, waitForReferences = false, through = null) =>
      read(graph => graph.journal_feed_page_json(limit, before === null ? undefined : BigInt(before),
        waitForReferences, through === null ? undefined : BigInt(through))),
    journalContentDays: () => read(graph => graph.journal_content_days_json()),
    pageIcons: names => read(graph => graph.page_icons_json(JSON.stringify(names))),
    getBacklinks: name => read(graph => graph.get_backlinks_json(name)),
    getBacklinkFilterContext: (name, targets) =>
      read(graph => graph.get_backlink_filter_context_json(name, JSON.stringify(targets))),
    getUnlinkedRefs: name => read(graph => graph.get_unlinked_refs_json(name)),
    getBlockRefCounts: () => read(graph => graph.get_block_ref_counts_json()),
    getBlockReferrers: uuid => read(graph => graph.get_block_referrers_json(uuid)),
    async resolveBlock(uuid) {
      const [group] = await read<(RefGroup | null)[]>(graph => graph.resolve_blocks_json(JSON.stringify([uuid])));
      return group;
    },
    resolveBlocks: uuids => read(graph => graph.resolve_blocks_json(JSON.stringify(uuids))),
    previewBlock: (uuid, maxNodes) => read(graph => graph.preview_block_json(uuid, maxNodes)),
    search: (query, limit) => read(graph => graph.search_json(query, limit)),
    quickSwitch: (query, limit, _editor, transliteration = false) =>
      read(graph => graph.capture_quick_switch_json(query, limit, transliteration)),
    captureQuickSwitch: (query, limit, transliteration = false) =>
      read(graph => graph.capture_quick_switch_json(query, limit, transliteration)),
    queryFacets: scope => read(graph => graph.query_facets_json(JSON.stringify(scope))),
    parseQuery: (text, dialect, properties) =>
      read(graph => graph.parse_query_json(text, dialect, JSON.stringify(properties))),
    queryRun: (query, view, context) =>
      read(graph => graph.query_run_json(JSON.stringify(query), JSON.stringify(view), JSON.stringify(context))),
    runGraphSearch: (source, pageLimit, blockLimit, _lane, explain, scope, pageMatchScope, views, transliteration) =>
      read(graph => graph.run_graph_search_json(source, pageLimit, blockLimit, JSON.stringify({
        explain, scope, pageMatchScope, pageView: views?.page, blockView: views?.block, transliteration,
      }))),
    // ---- assets and the browser ----
    // A refused or missing asset rejects (I-9): an empty byte array reads as a valid empty
    // file and blanks a PDF or image with no cause. The live backend rejects the same way.
    async readAsset(name: string, maxBytes: number) {
      await load();
      const url = assetUrl(name);
      if (!url) throw new Error(`asset ${name}: not a file inside the published assets`);
      const response = await fetch(url);
      if (!response.ok) throw new Error(`asset ${name}: HTTP ${response.status}`);
      return readBounded(response, maxBytes, `asset ${name}`);
    },
    async streamAsset(name: string) {
      await load();
      const url = assetUrl(name);
      if (!url) throw new Error(`asset ${name}: not a file inside the published assets`);
      return url;
    },
    async openExternal(url: string, confirmedScheme?: string) {
      checkBrowserHandoff(url, confirmedScheme, window.document?.baseURI ?? window.location.href);
      window.open(url, "_blank", "noopener");
    },
    async openAsset(name: string) {
      const url = assetUrl(name);
      if (url) await answered.openExternal(url);
    },
    async writeText(text: string) {
      const { writeClipboardTextStrict } = await import("./clipboard");
      await writeClipboardTextStrict(text);
    },
    async writeRich(text: string) {
      const { writeClipboardTextStrict } = await import("./clipboard");
      await writeClipboardTextStrict(text);
    },
    async confirm(message: string, _options: ConfirmOptions) {
      return window.confirm(message);
    },
  } satisfies Partial<Backend>;

  const constant = {
    graphBindingGeneration: () => 1,
    async appPlatform(): Promise<"android" | "ios" | "desktop"> {
      return "desktop";
    },
    async listKnownGraphs() {
      return [];
    },
    async gpuEnv() {
      return { software_forced: false, appimage: false, flatpak: false, packaged: false, portable: false };
    },
    // A published export has no backend clock; the browser's is the authority.
    async localClock() {
      const now = Date.now();
      return { offset_minutes: -new Date(now).getTimezoneOffset(), unix_ms: now };
    },
    async appArchitecture() {
      return "unknown";
    },
    async watcherLatencyRecent() {
      return [];
    },
    async debugInfo() {
      return { enabled: false, path: "", recorderActive: false, previousExitUnclean: false };
    },
    async debugLog() {},
    async diagnosticFrontendEvent() {},
    async diagnosticTimingEvent() {},
    async diagnosticSessionActive() {},
    async getAppBool(key: string, fallback: boolean) {
      return appBools.get(key) ?? fallback;
    },
    async setAppBool(key: string, value: boolean) {
      appBools.set(key, value);
    },
    async getAppString(key: string, fallback: string) {
      return appStrings.get(key) ?? fallback;
    },
    async setAppString(key: string, value: string | null) {
      if (value === null) appStrings.delete(key);
      else appStrings.set(key, value);
    },
    async setUiLocale() {},
    async loadSession() {
      return session;
    },
    async saveSession(data: string) {
      session = data;
    },
    async loadWorkspaces() {
      return workspaces;
    },
    async saveWorkspaces(data: string) {
      workspaces = data;
      return "durable" as const;
    },
    async loadNotices() {
      return notices;
    },
    async saveNotices(data: string) {
      notices = data;
    },
    async readCustomCss() {
      return "";
    },
    async getSmoothScroll() {
      return smoothScroll;
    },
    async setSmoothScroll(value: boolean) {
      smoothScroll = value;
    },
    async warmDone() {
      return true;
    },
    async indexingProgress() {
      return null;
    },
    async listInstalledPlugins() {
      return [];
    },
    async loadPluginRegistryCache() {
      return { kind: "absent" as const };
    },
    async readLocalImage() {
      return new Uint8Array();
    },
    async listTemplates(_name?: string) {
      return [];
    },
    async listTemplateNames() { return []; },
    async guidePages() {
      return [];
    },
    async activateEditor(path: string) {
      return { activation: ++activationSeq, target: path, prospective: false };
    },
    async activateAbsentEditor() {
      return { activation: ++activationSeq, target: "", prospective: true };
    },
    async retireEditorActivation() {
      return true;
    },
    async presentConflictOverride(): Promise<"authorised" | "superseded" | "withdrawn"> {
      return "withdrawn";
    },
    async openPdf() {
      return { highlights: [], page: null, scale: null };
    },
    async readHighlights() {
      return [];
    },
    async queryRegistry() {
      return { rows: [], generation: 0 };
    },
    async queryOgExpressible() {
      return false;
    },
    async printQuery(): Promise<never> {
      const { QueryPrintRefusedError } = await import("./backend");
      throw new QueryPrintRefusedError("not_applicable", null);
    },
    async takeIdentifierMigrationNotice() {
      return false;
    },
    async takeDataHomeFallbackNotice() {
      return null;
    },
    async gitAutocommitStatus() {
      return { available: false };
    },
    async gitSyncNotice() {
      return null;
    },
    async gitAppLifecycle() {},
    async getPageChanges() {
      return { available: false, reason: "Git is not available in a published graph.", head_commit: null,
        baseline_commit: null, current_text_sha256: "", identity_known: false, unattributed: false, blocks: [], removed: [] };
    },
    async getPageLastChanged() { return []; },
    async getBackupKeep() {
      return 0;
    },
    async listBackups() {
      return [];
    },
    async getCaptureEnterFiles() {
      return true;
    },
    async getLinkFirstMatch() {
      return false;
    },
    async getWatchMode() {
      return "inotify";
    },
    async listSpellcheckDictionaries() {
      return [];
    },
    async applySpellcheck() {},
    async setSystemBarAppearance() {},
    async closeGraphWindow() {},
    async trayApply() {
      return { supported: false, active: false, problem: null };
    },
    async openDevtools() {},
    async captureTarget() {
      return "main";
    },
    async bindCaptureGraph() {
      const result = await answered.loadGraph();
      if (result.kind === "focused_existing") throw new Error("published graph unavailable");
      return result.meta;
    },
    async captureReceipt() { return "pending" as const; },
    async defaultGraphParent() {
      return "";
    },
    async conflictInventory() {
      return { sync_conflicts: [], vcs_markers: [], queue: [] };
    },
    async listJournalConflicts() {
      return [];
    },
    async listJournalFilenameMigrations() {
      return [];
    },
    async listOrphanAssets() {
      return [];
    },
    async assetTrashStats() {
      return { count: 0, bytes: 0, pages: 0, journals: 0, conflicts: 0, other: 0 };
    },
    async clipboardFiles() {
      return { files: [], skipped: 0, truncated: false };
    },
    async detectMediaEditor() {
      return "";
    },
    async readClipboardImage() {
      return null;
    },
    async diagnosticReport() {
      return { text: "{}", suggestedFileName: "tine-diagnostics-export.json" };
    },
    async saveDiagnosticReport() {
      return false;
    },
    async clearDiagnostics() {},
    // Windows Defender is the native app's concern; a published export never shows the hint.
    async defenderHint() {
      return { show: false };
    },
    async dismissDefenderHint() {},
    onStorageTransition: unsubscribed,
    onGraphRescanComplete: unsubscribed,
    onConflictsChanged: unsubscribed,
    onGraphChanged: unsubscribed,
    onGraphChangedBulk: unsubscribed,
    onAssetChanged: unsubscribed,
    onGraphConfigChanged: unsubscribed,
    onCustomCssChanged: unsubscribed,
    onGraphReopened: unsubscribed,
    onQueryProjectionChanged: unsubscribed,
    onGraphWatchStatus: unsubscribed,
    onGraphUnreadablePages: unsubscribed,
    onGraphVerificationProgress: unsubscribed,
    // A read-only export binds a page host that never holds a page: every page
    // command is refused, so there is no mail, no debt and no draft file.
    async pageWindowReloaded() {
      return { session: 1, nextId: 1 };
    },
    async pageOwed() {
      return [];
    },
    onPageMail: unsubscribed,
    onGraphOpenWaiting: unsubscribed,
    onGraphOpenOwnerWaiting: unsubscribed,
    async legacyDraftsFile() {
      return null;
    },
    // Quick share is a native feature (DESIGN §3.9): a published export refuses the listing, so links are not offered.
    async quickShareList(): Promise<QuickShareListing> {
      throw { kind: "disabled" };
    },
  };

  const explicit: Record<string, unknown> = { ...answered, ...constant };
  return new Proxy(explicit, {
    get(target, property) {
      if (typeof property !== "string") return undefined;
      if (property in target) return target[property];
      if (OPTIONAL_METHODS.has(property)) return undefined;
      if (property === "then") return undefined;
      return async () => {
        throw new Error(PUBLISHED_READ_ONLY_MESSAGE);
      };
    },
    has(target, property) {
      return typeof property === "string" && !OPTIONAL_METHODS.has(property) ? true : property in target;
    },
  }) as unknown as Backend;
}
