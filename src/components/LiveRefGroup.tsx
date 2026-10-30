import { For, Show, createComputed, createEffect, createMemo, createResource, createSignal, createUniqueId, onCleanup, onMount, untrack, useContext, type JSX } from "solid-js";
import { nearestAncestors } from "../render/blockTrail";
import { backend } from "../backend";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { beginPageHeaderEdit, blockProperty, blockRef, blockWritable, resolveBlockRef, collapseEpochOf, ensurePageLoaded, formatForBlock, formatForPage, pageByName, setBlockProperty, setCollapsed, setCollapsedBlocks, node as docNode } from "../document";
import { loadSourcePage } from "../sourcePage";
import { blockDtoExternalId } from "../blockIdentity";
import { Block, BulletOpenContext, CollapseSurfaceContext, EmbedNavExitContext, OutlineScopeContext, SurfaceContext, type CollapseSurfaceApi } from "./Block";
import { RefBlocks } from "./RefBlocks";
import { detectMacro } from "./Rendered";
import { isBlockEmbedMacro } from "./blockParts";
import { observeNear, unobserveNear } from "../lazyObserve";
import type { BlockDto, PageKind, ReferenceBlockEvidence } from "../types";
import { graphEpoch, graphMeta } from "../graphSession";
import { OccurrenceControls, ReferenceExcerptBlocks, occurrenceSelection } from "./ReferenceEvidence";
import { editingId, startEditing } from "../editorController";
import { visibleBody } from "../render/block";
import { InlineText } from "../render/inline";
import { LinkDepthContext } from "./linkDepth";
import { readOr } from "../resourceRead";
import { openBlockInSidebar, openPageProps } from "../ui";
import "./LiveRefGroup.css";
import { ResultWriteTargetContext, liveWriteTarget, noticeChangedResults, resolveResultRow, resultWriteTarget } from "./queryResultIdentity";
import { resultTicket, resultsTicketCurrent } from "../queryResultsEpoch";
import { clearOnBindingInvalidated } from "../binding";
import { registerQueryResultFold } from "../collapseCommandSurface";

// The "near the viewport" lazy-mount observer is shared app-wide (block bodies
// use it too) — see src/lazyObserve.ts.

// Instrumentation seam (GH #185): counts how many times the collapse-state GC
// walk below actually runs. A regression test uses it to prove the walk fires on
// result-membership change but NOT on unrelated descendant edits. Cost is one
// integer increment per real prune.
export const __livRefGroupInternals = { pruneRuns: 0 };

// GH #746: a refresh answers with new objects even for unchanged blocks, and
// rows keyed by object would remount and render their text again after every
// save. Keep the previous object while its content is the same. Query rows
// carry their answer's results ticket on the object, so they are left as is.
const contentOf = new WeakMap<object, string>();
const content = (value: object) => {
  let text = contentOf.get(value);
  if (text === undefined) contentOf.set(value, text = JSON.stringify(value));
  return text;
};
function keepUnchanged<T extends object>(previous: Map<string, T> | undefined, entries: [string, T][]): Map<string, T> {
  return new Map(entries.map(([id, next]) => {
    const old = previous?.get(id);
    return [id, old && content(old) === content(next) ? old : next];
  }));
}

