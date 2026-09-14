// Backend abstraction. In the Tauri app we call Rust via `invoke`. In a plain
// browser (Vite dev / Playwright screenshots) we fall back to an in-memory mock
// seeded from a fixture graph, so the whole UI is exercisable without the shell.

import { notifyGraphRebound } from "./modeHooks";
import { DIAGNOSTIC_KINDS } from "./editor/queryIr";
import type { GraphSearchDisplayOptions } from "./editor/queryIr";
import type {
  Diagnostic,
  DiagnosticKind,
  ExecutionContext,
  ExplainEmptyResult,
  ParsedQuery,
  Query,
  QueryPrintDialect,
  QueryTextDialect,
  QueryResult,
  RegistrySnapshot,
  Span,
  ViewSettings,
} from "./editor/queryIr";
import type {
  ActivationExpectedRevision,
  ActivationIntent,
  ApplicationPageAdmission,
  ManagedApplicationMoveSubtreesRecoveryResult,
  ManagedApplicationMoveSubtreesRequest,
  ManagedApplicationMoveSubtreesResult,
  AdvancedQueryResult,
  BacklinkFilterContext,
  BacklinkFilterTarget,
  AssetInfo,
  EditorActivationHandle,
  SavePageResult,
  RenameOutcome,
  PageKind,
  GraphMeta,
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
  JournalFilenameMigration,
  SyncConflict,
  SyncConflictDiff,
  VcsMarkerConflict,
  ConflictObject,
  LiveSaveConflictCapture,
  MarkerConflictDiff,
  MergeDecision,
  ManagedPageMutationPreflightResult,
  PrintOpts,
  SparseV2Status,
  SparseV2CancelResult,
  SparseV2AdoptionResult,
  SparseV2ActivationProgressEvent,
  SparseV2Tick,
  SparseV2RuntimeStatusEvent,
  SparseV2TickEvent,
  SparseV2ErrorEvent,
  SyncAbsenceSweepEvent,
  SyncAbsenceSweepChangedEvent,
  SyncAbsenceSweepActionOutcome,
  SyncAbsenceSweepRestoreOutcome,
  SparseV2QueryRequest,
  SparseV2QueryReply,
  SparseV2EditorLoadRequest,
  SparseV2EditorSaveRequest,
  SparseV2EditorOutcome,
  StorageTransitionEvent,
  PdfState,
  QueryExecution,
  QueryPageScope,
  QueryExportBatch,
  QueryExportSpec,
} from "./types";
import { measureIssue248Async } from "./issue248Probe";
import { assetFileName } from "./media";
import { mockBackend } from "./mock";
import { recordGraphOpenCommand } from "./graphOpenTrace";

export type ConflictCapsuleAuthority =
  | { kind: "direct_durable"; expected_disk_rev: string }
  | { kind: "direct_live"; conflict_epoch: number }
  | { kind: "managed"; path: string; revision: string };

export interface ConflictCapsuleReview {
  diff: SyncConflictDiff;
  authority: ConflictCapsuleAuthority;
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

/** One raw graph file, as returned by `graphSourceFiles` — the input to the
 *  in-app lsdoc↔mldoc diff panel. `text` is the file's bytes exactly as on disk. */
export interface GraphSourceFile {
  rel: string;
  text: string;
  format: "md" | "org";
  bytes: number;
}

/** The reference-name inventory, digest-gated. `names` is null exactly when the
 *  digest the caller presented still describes the current set. */
export interface ReferencedPageNames {
  digest: number;
  names: string[] | null;
}

export type GraphFolderPickResult =
  | { status: "picked"; path: string }
  | { status: "permission-requested" | "permission-needed" | "cancelled" | "refused"; path?: string };

export type PreparedGraphFolder =
  | { status: "ready"; location: "local" | "icloud"; path?: string }
  | { status: "refused"; location?: undefined };

export interface ClipboardAssetFile {
  path: string;
  name: string;
  size: number;
}

export interface ClipboardFileList {
  files: ClipboardAssetFile[];
  skipped: number;
  truncated: boolean;
}

/** Result of an Android media-capture command. Successful photos and voice
 *  memos return a bounded native cache-file `path` which Rust streams directly
 *  into the graph. */
export interface MediaCaptureResult {
  status: "ok" | "recording" | "cancelled";
  path?: string | null;
  ext?: string | null;
}

/** Result of the process-wide native shutdown preparation. Android may request
 * an Activity exit only after `safe`; partial native progress must remain
 * shielded so retrying does not replay the frontend persistence transaction. */
export type TineQuitPreparation =
  | { status: "safe" }
  | { status: "refused"; detail: string }
  | { status: "partial"; safe_slots: string[]; detail: string };

export interface KnownGraph {
  path: string;
  name: string;
}

export interface InstalledPluginRecord {
  id: string;
  version: string;
  manifest_json: string;
  sha256: string;
  selected: boolean;
  enabled: boolean;
}

export interface PluginRegistryCacheEnvelope {
  schemaVersion: 1;
  indexJson: string;
  signature: string;
}

export type BackendErrorKind =
  | "save-conflict"
  | "direct-save-failure"
  | "sync-data-unavailable"
  | "managed-graph-mismatch"
  | "shared-frontier-mismatch"
  | "adoption-archived"
  | "sparse-shutdown-refused"
  | "asset-too-large"
  | "operation-cancelled"
  | "managed-actor-refusal"
  | "query-not-ready"
  | "query-unavailable"
  | "query-print-refused";

const BACKEND_ERROR_MESSAGES: Record<
  Exclude<
    BackendErrorKind,
    "save-conflict" | "direct-save-failure" | "managed-actor-refusal" | "query-print-refused" | "query-not-ready" | "query-unavailable"
  >,
  string
> = {
  "sync-data-unavailable": "This graph does not yet contain sync data from another device.",
  "managed-graph-mismatch": "The shared descriptor names another managed graph.",
  "shared-frontier-mismatch": "This device's notes are not in the shared provider frontier.",
  "adoption-archived": "Adoption stopped after this device's own history was archived.",
  "sparse-shutdown-refused": "Tine-managed storage could not verify a clean stop.",
  "asset-too-large": "The asset exceeds the safe size limit.",
  "operation-cancelled": "The operation was cancelled.",
};

/** The sole frontend family for JSON-tagged native failures. Components branch
 * on subclasses and never parse the payload string or user-facing wording. */
export class BackendError extends Error {
  constructor(readonly kind: BackendErrorKind, message: string) {
    super(message);
    this.name = "BackendError";
  }

  override toString(): string {
    return this.message;
  }
}

export class SyncDataUnavailableError extends BackendError {
  constructor() {
    super("sync-data-unavailable", BACKEND_ERROR_MESSAGES["sync-data-unavailable"]);
    this.name = "SyncDataUnavailableError";
  }
}

export class ManagedGraphMismatchError extends BackendError {
  constructor() {
    super("managed-graph-mismatch", BACKEND_ERROR_MESSAGES["managed-graph-mismatch"]);
    this.name = "ManagedGraphMismatchError";
  }
}

export type SharedFrontierMismatchSide = "local-only" | "shared-only" | "changed";
export type SharedFrontierMismatchCategory = "kind" | "preamble" | "outline" | "explicit-ids";
export interface SharedFrontierMismatchPath {
  path: string;
  side: SharedFrontierMismatchSide;
  categories: SharedFrontierMismatchCategory[];
}
/** The bounded, typed detail a clean-join refusal carries so the user who
 * asked for the join can reconcile it: counts plus at most 32 relative note
 * paths with their side or changed categories. Never note content. */
export interface SharedFrontierMismatchDetail {
  localPages: number;
  sharedPages: number;
  localOnly: number;
  sharedOnly: number;
  changed: number;
  paths: SharedFrontierMismatchPath[];
  omitted: number;
}
export const SHARED_FRONTIER_MISMATCH_MAX_PATHS = 32;

export class SharedFrontierMismatchError extends BackendError {
  constructor(readonly detail: SharedFrontierMismatchDetail | null = null) {
    super("shared-frontier-mismatch", BACKEND_ERROR_MESSAGES["shared-frontier-mismatch"]);
    this.name = "SharedFrontierMismatchError";
  }
}

export class AdoptionArchivedError extends BackendError {
  constructor() {
    super("adoption-archived", BACKEND_ERROR_MESSAGES["adoption-archived"]);
    this.name = "AdoptionArchivedError";
  }
}

export class SparseShutdownRefusedError extends BackendError {
  constructor() {
    super("sparse-shutdown-refused", BACKEND_ERROR_MESSAGES["sparse-shutdown-refused"]);
    this.name = "SparseShutdownRefusedError";
  }
}

export class AssetTooLargeError extends BackendError {
  constructor() {
    super("asset-too-large", BACKEND_ERROR_MESSAGES["asset-too-large"]);
    this.name = "AssetTooLargeError";
  }
}

/** `query_print` refused to print this IR in the requested dialect (§7.1, A4).
 *
 *  `NotApplicable` means the OG printer cannot express the query — the OG DSL is
 *  a partial language, so this is an ordinary, expected answer, not a fault.
 *  **Exactly one caller is entitled to see it: the save path**, which responds by
 *  switching to the `{{tine-query}}` dialect. Any other caller reaching here
 *  asked the OG printer without first calling `queryOgExpressible`, and that is a
 *  bug in that caller — which is why this rejects instead of returning `""`. A
 *  catch-all that turned a refusal into a silent no-op is how an unsaved edit
 *  comes to look saved.
 *
 *  `diagnostic` is the structured `Diagnostic` the printer produced, carried in
 *  the envelope's `detail` so callers read `kind`/`message`/`suggestions` as
 *  objects rather than parsing prose (I-9). */
export class QueryPrintRefusedError extends BackendError {
  constructor(
    readonly reasonCode: string,
    readonly diagnostic: Diagnostic | null,
  ) {
    super(
      "query-print-refused",
      diagnostic?.message ?? `The query could not be printed (reason code: ${reasonCode}).`,
    );
    this.name = "QueryPrintRefusedError";
  }

