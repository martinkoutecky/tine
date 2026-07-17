import { For, Show, createEffect, createMemo, createResource, createSignal, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { doc, ensurePageLoaded, formatForPage, pageByName } from "../store";
import { Block, CollapseSurfaceContext, SurfaceContext, type CollapseSurfaceApi } from "./Block";
import { RefBlocks } from "./RefBlocks";
import { observeNear, unobserveNear } from "../lazyObserve";
import type { BlockDto, PageKind, ReferenceBlockEvidence } from "../types";
import { graphEpoch, graphMeta } from "../ui";
import { OccurrenceControls } from "./ReferenceEvidence";
import { startEditing } from "../editorController";
import { isBuiltinHidden, rawOffsetToVisibleOffset } from "../editor/properties";
import { visibleBody } from "../render/block";

// The "near the viewport" lazy-mount observer is shared app-wide (block bodies
// use it too) — see src/lazyObserve.ts.

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
  blocks: BlockDto[];
  embedId?: string;
  showBreadcrumb?: boolean;
  surface: "ref" | "query" | "embed";
  evidence?: ReferenceBlockEvidence[];
}): JSX.Element {
  const [near, setNear] = createSignal(false);
  let el: HTMLDivElement | undefined;
  onMount(() => {
    if (!el) return;
    const node = el;
    observeNear(node, () => setNear(true));
    onCleanup(() => unobserveNear(node));
  });

  // Load the source page only once the group is near the viewport.
  const [ready] = createResource(
    () => (near() ? { p: props.page, k: props.kind } : null),
    async ({ p, k }) => {
      const occupied = pageByName(p);
      if (occupied) return occupied.kind === k;
      const epoch = graphEpoch();
      const root = graphMeta()?.root ?? "";
      const dto = await backend().getPage(p, k);
      // The component may have unmounted while this read was in flight. Never
      // let an old graph's DTO enter the new graph's shared working set.
      if (graphEpoch() !== epoch || (graphMeta()?.root ?? "") !== root) return false;
      // Page names are not unique across kinds, while the frontend working set
      // is name-keyed. Refuse a page/journal twin that occupied the slot during
      // the await, and reject a mismatched backend response defensively.
      const after = pageByName(p);
      if (after) return after.kind === k;
      if (!dto || dto.name !== p || dto.kind !== k) return false;
      ensurePageLoaded(dto);
      return pageByName(p)?.kind === k;
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
  const [localCollapsed, setLocalCollapsed] = createSignal<Record<string, boolean>>({});
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
    const previous = initialCollapsed.get(id);
    if (previous !== undefined) return previous;
    const depth = relativeDepth(id);
    const hasChildren = (doc.byId[id]?.children.length ?? 0) > 0;
    // Released OG initializes reference/query disclosure from the source state
    // and default-open level 2, then keeps that copy local to the result view.
    // Tine's displayed hit is relative depth 0, so branches immediately below it
    // default folded. Embeds deliberately retain source disclosure semantics.
    const initial = stored || (props.surface !== "embed" && depth !== null && depth >= 1 && hasChildren);
    initialCollapsed.set(id, initial);
    return initial;
  };
  const collapseSurface: CollapseSurfaceApi = {
    collapsed: (id, stored) => {
      const local = localCollapsed();
      return Object.prototype.hasOwnProperty.call(local, id) ? local[id] : defaultCollapsed(id, stored);
    },
    toggle: (id, current) => setLocalCollapsed((state) => ({ ...state, [id]: !current })),
    setMany: (ids, collapsed) => setLocalCollapsed((state) => {
      const next = { ...state };
      for (const id of ids) next[id] = collapsed;
      return next;
    }),
  };
  // Result DTOs are replaced during filter/query refresh. Retain local choices
  // for stable roots and their live descendants, but discard state once a root
  // leaves this group so an old choice cannot leak into a later membership.
  createEffect(() => {
    if (!ready()) return;
    const present = new Set<string>();
    const visit = (id: string) => {
      if (present.has(id)) return;
      present.add(id);
      for (const child of doc.byId[id]?.children ?? []) visit(child);
    };
    for (const root of resultRootIds()) visit(root);
    for (const id of initialCollapsed.keys()) {
      if (!present.has(id)) initialCollapsed.delete(id);
    }
    setLocalCollapsed((state) => {
      let changed = false;
      const next: Record<string, boolean> = {};
      for (const [id, value] of Object.entries(state)) {
        if (present.has(id)) next[id] = value;
        else changed = true;
      }
      return changed ? next : state;
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
                  <Block id={id} hideRefCount={!!props.embedId && id === props.embedId} />
                </Show>
              </>
            );
          }}
        </For>
        </SurfaceContext.Provider>
        </CollapseSurfaceContext.Provider>
      </Show>
    </div>
  );
}
