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
  | "updater_failure" | "updater_manual_only" | "close_discarded_unsaved" | "error_toast" | "journal_feed";

/** Closed tokens only: journal names, paths and backend error text never enter
 * the always-on report (I-5). These describe the read/publication boundary. */
export type JournalFeedReason =
  | "template-deferred" | "template-stale" | "template-error" | "template-day-changed"
  | "active-edit" | "owner-retired" | "window-unbound" | "read-stale"
  | "day-mismatch" | "generation-changed" | "admission-stale" | "admission-refused"
  | "feed-error" | "route-unpublished";

export type JournalFeedCause = "empty-rejection" | "other-rejection";

/** Why a close discarded drafts: a save failed, or saves were still running. */
export type DiscardReason = "failed" | "still-saving";

export interface DiagnosticFrontendFields {
  journalFeedReason?: JournalFeedReason;
  journalFeedCause?: JournalFeedCause;
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
  packaged: boolean | null;
  /** A portable Windows instance (`tine-data` beside the exe, GH #719): the
   *  updater's installer would add a second, installed copy, so it updates by
   *  download. Absent from an older backend. */
  portable: boolean;
  /** The package manager that built this copy (compile-time `TINE_PACKAGED_BY`,
   *  tine#187): it owns updates, so the updater never checks, notifies or
   *  installs and About names it. Null/absent = not a package-manager build. */
  packaged_by?: string | null;
}

/** Settings → Graph → Git (ADR 0078): the bound graph's autocommit and sync state. On a
 *  platform without git only `available: false` is sent. */
export type GitPolicy = "off" | "commit" | "sync-on-demand" | "sync-automatically";
/** The last sync cycle (ADR 0078 §4): `code` is its refusal-table row. */
export interface GitSyncStatus {
  /** A cycle is running now. */
  running: boolean;
  /** Unix seconds when the last cycle finished. */
  at: number | null;
  code: string | null;
  message: string | null;
  /** The file a held or refused pull names. */
  path: string | null;
  /** Unix seconds of the last cycle that succeeded. */
  ok_at: number | null;
  failures: number;
  /** Shown outside Settings too. */
  persistent: boolean;
  line_endings?: GitLineEndingsProbe | null;
}
export interface GitLineEndingsProbe {
  safe: boolean;
  differing: number;
  sample: string | null;
  reason: string | null;
}
export interface GitRemote {
  name: string;
  url: string;
  /** `https://host[:port]` when the remote takes a sign-in. */
  https_origin: string | null;
}
/** Paths index roots, then each child list, in the exact currentText snapshot.
 * They are structural addresses, not runtime document ids or persisted id::. */
export type SeenBlockPath = number[];
export interface SeenPageChanges {
  available: boolean;
  /** Why the page is unavailable; on an available result, a note on how the
   * baseline was found (rewritten history, first-open fallback, walk bound). */
  reason: string | null;
  head_commit: string | null;
  /** null means the empty-tree baseline when tracked; unavailable/absent also
   * has null and no marks. The UI only needs the marks and availability. */
  baseline_commit: string | null;
  /** SHA-256 of the supplied UTF-8 currentText. Discard stale responses. */
  current_text_sha256: string;
  identity_known: boolean;
  /** More than 200 touching commits, or a baseline past the 500-commit walk:
   * the net diff counts every author. */
  unattributed: boolean;
  /** Only other-author committed changes still unchanged in currentText. */
  blocks: { block_path: SeenBlockPath; persisted_id: string | null }[];
  removed: {
    /** One removed block body, properties included; descendants have own rows. */
    text: string;
    baseline_block_path: SeenBlockPath;
    /** null = page roots. */
    parent_path: SeenBlockPath | null;
    /** null = before the first child/root of parent_path. */
    previous_sibling_path: SeenBlockPath | null;
  }[];
}
export interface GitAutocommitStatus {
  available: boolean;
  /** The graph is inside a git work tree. */
  repository?: boolean;
  branch?: string | null;
  /** Why Tine cannot commit here: [code, message]. */
  unusable?: [string, string] | null;
  policy?: GitPolicy;
  interval_minutes?: number;
  last?: {
    /** One `.gitignore` setup event, retained until the graph worker ends:
     *  the file, and the lines added (or attempted, when it failed). */
    gitignore_added?: string | null;
    gitignore_lines?: string[];
    gitignore_failed?: boolean;
    repack_deferred?: boolean;
    /** Unix seconds of this process's last autocommit. */
    committed_at: number | null;
    commit: string | null;
    refused: [string, string] | null;
    /** An `index.lock` held through several cycles over 30 minutes. */
    stale_lock: string | null;
    sync?: GitSyncStatus;
  };
  /** The tracked remote, when sync can run here. */
  remote?: GitRemote | null;
  /** Why the sync options are refused: [code, message]. */
  sync_unusable?: [string, string] | null;
  line_endings?: GitLineEndingsProbe | null;
  /** The username signed in for the remote's host on this device. */
  signed_in?: string | null;
}