  /** Whether this refusal is the expected "OG cannot say this" answer, as
   *  opposed to a malformed-input refusal the caller must surface. */
  get isNotApplicable(): boolean {
    return this.reasonCode === "not_applicable";
  }
}

export class OperationCancelledError extends BackendError {
  constructor() {
    super("operation-cancelled", BACKEND_ERROR_MESSAGES["operation-cancelled"]);
    this.name = "OperationCancelledError";
  }
}

export type QueryReadinessReason = "indexing" | "recovering" | "pending_edits" | "busy";

export class QueryNotReadyError extends BackendError {
  constructor(readonly reasonCode: QueryReadinessReason) {
    super("query-not-ready", reasonCode === "recovering" ? "Rebuilding the query index…" : "Updating query results…");
    this.name = "QueryNotReadyError";
  }
}

export class QueryUnavailableError extends BackendError {
  constructor(readonly reasonCode: string, message: string) {
    super("query-unavailable", message);
    this.name = "QueryUnavailableError";
  }
}

export class ManagedActorRefusalError extends BackendError {
  constructor(readonly reasonCode: string) {
    super("managed-actor-refusal", `Managed storage refused the operation (reason code: ${reasonCode}).`);
    this.name = "ManagedActorRefusalError";
  }
}

export class DirectSaveFailureError extends BackendError {
  constructor(readonly reasonCode: string, readonly ioErrorKind: string) {
    super("direct-save-failure", `Direct Files could not save (reason code: ${reasonCode}).`);
    this.name = "DirectSaveFailureError";
  }
}

/** A Direct Files revision conflict, classified once at the Tauri wire boundary.
 * Callers branch on this tag and never inspect arbitrary backend prose. */
export class SaveConflictError extends BackendError {
  constructor(
    readonly epoch: number | null,
    readonly reasonCode: string = "conflict.base_rev",
    readonly ioErrorKind: string | null = null,
  ) {
    super("save-conflict", "The page changed on disk while it was being edited.");
    this.name = "SaveConflictError";
  }
}

export function isSaveConflictError(error: unknown): error is SaveConflictError {
  return error instanceof SaveConflictError;
}

/** The one place a native rejection becomes a typed error. */
export function classifyNativeCallError(error: unknown): unknown {
  return classifyTaggedBackendError(error) ?? error;
}

type TaggedBackendPayload = { kind: string; reason_code?: unknown; detail?: unknown };

const SHARED_FRONTIER_SIDES: readonly SharedFrontierMismatchSide[] = ["local-only", "shared-only", "changed"];
const SHARED_FRONTIER_CATEGORIES: readonly SharedFrontierMismatchCategory[] = [
  "kind",
  "preamble",
  "outline",
  "explicit-ids",
];

function nonNegativeCount(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : null;
}

const REASON_CODE = /^[a-z][a-z_]*(?:\.[a-z][a-z_]*)*$/;

function readIoErrorKind(detail: unknown): string | null {
  if (!detail || typeof detail !== "object") return null;
  const value = (detail as Record<string, unknown>).io_error_kind;
  return typeof value === "string" && /^[A-Z][A-Za-z]{0,63}$/.test(value) ? value : null;
}

/** Validate the native detail object field by field; anything malformed
 * degrades to a detail-less refusal rather than trusting the payload. */
function readSharedFrontierMismatchDetail(raw: unknown): SharedFrontierMismatchDetail | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Record<string, unknown>;
  const localPages = nonNegativeCount(record.local_pages);
  const sharedPages = nonNegativeCount(record.shared_pages);
  const localOnly = nonNegativeCount(record.local_only);
  const sharedOnly = nonNegativeCount(record.shared_only);
  const changed = nonNegativeCount(record.changed);
  const omitted = nonNegativeCount(record.omitted);
  if (
    localPages === null || sharedPages === null || localOnly === null
    || sharedOnly === null || changed === null || omitted === null
    || !Array.isArray(record.paths) || record.paths.length > SHARED_FRONTIER_MISMATCH_MAX_PATHS
  ) {
    return null;
  }
  const paths: SharedFrontierMismatchPath[] = [];
  for (const entry of record.paths as unknown[]) {
    if (!entry || typeof entry !== "object") return null;
    const { path, side, categories } = entry as Record<string, unknown>;
    if (typeof path !== "string" || path.length === 0) return null;
    if (!SHARED_FRONTIER_SIDES.includes(side as SharedFrontierMismatchSide)) return null;
    const known: SharedFrontierMismatchCategory[] = [];
    if (categories !== undefined) {
      if (!Array.isArray(categories)) return null;
      for (const category of categories as unknown[]) {
        if (!SHARED_FRONTIER_CATEGORIES.includes(category as SharedFrontierMismatchCategory)) return null;
        known.push(category as SharedFrontierMismatchCategory);
      }
    }
    paths.push({ path, side: side as SharedFrontierMismatchSide, categories: known });
  }
  return { localPages, sharedPages, localOnly, sharedOnly, changed, paths, omitted };
}

/** Read the structured `Diagnostic` a `query-print-refused` envelope carries.
 *
 *  Validated field by field: a malformed payload degrades to a detail-less
 *  refusal (the caller still learns the print was REFUSED, which is the part that
 *  must never be lost) rather than being trusted into the UI.
 *  `DIAGNOSTIC_KINDS` is the mirror's own list, so a kind Rust adds and the
 *  mirror has not learned reads as malformed instead of flowing through
 *  mistyped. */
function readPrintDiagnostic(detail: unknown): Diagnostic | null {
  if (!detail || typeof detail !== "object") return null;
  const value = detail as Record<string, unknown>;
  if (typeof value.message !== "string") return null;
  if (typeof value.kind !== "string") return null;
  if (!(DIAGNOSTIC_KINDS as readonly string[]).includes(value.kind)) return null;
  const suggestions = Array.isArray(value.suggestions)
    && value.suggestions.every((s) => typeof s === "string")
    ? (value.suggestions as string[])
    : [];
  const span = value.span && typeof value.span === "object"
    && typeof (value.span as Record<string, unknown>).start === "number"
    && typeof (value.span as Record<string, unknown>).end === "number"
    ? (value.span as unknown as Span)
    : undefined;
  return {
    kind: value.kind as DiagnosticKind,
    message: value.message,
    suggestions,
    disabled: value.disabled === true,
    span,
  };
}

function classifyTaggedBackendError(error: unknown): BackendError | null {
  const message = typeof error === "string"
    ? error
    : error instanceof Error
      ? error.message
      : null;
  if (message === null || message[0] !== "{") return null;
  let payload: TaggedBackendPayload;
  try {
    payload = JSON.parse(message) as TaggedBackendPayload;
  } catch {
    return null;
  }
  if (!payload || typeof payload !== "object") return null;
  switch (payload.kind) {
    case "direct-save-failure": {
      const ioErrorKind = readIoErrorKind(payload.detail);
      return typeof payload.reason_code === "string" && REASON_CODE.test(payload.reason_code)
        && ioErrorKind !== null
        ? new DirectSaveFailureError(payload.reason_code, ioErrorKind)
        : null;
    }
    case "save-conflict": {
      const ioErrorKind = readIoErrorKind(payload.detail);
      const epoch = payload.detail && typeof payload.detail === "object"
        ? (payload.detail as Record<string, unknown>).epoch
        : undefined;
      const validEpoch = epoch === null
        || (typeof epoch === "number" && Number.isSafeInteger(epoch) && epoch >= 0);
      return typeof payload.reason_code === "string" && REASON_CODE.test(payload.reason_code)
        && payload.reason_code.startsWith("conflict.") && ioErrorKind !== null && validEpoch
        ? new SaveConflictError(epoch as number | null, payload.reason_code, ioErrorKind)
        : null;
    }
    case "sync-data-unavailable":
      return new SyncDataUnavailableError();
    case "managed-graph-mismatch":
      return new ManagedGraphMismatchError();
    case "shared-frontier-mismatch":
      return new SharedFrontierMismatchError(
        payload.detail === undefined ? null : readSharedFrontierMismatchDetail(payload.detail),
      );
    case "adoption-archived":
      return new AdoptionArchivedError();
    case "sparse-shutdown-refused":
      return new SparseShutdownRefusedError();
    case "asset-too-large":
      return new AssetTooLargeError();
    case "operation-cancelled":
      return new OperationCancelledError();
    case "query-not-ready":
      return payload.reason_code === "indexing" || payload.reason_code === "recovering"
        || payload.reason_code === "pending_edits" || payload.reason_code === "busy"
        ? new QueryNotReadyError(payload.reason_code)
        : null;
    case "query-unavailable": {
      const detail = payload.detail && typeof payload.detail === "object"
        ? (payload.detail as Record<string, unknown>).message : undefined;
      return typeof payload.reason_code === "string" && REASON_CODE.test(payload.reason_code)
        && typeof detail === "string" && detail.trim().length > 0
        ? new QueryUnavailableError(payload.reason_code, detail)
        : null;
    }
    case "managed-actor-refusal":
      return typeof payload.reason_code === "string" && REASON_CODE.test(payload.reason_code)
        ? new ManagedActorRefusalError(payload.reason_code)
        : null;
    case "query-print-refused":
      return typeof payload.reason_code === "string" && REASON_CODE.test(payload.reason_code)
        ? new QueryPrintRefusedError(payload.reason_code, readPrintDiagnostic(payload.detail))
        : null;
    default:
      return null;
  }
}

export type PluginRegistryCacheLoad =
  | { kind: "absent" }
  | { kind: "envelope"; envelope: PluginRegistryCacheEnvelope }
  | { kind: "unsafe"; reason: string };

export type LoadGraphResult =
  | {
      kind: "loaded" | "already_current";
      meta: GraphMeta;
      binding_generation: number;
      application_page_admission: ApplicationPageAdmission;
    }
  | { kind: "focused_existing"; window_label: string };

