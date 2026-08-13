// TS mirrors of the Rust DTOs (crates/logseq-core/src/model.rs).

export type PageKind = "journal" | "page";

export interface BlockDto {
  id: string;
  raw: string;
  collapsed: boolean;
  children: BlockDto[];
  /** Ancestor first-lines (search/reference results only). */
  breadcrumb?: string[];
  /** Synthetic read-only backlink row sourced from page-level properties. */
  page_property?: boolean;
  // M1 block-header facets, computed once off the Rust lsdoc projection and shipped
  // so the frontend reads them off the DTO (no parse on load) instead of re-deriving
  // with its own scanner. Omitted by the backend when empty (see model.rs BlockDto).
  marker?: string;
  priority?: string;
  heading_level?: number;
  scheduled?: string;
  deadline?: string;
  tags?: string[];
  properties?: [string, string][];
}

/** Node-and-byte-bounded subtree used only for block-reference previews/exports. */
export interface BlockPreview {
  group: RefGroup;
  /** Nodes omitted after the requested preview construction budget. */
  truncated: number;
}

/** One rendered query macro requested by a Copy / Export session. */
export interface QueryExportSpec {
  key: string;
  query: string;
  advanced: boolean;
}

/** Native hierarchy projection for one query macro. */
export interface QueryExportResult {
  key: string;
  groups: RefGroup[];
  shown: number;
  total: number;
  omitted_nodes: number;
}

/** Every result in this batch shared one native root/node/byte budget. */
export interface QueryExportBatch {
  results: QueryExportResult[];
  omitted_queries: number;
}

/** On-disk page format: markdown (default) or org. */
export type Format = "md" | "org";

export interface PageDto {
  name: string;
  kind: PageKind;
  title: string;
  pre_block: string | null;
  blocks: BlockDto[];
  /** Hash of the on-disk file at load time — the save baseline. `null`/absent for
   *  a page with no file yet. Sent back on save to detect external changes. */
  rev?: string | null;
  /** Format this page is stored in (drives org vs markdown inline rendering). */
  format?: Format;
  /** True for a source page Tine can't structurally round-trip: shown but not
   *  editable, so Tine never rewrites (and risks corrupting) it. */
  read_only?: boolean;
  /** Graph-root-relative path of the file this page was loaded from
   *  (`journals/2026_06_26.org`). Echoed back on save so a page pinned to a
   *  SPECIFIC file (a duplicate-day stray, #21) saves to its own file rather than
   *  being re-resolved by name to the canonical one. Empty for a brand-new page. */
  path?: string;
  /** Which live editor instance is issuing this save.
   *
   *  Stamped from the activation registry when the DTO is built, NOT carried on
   *  `FeedPage` — a token on the page object is copied by every clone and history
   *  snapshot, and the copy would then claim an identity it does not have.
   *  Absent for an editor-less writer; legal on the ordinary save path, refused on
   *  the override path. (GH #254 increment 3.) */
  activation?: number;
  /** Bundled in-app Guide page: read-only, ephemeral, and excluded from normal
   *  graph persistence/search/reference surfaces. */
  guide?: boolean;
}

/** What an activation request means for a path that already has a live editor. */
export type ActivationIntent = "reuse" | "replace";

/** The exact revision of a DTO being installed, or null only for the mounted
 * save fallback whose ordinary base-revision guard remains the write authority. */
export type ActivationExpectedRevision = string | null;

/** The outcome of activating an editor. */
export interface EditorActivationHandle {
  activation: number;
  /** The exact path this activation is live for. For an absent editor this is the
   *  prospective target resolved at activation time. */
  target: string;
  /** True when no file existed at activation time. */
  prospective: boolean;
}

/** Result of saving an editor page.
 *
 * Direct Files may return the activation that now owns a successful first
 * creation (including its resolved target). Managed storage keeps its existing
 * revision-only semantics and therefore omits `activation`. The string arm is
 * retained for compatibility with older/mock managed backends during the
 * transport transition. */
export type SavePageResult =
  | string
  | {
      revision: string;
      activation?: EditorActivationHandle;
    };

/** One authoritative Journals-feed transaction.  Cursor fields are ordinal
 * journal days, never counts of returned DTOs (a selected file may vanish). */
export interface JournalFeedPage {
  pages: PageDto[];
  next_before_day: number | null;
  done: boolean;
  as_of_day: number;
}

