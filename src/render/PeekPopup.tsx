import { Show, createContext, createEffect, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { RefBlocks } from "../components/RefBlocks";
import type { BlockDto } from "../types";

export const PeekContext = createContext(false);

const POPUP_MARGIN = 8;
const POPUP_OFFSET = 6;
const POPUP_FALLBACK_WIDTH = 600;
const POPUP_FALLBACK_HEIGHT = 320;

function countBlocks(blocks: BlockDto[]): number {
  let count = 0;
  for (const block of blocks) count += 1 + countBlocks(block.children);
  return count;
}

export function capBlockTree(blocks: BlockDto[], maxBlocks: number): { blocks: BlockDto[]; truncated: number } {
  if (maxBlocks <= 0) return { blocks: [], truncated: countBlocks(blocks) };

  let emitted = 0;
  let truncated = 0;
  const copy = (source: BlockDto[]): BlockDto[] => {
    const out: BlockDto[] = [];
    for (const block of source) {
      if (emitted >= maxBlocks) {
        truncated += countBlocks([block]);
        continue;
      }
      emitted++;
      out.push({ ...block, children: copy(block.children) });
    }
    return out;
  };

  return { blocks: copy(blocks), truncated };
}

export function PeekPopup(props: {
  anchor: () => HTMLElement | undefined;
  title?: JSX.Element;
  blocks: () => BlockDto[];
  page?: string;
  pageKind?: "journal" | "page";
  truncatedCount?: () => number;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
}): JSX.Element {
  let popupEl: HTMLDivElement | undefined;
  const [style, setStyle] = createSignal<Record<string, string>>({});

  const updatePosition = () => {
    const anchor = props.anchor();
    if (!anchor) return;

    const rect = anchor.getBoundingClientRect();
    const viewportWidth = window.innerWidth || document.documentElement.clientWidth || POPUP_FALLBACK_WIDTH;
    const viewportHeight = window.innerHeight || document.documentElement.clientHeight || POPUP_FALLBACK_HEIGHT;
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

  onMount(() => {
    const close = () => props.onPointerLeave();
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKeyDown);
    onCleanup(() => {
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKeyDown);
    });
  });

  return (
    <Portal>
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
        onKeyDown={(e) => {
          if (e.key === "Escape") props.onPointerLeave();
        }}
      >
        <Show when={props.title}>
          <div class="peek-popup-title">{props.title}</div>
        </Show>
        <PeekContext.Provider value={true}>
          <RefBlocks blocks={props.blocks()} page={props.page} pageKind={props.pageKind} />
        </PeekContext.Provider>
        <Show when={(props.truncatedCount?.() ?? 0) > 0}>
          <div class="peek-popup-more">{props.truncatedCount!()} more blocks</div>
        </Show>
      </div>
    </Portal>
  );
}
