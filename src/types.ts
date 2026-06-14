// TS mirrors of the Rust DTOs (crates/logseq-core/src/model.rs).

export type PageKind = "journal" | "page";

export interface BlockDto {
  id: string;
  raw: string;
  collapsed: boolean;
  children: BlockDto[];
}

export interface PageDto {
  name: string;
  kind: PageKind;
  title: string;
  pre_block: string | null;
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
  shortcuts: Record<string, string>;
}