export interface GuidePage {
  title: string;
  markdown: string;
  page: PageDto;
}

export interface GuideCopyResult {
  name: string;
  created: boolean;
  created_pages?: string[];
  skipped_pages?: string[];
  copied_assets?: string[];
}

export interface TemplateDto {
  name: string;
  blocks: BlockDto[];
  /** Page the template's defining block lives on (to jump to it for editing). */
  page: string;
  kind: PageKind;
}

export interface PageEntry {
  name: string;
  kind: PageKind;
  date_key: number | null;
  /** Graph-root-relative path of this specific file. Use it to open basename
   *  collisions without re-resolving by display name. */
  path: string;
}

/** An orphaned asset file (no block references it) — for the cleanup UI. */
export interface AssetInfo {
  name: string;
  size: number;
  /** Last-modified time as Unix seconds (≈ when the file entered the graph). */
  modified: number | null;
}

/** Asset trash totals plus protected non-asset recovery entries in logseq/.tine-trash. */
export interface TrashStats {
  count: number;
  bytes: number;
  pages: number;
  journals: number;
  conflicts: number;
  other: number;
}

/** One file in a journal-day conflict (duplicate files for the same date). */
export interface JournalFile {
  name: string;
  /** Graph-root-relative path — lets the UI navigate straight to THIS file even
   *  when it shares a date with the canonical one (#21). */
  path: string;
  preview: string;
  canonical: boolean; // name is the date stem (yyyy_MM_dd) — the one to keep
}

/** A journal day that resolves to >1 file (e.g. a date-stem file + a title-named
 *  one), surfaced so the user can reconcile them. */
export interface JournalConflict {
  title: string;
  files: JournalFile[];
}

/** A sync-tool conflict copy (Syncthing/Dropbox) shadowing a real page — a
 *  `*.sync-conflict-*.md` (or Dropbox `(conflicted copy)`) file. Excluded from
 *  the page list; surfaced here so the user can review + merge it. */
export interface SyncConflict {
  /** Graph-root-relative path of the conflict copy. */
  path: string;
  /** Display name of the page it shadows (decoded page name / journal title). */
  base_name: string;
  /** Graph-root-relative path of the winning file, if it still exists. */
  base_path: string | null;
  kind: PageKind;
  /** Device/timestamp suffix from the conflict filename (best-effort label). */
  tag: string;
  /** One-line content preview of the conflict copy. */
  preview: string;
}

export interface SparseV2WatcherStatus {
  latest_enqueue: number;
  acknowledged: number;
  drain_in_flight: boolean;
  pending: boolean;
  pending_requires_full_scan: boolean;
  deferred: boolean;
  quiescing: boolean;
  sequence_exhausted: boolean;
}

export interface SparseV2Tick {
  state: string;
  detail: string | null;
  epoch: number | null;
}

/** A watcher update scoped to the graph binding that produced it. */
export interface SparseV2TickEvent {
  binding_generation: number;
  tick: SparseV2Tick;
}

/** A watcher failure scoped to the graph binding that produced it. */
export interface SparseV2ErrorEvent {
  binding_generation: number;
  message: string;
}

export interface SparseV2RuntimeStatus {
  lifecycle: "active" | "terminal" | "stopped_safe" | "stopped_crashed";
  recovery: "first_promotion" | "resumed_own_unsafe" | "adopted_safe_handoff" | "took_over_crashed_unsafe" | null;
  watcher: SparseV2WatcherStatus;
  last_tick: SparseV2Tick | null;
  detail: string | null;
  shared_role: "initiator" | "joiner" | null;
  shared_phase: "share_prepared" | "joining" | "active" | null;
  provider_pending: number;
}

export type SparseV2Availability =
  | { state: "legacy_default" }
  | { state: "joinable"; descriptor_digest: string }
  | { state: "active" }
  | { state: "retryable"; stage: "absent" | "shadow_import" | "verified_local" | "local_active"; detail: string }
  | { state: "blocked"; reason_code: string; scenario_id: string }
  | { state: "refused"; reason_code: string; scenario_id: string; detail: string | null };

/** Native, binding-scoped advisory envelope for pre-mutation bulk admission.
 * The managed actor remains the final save authority. */
export type ApplicationPageAdmission =
  | { binding_generation: number; authority: "direct" }
  | {
      binding_generation: number;
      authority: "managed_writable";
      application_save_page_blocks: number;
      application_page_request_text_bytes: number;
      application_page_max_depth: number;
    }
  | { binding_generation: number; authority: "managed_unavailable" };

