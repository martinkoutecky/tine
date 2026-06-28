import { For, Show, createResource, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { doc, ensurePageLoaded, pageByName } from "../store";
import { Block } from "./Block";
import { RefBlocks } from "./RefBlocks";
import type { BlockDto, PageKind } from "../types";

// One shared IntersectionObserver for ALL live-ref groups — a broad query/backlink
// set would otherwise spin up O(groups) observers. Each group registers a one-shot
// "near the viewport" callback; the observer unobserves it once fired.
const nearCbs = new WeakMap<Element, () => void>();
let sharedNearIO: IntersectionObserver | null = null;
function observeNear(el: Element, cb: () => void) {
  if (!sharedNearIO) {
    sharedNearIO = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          const fn = nearCbs.get(e.target);
          if (fn) {
            nearCbs.delete(e.target);
            sharedNearIO!.unobserve(e.target);
            fn();
          }
        }
      },
      { rootMargin: "1200px 0px" }
    );
  }
  nearCbs.set(el, cb);
  sharedNearIO.observe(el);
}
function unobserveNear(el: Element) {
  if (sharedNearIO && nearCbs.has(el)) {
    nearCbs.delete(el);
    sharedNearIO.unobserve(el);
  }
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

  const dtoById = (id: string) => props.blocks.find((b) => b.id === id);
  return (
    <div
      ref={el}
      class="live-ref-group"
      // Reserve approximate height while unmounted so the scrollbar stays sane.
      style={!near() ? { "min-height": `${Math.max(1, props.blocks.length) * 1.9}em` } : undefined}
    >
      <Show when={near()}>
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
      </Show>
    </div>
  );
}