// Renders result/backlink/embed blocks as LIVE editable <Block>s, but LAZILY:
// the group is a reserved-height placeholder until it scrolls within ~1.2 screens
// of the viewport (IntersectionObserver), at which point its source page is
// loaded and its blocks mount. This is the windowing trick that keeps a broad
// query (hundreds of hits across many pages) cheap — only what's near the
// viewport is ever mounted; the rest stays a cheap spacer and hydrates on scroll.
//
// Each block is the same component the main view uses, so editing a result edits
// the real block and saves to its page. Keyed by uuid so a reactive refresh
// reuses existing rows and never yanks the caret out of a block being edited.
interface LiveRefGroupProps {
  page: string;
  kind: PageKind;
  path?: string;
  blocks: BlockDto[];
  embedId?: string;
  /** The block whose `{{embed}}` macro renders this group (embed surface), or whose query
   *  these are the results of (query surface: GH #792, so one-level fold commands reach them). */
  hostBlockId?: string;
  showBreadcrumb?: boolean;
  /** "preview": a hover preview (D-20261023-VQ). Folds are the source's own
   *  (GH #748), so no local collapse surface; no breadcrumbs. */
  surface: "ref" | "query" | "embed" | "preview";
  /** Preview only: the stable surface key the popup watches for an active editor. */
  surfaceKey?: string;
  /** Preview only: a plain bullet click opens the source (the popup's choice). */
  onOpenBlock?: (id: string) => void;
  evidence?: ReferenceBlockEvidence[];
  /** Unlinked mentions retain their bounded excerpt UI until context is pulled in. */
  excerpt?: boolean;
  /** The caller already gated this group on viewport proximity (a query group that mounted its header): mount the
   *  rows with it. A second, independent IntersectionObserver gate would let the header render with no rows
   *  whenever the two observers disagree (the header gate fires, the row gate never does). */
  eager?: boolean;
  /** Query-owned bulk action; never supplied by reference or embed callers. */
  bulkFold?: { n: number; collapsed: boolean; epochs: ReadonlyMap<string, number> };
}

// GH #810: the height an embed's rows had when it last unmounted, so its placeholder
// reserves it. A scrolled-out outline window keeps a spacer of its measured height,
// and the window comes back with its groups unmounted until each one is near
// again: a placeholder of one row's estimate made the window about one embed
// shorter than its spacer, and every block below it jumped (and jumped back when
// the rows mounted). Keyed by the embedding block, so it reads no row of an
// offscreen group (I-25); other surfaces have no such identity and keep the
// estimate. Bounded and graph-scoped like BlockList's window heights; the width
// guards a pane resized in between.
const groupHeights = new Map<string, { width: number; height: number }>();
const MAX_GROUP_HEIGHTS = 512;
clearOnBindingInvalidated(() => groupHeights.clear());

export function LiveRefGroup(props: LiveRefGroupProps): JSX.Element {
  const [near, setNear] = createSignal(untrack(() => props.eager === true));
  const [reserved, setReserved] = createSignal(0);
  let el: HTMLDivElement | undefined;
  // Kept current while alive: a cleanup must not read props, whose getters may
  // already answer for an unmounted embed.
  let key: string | undefined;
  createComputed(() => {
    key = props.hostBlockId === undefined ? undefined
      : `${props.hostBlockId}\0${props.embedId ?? props.page}\0${props.blocks.length}`;
  });
  onMount(() => {
    if (!el || near()) return;
    const node = el;
    const remembered = key === undefined ? undefined : groupHeights.get(key);
    if (remembered && remembered.width === node.clientWidth) setReserved(remembered.height);
    observeNear(node, () => setNear(true));
    onCleanup(() => unobserveNear(node));
  });
  onCleanup(() => {
    if (key === undefined || !el?.isConnected || !untrack(near)) return;
    const height = el.getBoundingClientRect().height;
    if (height <= 0) return;
    groupHeights.delete(key);
    groupHeights.set(key, { width: el.clientWidth, height });
    if (groupHeights.size > MAX_GROUP_HEIGHTS) groupHeights.delete(groupHeights.keys().next().value!);
  });

  return (
    <div ref={el} class="live-ref-group"
      style={!near() ? { "min-height": reserved() ? `${reserved()}px` : `${Math.max(1, props.blocks.length) * 1.9}em` } : undefined}>
      <Show when={near()}><MountedRefGroup {...props} /></Show>
    </div>
  );
}

