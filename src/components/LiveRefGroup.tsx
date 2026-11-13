import { For, Show, batch, createEffect, createMemo, createSignal, createUniqueId, on, onCleanup, onMount, untrack, useContext, type JSX, type Signal } from "solid-js";
import { nearestAncestors } from "../render/blockTrail";
import { createGraphResource } from "../graphResource";
import { backend } from "../backend";
import { graphOwner, latestOwner } from "../owned";
import { admitRead, beginPageHeaderEdit, blockProperty, blockRef, resolveBlockRef, collapseEpochOf, ensurePageLoaded, formatForBlock, formatForPage, pageByName, pinPageNearViewport, setBlockProperty, setCollapsed, setCollapsedBlocks, node as docNode } from "../document";
import { loadSourcePage } from "../sourcePage";
import { blockDtoExternalId } from "../blockIdentity";
import { Block, BulletOpenContext, CollapseSurfaceContext, EmbedNavExitContext, OutlineScopeContext, SurfaceContext, type CollapseSurfaceApi } from "./Block";
import { RefBlocks } from "./RefBlocks";
import { detectMacro } from "./Rendered";
import { isBlockEmbedMacro } from "./blockParts";
import { observeNear, trackNear, unobserveNear } from "../lazyObserve";
import type { BlockDto, PageKind, ReferenceBlockEvidence } from "../types";
import { graphEpoch, graphMeta } from "../graphSession";
import { OccurrenceControls, ReferenceExcerptBlocks, occurrenceSelection } from "./ReferenceEvidence";
import { editingId, startEditing } from "../editorController";
import { visibleBody } from "../render/block";
import { InlineText } from "../render/inline";
import { LinkDepthContext } from "./linkDepth";
import { foldsByDefaultInResults } from "../refOpenLevel";
import { readOr } from "../resourceRead";
import { openBlockInSidebar, openPageProps } from "../ui";
import "./LiveRefGroup.css";
import { ResultWriteTargetContext, liveWriteTarget, noticeChangedResults, resolveResultRow, resultWriteTarget, syntheticResultRowLive } from "./queryResultIdentity";
import { resultTicket, resultsTicketActionable } from "../queryResultsEpoch";
import { ReservedGroupGeometryContext, reservedGroupHeight } from "../reservedGroupHeight";
import { preserveAboveViewport } from "../scrollContainer";
import { registerQueryResultFold } from "../collapseCommandSurface";
import { observeViewportWindow } from "../viewportWindow";
import { printingOutline, registerOutlineWindow } from "../outlineViewport";
import { queueReferenceCommit, queueReferenceRead } from "../referenceAdmission";
import { trackRetirementPin } from "../retirementPins";
import { BlockList } from "./BlockList";
import { CommentSurface } from "./CommentSurface";
import { promotePagePreamble, pageInstanceGeneration, topSelected } from "../document";
import { rendersAsPageHeader, splitPagePreamble } from "../editor/properties";
import { t } from "../i18n";
import { retainResultDraft } from "../queryResultDrafts";

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
  /** Owner-provided identity for reference groups; never derived from a row. */
  heightKey?: string;
  page: string;
  kind: PageKind;
  path?: string;
  blocks: BlockDto[];
  embedId?: string;
  /** A page transclusion renders the loaded page's current roots, rather than
   * retaining a reference result's membership list across editor sessions. */
  wholePage?: boolean;
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
// offscreen group (I-25). Reference owners supply heightKey; queries also retain
// their outer shell's height. The bounded, graph-scoped cache checks pane width.

interface LocalCollapseRow { v: boolean; bulk?: boolean }
interface RefOccurrence {
  surface: string;
  initialCollapsed: Map<string, boolean>;
  localCollapsed: Signal<Record<string, LocalCollapseRow>>;
  contextRoots: Signal<Record<string, string>>;
  excerptFull: Signal<Record<string, boolean>>;
  pendingEdits: Signal<number>;
  geometry: Map<string, { width: number; height: number }>;
}

