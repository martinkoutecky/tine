import { For, Show, createEffect, createMemo, createResource, createSignal, createUniqueId, onCleanup, onMount, untrack, useContext, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { blockProperty, collapseEpochOf, ensurePageLoaded, pageByName, setBlockProperty, node as docNode } from "../document";
import { Block, CollapseSurfaceContext, EmbedNavExitContext, OutlineScopeContext, SurfaceContext, type CollapseSurfaceApi } from "./Block";
import { RefBlocks } from "./RefBlocks";
import { observeNear, unobserveNear } from "../lazyObserve";
import type { BlockDto, PageKind, ReferenceBlockEvidence } from "../types";
import { graphEpoch, graphMeta } from "../graphSession";
import { OccurrenceControls, occurrenceSelection } from "./ReferenceEvidence";
import { startEditing } from "../editorController";
import { visibleBody } from "../render/block";
import { LinkDepthContext } from "./linkDepth";
import { readOr } from "../resourceRead";

// The "near the viewport" lazy-mount observer is shared app-wide (block bodies
// use it too) — see src/lazyObserve.ts.

// Instrumentation seam (GH #185): counts how many times the collapse-state GC
// walk below actually runs. A regression test uses it to prove the walk fires on
// result-membership change but NOT on unrelated descendant edits. Cost is one
// integer increment per real prune.
export const __livRefGroupInternals = { pruneRuns: 0 };

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
  /** The block whose `{{embed}}` macro renders this group (embed surface only). */
  hostBlockId?: string;
  showBreadcrumb?: boolean;
  surface: "ref" | "query" | "embed";
  evidence?: ReferenceBlockEvidence[];
  /** The caller already gated this group on viewport proximity (a query group that mounted its header): mount the
   *  rows with it. A second, independent IntersectionObserver gate would let the header render with no rows
   *  whenever the two observers disagree (the header gate fires, the row gate never does). */
  eager?: boolean;
}

export function LiveRefGroup(props: LiveRefGroupProps): JSX.Element {
  const [near, setNear] = createSignal(untrack(() => props.eager === true));
  let el: HTMLDivElement | undefined;
  onMount(() => {
    if (!el || near()) return;
    const node = el;
    observeNear(node, () => setNear(true));
    onCleanup(() => unobserveNear(node));
  });

  return (
    <div ref={el} class="live-ref-group"
      style={!near() ? { "min-height": `${Math.max(1, props.blocks.length) * 1.9}em` } : undefined}>
      <Show when={near()}><MountedRefGroup {...props} /></Show>
    </div>
  );
}