// Offscreen groups own only their spacer and observation. Their row maps,
// resources and local disclosure state are created together on first approach.
// The mounted child keeps the existing render-once lifecycle and keyed rows.
function MountedRefGroup(props: LiveRefGroupProps): JSX.Element {
  const linkDepth = useContext(LinkDepthContext);
  // The host block's own outline and surface, read before this group provides its own.
  const hostScope = useContext(OutlineScopeContext);
  const hostSurface = useContext(SurfaceContext);
  const readScope = {};
  let alive = true;
  onCleanup(() => { alive = false; });

  // Load the source page only once the group is near the viewport.
  const [readyResource] = createResource(
    () => ({ p: props.page, k: props.kind, path: props.path }),
    async ({ p, k, path }) => {
      const occupied = pageByName(p);
      if (occupied) return occupied.kind === k && (!path || occupied.id === path);
      const epoch = graphEpoch();
      const root = graphMeta()?.root ?? "";
      const owner = latestOwner(readScope, "page", graphOwner(() => alive &&
        graphEpoch() === epoch && (graphMeta()?.root ?? "") === root));
      const result = await readOwned(owner, path ? backend().getPageByPath(path) : backend().getPage(p, k));
      if (result.kind === "stale") return false;
      const dto = result.value;
      // The component may have unmounted while this read was in flight. Never
      // let an old graph's DTO enter the new graph's shared working set.
      if (graphEpoch() !== epoch || (graphMeta()?.root ?? "") !== root) return false;
      // Page names are not unique across kinds, while the frontend working set
      // is name-keyed. Refuse a page/journal twin that occupied the slot during
      // the await, and reject a mismatched backend response defensively.
      const after = pageByName(p);
      if (after) return after.kind === k && (!path || after.id === path);
      if (!dto || dto.name !== p || dto.kind !== k || (path && dto.id !== path)) return false;
      if (ensurePageLoaded(dto)) return false; // declined: stay on the DTO path
      const loaded = pageByName(p);
      return loaded?.kind === k && (!path || loaded.id === path);
    }
  );
  // A source page that failed to load leaves the group on its DTO path below,
  // which is the same path it uses before the page is near the viewport.
  const liveNode = (id: string) => {
    const page = pageByName(props.page);
    if (!page || page.kind !== props.kind || (props.path && page.id !== props.path)) return undefined;
    const dto = dtoById(id);
    if (!dto) {
      // Ancestors, descendants and a materialized page header already carry
      // live keys, but still belong to this group's exact source page.
      const source = docNode(id);
      return source?.page === props.page ? source : undefined;
    }
    if (props.surface !== "query") {
      // Every DTO -> live row crosses the same page-qualified boundary. A
      // remapped duplicate stays on its DTO: passing its old key to Block or
      // outline navigation would expose a different page's node (GH #748).
      // A runtime locator stays valid on that page when Copy block ref stamps
      // its first authored id; the row keeps its live key until the next load.
      const source = resolveBlockRef({ uuid: blockDtoExternalId(dto), page: props.page, pageKind: props.kind, ...(props.path ? { path: props.path } : {}) }, { navigation: true });
      return source === id && blockWritable(id) ? docNode(id) : undefined;
    }
    const source = resolveResultRow({ id, dto, page: props.page, kind: props.kind, path: props.path }, "mount");
    return source?.id === id ? source : undefined;
  };
  const ready = () => readOr(readyResource, undefined, "reference group source page") && queryReady();

  // O(1) id → dto. The prior `props.blocks.find` inside the per-row <For> was
  // O(N) per row → O(N²) per group (250k iterations on a 500-block hub group).
  const byId = createMemo<Map<string, BlockDto>>((previous) =>
    keepUnchanged(props.surface === "query" ? undefined : previous, props.blocks.map((b) => [b.id, b])));
  const evidenceById = createMemo<Map<string, ReferenceBlockEvidence>>((previous) =>
    keepUnchanged(previous, (props.evidence ?? []).map((item) => [item.block_id, item])));
  const dtoById = (id: string) => byId().get(id);
  const queryReady = createMemo(() => props.surface !== "query"
    || props.blocks.every(block => block.page_property ? resultsTicketCurrent(resultTicket(block)) : !!liveNode(block.id)));
  // OG groups a page's matches by `:block/parent` and renders ONE breadcrumb per
  // parent group (block.cljs custom-query-results: `(group-by :block/parent ..)` +
  // `breadcrumb-with-container`), with the page's own top-level blocks first.
  // A hydrated page names the parent exactly; before hydration the DTO breadcrumb
  // is the best available key (equal labels, same parent in practice).
  const parentKey = (id: string): string => {
    if (ready() && liveNode(id)) return `p:${liveNode(id)!.parent ?? ""}`;
    return `c:${(dtoById(id)?.breadcrumb ?? []).join("\u0001")}`;
  };
  // GH #486: the page-property row (a synthetic backend id) becomes the source
  // page's own header block only while that header is being edited, so
  // the edit happens in place, live, exactly as on the source page.
  const headerBlockId = (): string | null => {
    if (!ready()) return null;
    const page = pageByName(props.page);
    const first = page && page.kind === props.kind ? docNode(page.roots[0] ?? "") : undefined;
    return first?.originatedFromPageHeader && editingId() === first.id ? first.id : null;
  };
  const liveIdOf = (b: BlockDto): string => (b.page_property && headerBlockId()) || b.id;
  // GH #753: the materialized header survives editing. Return to the DTO
  // renderer on exit, using current header text until the backlink refresh
  // arrives (including an emptied header, whose transient node is removed).
  const readOnlyRow = (b: BlockDto): BlockDto => {
    if (!b.page_property || !ready()) return b;
    const page = pageByName(props.page);
    if (!page || page.kind !== props.kind || (props.path && page.id !== props.path)) return b;
    const first = docNode(page.roots[0] ?? "");
    return { ...b, raw: first?.originatedFromPageHeader ? first.raw : page.preBlock ?? "" };
  };
  // OG edits the referencing page's properties block in place (og-1.0.0
  // block.cljs block-content/properties-cp on a pre-block). Tine's edit path for
  // it is the source page's header editor (Page.tsx editPageHeader); an org page
  // or a non-header preamble has none there either and opens the properties panel.
  const editPageProperties = async (event: MouseEvent): Promise<void> => {
    const x = event.clientX;
    const y = event.clientY;
    if (!(await loadSourcePage({ page: props.page, pageKind: props.kind, path: props.path }, "Its properties were not opened."))) return;
    const id = beginPageHeaderEdit(props.page);
    if (id) startEditing(id, docNode(id).raw.length, null, surface);
    else openPageProps(props.page, x, y);
  };
  const grouping = createMemo(() => {
    const ids = props.blocks.map(liveIdOf);
    if (!props.showBreadcrumb) return { ids, starts: new Set<string>(ids) };
    const groups = new Map<string, string[]>();
    for (const id of ids) {
      const key = parentKey(id);
      const list = groups.get(key);
      if (list) list.push(id);
      else groups.set(key, [id]);
    }
    // Hydrated top-level blocks have a null parent (`p:`); DTO ones an empty crumb.
    const isTop = (key: string, l: string[]) =>
      key === "p:" || (key.startsWith("c:") && (dtoById(l[0])?.breadcrumb ?? []).length === 0);
    const entries = [...groups.entries()];
    const ordered = [
      ...entries.filter(([k, l]) => isTop(k, l)),
      ...entries.filter(([k, l]) => !isTop(k, l)),
    ].map(([, l]) => l);
    return { ids: ordered.flat(), starts: new Set(ordered.map((l) => l[0])) };
  });
  // A breadcrumb changes only its parent group's root. This presentation state
  // belongs to this mounted occurrence, never to the source page or session.
  const [contextRoots, setContextRoots] = createSignal<Record<string, string>>({});
  const contextRootIds = createMemo(() => new Set(Object.values(contextRoots())));
  createEffect(() => {
    const keys = new Set(grouping().ids.map(parentKey));
    untrack(() => setContextRoots((roots) => {
      const entries = Object.entries(roots).filter(([key]) => keys.has(key));
      return entries.length === Object.keys(roots).length ? roots : Object.fromEntries(entries);
    }));
  });
  const displayed = createMemo(() => {
    const ids: string[] = [];
    const starts = new Map<string, string>();
    for (const id of grouping().ids) {
      const key = parentKey(id);
      const root = contextRoots()[key];
      if (root && docNode(root)) {
        if (grouping().starts.has(id)) {
          ids.push(root);
          starts.set(root, key);
        }
      } else {
        ids.push(id);
        if (grouping().starts.has(id)) starts.set(id, key);
      }
    }
    return { ids, starts };
  });
  const groupedIds = () => displayed().ids;
  /** True when `id` starts a parent group, i.e. its breadcrumb is not a repeat. */
  const startsParentGroup = (id: string): boolean => displayed().starts.has(id);
  interface Crumb { label: string; id?: string }
  // GH #643 (query trails): "…" is the elision marker, not block text.
  const crumbLabel = (c: Crumb): JSX.Element => c.label === "…" && !c.id
    ? <>…</>
    : <span class="ref-crumb-label"><InlineText text={c.label || "…"} format={c.id ? formatForBlock(c.id) : formatForPage(props.page)} /></span>;
  const liveBreadcrumb = (id: string): Crumb[] | null => {
    if (!ready() || !liveNode(id)) return null;

    // The loaded source page is authoritative after hydration; never invent
    // ancestor IDs from result-row labels (src/render/blockTrail.ts).
    const trail = nearestAncestors(id);
    if (!trail) return null;
    // Labels are markup, rendered inline like the block itself (OG block.cljs
    // `breadcrumb`: `map-inline` over the title, no character cut); CSS caps width.
    const tail = trail.ids.map((parent): Crumb => ({ id: parent, label: (visibleBody(docNode(parent)!.raw, formatForBlock(parent))[0] ?? "").trim() }));
    return trail.more ? [{ label: "…" }, ...tail] : tail;
  };
  // A ref/query/embed group can render a block that ALSO lives in the main outline
  // of the same page (e.g. the journal agenda re-lists today's scheduled/deadline
  // bullets). Give this group its own edit "surface" so an UNSCOPED keyboard nav
  // (Up/Down) into such a block focuses the MAIN-outline instance, not this copy —
  // otherwise both instances (same "main" surface) call focus() and the off-screen
  // copy wins, stealing the caret and scrolling the viewport to it. Same mechanism
  // as the right sidebar (see startEditing / focusSurfaceFor). One key per group.
  // An embed's key names its host in the host's surface, so a caret kept on the
  // embed root survives the host moving (Tab, D-20261030-EMB) and remounting it.
  const surface = props.surfaceKey ?? (props.surface === "embed" && props.hostBlockId
    ? `embed:${hostSurface}\u0000${props.hostBlockId}` : `${props.surface === "embed" ? "embed" : "ref"}:` + createUniqueId());
  const resultRootIds = createMemo(() => new Set(groupedIds()));
  const initialCollapsed = new Map<string, boolean>();
  // Local fold rows. The embedded ROOT has a durable occurrence-owned override on
  // its macro host (GH #360); nested rows remain local presentation state whose
  // `epoch` remembers the source collapse generation at fold time, so a later
  // source write reclaims authority. Ref/query surfaces keep their pre-existing
  // local-copy semantics and ignore `epoch`.
  interface LocalCollapseRow { v: boolean; epoch: number; bulk?: boolean }
  const [localCollapsed, setLocalCollapsed] = createSignal<Record<string, LocalCollapseRow>>({});
  const isEmbed = () => props.surface === "embed";
  const embedRootOverride = (id: string): boolean | null => {
    if (!isEmbed() || id !== props.embedId || !props.hostBlockId) return null;
    const value = blockProperty(props.hostBlockId, "collapsed")?.toLowerCase();
    return value === "true" ? true : value === "false" ? false : null;
  };
  const relativeDepth = (id: string): number | null => {
    const roots = resultRootIds();
    if (roots.has(id)) return 0;
    let depth = 0;
    let current = liveNode(id);
    const seen = new Set<string>();
    while (current?.parent && !seen.has(current.id)) {
      seen.add(current.id);
      depth += 1;
      if (roots.has(current.parent)) return depth;
      current = docNode(current.parent);
    }
    return null;
  };
  const defaultCollapsed = (id: string, stored: boolean): boolean => {
    // Embeds are live and source-authoritative (GH #360): never snapshot.
    if (isEmbed()) return stored;
    // Pulled-in context respects descendant source folds. The selected root
    // itself is forced open by Block, just like a parked sidebar block.
    if (contextRootIds().size > 0) {
      let current = liveNode(id);
      const seen = new Set<string>();
      while (current && !seen.has(current.id)) {
        if (contextRootIds().has(current.id)) return stored;
        if (resultRootIds().has(current.id)) break;
        seen.add(current.id);
        current = current.parent ? docNode(current.parent) : undefined;
      }
    }
    const previous = initialCollapsed.get(id);
    if (previous !== undefined) return previous;
    const depth = relativeDepth(id);
    const hasChildren = (liveNode(id)?.children.length ?? 0) > 0;
    // Released OG initializes reference/query disclosure from the source state
    // and default-open level 2, then keeps that copy local to the result view.
    // Tine's displayed hit is relative depth 0, so branches immediately below it
    // default folded.
    const initial = stored || (props.surface !== "embed" && depth !== null && depth >= 1 && hasChildren);
    initialCollapsed.set(id, initial);
    return initial;
  };
  // D-20261030-EMB: inside a block embed, folds below its root are the
  // source's own `collapsed::`, shared everywhere (OG). Page embeds keep 0.8's
  // local folds below their page's blocks.
  const sourceFolds = () => isEmbed() && !!props.embedId;
  const collapseSurface: CollapseSurfaceApi = {
    collapsed: (id, stored) => {
      if (isEmbed()) {
        const override = embedRootOverride(id);
        if (override !== null) return override;
        if (sourceFolds()) return stored;
        // Nested local folds govern only while the source hasn't written
        // another collapse since. The root never enters this map: its explicit
        // true/false host property survives remount and reload.
        const local = localCollapsed()[id];
        return local && local.epoch === collapseEpochOf(id) ? local.v : stored;
      }
      const local = localCollapsed();
      // Bulk actions saved the source. Follow it live, including Undo, while
      // ordinary per-result disclosure keeps its released local semantics.
      if (local[id]?.bulk) return stored;
      return Object.prototype.hasOwnProperty.call(local, id) ? local[id].v : defaultCollapsed(id, stored);
    },
    toggle: (id, current) => {
      if (isEmbed() && id === props.embedId && props.hostBlockId) {
        setBlockProperty(props.hostBlockId, "collapsed", String(!current));
        return;
      }
      if (sourceFolds()) { setCollapsed(id, !current); return; }
      setLocalCollapsed((state) => ({ ...state, [id]: { v: !current, epoch: collapseEpochOf(id) } }));
    },
    setMany: (ids, collapsed) => sourceFolds() ? setCollapsedBlocks(ids, collapsed) : setLocalCollapsed((state) => {
      const next = { ...state };
      for (const id of ids) next[id] = { v: collapsed, epoch: collapseEpochOf(id) };
      return next;
    }),
  };
  // GH #792: the one-level fold commands treat a query block's result rows as its children.
  if (props.surface === "query") onCleanup(registerQueryResultFold({
    hostId: () => props.hostBlockId,
    roots: () => groupedIds().filter((id) => ready() && !!liveNode(id)),
    surface: collapseSurface,
  }));
  let lastBulkVersion: number | undefined;
  createEffect(() => {
    const request = props.bulkFold;
    if (props.surface !== "query" || !request || !ready() || lastBulkVersion === request.n) return;
    lastBulkVersion = request.n;
    untrack(() => {
      const ids = new Set<string>();
      const visit = (id: string) => { if (ids.has(id)) return; ids.add(id); for (const child of liveNode(id)?.children ?? []) visit(child); };
      for (const id of groupedIds()) if (request.epochs.has(id)) visit(id);
      setLocalCollapsed(state => {
        const next = { ...state };
        for (const id of ids) if (request.epochs.get(id) === collapseEpochOf(id)) {
          next[id] = { v: request.collapsed, epoch: collapseEpochOf(id), bulk: true };
        }
        return next;
      });
    });
  });
  // Result DTOs are replaced during filter/query refresh. Retain local choices
  // for stable roots and their live descendants, but discard state once a root
  // leaves this group so an old choice cannot leak into a later membership.
  //
  // GH #185: prune only when result-root MEMBERSHIP changes — the sole moment a
  // stale collapse choice could leak into a new membership. DTO membership and
  // pulled-in context are the dependencies (plus `ready()`); the subtree walk below
  // is wrapped in `untrack` so its doc.byId[...].children reads no longer
  // subscribe this effect to every descendant. Previously they did, so any
  // structural edit anywhere in a large reference subtree re-ran the whole
  // O(subtree) GC walk. A key for a block deleted or moved out of the group
  // between refreshes is never read (only mounted blocks query collapse state)
  // and is reclaimed at the next membership change.
  createEffect(() => {
    if (!ready()) return;
    props.blocks;
    contextRootIds();
    // Page-qualified resolution also tracks identity structure. Its unchanged
    // root IDs must not turn an unrelated child edit into another GC walk.
    const roots = untrack(resultRootIds);
    untrack(() => {
      __livRefGroupInternals.pruneRuns += 1;
      const present = new Set<string>();
      const visit = (id: string) => {
        if (present.has(id)) return;
        present.add(id);
        for (const child of liveNode(id)?.children ?? []) visit(child);
      };
      for (const root of roots) visit(root);
      for (const id of initialCollapsed.keys()) {
        if (!present.has(id)) initialCollapsed.delete(id);
      }
      setLocalCollapsed((state) => {
        let changed = false;
        const next: Record<string, LocalCollapseRow> = {};
        for (const [id, value] of Object.entries(state)) {
          if (present.has(id)) next[id] = value;
          else changed = true;
        }
        return changed ? next : state;
      });
    });
  });
  onCleanup(() => initialCollapsed.clear());
  return (
        <BulletOpenContext.Provider value={props.surface === "preview" ? props.onOpenBlock ?? null : null}>
        <CollapseSurfaceContext.Provider value={props.surface === "preview" ? null : collapseSurface}>
        <SurfaceContext.Provider value={surface}>
        <ResultWriteTargetContext.Provider value={(id) => {
          if (props.surface !== "query") return undefined;
          // A live descendant answers to the result row it is shown under: that
          // row resolves again at commit, and the block must still be inside it.
          const rootOf = (blockId: string) => {
            let current = docNode(blockId) as ReturnType<typeof docNode> | undefined;
            while (current && !dtoById(current.id)) current = current.parent ? docNode(current.parent) : undefined;
            return current?.id;
          };
          const root = rootOf(id);
          const dto = root ? dtoById(root) : undefined;
          if (!root || !dto) return liveWriteTarget(id);
          const row = resultWriteTarget({ id: root, dto, page: props.page, kind: props.kind, path: props.path });
          return () => {
            const resolved = row();
            if (resolved === null) return null;
            if (resolved === root && rootOf(id) === root) return id;
            noticeChangedResults(1);
            return null;
          };
        }}>
        {/* GH #415: Up from the first row of an embed's ROOT row exits the embed
            into the host page; the other rows stay surface-local. */}
        <EmbedNavExitContext.Provider value={
          props.surface === "embed" && props.hostBlockId
            ? { hostBlockId: props.hostBlockId, firstRoot: () => props.blocks[0]?.id, rowHost: (id) => {
              // D-20261030-EMB: a whole-block host's one bullet is its root row.
              const host = props.hostBlockId!;
              return id === props.embedId && isBlockEmbedMacro(detectMacro(docNode(host)?.raw ?? "", formatForBlock(host)))
                ? { id: host, scope: hostScope } : null;
            } }
            : null
        }>
        {/* Master GH #341: arrow navigation out of an edited block in this group
            stays in THIS rendered surface and moves to the adjacent RENDERED block,
            not to the source page's sibling (which mounts an editor outside this
            view and hides the caret). `roots` tracks result membership reactively;
            navOnly keeps structural mutations (merges/indents/moves) on page order. */}
        <OutlineScopeContext.Provider value={{
          // A row left on its read-only copy (not the live block of
          // its own page) never enters navigation or selection: its id may name
          // another page's block.
          get roots() { return groupedIds().filter((id) => ready() && !!liveNode(id)); },
          // A preview shows source folds, so it navigates through them too.
          collapsed: (id, stored) => props.surface === "preview" ? stored
            : contextRootIds().has(id) ? false : collapseSurface.collapsed(id, stored),
          navOnly: true,
        }}>
        <LinkDepthContext.Provider value={linkDepth + 1}>
        <For each={groupedIds()}>
          {(id) => {
            const crumb = () => {
              const all = liveBreadcrumb(id) ?? (dtoById(id)?.breadcrumb ?? []).map((label) => ({ label } as Crumb));
              const tail = all.slice(-3);
              return all.length > 3 ? [{ label: "…" }, ...tail] : tail;
            };
            return (
              <>
                <Show when={props.showBreadcrumb && crumb().length > 0 && startsParentGroup(id)}>
                  <div class="ref-breadcrumb">
                    <For each={crumb()}>
                      {(c, i) => (
                        <>
                          <Show when={i() > 0}>
                            <span class="ref-crumb-sep">›</span>
                          </Show>
                          <Show when={c.id && props.surface !== "embed"} fallback={<span class="ref-crumb">{crumbLabel(c)}</span>}>
                            <span role="button" tabIndex={0} class="ref-crumb" onKeyDown={(event) => {
                              if (event.target !== event.currentTarget || (event.key !== "Enter" && event.key !== " ")) return;
                              event.preventDefault();
                              event.currentTarget.click();
                            }} onClick={(event) => {
                              event.stopPropagation();
                              if (!c.id || !docNode(c.id)) return;
                              if (event.shiftKey) openBlockInSidebar(blockRef(c.id));
                              else {
                                const key = displayed().starts.get(id);
                                if (key !== undefined) setContextRoots((roots) => ({ ...roots, [key]: c.id! }));
                              }
                            }}>{crumbLabel(c)}</span>
                          </Show>
                        </>
                      )}
                    </For>
                  </div>
                </Show>
                <Show
                  when={ready() && liveNode(id) && (!props.excerpt || contextRootIds().has(id))}
                  fallback={
                    <Show when={dtoById(id)}>
                      {(d) => props.excerpt
                        ? <ReferenceExcerptBlocks blocks={[d()]} evidence={evidenceById().has(id) ? [evidenceById().get(id)!] : []} page={props.page} kind={props.kind} path={props.path} />
                        : props.surface === "preview"
                          // Until the source page is loaded the preview shows its
                          // capped copy; edits only ever reach the live block.
                          ? <RefBlocks blocks={[readOnlyRow(d())]} page={props.page} pageKind={props.kind} path={props.path}
                              readOnly pageBody onOpenBlock={props.onOpenBlock} />
                          : <RefBlocks blocks={[readOnlyRow(d())]} page={props.page} pageKind={props.kind} path={props.path}
                              readOnly={!d().page_property && !liveNode(id)}
                              queryResult={props.surface === "query"}
                              onEditPageProperties={(event) => { void editPageProperties(event); }} />}
                    </Show>
                  }
                >
                  <Show when={evidenceById().get(id)}>
                    {(item) => (
                      <div class="reference-live-evidence">
                        <OccurrenceControls
                          evidence={item()}
                          onOccurrence={(span) => startEditing(
                            id,
                            occurrenceSelection(liveNode(id)?.raw ?? "", span, props.page),
                            null,
                            surface,
                          )}
                        />
                      </div>
                    )}
                  </Show>
                  <Block id={id} forceExpanded={contextRootIds().has(id)} hideRefCount={!!props.embedId && id === props.embedId} />
                </Show>
              </>
            );
          }}
        </For>
        </LinkDepthContext.Provider>
        </OutlineScopeContext.Provider>
        </EmbedNavExitContext.Provider>
        </ResultWriteTargetContext.Provider>
        </SurfaceContext.Provider>
        </CollapseSurfaceContext.Provider>
        </BulletOpenContext.Provider>
  );
}
