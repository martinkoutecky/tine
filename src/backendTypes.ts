/** Plain data shapes carried by `Backend` (diagnostics, backups, graph change
 *  events). Split from `backend.ts` along the type/implementation seam; import
 *  them from `./backend`, which re-exports every one. */
import type { GraphMeta, PageDto } from "./types";
import type { EditKinds } from "./editKind";

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
  /** Running inside a Flatpak sandbox: it cannot update itself in place, so the
   *  updater offers a manual download there. */
  flatpak: boolean;
}

export interface BackupInfo {
  /** `YYYY-MM-DD_HH-MM-SS` (UTC). */
  stamp: string;
  files: number;
}

export interface GraphChange {
  answers?: import("./backend").GraphAnswersChange | null;
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

/** The parser-comparison input: every eligible file, and `path: reason` for
 *  each file left out (unreadable, undecodable, over the size limit). */
export interface GraphSources {
  files: GraphSourceFile[];
  skipped: string[];
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

/** The desktop system tray's state after `tray_apply` (src-tauri/src/tray.rs).
 *  `supported` is false on mobile (and the mock): the tray settings are absent.
 *  `problem` is a fixed sentence when an icon was requested and the desktop has
 *  no tray to show it (the options are then off for this run). */
export interface TrayStatus {
  supported: boolean;
  active: boolean;
  problem: string | null;
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
  | { kind: "loaded" | "already_current"; meta: GraphMeta; binding_generation: number; config_problem?: { kind: "config-read"; message: string } | null }
  | { kind: "focused_existing"; window_label: string };

export interface CaptureGraphBindingResult {
  binding_generation: number;
}

export interface GraphAccessInspection {
  graph_root: string;
  external_assets_path: string | null;
  approved: boolean;
}

/** Typed result of `trashAsset`: `referenced` = another reference remains, file kept. */
export type TrashAssetOutcome = "trashed" | "referenced";

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

/** `graph-custom-css-changed`: an outside actor created, replaced or deleted
 *  `logseq/custom.css`. Carries only the binding; the window re-reads it. */
export interface CustomCssChange {
  binding_generation?: number;
}