export interface BackupInfo {
  /** Present for an on-demand old-location snapshot; restore requires confirmation. */
  old_root?: string;
  /** What `restoreBackup` takes: `zip:<name>` or `dir:<name>` (a zip and a
   *  legacy snapshot directory with the same name are distinct backups). */
  id: string;
  /** The snapshot name: `YYYY-MM-DD_HH-MM-SS` (UTC), optionally followed by a
   *  suffix such as `-pre-restore` and a same-second counter. */
  stamp: string;
  files: number;
  /** `damaged` and `unsupported` (made by a newer Tine) cannot be restored. */
  status: "ok" | "damaged" | "unsupported";
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

export type GraphFolderPickResult =
  | { status: "picked"; path: string }
  // Android could not resolve the selected provider folder to local files.
  // A status survives the Rust bridge, which stringifies plugin rejections.
  | { status: "local-folder-required"; path?: undefined }
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
  display_path?: string;
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
  | { kind: "loaded" | "already_current"; meta: GraphMeta; binding_generation: number; config_problem?: { kind: "config-read"; message: string } | null;
      /** Crash-draft storage status (STEP3 §4); null with no page host. */
      draft_status?: import("./document").DraftStatus | null }
  | { kind: "focused_existing"; window_label: string };

export interface CaptureGraphBindingResult {
  binding_generation: number;
}

export interface GraphAccessInspection {
  graph_root: string;
  display_graph_root?: string;
  display_external_assets_path?: string | null;
  wsl_rootfs?: boolean;
  external_assets_path: string | null;
  approved: boolean;
}

/** Typed result of `trashAsset`: `referenced` = another reference remains, file kept. */
export type TrashAssetOutcome = "trashed" | "referenced";

/** Native publication delta of a watcher event.
 * Values are final counts for only the changed targets; zero clears a badge. */
export interface GraphAnswersChange {
  rev: string;
  inventoryChanged: boolean;
  blockRefCounts: Record<string, number>;
  /** Absent with older hosts and synthetic legacy notifications. */
  summary?: ChangeSummary;
}

/** Publication invalidation evidence for the shared keyed refresh layer.
 * `blocks` includes runtime and persisted ids, plus affected ancestors.
 * `props` maps source-page/alias keys to changed property keys; `linksTo`
 * includes old and new explicit targets even if membership did not change. */
export interface ChangeSummary {
  rev: string;
  pages: string[];
  blocks: string[];
  linksTo: string[];
  /** S2 fields are optional for older hosts. */
  textTo?: string[];
  tagsTo?: string[];
  tagTableTo?: string[];
  refsTo?: string[];
  /** Mounted query subscription identities affected by this publication. */
  queryTo?: string[];
  /** S5 hints; absent on older hosts means conservative family invalidation. */
  days?: boolean;
  journals?: boolean;
  templates?: boolean;
  props: Record<string, string[]>;
  inventoryChanged: boolean;
  full: boolean;
}

/** `graph-custom-css-changed`: an outside actor created, replaced or deleted
 *  `logseq/custom.css`. Carries only the binding; the window re-reads it. */
export interface CustomCssChange {
  global?: boolean;
  binding_generation?: number;
}
