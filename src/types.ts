// TS mirrors of the Rust DTOs (crates/logseq-core/src/model.rs).

export type PageKind = "journal" | "page";

export interface BlockDto {
  id: string;
  raw: string;
  collapsed: boolean;
  children: BlockDto[];
  /** Ancestor first-lines (search/reference results only). */
  breadcrumb?: string[];
  // M1 block-header facets, computed once off the Rust lsdoc projection and shipped
  // so the frontend reads them off the DTO (no parse on load) instead of re-deriving
  // with its own scanner. Omitted by the backend when empty (see model.rs BlockDto).
  marker?: string;
  priority?: string;
  heading_level?: number;
  scheduled?: string;
  deadline?: string;
  properties?: [string, string][];
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
  /** Graph-root-relative path of the file this page was loaded from
   *  (`journals/2026_06_26.org`). Echoed back on save so a page pinned to a
   *  SPECIFIC file (a duplicate-day stray, #21) saves to its own file rather than
   *  being re-resolved by name to the canonical one. Empty for a brand-new page. */
  path?: string;
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
}

/** An orphaned asset file (no block references it) — for the cleanup UI. */
export interface AssetInfo {
  name: string;
  size: number;
  /** Last-modified time as Unix seconds (≈ when the file entered the graph). */
  modified: number | null;
}

/** Count + total bytes of the recoverable asset trash (logseq/.tine-trash). */
export interface TrashStats {
  count: number;
  bytes: number;
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

export interface RefGroup {
  page: string;
  kind: PageKind;
  blocks: BlockDto[];
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
}

export interface Rect {
  top: number;
  left: number;
  width: number;
  height: number;
}

export interface Highlight {
  id: string;
  page: number;
  position: { page: number; bounding: Rect; rects: Rect[] };
  color: string;
  text: string | null;
  image: number | null;
}