// Offscreen groups own only their spacer and observation. Their row maps,
// resources and local disclosure state are created together on first approach.
// The mounted child keeps the existing render-once lifecycle and keyed rows.
function MountedRefGroup(props: LiveRefGroupProps): JSX.Element {
  const linkDepth = useContext(LinkDepthContext);
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
  const ready = () => readOr(readyResource, undefined, "reference group source page");

  // O(1) id → dto. The prior `props.blocks.find` inside the per-row <For> was
  // O(N) per row → O(N²) per group (250k iterations on a 500-block hub group).
  const byId = createMemo(() => new Map(props.blocks.map((b) => [b.id, b] as const)));
  const evidenceById = createMemo(() => new Map((props.evidence ?? []).map((item) => [item.block_id, item])));
  const dtoById = (id: string) => byId().get(id);
  // OG groups a page's matches by `:block/parent` and renders ONE breadcrumb per
  // parent group (block.cljs custom-query-results: `(group-by :block/parent ..)` +
  // `breadcrumb-with-container`), with the page's own top-level blocks first.
  // A hydrated page names the parent exactly; before hydration the DTO breadcrumb
  // is the best available key (equal labels, same parent in practice).
  const parentKey = (id: string): string => {
    if (ready() && docNode(id)) return `p:${docNode(id).parent ?? ""}`;
    return `c:${(dtoById(id)?.breadcrumb ?? []).join("\u0001")}`;
  };
  const grouping = createMemo(() => {
    const ids = props.blocks.map((b) => b.id);
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
  const groupedIds = () => grouping().ids;
  /** True when `id` starts a parent group, i.e. its breadcrumb is not a repeat. */
  const startsParentGroup = (id: string): boolean => grouping().starts.has(id);
  const liveBreadcrumb = (id: string): string[] | null => {
    if (!ready() || !docNode(id)) return null;

    // The loaded source page is authoritative after hydration. Walk only the
    // nearest four ancestors: three labels are rendered and the fourth proves
    // that an ellipsis is needed. This keeps breadcrumb work O(1) per hit even
    // for malformed or unusually deep outlines, and never invents ancestor IDs
    // from result-row labels.
    const nearest: string[] = [];
    const seen = new Set([id]);
    let parent = docNode(id).parent;
    while (parent !== null && nearest.length < 4) {
      if (seen.has(parent)) return null;
      const ancestor = docNode(parent);
      if (!ancestor) return null;
      seen.add(parent);
      const line = (visibleBody(ancestor.raw)[0] ?? "").trim();
      const chars = [...line];
      nearest.push(chars.length > 60 ? `${chars.slice(0, 60).join("")}…` : line);
      parent = ancestor.parent;
    }
    const tail = nearest.slice(0, 3).reverse();
    return nearest.length > 3 ? ["…", ...tail] : tail;
  };
  // A ref/query/embed group can render a block that ALSO lives in the main outline
  // of the same page (e.g. the journal agenda re-lists today's scheduled/deadline
  // bullets). Give this group its own edit "surface" so an UNSCOPED keyboard nav
  // (Up/Down) into such a block focuses the MAIN-outline instance, not this copy —
  // otherwise both instances (same "main" surface) call focus() and the off-screen
  // copy wins, stealing the caret and scrolling the viewport to it. Same mechanism
  // as the right sidebar (see startEditing / focusSurfaceFor). One key per group.
  const surface = `${props.surface === "embed" ? "embed" : "ref"}:` + createUniqueId();
  const resultRootIds = createMemo(() => new Set(props.blocks.map((block) => block.id)));
  const initialCollapsed = new Map<string, boolean>();
  // Local fold rows. The embedded ROOT has a durable occurrence-owned override on
  // its macro host (GH #360); nested rows remain local presentation state whose
  // `epoch` remembers the source collapse generation at fold time, so a later
  // source write reclaims authority. Ref/query surfaces keep their pre-existing
  // local-copy semantics and ignore `epoch`.
  interface LocalCollapseRow { v: boolean; epoch: number }
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
    let current = docNode(id);
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
    const previous = initialCollapsed.get(id);
    if (previous !== undefined) return previous;
    const depth = relativeDepth(id);
    const hasChildren = (docNode(id)?.children.length ?? 0) > 0;
    // Released OG initializes reference/query disclosure from the source state
    // and default-open level 2, then keeps that copy local to the result view.
    // Tine's displayed hit is relative depth 0, so branches immediately below it
    // default folded.
    const initial = stored || (props.surface !== "embed" && depth !== null && depth >= 1 && hasChildren);
    initialCollapsed.set(id, initial);
    return initial;
  };
  const collapseSurface: CollapseSurfaceApi = {
    collapsed: (id, stored) => {
      if (isEmbed()) {
        const override = embedRootOverride(id);
        if (override !== null) return override;
        // Nested local folds govern only while the source hasn't written
        // another collapse since. The root never enters this map: its explicit
        // true/false host property survives remount and reload.
        const local = localCollapsed()[id];
        return local && local.epoch === collapseEpochOf(id) ? local.v : stored;
      }
      const local = localCollapsed();
      return Object.prototype.hasOwnProperty.call(local, id) ? local[id].v : defaultCollapsed(id, stored);
    },
    toggle: (id, current) => {
      if (isEmbed() && id === props.embedId && props.hostBlockId) {
        setBlockProperty(props.hostBlockId, "collapsed", String(!current));
        return;
      }
      setLocalCollapsed((state) => ({ ...state, [id]: { v: !current, epoch: collapseEpochOf(id) } }));
    },
    setMany: (ids, collapsed) => setLocalCollapsed((state) => {
      const next = { ...state };
      for (const id of ids) next[id] = { v: collapsed, epoch: collapseEpochOf(id) };
      return next;
    }),
  };
  // Result DTOs are replaced during filter/query refresh. Retain local choices
  // for stable roots and their live descendants, but discard state once a root
  // leaves this group so an old choice cannot leak into a later membership.
  //
  // GH #185: prune only when result-root MEMBERSHIP changes — the sole moment a
  // stale collapse choice could leak into a new membership. `resultRootIds()` is
  // the effect's one reactive dependency (plus `ready()`); the subtree walk below
  // is wrapped in `untrack` so its doc.byId[...].children reads no longer
  // subscribe this effect to every descendant. Previously they did, so any
  // structural edit anywhere in a large reference subtree re-ran the whole
  // O(subtree) GC walk. A key for a block deleted or moved out of the group
  // between refreshes is never read (only mounted blocks query collapse state)
  // and is reclaimed at the next membership change.
  createEffect(() => {
    if (!ready()) return;
    const roots = resultRootIds();
    untrack(() => {
      __livRefGroupInternals.pruneRuns += 1;
      const present = new Set<string>();
      const visit = (id: string) => {
        if (present.has(id)) return;
        present.add(id);
        for (const child of docNode(id)?.children ?? []) visit(child);
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
        <CollapseSurfaceContext.Provider value={collapseSurface}>
        <SurfaceContext.Provider value={surface}>
        {/* GH #415: Up from the first row of an embed's ROOT row exits the embed
            into the host page; the other rows stay surface-local. */}
        <EmbedNavExitContext.Provider value={
          props.surface === "embed" && props.hostBlockId
            ? { hostBlockId: props.hostBlockId, firstRoot: () => props.blocks[0]?.id }
            : null
        }>
        {/* Master GH #341: arrow navigation out of an edited block in this group
            stays in THIS rendered surface and moves to the adjacent RENDERED block,
            not to the source page's sibling (which mounts an editor outside this
            view and hides the caret). `roots` tracks result membership reactively;
            navOnly keeps structural mutations (merges/indents/moves) on page order. */}
        <OutlineScopeContext.Provider value={{
          get roots() { return groupedIds(); },
          collapsed: (id, stored) => collapseSurface.collapsed(id, stored),
          navOnly: true,
        }}>
        <LinkDepthContext.Provider value={linkDepth + 1}>
        <For each={groupedIds()}>
          {(id) => {
            const crumb = () => {
              const all = liveBreadcrumb(id) ?? dtoById(id)?.breadcrumb ?? [];
              const tail = all.slice(-3);
              return all.length > 3 ? ["…", ...tail] : tail;
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
                          <span class="ref-crumb">{c}</span>
                        </>
                      )}
                    </For>
                  </div>
                </Show>
                <Show
                  when={ready() && docNode(id)}
                  fallback={
                    <Show when={dtoById(id)}>
                      {(d) => <RefBlocks blocks={[d()]} page={props.page} pageKind={props.kind} />}
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
                            occurrenceSelection(docNode(id)?.raw ?? "", span, props.page),
                            null,
                            surface,
                          )}
                        />
                      </div>
                    )}
                  </Show>
                  <Block id={id} hideRefCount={!!props.embedId && id === props.embedId}
                    dragHostId={props.surface === "embed" && id === props.embedId ? props.hostBlockId : undefined} />
                </Show>
              </>
            );
          }}
        </For>
        </LinkDepthContext.Provider>
        </OutlineScopeContext.Provider>
        </EmbedNavExitContext.Provider>
        </SurfaceContext.Provider>
        </CollapseSurfaceContext.Provider>
  );
}
