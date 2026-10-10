import { type BlockDto, type Format, type PageDto, type RefGroup } from "../types";
import { Node, FeedPage, DocState, doc, freshId } from "./model";
import { seedFacets, facetsFromDto } from "../render/facets";
import { trimBlockTrailingSpace } from "../editor/format";
import { isPageHeaderPropertiesOnly, pageHeaderKeys } from "../editor/properties";
import { journalTitle, appNow } from "../journal";
import { rawWithCollapsed } from "./edits/properties";
import { existingBlockId, orgRawWithProperty } from "./edits/identity";

/** Wire DTO constructors live here; callers choose the intent and supply content. */
export function emptyPage(name: string, kind: "journal" | "page"): PageDto {
  return { name, kind, title: name, pre_block: null, blocks: [{ id: `new-${name}`, raw: "", collapsed: false, children: [] }] };
}

export function withToday(js: PageDto[]): PageDto[] {
  const title = journalTitle(appNow());
  return js.some((p) => p.name === title) ? js : [emptyPage(title, "journal"), ...js];
}

export function toLoadablePage(dto: PageDto, name: string): PageDto {
  return dto.blocks.length ? dto : { ...dto, blocks: [{ id: `new-${name}`, raw: "", collapsed: false, children: [] }] };
}

export function carryTodayPage(name: string): PageDto {
  return emptyPage(name, "journal");
}

export function captureScratchPage(name: string, blockId: string): PageDto {
  if (!blockId.trim()) throw new Error("Quick Capture scratch block id must not be empty");
  return { name, kind: "page", title: name, pre_block: null, blocks: [{ id: blockId, raw: "", collapsed: false, children: [] }], rev: null };
}

export function captureEmptyPage(name: string, kind: "journal" | "page"): PageDto {
  return { name, kind, title: name, pre_block: null, blocks: [], rev: null };
}

export function journalTemplatePage(title: string, blocks: BlockDto[], existing?: PageDto | null): PageDto {
  return { name: title, kind: "journal", title, pre_block: existing?.pre_block ?? null, blocks, format: existing?.format };
}

export function demoJournalPage(title: string): PageDto {
  return { name: title, kind: "journal", title, pre_block: null, blocks: [{ id: "", raw: "👋 This is **today's journal** — your daily notes land here. Try your quick-capture hotkey, or open [[Welcome to Tine]] for the tour.", collapsed: false, children: [] }] };
}

export function switcherPage(name: string): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks: [{ id: "", raw: "", collapsed: false, children: [] }] };
}

/** A saved query workspace: one query block whose properties are written where
 *  the page's FORMAT keeps them. Markdown appends `key:: value` lines (the bytes
 *  it always wrote); Org puts them in a `:PROPERTIES:` drawer, because a markdown
 *  line in an Org file is visible body text that is never read back (GH #25).
 *  `format` is the extension of the id the backend resolved for the new page. */
export function queryWorkspacePage(name: string, query: string, properties: ReadonlyArray<readonly [string, string]>, format: Format): PageDto {
  const raw = properties.reduce(
    (text, [key, value]) => (format === "org" ? orgRawWithProperty(text, key, value) : `${text}\n${key}:: ${value}`), query);
  return { name, kind: "page", title: name, pre_block: null, format, blocks: [{ id: "", raw, collapsed: false, children: [] }] };
}

/** The Favorites arrangement page (family 22), written whole through
 *  `createPage`. `marker` is the verbatim pre-block text (the caller picks
 *  `tine/favorites:: true` or, in Org, `#+tine/favorites: true`). Collapse
 *  persists in the raw in the page's own format (`collapsed:: true` / an Org
 *  `:PROPERTIES:` drawer), since the save path writes raw only. */
export function favoritesArrangementPage(name: string, marker: string, blocks: BlockDto[], format: Format): PageDto {
  const persist = (nodes: BlockDto[]): BlockDto[] => nodes.map((b) =>
    ({ ...b, raw: rawWithCollapsed(b.raw, b.collapsed, format), children: persist(b.children) }));
  return { name, kind: "page", title: name, pre_block: marker, blocks: persist(blocks) };
}
/** Read side: the page's blocks with the collapsed property left in the flag only. */
export function favoritesArrangementBlocks(blocks: readonly BlockDto[], format: Format): BlockDto[] {
  return blocks.map((b) => ({ ...b, raw: rawWithCollapsed(b.raw, false, format), children: favoritesArrangementBlocks(b.children, format) }));
}

export function appendAliasDraft(owner: PageDto, draft: PageDto): PageDto {
  return { ...owner, blocks: [...owner.blocks, ...aliasDraftBlocks(draft)] };
}

/** The blocks an alias draft contributes to its owner, as appended. */
export function aliasDraftBlocks(draft: PageDto): BlockDto[] {
  return draft.pre_block
    ? [{ id: "", raw: draft.pre_block, collapsed: false, children: [] }, ...draft.blocks]
    : draft.blocks;
}

/** `owner` with the copy of a draft that already landed at its end (`landed`)
 *  replaced by the draft's current blocks, or null when the owner's tail is no
 *  longer exactly that copy (the owner changed since: the caller refuses).
 *  Compared by text and nesting; block ids are assigned per load. O(owner). */
