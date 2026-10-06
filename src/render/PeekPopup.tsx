import { Show, createContext, createEffect, createSignal, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { FloatingPortal } from "../components/FloatingPortal";
import { RefBlocks } from "../components/RefBlocks";
import type { BlockDto } from "../types";
import { registerTransientLayer } from "../transientLayers";
import { useOwnerWindow, viewportOf } from "../windowRealm";

export const PeekContext = createContext(false);

const POPUP_MARGIN = 8;
const POPUP_OFFSET = 6;
const POPUP_FALLBACK_WIDTH = 600;
const POPUP_FALLBACK_HEIGHT = 320;
const MAX_PEEK_BLOCK_DEPTH = 64;
const MAX_PEEK_BLOCKS = 100;
const MAX_PEEK_COUNT = 2_000;

function countBlocks(blocks: readonly BlockDto[]): number {
  let count = 0;
  const stack = [{ source: blocks, index: 0 }];
  while (stack.length) {
    const frame = stack[stack.length - 1];
    if (frame.index >= frame.source.length) { stack.pop(); continue; }
    const block = frame.source[frame.index++];
    if (++count > MAX_PEEK_COUNT) return MAX_PEEK_COUNT + 1;
    if (block.children.length) stack.push({ source: block.children, index: 0 });
  }
  return count;
}

/** Return at most 100 blocks across 64 levels. `truncated` is exact through 2000;
 * 2001 means at least 2001 more, so callers display "2000+". Visits at most
 * 2101 blocks and never recurses; emitted nodes are cloned, not mutated. */
export function capBlockTree(blocks: BlockDto[], maxBlocks: number): { blocks: BlockDto[]; truncated: number } {
  if (maxBlocks <= 0) return { blocks: [], truncated: countBlocks(blocks) };
  const limit = Math.min(maxBlocks, MAX_PEEK_BLOCKS);

  let emitted = 0;
  let truncated = 0;
  const out: BlockDto[] = [];
  const stack: Array<{ source: readonly BlockDto[]; index: number; target: BlockDto[]; depth: number }> =
    [{ source: blocks, index: 0, target: out, depth: 1 }];
  while (stack.length) {
    const frame = stack[stack.length - 1];
    if (frame.index >= frame.source.length) {
      stack.pop();
      continue;
    }
    const block = frame.source[frame.index++];
    if (emitted >= limit || frame.depth > MAX_PEEK_BLOCK_DEPTH) {
      truncated += countBlocks([block]);
      if (truncated > MAX_PEEK_COUNT) return { blocks: out, truncated: MAX_PEEK_COUNT + 1 };
      continue;
    }
    emitted++;
    const children: BlockDto[] = [];
    frame.target.push({ ...block, children });
    if (block.children.length) stack.push({ source: block.children, index: 0, target: children, depth: frame.depth + 1 });
  }
  return { blocks: out, truncated };
}

/** Render supplied preview blocks in an anchored portal. Caller caps the tree
 * and provides omitted count. Registers a transient dismissal layer and scroll
 * and resize listeners, removed on unmount. O(supplied rendered blocks). */
export function PeekPopup(props: {
  anchor: () => HTMLElement | undefined;
  title?: JSX.Element;
  blocks: () => BlockDto[];
  page?: string;
  pageKind?: "journal" | "page";
  truncatedCount?: () => number;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
  onDismiss: () => void;
}): JSX.Element {
  let popupEl: HTMLDivElement | undefined;
  const ownerWindow = useOwnerWindow();
  const layerId = `peek-popup-${createUniqueId()}`;
  const [style, setStyle] = createSignal<Record<string, string>>({});

  const updatePosition = () => {
    const anchor = props.anchor();
    if (!anchor) return;

    const rect = anchor.getBoundingClientRect();
    const viewport = viewportOf(anchor);
    const root = anchor.ownerDocument.documentElement;
    const viewportWidth = viewport.width || root.clientWidth || POPUP_FALLBACK_WIDTH;
    const viewportHeight = viewport.height || root.clientHeight || POPUP_FALLBACK_HEIGHT;
    const width = Math.min(
      popupEl?.offsetWidth || POPUP_FALLBACK_WIDTH,
      Math.max(0, viewportWidth - POPUP_MARGIN * 2),
    );
    const height = popupEl?.offsetHeight || POPUP_FALLBACK_HEIGHT;
    const maxLeft = Math.max(POPUP_MARGIN, viewportWidth - width - POPUP_MARGIN);
    const left = Math.min(Math.max(rect.left, POPUP_MARGIN), maxLeft);
    const openAbove = rect.bottom + POPUP_OFFSET + height > viewportHeight - POPUP_MARGIN && rect.top > height;

    setStyle(openAbove
      ? { left: `${left}px`, bottom: `${viewportHeight - rect.top + POPUP_OFFSET}px` }
      : { left: `${left}px`, top: `${rect.bottom + POPUP_OFFSET}px` });
  };

  createEffect(() => {
    props.anchor();
    props.blocks();
    updatePosition();
    queueMicrotask(updatePosition);
  });

  createEffect(() => {
    const unregister = registerTransientLayer({
      id: layerId,
      root: () => popupEl ?? null,
      dismiss: () => { props.onDismiss(); return true; },
    });
    onCleanup(unregister);
  });

  onMount(() => {
    const close = () => props.onPointerLeave();
    // Dismiss when the PAGE BEHIND scrolls, but NOT when the user scrolls the
    // popup's own overflow body (or drags its scrollbar) — capture-phase scroll
    // fires for every scroller, so filter out events originating inside the popup.
    const onScroll = (e: Event) => {
      const t = e.target as Node | null;
      if (popupEl && t && (popupEl === t || popupEl.contains(t))) return;
      close();
    };
    ownerWindow.addEventListener("scroll", onScroll, true);
    ownerWindow.addEventListener("resize", close);
    onCleanup(() => {
      ownerWindow.removeEventListener("scroll", onScroll, true);
      ownerWindow.removeEventListener("resize", close);
    });
  });

  return (
    <FloatingPortal>
      <div
        class="peek-popup"
        ref={popupEl}
        role="dialog"
        tabIndex={-1}
        data-lenis-prevent
        style={style()}
        onMouseEnter={props.onPointerEnter}
        onMouseLeave={props.onPointerLeave}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <Show when={props.title}>
          <div class="peek-popup-title">{props.title}</div>
        </Show>
        <PeekContext.Provider value={true}>
          <RefBlocks blocks={props.blocks()} page={props.page} pageKind={props.pageKind} />
        </PeekContext.Provider>
        <Show when={(props.truncatedCount?.() ?? 0) > 0}>
          <div class="peek-popup-more">{props.truncatedCount!() > MAX_PEEK_COUNT
            ? `${MAX_PEEK_COUNT}+ more blocks` : `${props.truncatedCount!()} more blocks`}</div>
        </Show>
      </div>
    </FloatingPortal>
  );
}
