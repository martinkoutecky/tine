import { For, Show, createEffect, createMemo, createResource, createSignal, createUniqueId, onCleanup, onMount, untrack, useContext, type JSX } from "solid-js";
import { backend } from "../backend";
import { blockProperty, collapseEpochOf, doc, ensurePageLoaded, formatForPage, pageByName, setBlockProperty } from "../store";
import { Block, CollapseSurfaceContext, EmbedNavExitContext, OutlineScopeContext, SurfaceContext, type CollapseSurfaceApi } from "./Block";
import { RefBlocks } from "./RefBlocks";
import { observeNear, unobserveNear } from "../lazyObserve";
import type { BlockDto, PageKind, ReferenceBlockEvidence } from "../types";
import { graphEpoch, graphMeta } from "../ui";
import { OccurrenceControls } from "./ReferenceEvidence";
import { startEditing } from "../editorController";
import { isBuiltinHidden, rawOffsetToVisibleOffset } from "../editor/properties";
import { graphBinding } from "../persistence";
import { visibleBody } from "../render/block";
import { LinkDepthContext } from "./linkDepth";

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
export function LiveRefGroup(props: {
  page: string;
  kind: PageKind;
  path?: string;
  blocks: BlockDto[];
  embedId?: string;
  hostBlockId?: string;
  showBreadcrumb?: boolean;
  surface: "ref" | "query" | "embed";
  evidence?: ReferenceBlockEvidence[];
}): JSX.Element {
  const linkDepth = useContext(LinkDepthContext);
  const [near, setNear] = createSignal(false);
  let active = true;
  let el: HTMLDivElement | undefined;
  onCleanup(() => {
    active = false;
  });
  onMount(() => {
    if (!el) return;
    const node = el;
    observeNear(node, () => setNear(true));
    onCleanup(() => unobserveNear(node));
  });

  // Load the source page only once the group is near the viewport.
  const [ready] = createResource(
    () => (near() ? { p: props.page, k: props.kind, path: props.path } : null),
    async ({ p, k, path }) => {
      const occupied = pageByName(p);
      if (occupied) return occupied.kind === k && (!path || occupied.path === path);
      const epoch = graphEpoch();
      const root = graphMeta()?.root ?? "";
      const binding = graphBinding();
      const dto = path ? await backend().getPageByPath(path) : await backend().getPage(p, k);
      // The component may have unmounted while this read was in flight. Never
      // let an old graph's DTO enter the new graph's shared working set.
      if (
        !active
        || graphEpoch() !== epoch
        || (graphMeta()?.root ?? "") !== root
        || graphBinding() !== binding
      ) return false;
      // Page names are not unique across kinds, while the frontend working set
      // is name-keyed. Refuse a page/journal twin that occupied the slot during
      // the await, and reject a mismatched backend response defensively.
      const after = pageByName(p);
      if (after) return after.kind === k && (!path || after.path === path);
      if (!dto || dto.name !== p || dto.kind !== k || (path && dto.path !== path)) return false;
      await ensurePageLoaded(dto, {
        expectedGraphBinding: binding,
        isRequestLive: () => active
          && graphEpoch() === epoch
          && (graphMeta()?.root ?? "") === root,
      });
      // Several visible refs/embeds from the same unloaded source can race the
      // same activation. Exactly one installer wins; the others correctly get
      // stale-instance after their captured empty slot has been occupied. That
      // is success for hydration when the winner installed the exact requested
      // page, not a reason to leave those sibling groups on shallow fallbacks.
      // A different kind/path (the actual safety conflict) still stays refused.
      const loaded = pageByName(p);
      return loaded?.kind === k && (!path || loaded.path === path);
    }
  );

  // O(1) id → dto. The prior `props.blocks.find` inside the per-row <For> was
  // O(N) per row → O(N²) per group (250k iterations on a 500-block hub group).
  const byId = createMemo(() => new Map(props.blocks.map((b) => [b.id, b] as const)));
  const evidenceById = createMemo(() => new Map((props.evidence ?? []).map((item) => [item.block_id, item])));
  const dtoById = (id: string) => byId().get(id);
  const liveBreadcrumb = (id: string): string[] | null => {
    if (!ready() || !doc.byId[id]) return null;

    // The loaded source page is authoritative after hydration. Walk only the
    // nearest four ancestors: three labels are rendered and the fourth proves
    // that an ellipsis is needed. This keeps breadcrumb work O(1) per hit even
    // for malformed or unusually deep outlines, and never invents ancestor IDs
    // from result-row labels.
    const nearest: string[] = [];
    const seen = new Set([id]);
    let parent = doc.byId[id].parent;
    while (parent !== null && nearest.length < 4) {
      if (seen.has(parent)) return null;
      const ancestor = doc.byId[parent];
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
  // Local fold rows. The embedded ROOT has a durable occurrence-owned override
  // on its macro host (GH #360); nested rows remain local presentation state.
  // For those nested rows, `epoch` remembers the source collapse generation at
  // fold time, so a later source move reclaims authority. Ref/query surfaces
  // keep their pre-existing local-copy semantics and ignore `epoch`.
  interface LocalCollapseRow { v: boolean; epoch: number }
  const [localCollapsed, setLocalCollapsed] = createSignal<Record<string, LocalCollapseRow>>({});
  const isEmbed = () => props.surface === "embed";
  const embedRootOverride = (id: string): boolean | null => {
    if (!isEmbed() || id !== props.embedId || !props.hostBlockId) return null;
    const value = blockProperty(props.hostBlockId, "collapsed")?.toLowerCase();
    if (value === "true") return true;
    if (value === "false") return false;
    return null;
  };
  const relativeDepth = (id: string): number | null => {
    const roots = resultRootIds();
    if (roots.has(id)) return 0;
    let depth = 0;
    let current = doc.byId[id];
    const seen = new Set<string>();
    while (current?.parent && !seen.has(current.id)) {
      seen.add(current.id);
      depth += 1;
      if (roots.has(current.parent)) return depth;
      current = doc.byId[current.parent];
    }
    return null;
  };
  const defaultCollapsed = (id: string, stored: boolean): boolean => {
    // Embeds are live and source-authoritative (GH #360): never snapshot.
    if (isEmbed()) return stored;
    const previous = initialCollapsed.get(id);
    if (previous !== undefined) return previous;
    const depth = relativeDepth(id);
    const hasChildren = (doc.byId[id]?.children.length ?? 0) > 0;
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
        const local = localCollapsed()[id];
        // Nested local folds govern only while the source hasn't written
        // another collapse since. The root never enters this map: its explicit
        // true/false host property survives remount and reload.
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
      setLocalCollapsed((state) => ({
        ...state,
        [id]: { v: !current, epoch: collapseEpochOf(id) },
      }));
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
        for (const child of doc.byId[id]?.children ?? []) visit(child);
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
    <div
      ref={el}
      class="live-ref-group"
      // Reserve approximate height while unmounted so the scrollbar stays sane.
      style={!near() ? { "min-height": `${Math.max(1, props.blocks.length) * 1.9}em` } : undefined}
    >
      <Show when={near()}>
        <CollapseSurfaceContext.Provider value={collapseSurface}>
        <SurfaceContext.Provider value={surface}>
        {/* GH #341: arrow navigation out of an edited block inside this group
            must stay in THIS rendered surface and move to the adjacent
            RENDERED block — not to the source page's sibling (which mounts an
            editor outside this view and hides the caret). The roots getter
            tracks result membership reactively; navOnly keeps structural
            mutations (merges/indents) on page order, never on display order. */}
        <OutlineScopeContext.Provider value={{
          get roots() { return props.blocks.map((b) => b.id); },
          collapsed: (id, stored) => collapseSurface.collapsed(id, stored),
          navOnly: true,
        }}>
        {/* GH #415: Up from the first visual row of an embed's ROOT row exits
            the embed into the host page; the other rows stay surface-local.
            Only a block embed carries a host block id. */}
        <EmbedNavExitContext.Provider value={
          props.surface === "embed" && props.hostBlockId ? { hostBlockId: props.hostBlockId } : null
        }>
        <LinkDepthContext.Provider value={linkDepth + 1}>
        <For each={props.blocks.map((b) => b.id)}>
          {(id) => {
            const crumb = () => {
              const all = liveBreadcrumb(id) ?? dtoById(id)?.breadcrumb ?? [];
              const tail = all.slice(-3);
              return all.length > 3 ? ["…", ...tail] : tail;
            };
            return (
              <>
                <Show when={props.showBreadcrumb && crumb().length > 0}>
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
                  when={ready() && doc.byId[id]}
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
                          onOccurrence={(offset) => startEditing(
                            id,
                            rawOffsetToVisibleOffset(
                              doc.byId[id]?.raw ?? "",
                              offset,
                              isBuiltinHidden,
                              formatForPage(props.page),
                            ),
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
        </EmbedNavExitContext.Provider>
        </OutlineScopeContext.Provider>
        </SurfaceContext.Provider>
        </CollapseSurfaceContext.Provider>
      </Show>
    </div>
  );
}
