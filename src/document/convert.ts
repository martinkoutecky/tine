import { type BlockDto, type Format, type PageDto, type RefGroup } from "../types";
import { Node, FeedPage, DocState, doc } from "./model";
import { seedFacets, facetsFromDto } from "../render/facets";
import { trimBlockTrailingSpace } from "../editor/format";
import { isPageHeaderPropertiesOnly, parsePageHeaderPropertyLine } from "../editor/properties";
import { pushToast } from "../toasts";

// ---------------------------------------------------------------------------
// Loading / serializing
// ---------------------------------------------------------------------------

function flatten(
  dtos: BlockDto[],
  parent: string | null,
  pageName: string,
  byId: Record<string, Node>,
  format: Format
): string[] {
  return dtos.map((d) => {
    // Seed the header-facet cache from the backend (one Rust lsdoc parse, shipped) so
    // the rendered chip reads off the DTO — zero frontend parse on load (M1 / P1).
    seedFacets(d.raw, format, facetsFromDto(d));
    // Cross-page id:: collision guard: if another LOADED page already owns this
    // id (two files share a persisted `id::` — copy-pasted raw, or a sync hiccup),
    // give this block a fresh store key instead of overwriting the other page's
    // node. Without this, the global byId entry is clobbered and saving one page
    // serializes the other's content. The block's raw (incl. its id:: line) is
    // untouched, so the file on disk is unchanged. Rust dedups ids WITHIN a page,
    // so this only fires across pages.
    const existing = byId[d.id];
    const key = existing && existing.page !== pageName ? `dup~${crypto.randomUUID()}` : d.id;
    const childIds = flatten(d.children, key, pageName, byId, format);
    byId[key] = {
      id: key,
      raw: d.raw,
      collapsed: d.collapsed,
      parent,
      page: pageName,
      children: childIds,
    };
    return key;
  });
}

export function toFeedPage(dto: PageDto & { id?: string }, byId: Record<string, Node>): FeedPage {
  const roots = flatten(dto.blocks, null, dto.name, byId, dto.format ?? "md");
  return {
    name: dto.name,
    kind: dto.kind,
    title: dto.title,
    preBlock: dto.pre_block,
    roots,
    format: dto.format ?? "md",
    readOnly: dto.read_only ?? false,
    guide: dto.guide ?? false,
    id: dto.id,
  };
}

function removeNodeSubtree(s: DocState, id: string) {
  const n = s.byId[id];
  if (!n) return;
  for (const c of n.children) removeNodeSubtree(s, c);
  delete s.byId[id];
}

/** Drop a page's blocks from the shared byId map (before replacing it). Walks the
 *  page's own root subtrees — O(page size) — rather than sweeping all of `byId`
 *  (which made loading K pages into an N-node feed O(K·N)). */
export function purgePageNodes(s: DocState, pageName: string) {
  const page = s.pages.find((p) => p.name === pageName);
  if (!page) return;
  for (const r of page.roots) removeNodeSubtree(s, r);
}

/** Merge a page into the working set, replacing any prior copy of that page.
 *  Other loaded pages (and their nodes) are left untouched — so a page open in
 *  the sidebar survives navigating the main view elsewhere. */
function toDto(id: string): BlockDto {
  const n = doc.byId[id];
  // Trim a block's trailing space only here, at the disk-write boundary — OG
  // keeps the space while you edit and trims on save. (The live editor buffer
  // keeps it so backspacing to a trailing space doesn't eat the space out from
  // under the caret.) `trimBlockTrailingSpace` is idempotent and only touches
  // whitespace at the very end of the block, so a block with nothing to trim
  // serializes byte-identically — no churn, no property reordering.
  return { id: n.id, raw: trimBlockTrailingSpace(n.raw), collapsed: n.collapsed, children: n.children.map(toDto) };
}

/** Mirror of Rust `first_root_is_promotable_page_header` (model.rs): a childless
 *  first root whose raw is exactly canonical page-header properties and carries
 *  no `id::` line (an id-bearing block is a real referenced outline block, not a
 *  header, and the Rust promote branch/firewall both leave it as a bullet). */
function isPromotablePageHeaderRoot(node: Node): boolean {
  const canonicalRaw = node.raw.replace(/\n+$/, "");
  return (
    node.children.length === 0 &&
    isPageHeaderPropertiesOnly(canonicalRaw) &&
    !canonicalRaw.split("\n").some((line) => parsePageHeaderPropertyLine(line)?.key.toLowerCase() === "id")
  );
}