export interface CaptureGraphBindingResult {
  binding_generation: number;
}

export interface GraphAccessInspection {
  graph_root: string;
  external_assets_path: string | null;
  approved: boolean;
}

export interface Backend {
  inspectGraphAccess(path: string): Promise<GraphAccessInspection>;
  approveExternalAssets(graphRoot: string, assetsPath: string): Promise<void>;
  loadGraph(path: string): Promise<LoadGraphResult>;
  openGraphWindow(path: string): Promise<LoadGraphResult>;
  startupGraphPath(): Promise<string | null>;
  onStorageTransition(cb: (progress: StorageTransitionEvent) => void): Promise<() => void>;
  captureTarget(): Promise<string>;
  /** Lease the graph selected for this Quick Capture show before issuing
   * graph-scoped reads from its independent WebView. */
  bindCaptureGraph(): Promise<void>;
  listKnownGraphs(): Promise<KnownGraph[]>;
  forgetKnownGraph(path: string): Promise<void>;
  /** Show a known graph's root folder in the OS file manager (desktop only). */
  revealKnownGraph(path: string): Promise<void>;
  appPlatform(): Promise<"android" | "ios" | "desktop">;
  /** Compile-time process architecture. Used to avoid offering updater targets
   * that the signed release manifest deliberately does not publish. */
  appArchitecture(): Promise<string>;
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
  /** Verify every managed runtime can stop cleanly without exiting the app.
   * Android calls this before handing the final activity exit to SafeBack. */
  prepareQuit(): Promise<TineQuitPreparation>;
  closeGraphWindow(): Promise<void>;
  /** Toggle the WebView developer tools (WebKit Web Inspector) for theme/CSS
   *  debugging. No-op on a build without devtools compiled in. */
  openDevtools(): Promise<void>;
  /** Scaffold a brand-new demo graph (onboarding "create new graph"); returns
   *  the created graph's root path to then `loadGraph`. Creates the graph in
   *  `dir` if empty, else in a fresh `tine-demo` subfolder. */
  createGraph(dir: string): Promise<string>;
  /** Page names that exist only through references in the warmed graph cache.
   *  Pass the digest of the set you already hold: the answer omits `names`
   *  entirely when nothing changed, which is the usual case between saves and
   *  saves several thousand strings of IPC and JSON parsing on the UI thread.
   *  A `null` `names` means "keep what you have", never "the set is empty". */
  referencedPageNames(knownDigest?: number | null): Promise<ReferencedPageNames>;
  listPages(): Promise<PageEntry[]>;
  journalFeedPage(limit: number, beforeDay: number | null): Promise<import("./types").JournalFeedPage>;
  /** Journal date-keys (yyyymmdd) whose page has real content. */
  journalContentDays(): Promise<number[]>;
  getPage(name: string, kind: "journal" | "page"): Promise<PageDto | null>;
  /** Raw source text of every md/org file in the open graph (+journals when
   *  asked), for the "Help improve Tine" diff panel. Read-only, local. */
  graphSourceFiles(includeJournals: boolean): Promise<GraphSourceFile[]>;
  /** Save a page. `baseRev` is the revision the editor loaded. Direct Files
   *  binds `force` to `conflictEpoch`; managed storage binds it to the exact
   *  managed path and revision observed after refusal. */
  savePage(
    page: PageDto,
    baseRev: string | null,
    force?: boolean,
    conflictEpoch?: number | null,
    managedConflictObservation?: { path: string; revision: string } | null,
  ): Promise<SavePageResult>;
  /** Publish the durable recovery record for one Direct cross-page move BEFORE
   *  the first page is written (packet B2, I-3/I-2). `destination` and
   *  `sources` are the POST-move DTOs, in the exact order the choreography
   *  saves them. Resolves to the record id, or `null` when no record was
   *  composed — a degenerate move, or an unavailable app-private root. `null`
   *  never refuses the move; see `docs/contracts/direct-move-recovery.md` §4. */
  beginDirectCrossPageMove(destination: PageDto, sources: PageDto[]): Promise<string | null>;
  /** Retire that record once every participant is durably terminal. Resolves to
   *  whether it was retired; a record left behind is converged at the next open. */
  finishDirectCrossPageMove(moveId: string): Promise<boolean>;
  /** X1 native bridge only. Production gesture routing remains disabled until
   * X2 owns quiescence, leases, publication, and semantic history. */
  moveManagedApplicationSubtrees(
    bindingGeneration: number,
    request: ManagedApplicationMoveSubtreesRequest,
  ): Promise<ManagedApplicationMoveSubtreesResult>;
  /** Retire response-replay evidence after the committed page pair is installed. */
  acknowledgeManagedApplicationMove(
    bindingGeneration: number,
    episodeId: string,
    batchId: string,
  ): Promise<void>;
  /** Resolve one exact deferred move episode without routing a new gesture. */
  recoverManagedApplicationSubtrees(
    bindingGeneration: number,
    request: ManagedApplicationMoveSubtreesRequest,
  ): Promise<ManagedApplicationMoveSubtreesRecoveryResult>;
  preflightManagedPageMutation(
    page: PageDto,
    baseRevision: string | null,
    bindingGeneration: number,
  ): Promise<ManagedPageMutationPreflightResult>;
  sparseV2Status(): Promise<SparseV2Status>;
  onSparseV2Status(cb: (event: SparseV2RuntimeStatusEvent) => void): Promise<() => void>;
  onSparseV2Tick(cb: (event: SparseV2TickEvent) => void): Promise<() => void>;
  onSparseV2Error(cb: (event: SparseV2ErrorEvent) => void): Promise<() => void>;
  onSparseV2ActivationProgress(
    bindingGeneration: number,
    cb: (progress: SparseV2ActivationProgressEvent["progress"]) => void
  ): Promise<() => void>;
  activateSparseV2(): Promise<SparseV2Status>;
  cancelSparseV2(): Promise<SparseV2CancelResult>;
  cancelSparseV2Cold(path: string): Promise<SparseV2CancelResult>;
  prepareSparseV2Share(): Promise<SparseV2Status>;
  joinSparseV2Shared(): Promise<SparseV2Status>;
  /** Adopt another device's shared graph, archiving this device's own managed history. */
  adoptSparseV2Shared(): Promise<SparseV2AdoptionResult>;
  /** Where a set-aside managed history is archived, knowable before adoption runs. */
  sparseV2RecoveryLocation(): Promise<string>;
  sparseV2Query(request: SparseV2QueryRequest): Promise<SparseV2QueryReply>;
  sparseV2EditorLoad(request: SparseV2EditorLoadRequest): Promise<SparseV2EditorOutcome>;
  sparseV2EditorSave(request: SparseV2EditorSaveRequest): Promise<SparseV2EditorOutcome>;
  sparseV2Tick(): Promise<SparseV2Tick>;
  listAbsenceSweeps(): Promise<SyncAbsenceSweepEvent[]>;
  onAbsenceSweepChanged(
    bindingGeneration: number,
    cb: (sweep: SyncAbsenceSweepEvent) => void,
  ): Promise<() => void>;
  reapplyAbsenceSweep(sweepId: string): Promise<SyncAbsenceSweepActionOutcome>;
  restoreAbsenceSweep(sweepId: string): Promise<SyncAbsenceSweepRestoreOutcome>;
  keepAbsenceSweepDeletion(sweepId: string): Promise<void>;
  sparseV2CleanShutdown(): Promise<import("./types").SparseV2RuntimeStatus>;
  /** Bundled read-only Guide pages, compiled from the same templates as the demo graph. */
  guidePages(): Promise<GuidePage[]>;
  /** Copy the bundled Guide into the real graph under `tine-guide/`. */
  copyGuideIntoGraph(title: string): Promise<GuideCopyResult>;
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
  /** Rename a page and update all [[refs]]/#tags across the graph. */
  renamePage(old: string, next: string, expectedPath?: string): Promise<RenameOutcome>;
  publishHtml(): Promise<[string, number]>;
  /** Render one page to a self-contained HTML document (assets inlined, no
   *  sidebar) for the print-to-PDF export, with the dialog's options. Rejects if
   *  the page doesn't exist. */
  pagePrintHtml(name: string, opts: PrintOpts): Promise<string>;
  // ---- The six query commands (SPEC §7.1, N23) --------------------------
  //
  // ONE engine, in Rust. `parseQuery` and `printQuery` are the only producers
  // and consumers of query TEXT in the app: the frontend holds the IR and the
  // view, never a DSL string it parsed itself.

  /** Text → `{query, view}` (§7.1).
   *
   *  `macroQuery` / `macroTql` take the COMPLETE raw macro argument, WITHOUT the
   *  outer `{{`/`}}`, and are the only inputs that split a trailing options map —
   *  once, in Rust. `macroQuery` also picks OG vs advanced with the one Rust
   *  discriminator, so a `:find` inside a string literal is text on both sides.
   *  `og` / `tql` / `advanced` are explicit form inputs.
   *
   *  `blockProperties` are the host block's `tine.*` properties, which take
   *  precedence over directives lifted from the query text (§4.1). Passing them
   *  is `Macro.tsx`'s job and is merged here and nowhere else. */
  parseQuery(
    text: string,
    dialect: QueryTextDialect,
    blockProperties?: [string, string][],
  ): Promise<ParsedQuery>;
  /** IR → text (§4.3, §7.1).
   *
   *  Rejects with {@link QueryPrintRefusedError} when `dialect` is `og` and the IR
   *  is not OG-expressible; the save path is the ONE caller entitled to see that
   *  and answers by switching dialect. Everyone else must call
   *  {@link Backend.queryOgExpressible} first.
   *
   *  `preserveForm` re-emits `source.original` plus the changed options map once,
   *  WITHOUT re-lowering the IR (§4.3.1) — the source-preserving title edit. It
   *  requires a source-backed query and its matching macro dialect; a builder
   *  query is refused. Title editing is not a filter conversion. */
  printQuery(
    query: Query,
    view: ViewSettings,
    dialect: QueryPrintDialect,
    preserveForm?: boolean,
  ): Promise<string>;
  /** Whether the OG DSL can say this query, so the save path can choose the
   *  macro name (Q3) without provoking a rejection. */
  queryOgExpressible(query: Query, view: ViewSettings): Promise<boolean>;
  /** The observed property registry (§6.1), with the generation that invalidates
   *  a cached parse's suggestions. */
  queryRegistry(): Promise<RegistrySnapshot>;
  /** Run an already-parsed IR through the one walk. `context.current_page` binds
   *  `?current-page`; an absent one leaves it UNBOUND rather than guessing (§4.4). */
  queryRun(query: Query, view: ViewSettings, context?: ExecutionContext): Promise<QueryResult>;
  /** Why the query returned nothing (Q14, N19): for a root `and`, one row per
   *  top-level conjunct with the count matching it alone and the count matching
   *  all the others without it. Carries the diagnostics and the support report,
   *  because "never bound" and "nothing matched" are different answers. */
  queryExplainEmpty(
    query: Query,
    view: ViewSettings,
    context?: ExecutionContext,
  ): Promise<ExplainEmptyResult>;