export interface ManagedApplicationMoveRawRewrite {
  expected_raw: string;
  desired_raw: string;
}

export interface ManagedApplicationMoveRoot {
  identity: string;
  raw_rewrite: ManagedApplicationMoveRawRewrite | null;
}

export type ManagedApplicationMovePlacement =
  | { placement: "root"; position: number }
  | { placement: "child"; parent_identity: string; position: number };

export interface ManagedApplicationMoveSubtreesRequest {
  episode_id: string;
  source_path: string;
  source_revision: string;
  destination_path: string;
  destination_revision: string;
  roots: ManagedApplicationMoveRoot[];
  placement: ManagedApplicationMovePlacement;
  admission: {
    application_save_page_blocks: number;
    application_page_request_text_bytes: number;
    application_page_max_depth: number;
  };
}

export type ManagedApplicationMoveConflict =
  | "stale_source"
  | "stale_destination"
  | "missing_source"
  | "missing_destination"
  | "ambiguous_source"
  | "ambiguous_destination"
  | "same_page"
  | "read_only"
  | "missing_or_foreign_root"
  | "duplicate_root"
  | "nested_root"
  | "missing_or_foreign_parent"
  | "invalid_placement"
  | "expected_raw_changed"
  | "admission_changed"
  | "destination_too_large"
  | "destination_too_deep"
  | "destination_text_too_large"
  | "episode_mismatch"
  | "episode_not_committed"
  | "batch_collision";

export interface ManagedApplicationMovedPage {
  page: PageDto;
  revision: string;
}

export type ManagedApplicationMovePhase =
  | "bindings"
  | "planning"
  | "draft"
  | "capture"
  | "finalize"
  | "tail_reservation"
  | "publication"
  | "archive_stage"
  | "tail_admission"
  | "sqlite_drain"
  | "projection_drain";

export type ManagedApplicationMoveDeferred =
  | { status: "retryable_external_work" }
  | {
      status: "retryable_retained_publication";
      batch_id: string;
      phase: ManagedApplicationMovePhase;
    }
  | {
      status: "blocked_recovery";
      batch_id: string | null;
      phase: ManagedApplicationMovePhase;
      retained_publication: boolean;
    }
  | {
      status: "revoked";
      batch_id: string | null;
      phase: ManagedApplicationMovePhase;
    };

export type ManagedApplicationMoveSubtreesOutcome =
  | {
      status: "committed";
      episode_id: string;
      batch_id: string;
      recovered: boolean;
      source: ManagedApplicationMovedPage;
      destination: ManagedApplicationMovedPage;
    }
  | { status: "no_commit"; episode_id: string; reason: ManagedApplicationMoveConflict }
  | { status: "deferred"; episode_id: string; state: ManagedApplicationMoveDeferred };

/** Binding-tagged X1 result. X2 may install it only if this generation and its
 * page instances still own the busy episode. */
export interface ManagedApplicationMoveSubtreesResult {
  binding_generation: number;
  application_page_admission: ApplicationPageAdmission;
  outcome: ManagedApplicationMoveSubtreesOutcome;
}

/** Exact X1.5 replay observation. A successor generation is present only when
 * the predecessor actor was already stopped and recovery reopened it. */
export interface ManagedApplicationMoveSubtreesRecoveryResult {
  previous_binding_generation: number;
  binding_generation: number;
  status: SparseV2Status;
  application_page_admission: ApplicationPageAdmission;
  episode_id: string;
  outcome: ManagedApplicationMoveSubtreesOutcome;
}

/** Opaque one-shot acknowledgement that exact managed save preparation
 * completed without authoring. It is useful only to the immutable frontend
 * plan that requested it; the real save revalidates everything. */
export type ManagedPageMutationPreflightResult =
  | {
      status: "accepted";
      binding_generation: number;
      page_name: string;
      page_path: string;
      base_revision: string | null;
    }
  | { status: "refused" | "deferred" };

export type SparseV2Status = SparseV2Availability & {
  runtime: SparseV2RuntimeStatus | null;
  can_activate: boolean;
  can_retry: boolean;
  can_cancel: boolean;
  cancel_reason: string | null;
  binding_generation: number;
  application_page_admission: ApplicationPageAdmission;
};

