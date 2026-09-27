import { type PageKind, type Format } from "../types";
import { createStore } from "solid-js/store";
import { createRoot, createMemo } from "solid-js";
import { graphMeta } from "../graphSession";
import { sheetConfigFromRaw } from "../sheet/config";

export interface Node {
  id: string;
  raw: string;
  /** Raw bytes from the loaded DTO; unchanged blocks keep their trailing space on save. */
  loadedRaw?: string;
  collapsed: boolean;
  parent: string | null; // null = a root of its page
  page: string; // owning page name
  children: string[];
  /** Frontend-only editing provenance for an existing unbulleted Markdown page
   * header. Spread-based undo snapshots retain it; DTO serialization consumes
   * it and never sends it over the wire. */
  originatedFromPageHeader?: boolean;
}

export interface FeedPage {
  name: string;
  kind: PageKind;
  title: string;
  preBlock: string | null;
  roots: string[];
  /** On-disk format (drives org vs markdown inline rendering). */
  format: Format;
  /** True for an org page Tine can't round-trip — shown but not editable. */
  readOnly: boolean;
  /** Bundled in-app Guide page: read-only and ephemeral. */
  guide: boolean;
  /** Concrete file identity returned by the backend; absent until first save. */
  id?: string;
}

export interface DocState {
  byId: Record<string, Node>;
  // The working set: every page currently loaded in the frontend — the main
  // view's pages PLUS any page a satellite surface (sidebar, query result,
  // embed) has pulled in on demand. All share one `byId` keyed by stable block
  // uuid, so a block rendered in two places is the SAME node and edits to it
  // propagate everywhere via SolidJS reactivity (OG's "everything is a block",
  // adapted to lazy loading — the Rust cache is the full graph DB).
  pages: FeedPage[];
  // Page names the MAIN content area shows, in order (a single page, or the
  // journals feed). A subset of `pages`.
  feed: string[];
  loaded: boolean;
}

export const [doc, setDoc] = createStore<DocState>({ byId: {}, pages: [], feed: [], loaded: false });

export type ReadonlyNode = Readonly<Omit<Node, "children">> & { readonly children: readonly string[] };
export type ReadonlyFeedPage = Readonly<Omit<FeedPage, "roots">> & { readonly roots: readonly string[] };

/** Live document queries. Each call reads the Solid store in the caller's tracking scope. */
export function node(id: string): ReadonlyNode { return doc.byId[id]; }
export function childIds(id: string): readonly string[] { return doc.byId[id]?.children ?? []; }
export function pageRoots(name: string): readonly string[] { return pageByName(name)?.roots ?? []; }
export function loadedPage(name: string): ReadonlyFeedPage | undefined { return pageByName(name); }
export function feedNames(): readonly string[] { return doc.feed; }
export function isLoaded(name?: string): boolean { return name === undefined ? doc.loaded : !!pageByName(name); }

export function docHasBlockIdentity(id: string): boolean {
  if (doc.byId[id]) return true;
  const normalized = id.toLowerCase();
  if (Object.keys(doc.byId).some((key) => key.toLowerCase() === normalized && !!doc.byId[key])) return true;
  // setRaw updates a loaded node's raw synchronously without re-keying by a
  // newly typed/pasted id property. Treat either Markdown or Org id syntax as
  // live ownership so the final paste/redo checks fail closed in that window.
  const escaped = id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const rawIdentity = new RegExp(
    `(?:^|\\r?\\n)[ \\t]*(?:id[ \\t]*::|:id:)[ \\t]*${escaped}(?=[ \\t]*(?:\\r?\\n|$))`,
    "i",
  );
  return Object.values(doc.byId).some((node) => rawIdentity.test(node.raw));
}


// name → index into `doc.pages`, rebuilt only when the working set's membership
// changes (add / remove / rename / evict), NOT on a keystroke. Turns the O(pages)
// linear `find` in `pageByName`/`formatForPage`/`mainPages` — which run in the
// per-block render hot path and ~7×/page render — into an O(1) lookup. We map to
// the index (not the proxy) and read `doc.pages[idx]` live, so a property change
// (roots/preBlock/format) stays fine-grained-reactive and the index never goes
// stale: the memo re-derives whenever any page's `name` or the array length moves.
const pageIndexByName = createRoot(() =>
  createMemo(() => {
    const m = new Map<string, number>();
    doc.pages.forEach((p, i) => m.set(p.name, i));
    return m;
  })
);

/** The pages shown in the main content area, in feed order. Memoized: the O(feed)
 *  resolve runs once per structural change, not on each of its ~7 calls per render. */
export const mainPages = createRoot(() =>
  createMemo((): readonly ReadonlyFeedPage[] => {
    const idx = pageIndexByName();
    return doc.feed
      .map((n) => {
        const i = idx.get(n);
        return i === undefined ? undefined : doc.pages[i];
      })
      .filter(Boolean) as FeedPage[];
  })
);

/** A loaded page record by name (anywhere in the working set), or undefined. */
export function pageByName(name: string): ReadonlyFeedPage | undefined {
  const i = pageIndexByName().get(name);
  return i === undefined ? undefined : doc.pages[i];
}

/** Record the identity chosen for a new page after its first successful save. */
export function setPageId(name: string, id: string): void {
  const i = pageIndexByName().get(name);
  if (i !== undefined) setDoc("pages", i, "id", id);
}

/** The format ("md"/"org") to parse a page's inline content with. Exact for a
 *  loaded page; for one that isn't loaded (e.g. the source of a backlink) fall back
 *  to the graph's preferred format — correct for single-format graphs, a safe guess
 *  otherwise (and far better than always assuming Markdown). Used by the inline
 *  renderers (InlineText callers) so org markup in property values / breadcrumbs /
 *  reference previews / block-refs renders as org, not literally. */
export function formatForPage(name: string | undefined): Format {
  if (name) {
    const p = pageByName(name);
    if (p?.format) return p.format;
  }
  return graphMeta()?.preferred_format ?? "md";
}

/** Like {@link formatForPage} but keyed by a block id (→ its page). */
export function formatForBlock(id: string | undefined): Format {
  return formatForPage(id ? doc.byId[id]?.page : undefined);
}

export function blockIsGridView(id: string | undefined): boolean {
  const n = id ? doc.byId[id] : undefined;
  return !!n && sheetConfigFromRaw(n.raw, formatForBlock(id)).view === "grid";
}

export function blockIsOpaqueSheetView(id: string | undefined): boolean {
  const n = id ? doc.byId[id] : undefined;
  const view = n ? sheetConfigFromRaw(n.raw, formatForBlock(id)).view : null;
  return view === "grid" || view === "table" || view === "board";
}

let idCounter = 0;
export function freshId(): string {
  return `b${Date.now().toString(36)}-${idCounter++}`;
}
