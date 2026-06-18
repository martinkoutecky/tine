// TS mirrors of the Rust DTOs (crates/logseq-core/src/model.rs).

export type PageKind = "journal" | "page";

export interface BlockDto {
  id: string;
  raw: string;
  collapsed: boolean;
  children: BlockDto[];
  /** Ancestor first-lines (search/reference results only). */
  breadcrumb?: string[];
}

export interface PageDto {
  name: string;
  kind: PageKind;
  title: string;
  pre_block: string | null;
  blocks: BlockDto[];
  /** Hash of the on-disk file at load time — the save baseline. `null`/absent for
   *  a page with no file yet. Sent back on save to detect external changes. */
  rev?: string | null;
}

export interface TemplateDto {
  name: string;
  blocks: BlockDto[];
}

export interface PageEntry {
  name: string;
  kind: PageKind;
  date_key: number | null;
}

export interface RefGroup {
  page: string;
  kind: PageKind;
  blocks: BlockDto[];
}

export interface GraphMeta {
  root: string;
  journals_dir: string;
  pages_dir: string;
  preferred_workflow: string; // "now" | "todo"
  shortcuts: Record<string, string>;
  start_of_week: number; // 0=Sunday … 6=Saturday
  block_hidden_properties: string[];
  default_journal_template: string | null;
  favorites: string[];
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