/** A status snapshot scoped to the graph binding that produced it. */
export interface SparseV2RuntimeStatusEvent {
  binding_generation: number;
  runtime: SparseV2RuntimeStatus;
  application_page_admission: ApplicationPageAdmission;
}

export interface SparseV2CancelResult {
  status: SparseV2Status;
  binding_generation: number;
  recovery_statement: string;
}

/** Privacy-safe native progress shared by terminal diagnostics and cold-start UI. */
export type StartupProgressPhase =
  | "lookup.entry"
  | "lookup.app_data"
  | "lookup.settings_stat"
  | "lookup.settings_read"
  | "lookup.settings_parse"
  | "lookup.complete"
  | `managed_open.${string}`;

export interface StartupProgressEvent {
  phase: StartupProgressPhase;
  elapsed_ms: number;
  terminal: boolean;
  outcome?: "ok" | "error";
}

export type SparseV2ActivationPhase =
  | "private_setup"
  | "source_capture"
  | "pre_enrollment_reservation"
  | "archive_setup"
  | "initial_enrollment"
  | "bootstrap_import_preparation"
  | "immutable_publication_install"
  | "backup_proof"
  | "sqlite_open_build"
  | "shadow_reconstruction_byte_verification"
  | "promotion_receipt_confirmation"
  | "reconciliation_baseline_actor_open";

export type SparseV2BootstrapPreparationSubphase =
  | "source_protocol"
  | "operation_spool"
  | "partition"
  | "detached_authoring"
  | "sealing";

export interface SparseV2BootstrapPreparationSummary {
  source_files: number;
  source_bytes: number;
  parser_nodes: number;
  operations: number;
  parts: number;
  prepared_bytes: number;
  source_protocol_micros: number;
  operation_spool_micros: number;
  partition_micros: number;
  detached_authoring_micros: number;
  sealing_micros: number;
}

export type SparseV2ActivationProgress =
  | { kind: "phase"; phase: SparseV2ActivationPhase }
  | { kind: "bootstrap_preparation_subphase"; subphase: SparseV2BootstrapPreparationSubphase }
  | { kind: "bootstrap_detached_authoring"; completed: number; total: number }
  | { kind: "bootstrap_preparation_summary"; summary: SparseV2BootstrapPreparationSummary };

export interface SparseV2ActivationProgressEvent {
  binding_generation: number;
  progress: SparseV2ActivationProgress;
}

export type SparseV2EntityId =
  | { entity_type: "page"; id: string }
  | { entity_type: "block"; id: string };

export type SparseV2QueryRequest =
  | { kind: "resolve_page"; path: string; name: string; page_kind: PageKind }
  | { kind: "resolve_page_by_name"; name: string; page_kind: PageKind }
  | { kind: "list_pages"; page_kind: PageKind | null; limit: number }
  | { kind: "load_page"; page_id: string; block_limit: number }
  | { kind: "search"; query: string; limit: number }
  | { kind: "properties_for_owner"; owner: SparseV2EntityId; limit: number }
  | { kind: "properties_named"; name: string; value: string | null; limit: number }
  | { kind: "tags"; tag: string; limit: number }
  | { kind: "tasks"; marker: string | null; limit: number }
  | { kind: "references_to_page_name"; name: string; limit: number }
  | { kind: "references_to_logseq_uuid"; logseq_uuid: string; limit: number };

export interface SparseV2Page {
  page_id: string;
  home_document_id: string;
  name: string;
  path: string;
  kind: PageKind;
  preamble: string | null;
}

export interface SparseV2Block {
  block_id: string;
  page_id: string;
  home_document_id: string;
  parent_block_id: string | null;
  order: string;
  content: string;
  heading_level: number | null;
  collapsed: boolean;
  logseq_uuid: string | null;
}

export interface SparseV2PageWithBlocks {
  page: SparseV2Page;
  blocks: SparseV2Block[];
}

export type SparseV2PageNameResolution =
  | { status: "missing" }
  | { status: "exact"; page: SparseV2Page }
  | { status: "ambiguous" };

export interface SparseV2SearchHit {
  entity: SparseV2EntityId;
  page_id: string;
  text: string;
  rank: number;
}

export interface SparseV2Property {
  owner: SparseV2EntityId;
  page_id: string;
  name: string;
  value: string;
}

export interface SparseV2Tag {
  owner: SparseV2EntityId;
  page_id: string;
  tag: string;
}

export interface SparseV2Task {
  block_id: string;
  page_id: string;
  marker: string;
  priority: string | null;
  scheduled: string | null;
  deadline: string | null;
}

