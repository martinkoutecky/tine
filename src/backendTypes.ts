/** Plain data shapes carried by `Backend` (diagnostics, backups, graph change
 *  events). Split from `backend.ts` along the type/implementation seam; import
 *  them from `./backend`, which re-exports every one. */
import type { GraphMeta } from "./types";

export interface DebugInfo {
  enabled: boolean;
  path: string;
  /** The flight recorder is persisted in app data for this run. */
  recorderActive: boolean;
  /** The previous run ended without an orderly shutdown. */
  previousExitUnclean: boolean;
}

export interface DiagnosticReport {
  text: string;
  suggestedFileName: string;
}

export type DiagnosticFrontendKind =
  | "uncaught_error" | "unhandled_rejection" | "heartbeat_delay"
  | "updater_failure" | "updater_manual_only" | "close_discarded_unsaved" | "error_toast";

/** Why a close discarded drafts: a save failed, or saves were still running. */
export type DiscardReason = "failed" | "still-saving";

export interface DiagnosticFrontendFields {
  line?: number;
  column?: number;
  delayMs?: number;
  updaterStage?: string;
  updaterCause?: string;
  closeReason?: DiscardReason;
  pages?: number;
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
  binding_generation?: number;
  path?: string;
  name: string;
  kind: "journal" | "page";
  created: boolean;
  removed: boolean;
}

/** One coalesced watcher publication of files outside actors created, replaced
 *  or deleted under the graph's assets capability. Paths are relative to
 *  `assets/`; absolute device paths never cross the bridge. */
export interface AssetChangedBatch {
  paths: string[];
  binding_generation?: number;
}

export interface GraphConfigChange {
  binding_generation: number;
  meta: GraphMeta;
}

/** One raw graph file, as returned by `graphSourceFiles` — the input to the
 *  in-app lsdoc↔mldoc diff panel. `text` is the file's bytes exactly as on disk. */
export interface GraphSourceFile {
  rel: string;
  text: string;
  format: "md" | "org";
  bytes: number;
}

export type GraphFolderPickResult =
  | { status: "picked"; path: string }
  | { status: "permission-requested" | "permission-needed" | "cancelled"; path?: string };

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

export type PluginRegistryCacheLoad =
  | { kind: "absent" }
  | { kind: "envelope"; envelope: PluginRegistryCacheEnvelope }
  | { kind: "unsafe"; reason: string };

export type LoadGraphResult =
  | { kind: "loaded" | "already_current"; meta: GraphMeta; binding_generation: number }
  | { kind: "focused_existing"; window_label: string };

export interface CaptureGraphBindingResult {
  binding_generation: number;
}

export interface GraphAccessInspection {
  graph_root: string;
  external_assets_path: string | null;
  approved: boolean;
}
