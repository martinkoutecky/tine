// TS mirrors of the Rust DTOs (crates/logseq-core/src/model.rs).

export type PageKind = "journal" | "page";

export interface BlockDto {
  id: string;
  /** Parser-owned identity presence; older/unprojected DTOs leave it unknown. */
  has_id?: boolean;
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

/** One reviewed query publication over the same text and host properties sent
 * to `parseQuery`. Publication writes a graph query leaf, preserves replaced
 * output in recovery and refuses a changed source fingerprint. */
export interface QueryPublicationRequest {
  argument: string;
  dialect: import("./editor/queryIr").QueryTextDialect;
  properties: [string, string][];
  currentPage?: string | null;
  hostBlockId?: string | null;
  folder?: string | null;
  replace?: boolean;
  assetBudgetBytes?: number | null;
  name: string;
}

export interface QueryPublicationPlan {
  anchor: "block" | "page";
  rowCount: number;
  pages: { name: string; path: string; journal: boolean }[];
  folder: string;
  path: string;
  exists: boolean;
  suggestedFolder: string | null;
  fingerprint: string;
}

export interface PublicationReceipt {
  path: string;
  pages: number;
  files: number;
  retired: string | null;
  warnings: string[];
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
  /** True for an org page Tine can't round-trip byte-for-byte: shown but not
   *  editable, so Tine never rewrites (and risks corrupting) it. */
  read_only?: boolean;
  /** Bundled in-app Guide page: read-only, ephemeral, and excluded from normal
   *  graph persistence/search/reference surfaces. */
  guide?: boolean;
}

/** One crash-surviving draft (og ADR 0061): the page as the editor held it when
 *  it could not be saved. `id` is `<session>:<page name>`; `kind` "live-conflict"
 *  is the Concord live-draft capsule (og 21a): a draft whose save was refused
 *  because its file changed on disk, restorable into the in-page resolver. */
export interface DraftRecord {
  id: string;
  kind: "unsaved" | "live-conflict";
  session: string;
  page_name: string;
  path: string | null;
  reason: "conflict" | "save-failed";
  saved_at: number;
  page: PageDto;
  /** Revision the draft was edited from (its Concord-ledger base); live-conflict only. */
  base_rev?: string | null;
  /** Disk revision observed when the save was refused; live-conflict only. */
  observed_rev?: string | null;
}

/** A page loaded from one concrete file. Its identity is returned unchanged on save. */
export interface PageRead extends PageDto {
  id: string;
}

/** What the backend rename did; `unchanged`: nothing written — a case-only
 *  rename, an empty old name, or a name no file and no reference uses (e.g. a
 *  never-saved page nobody links to). `touched` lists every page file it moved,
 *  trashed or rewrote, so the frontend refreshes only those (GH #535). */
export interface RenameDone {
  outcome: "renamed" | "merged" | "unchanged";
  touched: RenameTouchedPage[];
  /** Referrers left byte-identical because they carry VCS conflict markers
   *  (og 21a, master a8fd4230d): their references still name the old page. */
  skipped_conflicted_referrers?: string[];
  /** The new `:default-home` page when the rename moved the home page with it
   *  (og 22b, OG `rename-page-aux`); null otherwise. */
  home_page?: string | null;
}

/** One page file a rename or merge wrote. */
export interface RenameTouchedPage {
  /** Graph-relative path before the rename: the loaded page's `id`. */
  path: string;
  /** The file left this path (moved, or trashed by a merge); otherwise it was
   *  rewritten in place. */
  moved: boolean;
}

export type ResolvedPage =
  | { kind: "existing"; id: string; others: string[] }
  | { kind: "alias"; owners: string[] }
  | { kind: "absent"; id: string };

/** One `page_inventory` row. `key` is core `refs::page_key(name)`; `target` is
 *  the backend's answer for that name (precedence already decided). */
export interface PageInventoryEntry {
  key: string;
  name: string;
  is_journal: boolean;
  day: number | null;
  target: ResolvedPage;
}

/** `page_inventory` result; `rev` is the graph revision it was read at. */
export interface PageInventory {
  rev: string;
  entries: PageInventoryEntry[];
  /** Graph-relative paths whose page name couldn't be read; left out of `entries`
   *  names but never failing the inventory. */
  unreadable?: string[];
}

/** One authoritative Journals-feed transaction.  Cursor fields are ordinal
 * journal days, never counts of returned DTOs (a selected file may vanish). */
export interface JournalFeedPage {
  pages: PageRead[];
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
  /** 3-way classification against the base (absent on 2-way diffs). */
  verdict?: Diff3Verdict | null;
  /** Decision the base justifies; only pre-selected, never applied unconfirmed. */
  suggestion?: MergeDecision | null;
  /** Merged body offered for a `both-changed` row. Display only: the resolve
   *  re-derives it from the same inputs and never trusts this echo. */
  merged?: MergedProposal | null;
}

/** How a 3-way row relates to the common base. */
export type Diff3Verdict = "mine-only" | "theirs-only" | "both-changed";

/** "computed": two disjoint edits composed here; "artifact": the merge tool's
 *  own suggested resolution (Fossil), which Tine does not vouch for. */
export interface MergedProposal {
  text: string;
  source: "computed" | "artifact";
}

/** The full block-level diff of a conflict's two sides. */
export interface SyncConflictDiff {
  base_rev: string;
  conflict_rev: string;
  rows: DiffRow[];
  mine_pre: string | null;
  theirs_pre: string | null;
  pre_differs: boolean;
  blocks_identical: boolean;
  /** True when rows carry 3-way verdicts against a real base (absent = false). */
  three_way?: boolean;
  /** Identity of the Concord-ledger base a sync-copy 3-way diff used; the
   *  resolve sends it back so a "merged" row is applied only against the base
   *  the user reviewed. Absent on 2-way and marker diffs. */
  merge_base_rev?: string;
}

/** A user's per-row merge decision. */
export type MergeDecision = "mine" | "theirs" | "both" | "merged";

/** Where a conflict object came from: a sync tool's copy, VCS markers, an
 *  editor draft whose save was refused because the file changed (og 8e), or a
 *  journal day with more than one file (master 9dc54e4a7). */
export type ConflictSource = "sync-copy" | "vcs-markers" | "live-save" | "duplicate-journal";

/** The editor draft of a `live-save` conflict object. */
export interface LiveConflictDraft {
  /** The retained draft of a restored capsule; absent for an open editor's
   *  draft, which the resolver reads from the editor at each review. */
  page?: PageDto;
  /** Revision a restored draft was edited from; selects the ledger base. */
  base_rev: string | null;
  /** True for a draft restored from an earlier session's capsule record: the
   *  editor holds the disk version and the record is the only copy of the draft. */
  restored: boolean;
  /** The capsule record, for a restored draft. */
  record_id?: string;
}

/** One version of a page participating in a conflict. */
export interface ConflictSide {
  role: "mine" | "theirs" | "base";
  label: string;
  /** Graph-root-relative path, when the side is a file of its own. */
  path?: string | null;
}

/** One item of the conflict queue: DERIVED from disk on every refresh and never
 *  persisted, so it survives a restart by being recomputed. */
export interface ConflictObject {
  /** Stable derived id: `copy:<copy path>` / `markers:<path>` / `journal:<keeper path>`. */
  id: string;
  source: ConflictSource;
  page_name: string;
  /** Path of the page to open: the winner, or the marker-bearing file. */
  page_path: string;
  kind: PageKind;
  sides: ConflictSide[];
  /** Rows needing a decision, when computed (absent is not zero). */
  block_conflicts?: number | null;
  /** Marker tokens present, for a `vcs-markers` object. */
  markers?: string[];
  /** The draft, for a `live-save` object (derived from the editor or a capsule). */
  live?: LiveConflictDraft;
}

/** A page file carrying unresolved VCS merge conflict markers. */
export interface VcsMarkerConflict {
  path: string;
  name: string;
  kind: PageKind;
  markers: string[];
}

/** One answer from one walk: the listings and the queue cannot disagree. */
export interface ConflictInventory {
  sync_conflicts: SyncConflict[];
  vcs_markers: VcsMarkerConflict[];
  queue: ConflictObject[];
}

/** A marker-bearing page's own conflict, parsed from its marker sections. */
export interface MarkerConflictDiff {
  mine_label: string;
  theirs_label: string;
  regions: number;
  diff: SyncConflictDiff;
}

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
      /** Native-hydrated authored page facts for display columns; absent for references. */
      row?: import("./editor/queryIr").PageRow;
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
  /** config.edn `:ref/linked-references-collapsed-threshold` (OG default 100; 0 = always collapsed). */
  linked_references_collapsed_threshold: number;
  default_journal_template: string | null;
  /** config.edn `:default-home {:page "..."}`; absent/null = no home page. */
  default_home?: string | null;
  favorites: string[];
  /** `:tine/favorites-page`: the page holding the Favorites arrangement. */
  favorites_page?: string | null;
  journal_page_title_format: string; // :journal/page-title-format (default "MMM do, yyyy")
  journal_file_name_format: string; // :journal/file-name-format (default "yyyy_MM_dd")
  preferred_format: Format; // :preferred-format — new pages/journals ("md" | "org")
  macros: Record<string, string>; // :macros — user text-substitution macros ($1..$N)
  enable_timetracking: boolean; // :feature/enable-timetracking?, default true
  enable_search_remove_accents?: boolean; // :feature/enable-search-remove-accents?, default true
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
/** A title-named journal file and the date name it would get (proposed only). */
export type JournalFilenameMigration = { from: string; to: string };

export type JournalMigrationResult = {
  migrated: number;
  skipped: { file: string; reason: string }[];
};