export type SparseV2ReferenceSource =
  | { source_type: "preamble" }
  | { source_type: "block"; block_id: string; home_document_id: string };

export interface SparseV2ReferenceHit {
  source_page_id: string;
  source: SparseV2ReferenceSource;
  kind: string;
  raw_target: string;
  byte_start: number;
  byte_end: number;
  resolved_page_id: string | null;
  resolved_block_id: string | null;
}

/** Exact adjacent-tagged Serde wire shape: `{ kind, value }`. */
export type SparseV2QueryReply =
  | { kind: "page"; value: SparseV2Page | null }
  | { kind: "page_name"; value: SparseV2PageNameResolution }
  | { kind: "pages"; value: SparseV2Page[] }
  | { kind: "page_with_blocks"; value: SparseV2PageWithBlocks | null }
  | { kind: "search"; value: SparseV2SearchHit[] }
  | { kind: "properties"; value: SparseV2Property[] }
  | { kind: "tags"; value: SparseV2Tag[] }
  | { kind: "tasks"; value: SparseV2Task[] }
  | { kind: "references"; value: SparseV2ReferenceHit[] };

export type SparseV2EditorPageSelector =
  | { selector: "page_id"; page_id: string }
  | { selector: "name"; name: string; page_kind: PageKind };

export interface SparseV2EditorLoadRequest {
  page: SparseV2EditorPageSelector;
}

export type SparseV2EditorBlockKey =
  | { key_type: "existing"; value: string }
  | { key_type: "temporary"; value: string };

export interface SparseV2EditorBlock {
  key: SparseV2EditorBlockKey;
  parent: SparseV2EditorBlockKey | null;
  content: string;
}

export type SparseV2EditorSaveTarget =
  | { target: "existing"; page_id: string; revision: string }
  | { target: "new"; name: string; page_kind: PageKind; revision: string };

export interface SparseV2EditorSaveRequest {
  target: SparseV2EditorSaveTarget;
  preamble: string | null;
  blocks: SparseV2EditorBlock[];
}

export type SparseV2EditorOutcome = { status: string; [key: string]: unknown };

/** How one aligned block differs between the winner and the conflict copy. */
export type RowKind = "unchanged" | "modified" | "added" | "removed";

/** One side of a diff row. */
export interface BlockView {
  /** Persisted `id::`, or empty. */
  uuid: string;
  /** The block's full dedented body (may be multi-line); UI shows the first line. */
  text: string;
  child_count: number;
}

/** One aligned position in the two block trees. `id` is a stable path ("2.1")
 *  that the resolve step reproduces, so a decision maps back to the same block. */
export interface DiffRow {
  id: string;
  kind: RowKind;
  mine: BlockView | null;
  theirs: BlockView | null;
  children: DiffRow[];
}

/** The full block-level diff of a conflict copy against its winner. */
export interface SyncConflictDiff {
  base_rev: string;
  conflict_rev: string;
  rows: DiffRow[];
  mine_pre: string | null;
  theirs_pre: string | null;
  pre_differs: boolean;
  blocks_identical: boolean;
}

/** A user's per-row merge decision. */
export type MergeDecision = "mine" | "theirs" | "both";

export interface RefGroup {
  page: string;
  kind: PageKind;
  /** Exact owner for path-bearing search presentations; absent for legacy DSL results. */
  path?: string;
  blocks: BlockDto[];
  evidence?: ReferenceBlockEvidence[];
}

export interface BacklinkFilterTarget {
  page: string;
  kind: PageKind;
  block_id: string;
}

export interface BacklinkFilterEntry extends BacklinkFilterTarget {
  text: string;
  facets: string[];
  truncated?: boolean;
}

export interface BacklinkFilterContext {
  entries: BacklinkFilterEntry[];
  truncated?: boolean;
}

export type ReferenceKind = "explicit" | "plain";

export interface ReferenceOccurrence {
  matched_name: string;
  canonical: string;
  kind: ReferenceKind;
  /** UTF-16 offsets into the matching BlockDto.raw. */
  span: MatchSpan;
  rule: string;
}

export interface ReferenceBlockEvidence {
  block_id: string;
  occurrences: ReferenceOccurrence[];
  /** Total matches in the block before the bounded jump-target list is capped. */
  total?: number;
  truncated?: boolean;
}