export function LiveRefGroup(props: LiveRefGroupProps): JSX.Element {
  const [near, setNear] = createSignal(untrack(() => props.eager === true));
  const [inWindow, setInWindow] = createSignal(untrack(() => props.eager === true));
  const [visible, setVisible] = createSignal(false);
  const [forced, setForced] = createSignal(false);
  const [interaction, setInteraction] = createSignal(false);
  const [visited, setVisited] = createSignal(untrack(() => props.eager === true));
  let el: HTMLDivElement | undefined;
  // Kept current while alive: a cleanup must not read props, whose getters may
  // already answer for an unmounted embed.
  const ownerSurface = useContext(SurfaceContext);
  // Only reference surfaces retire. Queries, embeds and previews keep their
  // existing owners; this state also stays stable across reference retirement.
  const occurrence: RefOccurrence = {
    surface: props.surfaceKey ?? (props.surface === "embed" && props.hostBlockId
      ? `embed:${ownerSurface}\u0000${props.hostBlockId}` : `${props.surface === "embed" ? "embed" : "ref"}:` + createUniqueId()),
    initialCollapsed: new Map(), localCollapsed: createSignal({}), contextRoots: createSignal({}),
    excerptFull: createSignal({}), pendingEdits: createSignal(0), geometry: new Map(),
  };
  let alive = true;
  const [hasViewport, setHasViewport] = createSignal(false);
  onCleanup(() => { alive = false; });
  const windowed = () => props.surface === "ref" && !props.eager;
  const pinned = () => interaction() || occurrence.pendingEdits[0]() > 0 || (!!editingId() && docNode(editingId()!)?.page === props.page)
    || topSelected().some(id => docNode(id)?.page === props.page);
  const wanted = () => inWindow() || forced() || (near() && (pinned() || printingOutline()));
  const priority = () => visible() || forced() ? 0 : 1;
  const reserved = reservedGroupHeight(() => {
    const owner = props.heightKey ?? props.hostBlockId;
    return owner === undefined ? undefined : JSON.stringify([
      ownerSurface, props.surface, owner, props.kind, props.path ?? props.page,
      props.embedId, props.blocks.length, !!props.excerpt,
    ]);
  }, () => el, near, windowed());
  createEffect(() => { if (near()) setVisited(true); });
  createEffect(() => {
    if (!windowed() || !hasViewport()) return;
    if (!wanted()) { setNear(false); return; }
    if (near()) return;
    let current = true;
    void queueReferenceCommit(() => {
      if (el) preserveAboveViewport(el, () => setNear(true), undefined, true);
    }, () => current && alive && wanted(), priority);
    onCleanup(() => { current = false; });
  });
  createEffect(() => {
    if (!near() || !el || !windowed()) return;
    const pin = trackRetirementPin(el);
    createEffect(() => setInteraction(pin.pinned()));
    onCleanup(() => { pin.stop(); setInteraction(false); });
  });
  onMount(() => {
    if (!el) return;
    const node = el;
    if (windowed() && typeof node.ownerDocument.defaultView?.IntersectionObserver === "function") {
      setHasViewport(true);
      onCleanup(observeViewportWindow(node, (entry, margin) => {
        preserveAboveViewport(node, () => {
          if (!entry.isIntersecting && untrack(near) && entry.boundingClientRect) reserved.remember(entry.boundingClientRect.height, entry.boundingClientRect.width);
          setInWindow(entry.isIntersecting);
          if (entry.isIntersecting) setForced(false);
        }, undefined, false, entry, margin);
      }, "viewport"));
      onCleanup(observeViewportWindow(node, entry => {
        setVisible(entry.isIntersecting);
        if (entry.isIntersecting) setInWindow(true);
      }, 0));
      return;
    }
    if (near()) return;
    observeNear(node, entry => preserveAboveViewport(node, () => setNear(true), undefined, false, entry, 1200));
    onCleanup(() => unobserveNear(node));
  });
  createEffect(() => {
    if (!el || !windowed() || !visited()) return;
    onCleanup(registerOutlineWindow(props.blocks.map(block => block.id), el, () => {
      if (!untrack(inWindow)) setForced(true);
      setNear(true);
    }));
  });
  const estimate = () => Math.max(1, props.blocks.reduce((lines, block) => lines + (block.page_property ? block.raw.split("\n").length : 1), 0)) * 1.9;

  return (
    <div ref={el} class="live-ref-group"
      style={!near() ? { "min-height": reserved() ? `${reserved()}px` : `${estimate()}em` } : undefined}>
      <ReservedGroupGeometryContext.Provider value={occurrence.geometry}>
      <Show when={near()}><MountedRefGroup {...props} occurrence={occurrence} geometryHost={() => el}
        readCurrent={() => alive && (!windowed() || !hasViewport() || wanted())} priority={priority}
        sourcePinned={() => windowed() && (pinned() || forced())}
        schedule={windowed() && hasViewport()} /></Show>
      </ReservedGroupGeometryContext.Provider>
    </div>
  );
}

