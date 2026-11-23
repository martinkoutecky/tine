// The browser development backend (ADR 0081): used only outside Tauri and
// outside a published export, in the dev server, the screenshot harness and the
// unit tests. It answers no graph question itself: the browser core does, over
// the fixture files in `src/mock/` (`mockGraph.ts`), through the forward table
// the published export uses too. What it keeps is host state (assets, PDF
// highlight records, plugins, session, settings, the conflict demo); the
// store's write paths beyond one page's text are refused.
import type { GraphVerificationReport } from "./graphVerification";
import type { Backend, GpuEnv, DebugInfo, DiagnosticFrontendKind, DiagnosticReport, InstalledPluginRecord, PluginRegistryCacheEnvelope } from "./backend";
import type { ConfirmOptions } from "./backendInterface";
import { ASSET_INGRESS_MAX_BYTES } from "./clipboardImage";
import { graphIdentity, graphReads, graphStoreReads } from "./graphForwards";
import { mockConflictApi } from "./mockConflicts";
import { mockGraphFiles, openMockGraph } from "./mockGraph";
import { mockQuickShareCommands } from "./mockQuickShare";
import { mockGitCommands } from "./mockGit";
import { mockPageHost } from "./mockPageHost";
import { createMockCaptureReceipts } from "./mockCaptureReceipts";
import type { Highlight, PdfState } from "./types";
import { SAMPLE_PDF_B64 } from "./sample-pdf";
import { mockJournalFiles } from "./mockJournalFiles";
import { checkBrowserHandoff } from "./linkSchemePolicy";
import { MOCK_ABSENT_METHODS } from "./browserMock";
import { t } from "./i18n";

/** What the browser preview says for a question only the desktop app answers
 * (ADR 0081 Decision 4), in the active locale. */
export function mockRefusalMessage(): string {
  return t("backend-browser-preview-refused");
}


const mockPlugins: InstalledPluginRecord[] = [];
const mockDiagnostics: DiagnosticFrontendKind[] = [];
const mockPluginEntries = new Map<string, Uint8Array>();
let mockPluginRegistryCache: PluginRegistryCacheEnvelope | null = null;
const mockHighlights: Record<string, { label: string; highlights: Highlight[]; page?: number; scale?: number }> = {};
// In-memory UI session for the browser mock (no backend file).
let mockSession: string | null = null;
let mockWorkspaces: string | null = null;
let mockGuideAnnounced = false;
const mockAssets: Record<string, Uint8Array> = {};
const mockAppBools: Record<string, boolean> = {};
const mockAppStrings: Record<string, string> = {};

// A tiny valid silent WAV (0.2s, 8kHz/8-bit mono) so the mock audio asset actually
// renders the <audio> player — WAV is natively decodable in headless Chromium,
// unlike mp4/mp3 — letting the screenshot harness verify the audio controls + the
// widen toggle. (Real graphs hold mp3/mp4; those play in WebKitGTK, codec permitting.)
const SILENT_WAV_B64 =
  "UklGRmQGAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YUAGAACAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgIA==";

function decodeB64(b64: string): Uint8Array {
  const bin = atob(b64);
  const arr = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
  return arr;
}

export interface MockOptions {
  /** The URL search string whose flags select fixture files (default: the page's). */
  search?: string;
  /** Graph files to add or replace (null removes one) before the core opens the graph. */
  files?: Record<string, string | null>;
}