  runQuery(query: string): Promise<RefGroup[]>;
  /** Resolve all Copy / Export query macros under one cumulative native budget. */
  exportQuerySubtrees(specs: QueryExportSpec[]): Promise<QueryExportBatch>;
  /** Advanced (datalog-subset) query: maps the supported clauses onto the engine
   *  and reports what ran vs was ignored. `currentPage` resolves the typed
   *  `:current-page` input; callers without that input may use it as query-owner
   *  context for compatibility. */
  runAdvancedQuery(query: string, currentPage?: string): Promise<AdvancedQueryResult>;
  /** Property keys (each with their distinct values) for query-builder
   *  autocomplete. */
  queryFacets(autocomplete?: boolean): Promise<[string, string[]][]>;
  /** `alias::` → canonical page name pairs. */
  pageAliases(): Promise<[string, string][]>;
  /** `icon::` property for each named page that has one (page-name → icon). */
  pageIcons(names: string[]): Promise<Record<string, string>>;
  /** The subset of `names` that already name a page, journal or alias. */
  existingPageNames(names: string[]): Promise<string[]>;
  /** Persist favorited page names to config.edn `:favorites`. */
  setFavorites(names: string[]): Promise<void>;
  /** Record which page owns the Favorites arrangement (`:tine/favorites-page`). */
  setFavoritesPage(name: string): Promise<void>;
  /** Persist (or clear) config.edn `:default-home {:page "..."}`. */
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
  openPageFile(name: string, kind: "page" | "journal", path: string | undefined, reveal: boolean): Promise<void>;
  /** Open a graph asset in a SPECIFIC external editor (drawio/Excalidraw/…) so a
   *  diagram can be edited in place. `command` is that editor's configured command
   *  template (empty = OS opener). See GH #38 / mediaEditors.ts. */
  editAssetExternal(name: string, command: string): Promise<void>;
  /** Best-effort autodetect of an installed editor's launch command (probes disk,
   *  never executes). Returns a command template or "" if not found. */
  detectMediaEditor(id: string): Promise<string>;
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
  duplicateJournalDiff(canonical: string, stray: string): Promise<SyncConflictDiff | null>;
  resolveDuplicateJournalDay(
    canonical: string,
    stray: string,
    decisions: Record<string, string>,
    baseRev: string,
    strayRev: string,
    preChoice?: string,
  ): Promise<PageDto>;
  /** Request one watcher full pass. The returned sequence is completed by a
   *  later `graph-rescan-complete` event, after ordinary change events emit. */
  rescanGraphNow(): Promise<number>;
  onGraphRescanComplete(cb: (sequence: number) => void): Promise<() => void>;
  /** Journal files whose names don't round-trip to a date, with the names they
   *  would get. Proposed only — see `applyJournalFilenameMigrations`. */
  listJournalFilenameMigrations(): Promise<JournalFilenameMigration[]>;
  /** Apply the proposed journal renames after taking a snapshot. Returns how
   *  many files were renamed. */
  applyJournalFilenameMigrations(): Promise<number>;
  /** Move one journal file (by exact filename) to the recoverable trash. */
  trashJournalFile(name: string): Promise<void>;
  /** Raw contents of one journal file (by exact filename), for inspecting a
   *  duplicate day's files before reconciling. */
  readJournalFile(name: string): Promise<string>;
  /** Load a page from a SPECIFIC file by its graph-root-relative path — reaches a
   *  duplicate-day stray that shares a (kind,name) with the canonical file (#21). */
  getPageByPath(path: string): Promise<PageDto | null>;
  /** Activate an editor over an existing file. Deliberately separate from the
   *  mixed-purpose reads above: an activation exists exactly when a live editor
   *  does, so a read for export/preview/hydration cannot inherit an editor's
   *  override authority. (GH #254 increment 3.) */
  activateEditor(
    path: string,
    intent: ActivationIntent,
    expectedRevision: ActivationExpectedRevision,
  ): Promise<EditorActivationHandle | null>;
  /** Activate an editor for a page with no file yet, returning the prospective
   *  target it is live for. Reserves nothing on disk. */
  activateAbsentEditor(name: string, kind: PageKind): Promise<EditorActivationHandle | null>;
  /** Compare-and-retire: retires only if `activation` is still the live one, and
   *  reports whether it was. A retirement racing a newer activation must not
   *  revoke the newer editor. */
  retireEditorActivation(path: string, activation: number): Promise<boolean>;
  /** Present a conflict observation and learn its fate WITHOUT writing. The
   *  "Use disk version" half of the authority contract. */
  presentConflictOverride(
    path: string,
    baseRev: string | null,
    activation: number,
    conflictEpoch: number,
  ): Promise<"authorised" | "superseded" | "withdrawn">;
  /** Append the blocks of `src` (graph-root-relative path) onto `dst`, then trash
   *  `src` — fold a duplicate-day stray into the canonical day (#21). */
  mergePages(src: string, dst: string, rename?: { from: string; to: string }): Promise<void>;
  /** Move a stray file (graph-root-relative path) to a uniquely-named page so it
   *  stops colliding and becomes normally navigable (#21). */
  renameFileToPage(path: string, newName: string): Promise<void>;
  /** Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
   *  user to review + merge instead of them showing as garbage pages. */
  listSyncConflicts(): Promise<SyncConflict[]>;
  /** Pages whose on-disk bytes carry unresolved VCS merge-conflict markers
   *  (git/Fossil): readable, but quarantined from saves. */
  listVcsMarkerConflicts(): Promise<VcsMarkerConflict[]>;
  /** Block-level diff of a conflict copy against its winner (graph-root-relative
   *  paths). Read-only; null if a path is invalid or the file is gone. */
  syncConflictDiff(winner: string, conflict: string): Promise<SyncConflictDiff | null>;
  /** Path-free block-level 2-way diff of two raw page texts (Concord P3 seam —
   *  no graph or path coupling; future in-page conflict UI builds on it). */
  textBlockDiff(mine: string, theirs: string, format?: "md" | "org"): Promise<SyncConflictDiff>;
  /** Path-free 3-way variant: rows are classified against `base` and carry
   *  pre-selectable suggestions (ADR 0056). */
  textBlockDiff3(
    base: string,
    mine: string,
    theirs: string,
    format?: "md" | "org"
  ): Promise<SyncConflictDiff>;
  liveSaveConflictDiff(
    page: PageDto,
    baseRev: string | null,
    conflictEpoch: number,
  ): Promise<SyncConflictDiff>;
  captureLiveSaveConflict(
    page: PageDto,
    baseRev: string | null,
    conflictEpoch: number,
  ): Promise<LiveSaveConflictCapture | null>;
  /** App-private, graph-keyed recovery capsules for unresolved live drafts. */
  loadConflictCapsules?(root: string): Promise<ConflictObject[]>;
  storeConflictCapsule?(root: string, capsule: ConflictObject): Promise<void>;
  retireConflictCapsule?(root: string, pageName: string): Promise<void>;
  conflictCapsuleDiff?(conflict: ConflictObject): Promise<ConflictCapsuleReview>;
  resolveConflictCapsule?(
    conflict: ConflictObject,
    authority: ConflictCapsuleAuthority,
    decisions: Record<string, MergeDecision>,
    preChoice?: "mine" | "theirs" | "union",
  ): Promise<PageDto>;
  durableLiveSaveConflictDiff(page: PageDto, baseText: string | null): Promise<SyncConflictDiff>;
  resolveDurableLiveSaveConflict(
    page: PageDto,
    expectedDiskRev: string,
    decisions: Record<string, MergeDecision>,
    preChoice?: "mine" | "theirs" | "union",
  ): Promise<PageDto>;
  resolveLiveSaveConflict(
    page: PageDto,
    baseRev: string | null,
    conflictEpoch: number,
    decisions: Record<string, MergeDecision>,
    preChoice?: "mine" | "theirs" | "union",
  ): Promise<PageDto>;
  /** The Concord conflict queue (L3): one derived inventory of every page that
   *  needs the user's judgement, from BOTH artifact sources (conflict copies and
   *  VCS-marker pages). Derived from disk on every call — nothing is stored, so
   *  it survives a restart by being recomputed. */
  conflictQueue(): Promise<ConflictObject[]>;
  /** A marker-bearing page's own conflict: its `<<<<<<<` sections parsed into
   *  complete page texts and run through the same block diff (Concord L5).
   *  Read-only; null when the page has no (parseable) markers. */
  vcsMarkerConflictDiff(path: string): Promise<MarkerConflictDiff | null>;
  /** Apply per-row decisions to a marker-bearing page and write the CLEAN merged
   *  result — the one write Tine ever makes to such a file, and only as the
   *  direct result of this confirmation. `baseRev` guards against the VCS moving
   *  the file under the review (throws "conflict" if it did). */
  resolveVcsMarkerConflict(
    path: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    preChoice?: "mine" | "theirs" | "union"
  ): Promise<void>;
  /** Merge a conflict copy into its winner per the user's per-row decisions
   *  (row id → mine/theirs/both/merged), via the normal save path, then trash
   *  the copy. `baseRev` guards against the winner changing under the merge
   *  (throws "conflict" if it did); `mergeBaseRev` echoes the diff's
   *  `merge_base_rev` so a repinned merge base refuses the same way.
   *  `preChoice`: "mine" | "theirs" | "union". */
  resolveSyncConflict(
    winner: string,
    conflict: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    conflictRev: string,
    mergeBaseRev?: string | null,
    preChoice?: "mine" | "theirs" | "union"
  ): Promise<PageDto>;
  /** Discard a conflict copy without merging (move it to the recoverable trash). */
  trashSyncConflict(conflict: string): Promise<void>;
  /** Subscribe to the watcher's `conflicts-changed` event (a conflict copy
   *  appeared or vanished). Returns an unlisten fn. */
  onConflictsChanged(cb: () => void): Promise<() => void>;
  search(query: string, limit: number, lane?: string): Promise<RefGroup[]>;
  /** One Rust-authoritative graph selection plan for page and block hits.
   *
   *  `scope` is the PHYSICAL routed page a block search is confined to.
   *  `options` is the Display half — page membership scope and the two already
   *  resolved per-kind views (§7.6, Q3). They are separate members because they
   *  answer different questions, and overloading the physical one to carry
   *  membership would make "search inside this page" and "match pages by their
   *  content" the same request. Omitting `options` is exactly the request every
   *  caller sent before Display existed. */
  runGraphSearch(
    source: string,
    pageLimit: number,
    blockLimit: number,
    lane?: string,
    explain?: boolean,
    scope?: QueryPageScope,
    options?: GraphSearchDisplayOptions
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
  /** Stream a bounded native Android voice-memo temp into assets and retire the
   *  temp only after the graph copy commits. */
  importNativeCapture(path: string, name: string): Promise<string>;
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
  /** iOS: ensure a Tine-owned local/iCloud graph is locally readable before
   *  Rust enumerates it. Other platforms never call this command. */
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
  readHighlights(pdf: string): Promise<Highlight[]>;
  /** Ensure OG's PDF sidecar/annotation page exist and return highlights plus
   * the persisted last-view page and scale. */
  openPdf(pdf: string, label: string): Promise<PdfState>;
  writeHighlights(pdf: string, label: string, highlights: Highlight[], baseIds: string[]): Promise<void>;
  writePdfViewState(pdf: string, page: number, scale: number): Promise<void>;
  /** Save a cropped area-highlight PNG to OG's layout `assets/<key>/<page>_<id>_<stamp>.png`
   *  (non-dedup — the filename links the `.edn` `:image <stamp>` to the file).
   *  Returns the assets-relative path. */
  savePdfAreaImage(pdf: string, page: number, id: string, stamp: number, bytes: Uint8Array): Promise<string>;
  /** Move a just-written area crop to recoverable trash when its sidecar write fails. */
  rollbackPdfAreaImage(pdf: string, page: number, id: string, stamp: number): Promise<void>;
  /** Subscribe to external file changes (file watcher). Returns an unsubscribe. */
  onGraphChanged(cb: (c: GraphChange) => void): Promise<() => void>;
  /** Subscribe to coalesced external bulk revisions (Concord P2): one event
   *  per reconcile cycle that changed more than the bulk threshold of pages. */
  onGraphChangedBulk(cb: (bulk: GraphChangedBulk) => void): Promise<() => void>;
  /** Subscribe to externally changed graph assets. This is cache observation,
   *  not managed-storage or oplog admission. */
  onAssetChanged(cb: (batch: AssetChangedBatch) => void): Promise<() => void>;
  /** Subscribe to `logseq/config.edn` being re-read after an outside change.
   *  Carries the fresh GraphMeta; a graph whose settings did not move emits
   *  nothing. */
  onGraphConfigChanged(cb: (meta: GraphMeta) => void): Promise<() => void>;
  /** Subscribe to an admitted aggregate managed-storage change. */
  /** A committed query image changed; does not reload or replace live editors. */
  onQueryProjectionChanged(cb: () => void): Promise<() => void>;
  onSparseV2Changed(cb: () => void): Promise<() => void>;
  /** Subscribe to deduplicated managed-sync reconciliation failures. */
  onManagedSyncError(cb: (message: string) => void): Promise<() => void>;
  /** Direct Markdown folder-watch reconcile failure. Distinct from
   *  onManagedSyncError: managed graphs never emit it, so its copy must not
   *  talk about managed storage. */
  onGraphWatchError(cb: (message: string) => void): Promise<() => void>;
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
  /** Restore a snapshot (graph text at original paths, config, and sidecars;
   *  snapshots current state first). Destructive — confirm before calling. */
  restoreBackup(stamp: string): Promise<void>;
  /** Load the persisted UI session JSON (open tabs / active tab / zoom), or null.
   *  Stored atomically in a backend file so structured session state is independent
   *  of a particular WebView/origin and can be shared across windows. */
  loadSession(): Promise<string | null>;
  /** Persist the UI session JSON. */
  saveSession(data: string): Promise<void>;
  /** Load the current graph's device-local named-workspace registry JSON. */
  loadWorkspaces(): Promise<string>;
  /** Atomically persist the current graph's complete named-workspace registry. */
  saveWorkspaces(data: string): Promise<void>;
  /** The one-time notices this DEVICE has been told not to show again for the
   *  current graph, as `{"dismissed": string[]}` (§4.3 "Notice", D-11, I-18).
   *  Never travels with the graph: it says something about this device's user,
   *  not about the graph's content. A missing or damaged record reads as
   *  "nothing dismissed" rather than failing (D-3/G2). */
  loadNotices(): Promise<string>;
  /** Persist the complete dismissed-notice set for the current graph. */
  saveNotices(data: string): Promise<void>;
  /** True exactly ONCE if this launch migrated the app-data dir left by the
   *  desktop identifier rename chain dev.tine.app / page.tine.app ->
   *  page.tine.Tine (so the UI can explain that some app-level prefs may need
   *  re-setting). Self-clears after the first call. */
  takeIdentifierMigrationNotice(): Promise<boolean>;
  /** The directory this launch had to fall back to, exactly ONCE, when the
   *  normal app-data home could not be written (Tauri would otherwise have
   *  panicked before any window existed). `null` on every ordinary launch.
   *  Self-clears after the first call. */
  takeDataHomeFallbackNotice(): Promise<string | null>;
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
  diagnosticReport(buildCommit: string, buildTime: string): Promise<DiagnosticReport>;
  saveDiagnosticReport(buildCommit: string, buildTime: string): Promise<boolean>;
  clearDiagnostics(): Promise<void>;
  createGraphVerification(operationId: string): Promise<GraphVerificationReport>;
  cancelGraphVerification(operationId: string): Promise<void>;
  saveGraphVerificationReport(text: string): Promise<boolean>;
  onGraphVerificationProgress(cb: (progress: GraphVerificationProgress) => void): Promise<() => void>;
  diagnosticFrontendEvent(
    kind: "uncaught_error" | "unhandled_rejection" | "heartbeat_delay" | "updater_failure",
    line?: number,
    column?: number,
    delayMs?: number,
    updaterStage?: string,
    updaterCause?: string,
  ): Promise<void>;
  /** Whether the recorded session counts as live from now on. Mobile only: the
   *  OS reaps a backgrounded app without notice, and that is not a crash
   *  (GH #426). The backend ignores it on desktop. */
  diagnosticSessionActive(active: boolean): Promise<void>;
}

export interface DebugInfo {
  enabled: boolean;
  path: string;
  recorderActive: boolean;
  previousExitUnclean: boolean;
}

export interface DiagnosticReport {
  text: string;
  suggestedFileName: string;
}

export interface GraphVerificationReport {
  text: string;
  suggestedFileName: string;
  totalFiles: number;
  totalBytes: number;
  aggregateDigest?: string;
  complete: boolean;
}

export interface GraphVerificationProgress {
  operationId: string;
  processed: number;
  total: number;
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
  created: boolean;
  removed: boolean;
}

/** One aggregate watcher notification for an external bulk revision (a VCS
 *  checkout, branch switch, or big sync): emitted instead of N `graph-changed`
 *  events when one reconcile cycle changed more than the backend's bulk
 *  threshold of pages. Carries the same per-page change shape. */
export interface GraphChangedBulk {
  changes: GraphChange[];
}

/** One coalesced watcher epoch for ordinary files under the approved assets
 * capability. Paths are relative to assets/; absolute device paths never cross
 * the bridge. */
export interface AssetChangedBatch {
  paths: string[];
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Core commands that REOPEN the graph (`refresh_graph` on the Rust side), so the
 * frontend's graph-scoped state — editor activations, resolved paths — belongs
 * to a `Graph` that no longer exists once they return.
 *
 * Kept as an explicit list because it is a claim about the backend: each of
 * these reaches `refresh_graph`. Adding a command that reopens the graph without
 * adding it here reintroduces the round-15 blockers.
 *
 * The claim is checked, not asserted: `backend_command_parity.rs`'s
 * `rebinding_commands_are_exactly_the_commands_that_reopen_the_graph` re-derives
 * the set by scanning every `#[tauri::command]` in `src-tauri/src` for a
 * `refresh_graph(` call and fails on any difference in either direction.
 */
const REBINDING_COMMANDS = new Set([
  "set_default_home",
  "set_journal_title_format",
  "set_preferred_format",
  "set_timetracking_enabled",
  "set_show_brackets",
  "set_doc_mode_enter_for_new_block",
  "set_logical_outdenting",
  "set_guide_announced",
  "restore_backup",
]);

const DIAGNOSTIC_COMMANDS = new Set([
  "debug_info",
  "debug_log",
  "diagnostic_ipc_event",
  "diagnostic_frontend_event",
  "diagnostic_session_active",
  "diagnostic_report",
  "save_diagnostic_report",
  "clear_diagnostics",
]);
const SLOW_IPC_MS = 500;

class TauriBackend implements Backend {
  private invoke!: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
  private convertFileSrc!: (path: string, protocol?: string) => string;
  private ready: Promise<void>;
  private bindingGeneration = 0;