export function pageToDto(pageName: string): PageDto | null {
  const p = doc.pages.find((x) => x.name === pageName);
  if (!p) return null;
  let rootIds = p.roots;
  let preBlock = p.preBlock;
  const first = doc.byId[rootIds[0]];
  if (first?.originatedFromPageHeader) {
    // Enter temporarily leaves one or more trailing newlines in the live
    // page-header editor. Tolerate only that authoring artifact at the disk
    // firewall; keep the strict shared display predicate and live raw intact.
    const canonicalRaw = first.raw.replace(/\n+$/, "");
    if (first.children.length > 0 || (first.raw !== "" && !isPageHeaderPropertiesOnly(canonicalRaw))) {
      pushToast("Page-header properties must contain only valid key:: value lines before they can be saved.", "error");
      return null;
    }
    // Exact raw is authoritative here: ordinary toDto trimming must never eat a
    // page-header value or its separator trivia. An empty draft deletes the
    // header and emits no stray outline bullet.
    preBlock = canonicalRaw ? canonicalRaw + (p.preBlock ?? "") : p.preBlock;
    rootIds = rootIds.slice(1);
  } else if (first && !p.preBlock && isPromotablePageHeaderRoot(first)) {
    // GH #198: a flagless "properties-only first bullet" (empty preBlock) IS the
    // page header — the same shape setPageProperty/beginPageHeaderEdit already
    // treat as the header. Fold it into pre_block so the DTO is honest, instead
    // of leaning on the Rust promote branch: once disk already carries the
    // promoted preamble, the GH #163 preservation firewall refuses the
    // pre_block=None + first-root-properties DTO and jams the save queue with a
    // "will retry" toast forever. Folding here emits pre_block=properties, so
    // the firewall precondition (empty pre_block) is false and the save writes
    // the identical canonical preamble. Mirrors Rust's promotability rule.
    preBlock = first.raw.replace(/\n+$/, "");
    rootIds = rootIds.slice(1);
  }
  let blocks = rootIds.map(toDto);
  // Don't persist a lone placeholder block. A page that exists only for its
  // properties is loaded with one empty editable bullet (toLoadable); saving it
  // — e.g. after a page-property edit — must NOT write that bullet back as a
  // stray "- " and corrupt the round-trip. Symmetric with the load side;
  // reopening re-adds the editable bullet.
  if (blocks.length === 1 && blocks[0].raw.trim() === "" && blocks[0].children.length === 0) {
    blocks = [];
  }
  return {
    name: p.name,
    kind: p.kind,
    title: p.title,
    pre_block: preBlock,
    blocks,
    format: p.format,
    guide: p.guide,
    read_only: p.readOnly,
  };
}

// ---------------------------------------------------------------------------
// Virtual-guide resolution
//
// The in-app Guide is virtual — its pages live only in this store, never on
// disk — so the backend `((uuid))` / `{{embed [[page]]}}` resolvers (which scan
// the on-disk graph) can't see them. These fall back to the LOADED guide pages
// and are consulted ONLY on a backend miss, so a real-graph ref/embed always
// prefers the disk resolver and these never shadow it.
// ---------------------------------------------------------------------------

/** The block id (`id:: <uuid>` trailer) a guide node exposes to `((uuid))`
 *  references — matching the backend, which keys a block by its persisted id::. */
function guideBlockDurableId(raw: string): string | null {
  const m = /(?:^|\n)id:: *(\S+)/i.exec(raw);
  return m ? m[1] : null;
}

function findGuideNode(ids: string[], uuid: string): string | null {
  for (const id of ids) {
    const n = doc.byId[id];
    if (!n) continue;
    if (id === uuid || guideBlockDurableId(n.raw) === uuid) return id;
    const child = findGuideNode(n.children, uuid);
    if (child) return child;
  }
  return null;
}

/** Resolve a `((uuid))` block reference / block embed against the loaded guide
 *  pages. Returns null for any id not owned by a loaded guide page, so real
 *  refs fall through to the backend/disk resolver unchanged. */
export function resolveGuideBlockRef(uuid: string): RefGroup | null {
  for (const p of doc.pages) {
    if (!p.guide) continue;
    const hit = findGuideNode(p.roots, uuid);
    if (hit) return { page: p.name, kind: p.kind, blocks: [toDto(hit)] };
  }
  return null;
}

/** Serialize a loaded guide page (matched by its bare title, e.g.
 *  "Features/Tips & shortcuts") to a PageDto for in-app `{{embed [[page]]}}` —
 *  the embed macro carries no source context to remap the name, so we match on
 *  title. Null for non-guide/unloaded titles → the backend/disk path wins. */
export function resolveGuidePageDto(title: string): PageDto | null {
  const p = doc.pages.find((x) => x.guide && x.title === title);
  return p ? pageToDto(p.name) : null;
}

