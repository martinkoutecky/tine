import { For, Show, createMemo, createResource, createSignal, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { doc, ensurePageLoaded, pageByName } from "../store";
import { Block, SurfaceContext } from "./Block";
import { RefBlocks } from "./RefBlocks";
import { observeNear, unobserveNear } from "../lazyObserve";
import type { BlockDto, PageKind } from "../types";

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
export function LiveRefGroup(props: { page: string; kind: PageKind; blocks: BlockDto[]; embedId?: string; showBreadcrumb?: boolean }): JSX.Element {
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
      if (!pageByName(p)) {
        const dto = await backend().getPage(p, k);
        if (dto) ensurePageLoaded(dto);
      }
      return true;
    }
  );

  // O(1) id → dto. The prior `props.blocks.find` inside the per-row <For> was
  // O(N) per row → O(N²) per group (250k iterations on a 500-block hub group).
  const byId = createMemo(() => new Map(props.blocks.map((b) => [b.id, b] as const)));
  const dtoById = (id: string) => byId().get(id);
  // A ref/query/embed group can render a block that ALSO lives in the main outline
  // of the same page (e.g. the journal agenda re-lists today's scheduled/deadline
  // bullets). Give this group its own edit "surface" so an UNSCOPED keyboard nav
  // (Up/Down) into such a block focuses the MAIN-outline instance, not this copy —
  // otherwise both instances (same "main" surface) call focus() and the off-screen
  // copy wins, stealing the caret and scrolling the viewport to it. Same mechanism
  // as the right sidebar (see startEditing / focusSurfaceFor). One key per group.
  const surface = "ref:" + createUniqueId();
  return (
    <div
      ref={el}
      class="live-ref-group"
      // Reserve approximate height while unmounted so the scrollbar stays sane.
      style={!near() ? { "min-height": `${Math.max(1, props.blocks.length) * 1.9}em` } : undefined}
    >
      <Show when={near()}>
        <SurfaceContext.Provider value={surface}>
        <For each={props.blocks.map((b) => b.id)}>
          {(id) => {
            const crumb = () => dtoById(id)?.breadcrumb ?? [];
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
                  <Block id={id} hideRefCount={!!props.embedId && id === props.embedId} />
                </Show>
              </>
            );
          }}
        </For>
        </SurfaceContext.Provider>
      </Show>
    </div>
  );
}