export function replaceLandedAliasDraft(owner: PageDto, landed: BlockDto[], draft: PageDto): PageDto | null {
  const shape = (blocks: BlockDto[]): string =>
    JSON.stringify(blocks.map(function strip(b): unknown { return [b.raw.trimEnd(), b.children.map(strip)]; }));
  const keep = owner.blocks.length - landed.length;
  if (keep < 0 || shape(owner.blocks.slice(keep)) !== shape(landed)) return null;
  return { ...owner, blocks: [...owner.blocks.slice(0, keep), ...aliasDraftBlocks(draft)] };
}

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
    // A DTO built in the window (a created page, a template, the welcome
    // journal) has no runtime ids yet: each such block gets a fresh one, or
    // they would all collapse into one node.
    const existing = d.id ? byId[d.id] : undefined;
    const key = !d.id ? freshId() : existing && existing.page !== pageName ? `dup~${crypto.randomUUID()}` : d.id;
    const childIds = flatten(d.children, key, pageName, byId, format);
    byId[key] = {
      id: key,
      raw: d.raw,
      loadedRaw: d.raw,
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
  // Only an edited block gets OG's trailing-space trim. Loaded siblings must
  // retain their exact raw bytes when another block on the page is saved.
  const raw = n.loadedRaw === n.raw ? n.raw : trimBlockTrailingSpace(n.raw);
  return { id: n.id, raw, collapsed: n.collapsed, children: n.children.map(toDto) };
}

/** Properties that describe the block they sit on, never a page: a first
 *  bullet carrying one stays an outline block. An empty numbered-list item is
 *  exactly `logseq.order-list-type:: number`, and folding it into the page
 *  header turned the list into page properties and jammed every later save
 *  (GH #540). Mirror of Rust `BLOCK_SCOPED_PROPERTY_KEYS` (tine-store
 *  model.rs), which carries the OG provenance. */
export const BLOCK_SCOPED_PROPERTY_KEYS: readonly string[] = [
  "id",
  "heading",
  "collapsed",
  "background-color",
  "logseq.order-list-type",
];

/** Mirror of Rust `first_root_is_promotable_page_header` (model.rs): a childless
 *  first root whose raw is exactly canonical page-header properties and carries
 *  no block-scoped property (an `id::` block is a real referenced outline block,
 *  an empty numbered item a list item; the Rust promote branch/firewall both
 *  leave them as bullets). */
function isPromotablePageHeaderRoot(node: Node): boolean {
  const canonicalRaw = node.raw.replace(/\n+$/, "");
  return (
    node.children.length === 0 &&
    isPageHeaderPropertiesOnly(canonicalRaw) &&
    !pageHeaderKeys(canonicalRaw).some((key) => BLOCK_SCOPED_PROPERTY_KEYS.includes(key))
  );
}

/** Page `p`'s pre-block and outline roots as its file holds them: a marked
 *  page-header root (`beginPageHeaderEdit`) folds back into the pre-block,
 *  with the terminal newlines Enter leaves in the live editor removed. Null
 *  for an invalid header draft, which never reaches the disk. O(1). */
export function foldMarkedPageHeader(p: FeedPage): { preBlock: string | null; rootIds: string[] } | null {
  const first = doc.byId[p.roots[0]];
  if (!first?.originatedFromPageHeader) return { preBlock: p.preBlock, rootIds: p.roots };
  // Tolerate only Enter's authoring artifact at the disk firewall; keep the
  // strict shared display predicate and live raw intact.
  const canonicalRaw = first.raw.replace(/\n+$/, "");
  if (first.children.length > 0 || (first.raw !== "" && !isPageHeaderPropertiesOnly(canonicalRaw))) {
    return null;
  }
  // Exact raw is authoritative here: ordinary toDto trimming must never eat a
  // page-header value or its separator trivia. An empty draft deletes the
  // header and emits no stray outline bullet.
  return { preBlock: canonicalRaw ? canonicalRaw + (p.preBlock ?? "") : p.preBlock, rootIds: p.roots.slice(1) };
}

/** Project one loaded page for guarded save. Cost: O(loaded pages + blocks and
 * text bytes of this page). Ordinary loaded blocks with unchanged raw keep it exactly;
 * edited ones get OG's trailing-space trim (which retains a bare list marker's
 * final space). A page-header root is folded into pre_block with terminal
 * newlines removed, and a lone empty placeholder is omitted. Returns null for
 * an absent page or invalid page-header draft; the save caller leaves an
 * invalid draft dirty for retry. Reads only the in-memory document. */
export function pageToDto(pageName: string): PageDto | null {
  const p = doc.pages.find((x) => x.name === pageName);
  if (!p) return null;
  const folded = foldMarkedPageHeader(p);
  if (!folded) return null;
  let { rootIds, preBlock } = folded;
  const first = doc.byId[p.roots[0]];
  if (first && !first.originatedFromPageHeader && !p.preBlock && isPromotablePageHeaderRoot(first)) {
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
  return existingBlockId(raw, "md");
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