  constructor() {
    this.ready = import("@tauri-apps/api/core").then((m) => {
      this.invoke = m.invoke;
      this.convertFileSrc = m.convertFileSrc;
    });
  }

  private async call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    // Lease the graph binding at the synchronous call boundary. `ready` is
    // normally already fulfilled, but awaiting even a fulfilled promise yields;
    // a graph switch in that gap must not retarget queued work to the new graph.
    const bindingGeneration = this.bindingGeneration;
    await this.ready;
    const leasedArgs = bindingGeneration
      ? { ...(args ?? {}), bindingGeneration }
      : args;
    const started = performance.now();
    let slow = false;
    let slowTimer: ReturnType<typeof setTimeout> | undefined;
    const reportPhase = (phase: "slow" | "completed" | "failed", elapsedMs: number) => {
      if (DIAGNOSTIC_COMMANDS.has(cmd)) return;
      void this.invoke<void>("diagnostic_ipc_event", {
        command: cmd,
        phase,
        elapsedMs: Math.max(0, Math.round(elapsedMs)),
      }).catch(() => {});
    };
    if (!DIAGNOSTIC_COMMANDS.has(cmd)) {
      slowTimer = setTimeout(() => {
        slow = true;
        reportPhase("slow", performance.now() - started);
      }, SLOW_IPC_MS);
    }
    let result: T;
    try {
      result = await this.invoke<T>(cmd, leasedArgs);
    } catch (error) {
      if (slowTimer !== undefined) clearTimeout(slowTimer);
      recordGraphOpenCommand(cmd, started, "failed");
      reportPhase("failed", performance.now() - started);
      // Classify once, at the only frontend funnel (Harvest H2 E-1 wired only
      // save_page and left the resolver recovery branch dead).
      throw classifyNativeCallError(error);
    }
    if (slowTimer !== undefined) clearTimeout(slowTimer);
    recordGraphOpenCommand(cmd, started, "completed");
    if (slow) reportPhase("completed", performance.now() - started);
    // A command that makes the core REBIND — `refresh_graph` installs a fresh
    // `Graph`, with a fresh (empty) editor-activation registry — must announce
    // it, or this side keeps tokens naming editors the core has never heard of
    // and paths that may have been migrated.
    //
    // Announced HERE, at the one boundary every such command crosses, rather
    // than at each call site: the frontend entry points are fire-and-forget
    // `void backend().setX(...)`, six of the seven never announced, and the
    // seventh only did because it happened to be the one under review.
    // (GH #254 increment 3, round 15.)
    if (REBINDING_COMMANDS.has(cmd)) notifyGraphRebound();
    return result;
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
  startupGraphPath() {
    return this.call<string | null>("startup_graph_path");
  }
  async onStorageTransition(cb: (progress: StorageTransitionEvent) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<StorageTransitionEvent>("storage-transition", (event) => cb(event.payload));
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
  appArchitecture() {
    return this.call<string>("app_architecture");
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
  prepareQuit() {
    return this.call<TineQuitPreparation>("prepare_tine_quit");
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
  referencedPageNames(knownDigest?: number | null) {
    return this.call<ReferencedPageNames>("referenced_page_names", {
      knownDigest: knownDigest ?? null,
    });
  }
  listPages() {
    return this.call<PageEntry[]>("list_pages");
  }
  journalFeedPage(limit: number, beforeDay: number | null) {
    return this.call<import("./types").JournalFeedPage>("journal_feed_page", { limit, beforeDay });
  }
  journalContentDays() {
    return this.call<number[]>("journal_content_days");
  }
  getPage(name: string, kind: "journal" | "page") {
    return this.call<PageDto | null>("get_page", { name, kind });
  }
  graphSourceFiles(includeJournals: boolean) {
    return this.call<GraphSourceFile[]>("graph_source_files", { includeJournals });
  }
  async savePage(
    page: PageDto,
    baseRev: string | null,
    force = false,
    conflictEpoch: number | null = null,
    managedConflictObservation: { path: string; revision: string } | null = null,
  ) {
    return measureIssue248Async("frontend.ipcSaveRoundTripMs", () =>
      this.call<SavePageResult>("save_page", {
        page,
        baseRev,
        force,
        conflictEpoch,
        managedConflictObservation,
      })
    );
  }
  beginDirectCrossPageMove(destination: PageDto, sources: PageDto[]) {
    return this.call<string | null>("begin_direct_cross_page_move", { destination, sources });
  }
  finishDirectCrossPageMove(moveId: string) {
    return this.call<boolean>("finish_direct_cross_page_move", { moveId });
  }
  moveManagedApplicationSubtrees(
    bindingGeneration: number,
    request: ManagedApplicationMoveSubtreesRequest,
  ) {
    return this.call<ManagedApplicationMoveSubtreesResult>("move_managed_application_subtrees", {
      bindingGeneration,
      request,
    });
  }
  acknowledgeManagedApplicationMove(
    bindingGeneration: number,
    episodeId: string,
    batchId: string,
  ) {
    return this.call<void>("acknowledge_managed_application_move", {
      bindingGeneration,
      episodeId,
      batchId,
    });
  }
  recoverManagedApplicationSubtrees(
    bindingGeneration: number,
    request: ManagedApplicationMoveSubtreesRequest,
  ) {
    return this.call<ManagedApplicationMoveSubtreesRecoveryResult>(
      "recover_managed_application_subtrees",
      { bindingGeneration, request },
    );
  }
  preflightManagedPageMutation(
    page: PageDto,
    baseRevision: string | null,
    bindingGeneration: number,
  ) {
    return this.call<ManagedPageMutationPreflightResult>("preflight_managed_page_mutation", {
      page,
      baseRevision,
      bindingGeneration,
    });
  }
  sparseV2Status() {
    return this.call<SparseV2Status>("sparse_v2_status");
  }
  async onSparseV2Status(cb: (event: SparseV2RuntimeStatusEvent) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<SparseV2RuntimeStatusEvent>("sparse-v2-status", (event) => cb(event.payload));
  }
  async onSparseV2Tick(cb: (event: SparseV2TickEvent) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<SparseV2TickEvent>("sparse-v2-tick", (event) => cb(event.payload));
  }
  async onSparseV2Error(cb: (event: SparseV2ErrorEvent) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<SparseV2ErrorEvent>("sparse-v2-error", (event) => cb(event.payload));
  }
  async onSparseV2ActivationProgress(
    bindingGeneration: number,
    cb: (progress: SparseV2ActivationProgressEvent["progress"]) => void
  ): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<SparseV2ActivationProgressEvent>("sparse-v2-activation-progress", (event) => {
      if (event.payload.binding_generation === bindingGeneration) cb(event.payload.progress);
    });
  }
  async activateSparseV2() {
    const result = await this.call<SparseV2Status>("activate_sparse_v2");
    this.bindingGeneration = result.binding_generation;
    return result;
  }
  async cancelSparseV2() {
    const result = await this.call<SparseV2CancelResult>("cancel_sparse_v2");
    this.bindingGeneration = result.binding_generation;
    return result;
  }
  async cancelSparseV2Cold(path: string) {
    const result = await this.call<SparseV2CancelResult>("cancel_sparse_v2_cold", { path });
    this.bindingGeneration = result.binding_generation;
    return result;
  }
  async prepareSparseV2Share() {
    const result = await this.call<SparseV2Status>("prepare_sparse_v2_share");
    this.bindingGeneration = result.binding_generation;
    return result;
  }
  async joinSparseV2Shared() {
    const result = await this.call<SparseV2Status>("join_sparse_v2_shared");
    this.bindingGeneration = result.binding_generation;
    return result;
  }
  async adoptSparseV2Shared() {
    const result = await this.call<SparseV2AdoptionResult>("adopt_sparse_v2_shared");
    this.bindingGeneration = result.binding_generation;
    return result;
  }
  sparseV2RecoveryLocation() {
    return this.call<string>("sparse_v2_recovery_location");
  }
  sparseV2Query(request: SparseV2QueryRequest) {
    return this.call<SparseV2QueryReply>("sparse_v2_query", { request });
  }
  sparseV2EditorLoad(request: SparseV2EditorLoadRequest) {
    return this.call<SparseV2EditorOutcome>("sparse_v2_editor_load", { request });
  }
  sparseV2EditorSave(request: SparseV2EditorSaveRequest) {
    return this.call<SparseV2EditorOutcome>("sparse_v2_editor_save", { request });
  }
  sparseV2Tick() {
    return this.call<SparseV2Tick>("sparse_v2_tick");
  }
  listAbsenceSweeps() {
    return this.call<SyncAbsenceSweepEvent[]>("list_absence_sweeps");
  }
  async onAbsenceSweepChanged(
    bindingGeneration: number,
    cb: (sweep: SyncAbsenceSweepEvent) => void,
  ): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<SyncAbsenceSweepChangedEvent>("absence-sweep-changed", (event) => {
      if (event.payload.binding_generation === bindingGeneration) cb(event.payload.sweep);
    });
  }
  reapplyAbsenceSweep(sweepId: string) {
    return this.call<SyncAbsenceSweepActionOutcome>("reapply_absence_sweep", { sweepId });
  }
  restoreAbsenceSweep(sweepId: string) {
    return this.call<SyncAbsenceSweepRestoreOutcome>("restore_absence_sweep", { sweepId });
  }
  keepAbsenceSweepDeletion(sweepId: string) {
    return this.call<void>("keep_absence_sweep_deletion", { sweepId });
  }
  sparseV2CleanShutdown() {
    return this.call<import("./types").SparseV2RuntimeStatus>("sparse_v2_clean_shutdown");
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
  renamePage(old: string, next: string, expectedPath?: string) {
    return this.call<RenameOutcome>("rename_page", { old, new: next, expectedPath });
  }
  publishHtml() {
    return this.call<[string, number]>("publish_html");
  }
  pagePrintHtml(name: string, opts: PrintOpts) {
    return this.call<string>("page_print_html", { name, opts });
  }
  parseQuery(text: string, dialect: QueryTextDialect, blockProperties?: [string, string][]) {
    return this.call<ParsedQuery>("query_parse", { text, dialect, blockProperties });
  }
  printQuery(
    query: Query,
    view: ViewSettings,
    dialect: QueryPrintDialect,
    preserveForm = false,
  ) {
    return this.call<string>("query_print", { query, view, dialect, preserveForm });
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
  runQuery(query: string) {
    return this.call<RefGroup[]>("run_query", { query });
  }
  exportQuerySubtrees(specs: QueryExportSpec[]) {
    return this.call<QueryExportBatch>("export_query_subtrees", { specs });
  }
  runAdvancedQuery(query: string, currentPage?: string) {
    return this.call<AdvancedQueryResult>("run_advanced_query", { query, currentPage });
  }
  queryFacets(autocomplete = false) {
    return this.call<[string, string[]][]>(
      "query_facets",
      autocomplete ? { autocomplete: true } : undefined,
    );
  }
  pageAliases() {
    return this.call<[string, string][]>("page_aliases");
  }
  pageIcons(names: string[]) {
    return this.call<Record<string, string>>("page_icons", { names });
  }
  existingPageNames(names: string[]) {
    return this.call<string[]>("existing_page_names", { names });
  }
  setFavorites(names: string[]) {
    return this.call<void>("set_favorites", { names });
  }
  /** Record which page holds the Favorites arrangement (`:tine/favorites-page`).
   *  Membership stays in `:favorites`; this names the page that owns groups and
   *  order, and is what keeps that page out of everyone's Linked References. */
  setFavoritesPage(name: string) {
    return this.call<void>("set_favorites_page", { name });
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
  openAsset(name: string) {
    return this.call<void>("open_asset", { name });
  }
  openPageFile(name: string, kind: "page" | "journal", path: string | undefined, reveal: boolean) {
    return this.call<void>("open_page_file", { name, kind, path: path || null, reveal });
  }
  editAssetExternal(name: string, command: string) {
    return this.call<void>("edit_asset_external", { name, command });
  }
  detectMediaEditor(id: string) {
    return this.call<string>("detect_media_editor", { id });
  }
  listOrphanAssets() {
    return this.call<AssetInfo[]>("list_orphan_assets");
  }
  trashAsset(name: string) {
    return this.call<void>("trash_asset", { name });
  }
  search(query: string, limit: number, lane?: string) {
    return this.call<RefGroup[]>("search", { query, limit, lane });
  }
  async runGraphSearch(source: string, pageLimit: number, blockLimit: number, lane = "graph-search", explain = false, scope?: QueryPageScope, options?: GraphSearchDisplayOptions) {
    const execution = await this.call<QueryExecution>("run_graph_search", {
      source, pageLimit, blockLimit, lane, explain,
      scope: scope ?? null,
      // Members are serialized EXPLICITLY rather than spread: an options object
      // carrying a key this build does not know would otherwise cross the
      // bridge and be refused by the command's `deny_unknown_fields`.
      options: options
        ? {
          pageMatchScope: options.pageMatchScope ?? null,
          pageView: options.pageView ?? null,
          blockView: options.blockView ?? null,
        }
        : null,
    });
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
  saveAsset(name: string, bytes: Uint8Array) {
    if (bytes.byteLength > ASSET_INGRESS_MAX_BYTES) return Promise.reject(new Error("asset exceeds 64 MiB ingress limit"));
    return this.call<string>("save_asset", { name, bytesB64: bytesToBase64(bytes) });
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
  duplicateJournalDiff(canonical: string, stray: string) {
    return this.call<SyncConflictDiff | null>("duplicate_journal_diff", { canonical, stray });
  }
  resolveDuplicateJournalDay(
    canonical: string,
    stray: string,
    decisions: Record<string, string>,
    baseRev: string,
    strayRev: string,
    preChoice?: string,
  ) {
    return this.call<PageDto>("resolve_duplicate_journal_day", {
      canonical, stray, decisions, baseRev, strayRev, preChoice,
    });
  }
  rescanGraphNow() {
    return this.call<number>("rescan_graph_now");
  }
  async onGraphRescanComplete(cb: (sequence: number) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<{ sequence: number }>("graph-rescan-complete", (event) => cb(event.payload.sequence));
  }
  listJournalFilenameMigrations() {
    return this.call<JournalFilenameMigration[]>("list_journal_filename_migrations");
  }
  applyJournalFilenameMigrations() {
    return this.call<number>("apply_journal_filename_migrations");
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
  activateEditor(
    path: string,
    intent: ActivationIntent,
    expectedRevision: ActivationExpectedRevision,
  ) {
    return this.call<EditorActivationHandle | null>("activate_editor", {
      path,
      intent,
      expectedRevision,
    });
  }
  activateAbsentEditor(name: string, kind: PageKind) {
    return this.call<EditorActivationHandle | null>("activate_absent_editor", { name, kind });
  }
  retireEditorActivation(path: string, activation: number) {
    return this.call<boolean>("retire_editor_activation", { path, activation });
  }
  presentConflictOverride(
    path: string,
    baseRev: string | null,
    activation: number,
    conflictEpoch: number,
  ) {
    return this.call<"authorised" | "superseded" | "withdrawn">("present_conflict_override", {
      path,
      baseRev,
      activation,
      conflictEpoch,
    });
  }
  mergePages(src: string, dst: string, rename?: { from: string; to: string }) {
    return this.call<void>("merge_pages", {
      src,
      dst,
      renameFrom: rename?.from ?? null,
      renameTo: rename?.to ?? null,
    });
  }
  renameFileToPage(path: string, newName: string) {
    return this.call<void>("rename_file_to_page", { path, newName });
  }
  listSyncConflicts() {
    return this.call<SyncConflict[]>("list_sync_conflicts");
  }
  listVcsMarkerConflicts() {
    return this.call<VcsMarkerConflict[]>("list_vcs_marker_conflicts");
  }
  syncConflictDiff(winner: string, conflict: string) {
    return this.call<SyncConflictDiff | null>("sync_conflict_diff", { winner, conflict });
  }
  conflictQueue() {
    return this.call<ConflictObject[]>("conflict_queue");
  }
  vcsMarkerConflictDiff(path: string) {
    return this.call<MarkerConflictDiff | null>("vcs_marker_conflict_diff", { path });
  }
  resolveVcsMarkerConflict(
    path: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    preChoice?: "mine" | "theirs" | "union"
  ) {
    return this.call<void>("resolve_vcs_marker_conflict", {
      path,
      decisions,
      baseRev,
      preChoice: preChoice ?? "union",
    });
  }
  textBlockDiff(mine: string, theirs: string, format?: "md" | "org") {
    return this.call<SyncConflictDiff>("text_block_diff", { mine, theirs, format });
  }
  textBlockDiff3(base: string, mine: string, theirs: string, format?: "md" | "org") {
    return this.call<SyncConflictDiff>("text_block_diff3", { base, mine, theirs, format });
  }
  liveSaveConflictDiff(page: PageDto, baseRev: string | null, conflictEpoch: number) {
    return this.call<SyncConflictDiff>("live_save_conflict_diff", {
      page,
      baseRev,
      conflictEpoch,
    });
  }
  captureLiveSaveConflict(page: PageDto, baseRev: string | null, conflictEpoch: number) {
    return this.call<LiveSaveConflictCapture | null>("capture_live_save_conflict", {
      page,
      baseRev,
      conflictEpoch,
    });
  }
  loadConflictCapsules(root: string) {
    return this.call<ConflictObject[]>("load_conflict_capsules", { root });
  }
  storeConflictCapsule(root: string, capsule: ConflictObject) {
    return this.call<void>("store_conflict_capsule", { root, capsule });
  }
  retireConflictCapsule(root: string, pageName: string) {
    return this.call<void>("retire_conflict_capsule", { root, pageName });
  }
  conflictCapsuleDiff(conflict: ConflictObject) {
    const live = conflict.live;
    if (!live) return Promise.reject(new Error("conflict capsule has no retained draft"));
    return this.call<ConflictCapsuleReview>("conflict_capsule_diff", {
      page: live.page,
      baseRev: live.base_rev,
      conflictEpoch: live.conflict_epoch,
      baseText: live.base_text,
      diskRev: live.disk_rev,
    });
  }
  resolveConflictCapsule(
    conflict: ConflictObject,
    authority: ConflictCapsuleAuthority,
    decisions: Record<string, MergeDecision>,
    preChoice: "mine" | "theirs" | "union" = "union",
  ) {
    const live = conflict.live;
    if (!live) return Promise.reject(new Error("conflict capsule has no retained draft"));
    return this.call<PageDto>("resolve_conflict_capsule", {
      page: live.page,
      baseRev: live.base_rev,
      authority,
      decisions,
      preChoice,
    });
  }
  durableLiveSaveConflictDiff(page: PageDto, baseText: string | null) {
    return this.call<SyncConflictDiff>("durable_live_save_conflict_diff", { page, baseText });
  }
  resolveDurableLiveSaveConflict(
    page: PageDto,
    expectedDiskRev: string,
    decisions: Record<string, MergeDecision>,
    preChoice: "mine" | "theirs" | "union" = "union",
  ) {
    return this.call<PageDto>("resolve_durable_live_save_conflict", {
      page,
      expectedDiskRev,
      decisions,
      preChoice,
    });
  }
  resolveLiveSaveConflict(
    page: PageDto,
    baseRev: string | null,
    conflictEpoch: number,
    decisions: Record<string, MergeDecision>,
    preChoice: "mine" | "theirs" | "union" = "union",
  ) {
    return this.call<PageDto>("resolve_live_save_conflict", {
      page,
      baseRev,
      conflictEpoch,
      decisions,
      preChoice,
    });
  }
  resolveSyncConflict(
    winner: string,
    conflict: string,
    decisions: Record<string, MergeDecision>,
    baseRev: string,
    conflictRev: string,
    mergeBaseRev?: string | null,
    preChoice?: "mine" | "theirs" | "union"
  ) {
    return this.call<PageDto>("resolve_sync_conflict", {
      winner,
      conflict,
      decisions,
      baseRev,
      conflictRev,
      mergeBaseRev: mergeBaseRev ?? null,
      preChoice: preChoice ?? "union",
    });
  }
  trashSyncConflict(conflict: string) {
    return this.call<void>("trash_sync_conflict", { conflict });
  }
  async onConflictsChanged(cb: () => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen("conflicts-changed", () => cb());
  }
  importAsset(path: string, name?: string) {
    return this.call<string>("import_asset", { path, name });
  }
  importNativeCapture(path: string, name: string) {
    return this.call<string>("import_native_capture", { path, name });
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
    if (bytes.byteLength > ASSET_INGRESS_MAX_BYTES) return Promise.reject(new Error("image exceeds 64 MiB clipboard limit"));
    return this.call<void>("copy_image_to_clipboard", { bytesB64: bytesToBase64(bytes) });
  }
  readHighlights(pdf: string) {
    return this.call<Highlight[]>("read_highlights", { pdf });
  }
  openPdf(pdf: string, label: string) {
    return this.call<PdfState>("open_pdf", { pdf, label });
  }
  writeHighlights(pdf: string, label: string, highlights: Highlight[], baseIds: string[]) {
    return this.call<void>("write_highlights", { pdf, label, highlights, baseIds });
  }
  writePdfViewState(pdf: string, page: number, scale: number) {
    return this.call<void>("write_pdf_view_state", { pdf, page, scale });
  }
  savePdfAreaImage(pdf: string, page: number, id: string, stamp: number, bytes: Uint8Array) {
    if (bytes.byteLength > ASSET_INGRESS_MAX_BYTES) return Promise.reject(new Error("PDF area image exceeds 64 MiB ingress limit"));
    return this.call<string>("save_pdf_area_image", {
      pdf,
      page,
      id,
      stamp,
      bytesB64: bytesToBase64(bytes),
    });
  }
  rollbackPdfAreaImage(pdf: string, page: number, id: string, stamp: number) {
    return this.call<void>("rollback_pdf_area_image", { pdf, page, id, stamp });
  }
  async onGraphChanged(cb: (c: GraphChange) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<GraphChange>("graph-changed", (e) => cb(e.payload));
  }
  async onGraphChangedBulk(cb: (bulk: GraphChangedBulk) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<GraphChangedBulk>("graph-changed-bulk", (e) => cb(e.payload));
  }
  async onAssetChanged(cb: (batch: AssetChangedBatch) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<AssetChangedBatch>("asset-changed", (e) => cb(e.payload));
  }
  async onGraphConfigChanged(cb: (meta: GraphMeta) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<GraphMeta>("graph-config-changed", (e) => cb(e.payload));
  }
  async onQueryProjectionChanged(cb: () => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<number>("query-projection-changed", (event) => {
      if (event.payload === this.bindingGeneration) cb();
    });
  }
  async onSparseV2Changed(cb: () => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen("sparse-v2-changed", () => cb());
  }
  async onManagedSyncError(cb: (message: string) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<string>("managed-sync-error", (e) => cb(e.payload));
  }
  async onGraphWatchError(cb: (message: string) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<string>("graph-watch-error", (e) => cb(e.payload));
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
  loadWorkspaces() {
    return this.call<string>("load_workspaces");
  }
  saveWorkspaces(data: string) {
    return this.call<void>("save_workspaces", { data });
  }
  loadNotices() {
    return this.call<string>("load_notices");
  }
  saveNotices(data: string) {
    return this.call<void>("save_notices", { data });
  }
  takeIdentifierMigrationNotice() {
    return this.call<boolean>("take_identifier_migration_notice");
  }
  takeDataHomeFallbackNotice() {
    return this.call<string | null>("take_data_home_fallback_notice");
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
  diagnosticReport(buildCommit: string, buildTime: string) {
    return this.call<DiagnosticReport>("diagnostic_report", { buildCommit, buildTime });
  }
  saveDiagnosticReport(buildCommit: string, buildTime: string) {
    return this.call<boolean>("save_diagnostic_report", { buildCommit, buildTime });
  }
  clearDiagnostics() {
    return this.call<void>("clear_diagnostics");
  }
  createGraphVerification(operationId: string) {
    return this.call<GraphVerificationReport>("create_graph_verification", { operationId });
  }
  cancelGraphVerification(operationId: string) {
    return this.call<void>("cancel_graph_verification", { operationId });
  }
  saveGraphVerificationReport(text: string) {
    return this.call<boolean>("save_graph_verification_report", { text });
  }
  async onGraphVerificationProgress(cb: (progress: GraphVerificationProgress) => void): Promise<() => void> {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<GraphVerificationProgress>("graph-verification-progress", (event) => cb(event.payload));
  }
  diagnosticFrontendEvent(
    kind: "uncaught_error" | "unhandled_rejection" | "heartbeat_delay" | "updater_failure",
    line?: number,
    column?: number,
    delayMs?: number,
    updaterStage?: string,
    updaterCause?: string,
  ) {
    return this.call<void>("diagnostic_frontend_event", {
      kind,
      line,
      column,
      delayMs,
      updaterStage,
      updaterCause,
    });
  }
  diagnosticSessionActive(active: boolean) {
    return this.call<void>("diagnostic_session_active", { active });
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
    _backend = isTauri() ? new TauriBackend() : mockBackend();
  }
  return _backend;
}

// Browser/test fallback for the native-only recovery channel. Keeping this
// adapter beside the Backend boundary avoids teaching the fixture backend about
// app-data files while preserving the same async contract in UI tests.
const browserConflictCapsules = new Map<string, Map<string, ConflictObject>>();

export async function loadConflictCapsules(root: string): Promise<ConflictObject[]> {
  const current = backend();
  if (current.loadConflictCapsules) return current.loadConflictCapsules(root);
  return [...(browserConflictCapsules.get(root)?.values() ?? [])]
    .map((capsule) => structuredClone(capsule));
}

/** Synchronous browser/test cache view. Native callers return null and must
 * await the app-private file before graph activation. */
export function cachedConflictCapsules(root: string): ConflictObject[] | null {
  if (backend().loadConflictCapsules) return null;
  return [...(browserConflictCapsules.get(root)?.values() ?? [])]
    .map((capsule) => structuredClone(capsule));
}

export async function storeConflictCapsule(root: string, capsule: ConflictObject): Promise<void> {
  const current = backend();
  if (current.storeConflictCapsule) return current.storeConflictCapsule(root, capsule);
  const graph = browserConflictCapsules.get(root) ?? new Map<string, ConflictObject>();
  graph.set(capsule.page_name, structuredClone(capsule));
  browserConflictCapsules.set(root, graph);
}

export async function retireConflictCapsule(root: string, pageName: string): Promise<void> {
  const current = backend();
  if (current.retireConflictCapsule) return current.retireConflictCapsule(root, pageName);
  const graph = browserConflictCapsules.get(root);
  graph?.delete(pageName);
  if (graph?.size === 0) browserConflictCapsules.delete(root);
}

/** One semantic review surface. Native dispatch is storage-mode aware; the
 * browser fallback models the Direct path for UI tests and demos. */
export async function reviewConflictCapsule(
  conflict: ConflictObject,
): Promise<ConflictCapsuleReview> {
  const current = backend();
  if (current.conflictCapsuleDiff) return current.conflictCapsuleDiff(conflict);
  const live = conflict.live;
  if (!live) throw new Error("conflict capsule has no retained draft");
  if (live.disk_rev !== undefined) {
    const diff = await current.durableLiveSaveConflictDiff(live.page, live.base_text ?? null);
    return {
      diff,
      authority: { kind: "direct_durable", expected_disk_rev: diff.conflict_rev },
    };
  }
  return {
    diff: await current.liveSaveConflictDiff(live.page, live.base_rev, live.conflict_epoch),
    authority: { kind: "direct_live", conflict_epoch: live.conflict_epoch },
  };
}

export async function resolveConflictCapsule(
  conflict: ConflictObject,
  authority: ConflictCapsuleAuthority,
  decisions: Record<string, MergeDecision>,
  preChoice: "mine" | "theirs" | "union" = "union",
): Promise<PageDto> {
  const current = backend();
  if (current.resolveConflictCapsule) {
    return current.resolveConflictCapsule(conflict, authority, decisions, preChoice);
  }
  const live = conflict.live;
  if (!live) throw new Error("conflict capsule has no retained draft");
  if (authority.kind === "direct_durable") {
    return current.resolveDurableLiveSaveConflict(
      live.page,
      authority.expected_disk_rev,
      decisions,
      preChoice,
    );
  }
  if (authority.kind === "direct_live") {
    return current.resolveLiveSaveConflict(
      live.page,
      live.base_rev,
      authority.conflict_epoch,
      decisions,
      preChoice,
    );
  }
  throw new Error("the browser conflict demo has no managed actor");
}

/** Test-only backend injection for delayed/rejected native-boundary proofs. */
export function __setBackendForTest(value: Backend | null): void {
  if (import.meta.env.MODE === "test") _backend = value;
}

/** OG-visible graph property keys/values for the block editor. Kept separate
 * from query-builder facets even though both share the registered IPC command. */
export function autocompleteFacets(): Promise<[string, string[]][]> {
  return backend().queryFacets(true);
}