// The shell retains occurrence choices and geometry. Row maps, resources and
// rendering belong to each disposable child; unchanged rows stay keyed while
// that child is mounted.
function MountedRefGroup(props: LiveRefGroupProps & { occurrence: RefOccurrence; geometryHost: () => HTMLElement | undefined;
  readCurrent: () => boolean; priority: () => number; sourcePinned: () => boolean; schedule: boolean }): JSX.Element {
  const linkDepth = useContext(LinkDepthContext);
  // The host block's own outline and surface, read before this group provides its own.
  const hostScope = useContext(OutlineScopeContext);
  const readScope = {};
  let alive = true;
  onCleanup(() => { alive = false; });

  // GH #849: only the viewport moves a source page in or out of the working set.
  // A group near the view pins its page; once away it may be evicted (its rows
  // fall back to their DTO copies) and it loads again on coming back. So a page
  // answering a read can never evict what another read needs, and an idle page has
  // no loads left to do, however many source pages it references.
  const [nearView, setNearView] = createSignal(true);
  const [acquire, setAcquire] = createSignal(0);
  onCleanup(pinPageNearViewport(() => nearView() || props.sourcePinned() ? props.page : null));
  onMount(() => {
    const host = props.geometryHost();
    if (host) onCleanup(trackNear(host, setNearView));
  });
  createEffect(on(() => [nearView(), !!pageByName(props.page)] as const, ([isNear, present], previous) => {
    if (previous && isNear && !present && (!previous[0] || previous[1])) setAcquire(n => n + 1);
  }));
  // Load the source page only once the group is near the viewport. A string
  // source: the parent recomputing an equal group must not read again.
  const [readyResource] = createGraphResource(
    () => JSON.stringify([props.page, props.kind, props.path ?? null, acquire()]),
    async (source) => {
      const scheduled = props.schedule;
      const priority = props.priority;
      const [p, k, savedPath] = JSON.parse(source) as [string, PageKind, string | null];
      const path = savedPath ?? undefined;
      const occupied = pageByName(p);
      if (occupied) return occupied.kind === k && (!path || occupied.id === path);
      const epoch = graphEpoch();
      const root = graphMeta()?.root ?? "";
      const owner = latestOwner(readScope, "page", graphOwner(() => alive && props.readCurrent() &&
        graphEpoch() === epoch && (graphMeta()?.root ?? "") === root));
      const read = () => path ? backend().getPageByPath(path) : backend().getPage(p, k);
      const result = await admitRead(owner, () => scheduled ? queueReferenceRead(read, owner, priority) : read(), dto => [p, ...(dto ? [dto.name] : [])], dto => {
        // The component may have unmounted while this read was in flight. Never
        // let an old graph's DTO enter the new graph's shared working set.
        if (graphEpoch() !== epoch || (graphMeta()?.root ?? "") !== root) return false;
        // Page names are not unique across kinds, while the frontend working set
        // is name-keyed. Refuse a page/journal twin that occupied the slot during
        // the await, and reject a mismatched backend response defensively.
        const after = pageByName(p);
        if (after) return after.kind === k && (!path || after.id === path);
        if (!dto || dto.name !== p || dto.kind !== k || (path && dto.id !== path)) return false;
        if (ensurePageLoaded(dto)) return false;
        const loaded = pageByName(p);
        return loaded?.kind === k && (!path || loaded.id === path);
      }, scheduled ? commit => queueReferenceCommit(commit, owner, priority) : undefined);
      return result.kind === "current" && result.value;
    },
    { storage: initial => {
      const [read, write] = createSignal<boolean | undefined>(initial);
      const anchored = ((value: boolean | undefined | ((previous: boolean | undefined) => boolean | undefined)) => {
        let result: boolean | undefined;
        const change = () => { result = write(value); };
        const host = props.geometryHost();
        if (host) preserveAboveViewport(host, change, undefined, true);
        else change();
        return result;
      }) as typeof write;
      return [read, anchored];
    } }
  );
  // A source page that failed to load leaves the group on its DTO path below,
  // which is the same path it uses before the page is near the viewport.
  // Page lookup observes the working-set index. Stop another page loading from
  // invalidating this group's breadcrumbs and rows when its source is unchanged.
  const sourcePage = createMemo(() => pageByName(props.page));
  const liveNode = (id: string) => {
    const page = sourcePage();
    if (!page || page.kind !== props.kind || (props.path && page.id !== props.path)) return undefined;
    if (props.surface === "embed" && props.wholePage) {
      // These keys come from this exact loaded outline, not a result DTO whose
      // positional locator can now denote a different block after an edit.
      const source = docNode(id);
      return source?.page === props.page ? source : undefined;
    }
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
      // Hydration needs source identity, not write permission. Published pages
      // are read-only too: Block guards edits and uses presentation-only folds,
      // while the shallow membership DTO has no descendants to render.
      return source === id ? docNode(id) : undefined;
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
  const [promotedPreamble, setPromotedPreamble] = createSignal<{ id: string; node: ReturnType<typeof docNode>; instance: number | null } | null>(null);
  const promotedPreambleId = () => {
    const promoted = promotedPreamble();
    return promoted && docNode(promoted.id) === promoted.node && promoted.instance === pageInstanceGeneration(props.page) ? promoted.id : null;
  };
  let releasePreambleDraft: (() => void) | undefined;
  const releasePreamble = () => { releasePreambleDraft?.(); releasePreambleDraft = undefined; };
  onCleanup(releasePreamble);
  createEffect(() => {
    const id = promotedPreambleId();
    if (!id || editingId() !== id) releasePreamble();
  });
  const queryReady = createMemo(() => props.surface !== "query"
    // Promotion changes the answer epoch, including a matching peer header's
    // ticket. Its stale DTO must not hide the editor whose exact live source
    // this group still owns. Other rows still resolve through liveNode.
    || (!!promotedPreambleId() && props.blocks.some(block => block.page_preamble))
    // A synthetic row follows the ordinary rows' mount rule (syntheticResultRowLive):
    // neither the user's own typing and save (results unsettled, epoch unmoved) nor
    // a structural edit (epoch moved, page instance unchanged) swaps the whole group
    // to static rows and unmounts the editor being typed in.
    || props.blocks.every(block => block.page_property || block.page_preamble
      ? (block.page_preamble && !!promotedPreambleId()) || syntheticResultRowLive(block, props.page)
      : !!liveNode(block.id)));
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
    const page = sourcePage();
    const first = page && page.kind === props.kind ? docNode(page.roots[0] ?? "") : undefined;
    return first?.originatedFromPageHeader && editingId() === first.id ? first.id : null;
  };
  const liveIdOf = (b: BlockDto): string => (b.page_property && headerBlockId()) || (b.page_preamble && promotedPreambleId()) || b.id;
  // GH #753: the materialized header survives editing. Return to the DTO
  // renderer on exit, using current header text until the backlink refresh
  // arrives (including an emptied header, whose transient node is removed).
  // GH #886: the row keeps its object identity while its text is unchanged.
  // RefBlocks keys rows by object, so a fresh copy per evaluation rebuilt every
  // property row whenever the source page was evicted and loaded again; across
  // hundreds of groups that churn more than doubled renderer memory.
  const readOnlyRows = new WeakMap<BlockDto, BlockDto>();
  const readOnlyRow = (b: BlockDto): BlockDto => {
    if (!b.page_property) return b;
    const kept = readOnlyRows.get(b) ?? b;
    if (!ready()) return kept;
    const page = sourcePage();
    if (!page || page.kind !== props.kind || (props.path && page.id !== props.path)) return kept;
    const first = docNode(page.roots[0] ?? "");
    const raw = first?.originatedFromPageHeader ? first.raw : page.preBlock ?? "";
    if (raw.trimEnd() === b.raw.trimEnd()) {
      readOnlyRows.delete(b);
      return b;
    }
    if (kept !== b && kept.raw === raw) return kept;
    const row = { ...b, raw };
    readOnlyRows.set(b, row);
    return row;
  };
  // OG edits the referencing page's properties block in place (og-1.0.0
  // block.cljs block-content/properties-cp on a pre-block). Tine's edit path for
  // it is the source page's header editor (Page.tsx editPageHeader); an org page
  // or a non-header preamble has none there either and opens the properties panel.
  const editPageProperties = async (event: MouseEvent): Promise<void> => {
    const x = event.clientX;
    const y = event.clientY;
    const target = { page: props.page, pageKind: props.kind, path: props.path };
    const owner = graphOwner(() => alive);
    const setPending = props.occurrence.pendingEdits[1];
    setPending(n => n + 1);
    try {
      if (!(await loadSourcePage(target, "Its properties were not opened.")) || !owner()) return;
      const id = beginPageHeaderEdit(target.page);
      if (id) startEditing(id, docNode(id).raw.length, null, surface);
      else openPageProps(target.page, x, y);
    } finally { setPending(n => n - 1); }
  };
  const editPreamble = async (hit: BlockDto): Promise<void> => {
    // Pinned while the source page loads, as editPageProperties: a windowed
    // group must not unmount between the click and its editor (GH #886).
    const owner = graphOwner(() => alive);
    const setPending = props.occurrence.pendingEdits[1];
    setPending(n => n + 1);
    try {
      if (!(await loadSourcePage({ page: props.page, pageKind: props.kind, path: props.path }, t("query-calm-results-changed", { count: 1 }))) || !owner()) return;
      promoteLoadedPreamble(hit);
    } finally { setPending(n => n - 1); }
  };
  const promoteLoadedPreamble = (hit: BlockDto): void => {
    const page = pageByName(props.page);
    if (!page || (props.surface === "query" && !resultsTicketActionable(resultTicket(hit)))
      || splitPagePreamble(page.preBlock, page.format).content !== hit.raw) {
      if (props.surface === "query") noticeChangedResults(1);
      return;
    }
    // The answer still names the synthetic row while its editor owns a newly
    // promoted block. Hold that row across publication/epoch changes, just as
    // a result's open property draft does, until this source editor finishes.
    batch(() => {
      releasePreamble();
      releasePreambleDraft = retainResultDraft(hit.id);
      try {
        const id = promotePagePreamble(props.page);
        if (!id) { releasePreamble(); return; }
        setPromotedPreamble({ id, node: docNode(id), instance: pageInstanceGeneration(props.page) });
        startEditing(id, docNode(id).raw.length, null, surface);
      } catch (error) { releasePreamble(); throw error; }
    });
  };
  const grouping = createMemo(() => {
    let ids: readonly string[] = props.blocks.map(liveIdOf);
    const page = props.surface === "embed" && props.wholePage ? sourcePage() : undefined;
    if (props.surface === "embed" && props.wholePage && ready() && page
      && page.kind === props.kind && (!props.path || page.id === props.path)) {
      // GH #868: membership is only the fallback copy. Retaining its roots
      // while Block renders live children duplicates reparented roots and
      // omits newly outdented ones. The whole transclusion shares one topology.
      const first = docNode(page.roots[0] ?? "");
      const header = page.format === "md" && first && first.children.length === 0
        && (first.originatedFromPageHeader || (editingId() !== first.id && rendersAsPageHeader(first.raw, false)));
      ids = header ? page.roots.slice(1) : page.roots;
    }
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
  const [contextRoots, setContextRoots] = props.occurrence.contextRoots;
  const contextRootIds = createMemo(() => new Set(Object.values(contextRoots())));
  createEffect(() => {
    if (!ready()) return;
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
  const surface = props.occurrence.surface;
  const resultRootIds = createMemo(() => new Set(groupedIds()));
  const initialCollapsed = props.occurrence.initialCollapsed;
  // Only ref/query results own local disclosure here. Writable inner embed folds
  // belong to the source; a block embed's root keeps its host override (GH #360).
  const [localCollapsed, setLocalCollapsed] = props.occurrence.localCollapsed;
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
    // and `:ref/default-open-blocks-level` (default 2), then keeps that copy
    // local to the result view (GH #793). Tine's displayed hit is relative depth
    // 0 (OG level 1), so with level 2 the branches immediately below it default
    // folded; a larger level opens deeper, a level of 1 or less folds the hit.
    const initial = stored || (props.surface !== "embed" && depth !== null && foldsByDefaultInResults(depth, hasChildren));
    initialCollapsed.set(id, initial);
    return initial;
  };
  const collapseSurface: CollapseSurfaceApi = {
    collapsed: (id, stored) => {
      if (isEmbed()) {
        const override = embedRootOverride(id);
        if (override !== null) return override;
        return stored;
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
      if (isEmbed()) { setCollapsed(id, !current); return; }
      setLocalCollapsed((state) => ({ ...state, [id]: { v: !current } }));
    },
    setMany: (ids, collapsed) => isEmbed() ? setCollapsedBlocks(ids, collapsed) : setLocalCollapsed((state) => {
      const next = { ...state };
      for (const id of ids) next[id] = { v: collapsed };
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
          next[id] = { v: request.collapsed, bulk: true };
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
            ? { hostBlockId: props.hostBlockId, firstRoot: () => groupedIds()[0], rowHost: (id) => {
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
          // Result surfaces only: embeds and previews keep the ordinary split.
          splitAsChild: props.surface === "query" || props.surface === "ref" ? (id) => resultRootIds().has(id) : undefined,
        }}>
        <LinkDepthContext.Provider value={linkDepth + 1}>
        <CommentSurface page={pageByName(props.page)} roots={groupedIds()}><BlockList ids={groupedIds()} render={(id, authorNeighbors) => {
            const crumb = createMemo<Crumb[]>((previous) => {
              if (!props.showBreadcrumb) return [];
              const all = liveBreadcrumb(id) ?? (dtoById(id)?.breadcrumb ?? []).map((label) => ({ label } as Crumb));
              const tail = all.slice(-3);
              const next = all.length > 3 ? [{ label: "…" }, ...tail] : tail;
              // Identity resolution also observes other working-set loads.
              // Keep unchanged crumbs keyed so their inline bodies stay mounted.
              return previous && previous.length === next.length
                && previous.every((item, i) => item.id === next[i].id && item.label === next[i].label)
                ? previous : next;
            });
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
                        ? <ReferenceExcerptBlocks blocks={[d()]} evidence={evidenceById().has(id) ? [evidenceById().get(id)!] : []} page={props.page} kind={props.kind} path={props.path} fullState={props.occurrence.excerptFull} />
                        : props.surface === "preview"
                          // Until the source page is loaded the preview shows its
                          // capped copy; edits only ever reach the live block.
                          ? <RefBlocks blocks={[readOnlyRow(d())]} page={props.page} pageKind={props.kind} path={props.path}
                              readOnly pageBody onOpenBlock={props.onOpenBlock} />
                          : <RefBlocks blocks={[readOnlyRow(d())]} page={props.page} pageKind={props.kind} path={props.path}
                              pageBody={props.surface === "embed"} localFolds={props.surface === "embed"}
                              readOnly={!d().page_property && !liveNode(id)}
                              queryResult={props.surface === "query"}
                              onEditPreamble={d().page_preamble ? () => { void editPreamble(d()); } : undefined}
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
                  <Block id={id} authorNeighbors={authorNeighbors} forceExpanded={contextRootIds().has(id)} hideRefCount={!!props.embedId && id === props.embedId} />
                </Show>
              </>
            );
          }} /></CommentSurface>
        </LinkDepthContext.Provider>
        </OutlineScopeContext.Provider>
        </EmbedNavExitContext.Provider>
        </ResultWriteTargetContext.Provider>
        </SurfaceContext.Provider>
        </CollapseSurfaceContext.Provider>
        </BulletOpenContext.Provider>
  );
}