export interface MatchSpan {
  /** UTF-16 code-unit offsets into QueryHit.display_text; end is exclusive. */
  start: number;
  end: number;
}

export interface MatchEvidence {
  clause_id: number;
  field: "page_name" | "visible_content";
  mode: "contains" | "phrase" | "regex" | "fuzzy";
  spans: MatchSpan[];
  score?: number;
}

export type ObjectiveMatchClass = "exact" | "prefix" | "substring" | "fuzzy" | "body_evidence";

export interface QueryDiagnostic {
  code: string;
  message: string;
  span?: MatchSpan;
}

export interface QueryExplainNode {
  clause_id?: number;
  description: string;
  children: QueryExplainNode[];
}

export type QueryHit =
  | {
      entity: "page";
      page: PageEntry;
      display_text: string;
      evidence: MatchEvidence[];
      score: number;
      match_class?: ObjectiveMatchClass;
      matched_alias?: string;
    }
  | {
      entity: "block";
      page: string;
      kind: PageKind;
      /** Exact graph-root-relative file that physically owns this block hit. */
      path?: string;
      block: BlockDto;
      display_text: string;
      evidence: MatchEvidence[];
      score?: number;
      match_class?: ObjectiveMatchClass;
    };

export interface QueryExecution {
  hits: QueryHit[];
  diagnostics: QueryDiagnostic[];
  explanation: { branches: QueryExplainNode[] };
  /** Absent only when talking to an older backend or using an older test fixture. */
  has_more?: { pages: boolean; blocks: boolean };
  cancelled: boolean;
}

/** A single routed page used to scope block search. When present, `path` is the
 * authoritative file identity; otherwise kind plus canonical page name is used. */
export interface QueryPageScope {
  name: string;
  pageKind: PageKind;
  path?: string;
}

/** Result of an advanced (datalog) query: matched groups + which clause heads
 *  ran vs were ignored (`supported` is false only when nothing in the subset matched). */
export interface AdvancedQueryResult {
  groups: RefGroup[];
  ran: string[];
  ignored: string[];
  supported: boolean;
}

export interface GraphMeta {
  root: string;
  journals_dir: string;
  pages_dir: string;
  preferred_workflow: string; // "now" | "todo"
  shortcuts: Record<string, string>;
  start_of_week: number; // Logseq :start-of-week, 0=Monday … 6=Sunday (default 6)
  block_hidden_properties: string[];
  default_journal_template: string | null;
  favorites: string[];
  journal_page_title_format: string; // :journal/page-title-format (default "MMM do, yyyy")
  journal_file_name_format: string; // :journal/file-name-format (default "yyyy_MM_dd")
  preferred_format: Format; // :preferred-format — new pages/journals ("md" | "org")
  macros: Record<string, string>; // :macros — user text-substitution macros ($1..$N)
  enable_timetracking: boolean; // :feature/enable-timetracking?, default true
  show_brackets: boolean; // :ui/show-brackets?, default true
  /** :shortcut/doc-mode-enter-for-new-block?, false when absent / older backend. */
  doc_mode_enter_for_new_block?: boolean;
  /** :editor/logical-outdenting?, false when absent / older backend. */
  logical_outdenting?: boolean;
  logbook_with_second_support: boolean; // :logbook/settings :with-second-support?, default true
  logbook_enabled_in_timestamped_blocks: boolean;
  logbook_enabled_in_all_blocks: boolean;
  guide_announced: boolean; // :tine/guide-announced?, default false
}

export interface Rect {
  top: number;
  left: number;
  width: number;
  height: number;
  /** Coordinate-space dimensions from a current Logseq PDF sidecar. Absent on
   * rectangles written by older Tine versions, which already use page space. */
  source_width?: number;
  source_height?: number;
}

export interface Highlight {
  id: string;
  page: number;
  position: { page: number; bounding: Rect; rects: Rect[] };
  color: string;
  text: string | null;
  image: number | null;
}

export interface PdfState {
  highlights: Highlight[];
  page: number | null;
  scale: number | null;
}

/** Options for the print-to-PDF export (chosen in the pre-export dialog). Field
 *  names are snake_case to match the Rust `PrintOpts` serde deserialization. */
export interface PrintOpts {
  /** Expand `collapsed:: true` blocks (true = print the whole page). */
  expand_collapsed: boolean;
  /** Base body font size, px. */
  font_px: number;
  /** Page margin, mm (all four sides). */
  margin_mm: number;
}