/** The browser preview backend over a fresh memory graph of the fixture files. */
export function mockBackend(options: MockOptions = {}): Backend {
  const files = mockGraphFiles(options.search ?? globalThis.location?.search ?? "");
  for (const [path, text] of Object.entries(options.files ?? {})) {
    if (text === null) files.delete(path);
    else files.set(path, text);
  }
  const graph = openMockGraph(files);
  const reads = graphReads(graph.read);
  const pageHost = mockPageHost(graph, mockRefusalMessage);

  const explicit: Partial<Backend> = {
    ...reads,
    ...graphStoreReads(graph.read),
    ...pageHost,
    async loadGraph() {
      const { meta, config_problem } = await graphIdentity(graph.read);
      return { kind: "loaded" as const, binding_generation: 1, config_problem,
        meta: { ...meta, root: "/mock/graph", guide_announced: mockGuideAnnounced } };
    },
    async listKnownGraphs() {
      return [{ path: "/mock/graph", name: "graph" }];
    },
    async inspectGraphAccess(path: string) {
      return { graph_root: path || "/mock/graph", external_assets_path: null, approved: true };
    },
    async approveExternalAssets() {},
    async openGraphWindow() {
      return { kind: "focused_existing" as const, window_label: "main" };
    },
    async startupGraphPath() {
      return "/mock/graph";
    },
    async captureTarget() {
      return "main";
    },
    captureReceipt: createMockCaptureReceipts(),
    async bindCaptureGraph() {
      const result = await explicit.loadGraph!("");
      if (result.kind === "focused_existing") throw new Error("mock graph unavailable");
      return result.meta;
    },
    async forgetKnownGraph() {},
    async revealKnownGraph() {},
    async appPlatform(): Promise<"android" | "ios" | "desktop"> {
      const requested = new URLSearchParams(globalThis.location?.search ?? "").get("platform");
      if (requested === "android" || requested === "ios") return requested;
      return "desktop";
    },
    async listInstalledPlugins() {
      return mockPlugins.map((plugin) => ({ ...plugin }));
    },
    async installPlugin(manifestJson: string, wasm: Uint8Array) {
      const manifest = JSON.parse(manifestJson) as { id: string; version: string };
      const key = `${manifest.id}@${manifest.version}`;
      mockPluginEntries.set(key, wasm.slice());
      const record: InstalledPluginRecord = {
        id: manifest.id,
        version: manifest.version,
        manifest_json: manifestJson,
        sha256: "mock",
        selected: false,
        enabled: false,
      };
      mockPlugins.push(record);
      return { ...record };
    },
    async uninstallPlugin(id: string, version: string) {
      const index = mockPlugins.findIndex((record) => {
        const manifest = JSON.parse(record.manifest_json) as { id: string; version: string };
        return manifest.id === id && manifest.version === version;
      });
      if (index === -1) throw new Error("plugin version is not installed");
      mockPlugins.splice(index, 1);
      mockPluginEntries.delete(`${id}@${version}`);
      if (!mockPlugins.some((record) => (JSON.parse(record.manifest_json) as { id: string }).id === id)) {
        delete mockAppStrings[`plugin-settings:${id}`];
      }
    },
    async readPluginEntry(id: string, version: string) {
      const entry = mockPluginEntries.get(`${id}@${version}`);
      if (!entry) throw new Error("plugin version is not installed");
      return entry.slice();
    },
    async setPluginEnabled(id: string, version: string, enabled: boolean) {
      for (const record of mockPlugins) {
        const manifest = JSON.parse(record.manifest_json) as { id: string; version: string };
        if (manifest.id === id) {
          record.selected = manifest.version === version;
          record.enabled = record.selected && enabled;
        }
      }
    },
    async verifyPluginRegistry() {
      // Browser mock has no embedded native key. Registry tests mock this boundary.
    },
    async loadPluginRegistryCache() {
      if (mockPluginRegistryCache) {
        return { kind: "envelope" as const, envelope: { ...mockPluginRegistryCache } };
      }
      return { kind: "absent" as const };
    },
    async storePluginRegistryCache(indexJson, signature) {
      mockPluginRegistryCache = { schemaVersion: 1, indexJson, signature: signature.trim() };
    },
    async setSystemBarAppearance(): Promise<void> {},
    async closeGraphWindow(): Promise<void> {},
    async trayApply() {
      return { supported: false, active: false, problem: null };
    },
    async openDevtools(): Promise<void> {},
    async defaultGraphParent(): Promise<string> {
      return "/mock";
    },
    graphBindingGeneration: () => 1,
    // The bundled Guide is native content (crates/tine-guide); the preview has none, as a published export.
    async guidePages() {
      return [];
    },
    async setGuideAnnounced(announced: boolean): Promise<void> {
      mockGuideAnnounced = announced;
    },
    async suggestGraphName(_dir: string): Promise<string> { return "notes"; },
    async createGraph(dir: string, name: string): Promise<string> {
      return `${dir.replace(/[\\/]$/, "")}/${name.trim()}`; // no real scaffolding in the browser mock
    },
    async warmDone(): Promise<boolean> {
      return true;
    },
    async readCustomCss(): Promise<string> {
      return (globalThis as unknown as { __tineMockCustomCss?: string }).__tineMockCustomCss ?? "";
    },
    // Settings with no browser config edit stay no-ops: a change does not show in loadGraph.
    async setFavorites(): Promise<void> {},
    async setDefaultHome(): Promise<void> {},
    async setPreferredWorkflow(): Promise<void> {},
    async setTimetrackingEnabled(): Promise<void> {},
    async setShowBrackets(): Promise<void> {},
    async setEnableTooltip(): Promise<void> {},
    async setDocModeEnterForNewBlock(): Promise<void> {},
    async setLogicalOutdenting(): Promise<void> {},
    async setPreferredFormat(): Promise<void> {},
    async setJournalTitleFormat(): Promise<string[]> {
      return [];
    },
    async setDefaultJournalTemplate(): Promise<void> {},
    async setStartOfWeek(): Promise<void> {},
    async openExternal(url: string, confirmedScheme?: string): Promise<void> {
      checkBrowserHandoff(url, confirmedScheme, window.document?.baseURI ?? window.location.href);
      try {
        window.open(url, "_blank", "noreferrer");
      } catch { /* ignore */ }
    },
    // The query registry and the OG printer are native; the preview answers as a published export does.
    async queryRegistry() {
      return { rows: [], generation: 0 };
    },
    async queryOgExpressible() {
      return false;
    },
    async printQuery(): Promise<never> {
      const { QueryPrintRefusedError } = await import("./queryPrintRefusal");
      throw new QueryPrintRefusedError("not_applicable", null);
    },
    async readAsset(name: string, maxBytes: number): Promise<Uint8Array> {
      const bytes = mockAssets[name] ?? (name === "sample.pdf" ? decodeB64(SAMPLE_PDF_B64)
        : name === "voice_memo.wav" ? decodeB64(SILENT_WAV_B64) : new Uint8Array());
      if (bytes.byteLength > maxBytes) throw "asset-too-large";
      return bytes;
    },
    async streamAsset(name: string): Promise<string> {
      // Mock streams buffer bytes; use the existing asset-ingress ceiling.
      const bytes = await explicit.readAsset!(name, ASSET_INGRESS_MAX_BYTES);
      if (!bytes.length) return "";
      const type = name.toLowerCase().endsWith(".wav") ? "audio/wav" : "application/octet-stream";
      return URL.createObjectURL(new Blob([bytes as unknown as BlobPart], { type }));
    },
    async readLocalImage(_path: string): Promise<Uint8Array> {
      // The mock has no filesystem; local-file images never resolve here.
      return new Uint8Array();
    },
    async saveAsset(name: string, bytes: Uint8Array): Promise<string> {
      mockAssets[name] = bytes;
      return name;
    },
    async readClipboardImage(): Promise<Uint8Array | null> {
      return null; // no OS clipboard in the browser mock
    },
    async importAsset(path: string, name?: string): Promise<string> {
      return name ?? path.split("/").pop() ?? path;
    },
    async captureImportAsset(path: string, name?: string): Promise<string> {
      return name ?? path.split("/").pop() ?? path;
    },
    async captureSaveAsset(name: string, bytes: Uint8Array): Promise<string> {
      mockAssets[name] = bytes;
      return name;
    },
    async importNativeCapture(path: string, name: string): Promise<string> {
      return name || path.split("/").pop() || path;
    },
    async clipboardFiles() {
      return { files: [], skipped: 0, truncated: false };
    },
    async readTextFile(_path: string): Promise<string> {
      throw new Error("local text files are unavailable in the browser mock");
    },
    async openAsset(): Promise<void> {},
    async openPageFile(): Promise<void> {},
    async openGraphFile(): Promise<void> {},
    async editAssetExternal(): Promise<void> {},
    async detectMediaEditor(): Promise<string> {
      return ""; // nothing to probe in the browser mock
    },
    async listOrphanAssets() {
      return [
        { name: "old_screenshot_20260601_091500.png", size: 184_320, modified: 1_748_762_100 },
        { name: "unused_clip_20260512_140233.mp4", size: 5_242_880, modified: 1_747_051_353 },
      ];
    },
    async trashAsset() {
      return "trashed" as const; // no-op in the browser mock
    },
    async assetTrashStats() {
      return { count: 3, bytes: 1_572_864, pages: 1, journals: 0, conflicts: 0, other: 0 };
    },
    async emptyAssetTrash(): Promise<number> {
      return 3;
    },
    ...mockJournalFiles(),
    async readJournalFile(name: string): Promise<string> {
      return JSON.parse(await graph.read(["read_journal_file_json", name])) as string;
    },
    ...mockConflictApi,
    ...mockQuickShareCommands,
    ...mockGitCommands,
    async onConflictsChanged(): Promise<() => void> {
      return () => {};
    },
    async confirm(message: string, _options: ConfirmOptions): Promise<boolean> {
      // The browser/test env has a working global confirm (unlike the WebKitGTK
      // app), so defer to it. Read it off globalThis so test stubs (vi.stubGlobal)
      // are honoured.
      const c = (globalThis as { confirm?: (m?: string) => boolean }).confirm;
      return typeof c === "function" ? c(message) : true;
    },
    async revealExport(_path: string) {},
    async readCssFile(_global: boolean) { return { text: "", revision: null }; },
    async saveCssFile(_global: boolean, text: string, _revision: string | null) { return text; },
    async pickFolder(_title?: string, _defaultPath?: string): Promise<string | null> {
      return null; // no native dialog in the browser mock
    },
    async pickGraphFolder() {
      return { status: "cancelled" as const };
    },
    async prepareGraphFolder(_path: string) {
      return { status: "ready" as const, location: "local" as const };
    },
    async pickFile(): Promise<string | null> {
      return null;
    },
    async capturePhoto() {
      return { status: "cancelled" as const };
    },
    async startRecording() {
      return { status: "cancelled" as const };
    },
    async stopRecording() {
      return { status: "cancelled" as const };
    },
    async cancelRecording() {
      return { status: "cancelled" as const };
    },
    async writeText(text: string): Promise<void> {
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        // ignore
      }
    },
    async writeRich(text: string, _html: string): Promise<void> {
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        // ignore
      }
    },
    async copyAssetFileToClipboard(): Promise<boolean> { return false; },
    async copyImageToClipboard(): Promise<void> {},
    async onGraphChanged(): Promise<() => void> {
      return () => {}; // no external watcher in the browser mock
    },
    async onAssetChanged(): Promise<() => void> { return () => {}; },
    async onGraphConfigChanged(): Promise<() => void> { return () => {}; },
    async onCustomCssChanged(): Promise<() => void> { return () => {}; },
    async editGlobalCss(): Promise<void> {},
    async editCustomCss(): Promise<void> {},
    async getBackupKeep(): Promise<number> {
      return 12;
    },
    async setBackupKeep(): Promise<void> {},
    async getCaptureEnterFiles(): Promise<boolean> {
      return false;
    },
    async setCaptureEnterFiles(): Promise<void> {},
    async getLinkFirstMatch(): Promise<boolean> {
      return false; // the legacy key is read-only (migration source); never set
    },
    async getWatchMode(): Promise<string> {
      return "inotify";
    },
    async setWatchMode(): Promise<void> {},
    async listBackups() {
      return [];
    },
    async restoreBackup() {
      return { error: null, reloaded: null }; // no-op in the browser mock
    },
    async loadSession(): Promise<string | null> {
      return mockSession;
    },
    async saveSession(data: string): Promise<void> {
      mockSession = data;
    },
    async legacyDraftsFile() {
      return null;
    },
    async loadWorkspaces(): Promise<string> {
      if (!mockWorkspaces) {
        const blob = mockSession ? JSON.parse(mockSession) : {
          tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
          activeIndex: 0,
        };
        mockWorkspaces = JSON.stringify({
          version: 1,
          activeId: "default",
          workspaces: [{ id: "default", name: "", blob }],
        });
      }
      return mockWorkspaces;
    },
    async saveWorkspaces(data: string): Promise<"durable"> {
      mockWorkspaces = data; return "durable";
    },
    async gpuEnv(): Promise<GpuEnv> {
      return { software_forced: false, appimage: false, flatpak: false, packaged: false, portable: false };
    },
    async takeDataHomeFallbackNotice() {
      return null;
    },
    async getSmoothScroll(): Promise<boolean> {
      return false;
    },
    async setSmoothScroll(_value: boolean): Promise<void> {},
    async getAppBool(key: string, fallback: boolean): Promise<boolean> {
      const v = mockAppBools[key];
      return v === undefined ? fallback : v;
    },
    async setAppBool(key: string, value: boolean): Promise<void> {
      mockAppBools[key] = value;
    },
    async defenderHint() {
      return { show: false };
    },
    async dismissDefenderHint(): Promise<void> {},
    async addDefenderExclusion() {
      return { outcome: "declined" as const };
    },
    async getAppString(key: string, fallback: string): Promise<string> {
      const v = mockAppStrings[key];
      return v === undefined ? fallback : v;
    },
    async setAppString(key: string, value: string | null): Promise<void> {
      if (value === null) delete mockAppStrings[key]; else mockAppStrings[key] = value;
    },
    async setUiLocale(): Promise<void> { /* no native shell in the mock */ },
    async applySpellcheck(): Promise<void> { /* no native webview in the mock */ },
    // A representative set so the picker renders in the browser mock / harness.
    async listSpellcheckDictionaries(): Promise<string[]> { return ["cs_CZ", "de_DE", "en_GB", "en_US", "fr_FR", "sk_SK"]; },
    async debugInfo(): Promise<DebugInfo> { return { enabled: false, path: "", recorderActive: false, previousExitUnclean: false }; },
    async debugLog(_line: string): Promise<void> { /* no-op in the browser mock */ },
    async diagnosticReport(): Promise<DiagnosticReport> { return { text: JSON.stringify({ schemaVersion: 1, sessions: { current: mockDiagnostics.map((kind) => ({ event: "frontend", kind })) } }, null, 2), suggestedFileName: "tine-diagnostics.json" }; },
    async saveDiagnosticReport(): Promise<boolean> { return false; },
    async clearDiagnostics(): Promise<void> { mockDiagnostics.length = 0; },
    async createGraphVerification(): Promise<GraphVerificationReport> {
      const aggregateDigest = "0".repeat(64);
      const text = JSON.stringify({ schemaVersion: 1, tool: "tine-graph-bytes", algorithm: "sha256", complete: true, generatedAtUnixMs: Date.now(), files: [], aggregateDigest, errors: [] }, null, 2);
      return { text, suggestedFileName: "tine-graph-verification-mock.json", totalFiles: 0, totalBytes: 0, aggregateDigest, complete: true };
    },
    async cancelGraphVerification(): Promise<void> {},
    async saveGraphVerificationReport(): Promise<boolean> { return false; },
    async onGraphVerificationProgress(): Promise<() => void> { return () => {}; },
    async diagnosticSessionActive(): Promise<void> { /* no session marker in the mock */ },
    async diagnosticFrontendEvent(kind: DiagnosticFrontendKind): Promise<void> { mockDiagnostics.push(kind); },
    async localClock() {
      const now = Date.now();
      return { offset_minutes: -new Date(now).getTimezoneOffset(), unix_ms: now };
    },
    async appArchitecture(): Promise<string> { return "x86_64"; },
    async watcherLatencyRecent(): Promise<unknown[]> { return []; },
    // PDF highlight records are host state; the `hls__` page is graph text the desktop writes.
    async readHighlights(pdf: string): Promise<Highlight[]> {
      return mockHighlights[pdf]?.highlights ?? [];
    },
    async openPdf(pdf: string, label: string): Promise<PdfState> {
      const current = mockHighlights[pdf] ?? { label, highlights: [] };
      return {
        highlights: current.highlights,
        page: current.page ?? null,
        scale: current.scale ?? null,
      };
    },
    async writeHighlights(pdf: string, label: string, highlights: Highlight[], _baseHighlights: Highlight[]): Promise<Highlight[]> {
      return (mockHighlights[pdf] = { ...mockHighlights[pdf], label, highlights }).highlights;
    },
    async savePdfAreaImage(pdf: string, page: number, id: string, stamp: number, _bytes: Uint8Array): Promise<string> {
      return `${pdf.replace(/\.pdf$/i, "")}/${page}_${id}_${stamp}.png`;
    },
    async rollbackPdfAreaImage(): Promise<void> {},
  };

  // Anything else is a store write path or a native answer (rename, merge,
  // conflict resolution, print, publication, the Guide
  // copy): refused, as the published export refuses what it does not answer.
  return new Proxy(explicit as Record<string, unknown>, {
    get(target, property) {
      if (typeof property !== "string") return undefined;
      if (property in target) return target[property];
      if (property === "then" || MOCK_ABSENT_METHODS.has(property)) return undefined;
      return async () => {
        throw new Error(mockRefusalMessage());
      };
    },
    has(target, property) {
      return typeof property === "string" && !MOCK_ABSENT_METHODS.has(property) ? true : property in target;
    },
  }) as unknown as Backend;
}
