import { For, Show, createContext, createEffect, createSignal, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { openInNewTab } from "../router";
import { t } from "../i18n";
import { contextMenu, openPageContextMenu } from "../ui";
import { PagePropertyValue } from "../components/PagePropertyValue";
import { PropertyKey, visiblePageProperties } from "./PropertyKey";
import { FloatingPortal } from "../components/FloatingPortal";
import { RefBlocks } from "../components/RefBlocks";
import { LiveRefGroup } from "../components/LiveRefGroup";
import { editingId, endEdit } from "../editorController";
import type { BlockDto } from "../types";
import { dismissOnOutsidePointer, registerTransientLayer } from "../transientLayers";
import { isHTMLElementNode, isNodeValue, useOwnerWindow, viewportOf } from "../windowRealm";

export const PeekContext = createContext(false);

const POPUP_MARGIN = 8;
const POPUP_OFFSET = 6;
const POPUP_FALLBACK_WIDTH = 600;
const POPUP_FALLBACK_HEIGHT = 320;
/** OG 0.10.9 caps every hover preview at `:max-height 600` (components/block.cljs
 * page-preview-trigger and the block-ref tippy) and scrolls inside it. GH #735,
 * D-20261021-FA3 (deliberate OG difference): the cap is the larger of that and
 * half the window, so a tall (4K) window gets a bigger preview, and never more
 * than the window. */
export const PEEK_MAX_HEIGHT = 600;
export function peekMaxHeight(viewportHeight: number): number {
  return Math.max(0, Math.min(Math.max(PEEK_MAX_HEIGHT, viewportHeight / 2), viewportHeight - POPUP_MARGIN * 2));
}
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
  path?: string;
  blockId?: string;
  properties?: [string, string][];
  format?: "md" | "org";
  truncatedCount?: () => number;
  /** Extra title-row buttons, after Open source (GH #417: a reference's Edit link / Remove). */
  actions?: JSX.Element;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
  onDismiss: () => void;
  /** The popup's editor holds the caret: the opener must not close it on a timer. */
  onHold?: (held: boolean) => void;
}): JSX.Element {
  let popupEl: HTMLDivElement | undefined;
  const ownerWindow = useOwnerWindow();
  const layerId = `peek-popup-${createUniqueId()}`;
  const [style, setStyle] = createSignal<Record<string, string>>({});
  // D-20261023-VQ: the preview shows the live source blocks, editable in place.
  // While its editor holds the caret the popup stays put: pointer exit, page
  // scroll and viewport resize (a soft keyboard) do not close it, and the first
  // Escape ends the edit instead of closing the popup.
  // A secondary "ref:" surface, like references and embeds: unscoped edits and
  // arrow navigation do not mount a second editor here, and margin comments
  // render inline.
  const surfaceKey = `ref:peek-${layerId}`;
  const [editorFocused, setEditorFocused] = createSignal(false);
  const editingInside = () => editingId() !== null && editorFocused();
  createEffect(() => props.onHold?.(editingInside()));
  // Page menus live outside the preview DOM. Identify this preview's menu by
  // its exact opener, so unrelated menus retain normal outside dismissal.
  const ownsMenu = () => {
    const menu = contextMenu();
    return menu?.kind === "page" && !!menu.focusOwner && !!popupEl?.contains(menu.focusOwner);
  };
  let menuWasOpen = false;
  let pointerInside = false;
  let leftWhileEditing = false;
  createEffect(() => {
    if (editingInside() || !leftWhileEditing) return;
    leftWhileEditing = false;
    if (!pointerInside && !ownsMenu()) props.onPointerLeave(); // the ordinary grace period, now
  });
  createEffect(() => {
    const open = ownsMenu();
    if (open) props.onPointerEnter(); // cancel any pending hover-close timer
    else if (menuWasOpen && !pointerInside) props.onPointerLeave(); // resume the ordinary grace period
    menuWasOpen = open;
  });
  const openSource = (block?: string) => {
    if (!props.page) return;
    openInNewTab({ kind: "page", name: props.page, pageKind: props.pageKind ?? "page", ...(block ? { block } : {}) }, true);
    props.onDismiss();
  };

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
    const naturalHeight = Math.min(popupEl?.offsetHeight || POPUP_FALLBACK_HEIGHT, peekMaxHeight(viewportHeight));
    const maxLeft = Math.max(POPUP_MARGIN, viewportWidth - width - POPUP_MARGIN);
    const left = Math.min(Math.max(rect.left, POPUP_MARGIN), maxLeft);
    const below = Math.max(0, viewportHeight - POPUP_MARGIN - rect.bottom - POPUP_OFFSET);
    const above = Math.max(0, rect.top - POPUP_OFFSET - POPUP_MARGIN);
    // Prefer below, then above. If neither fits, use the larger side: never
    // clamp the popup back across its reference (GH #750).
    const openAbove = naturalHeight > below && (naturalHeight <= above || above > below);
    const room = openAbove ? above : below;
    // Pin the edge next to the reference and reserve the room on that side:
    // later growth (images, embeds, folds) extends away from the link and
    // scrolls inside the popup, never across it. Even a tiny viewport must
    // respect the available room.
    const maxHeight = `${Math.min(peekMaxHeight(viewportHeight), room)}px`;
    setStyle(openAbove
      ? { left: `${left}px`, bottom: `${viewportHeight - rect.top + POPUP_OFFSET}px`, "max-height": maxHeight }
      : { left: `${left}px`, top: `${rect.bottom + POPUP_OFFSET}px`, "max-height": maxHeight });
  };

  createEffect(() => {
    const unregister = registerTransientLayer({
      id: layerId,
      root: () => popupEl ?? null,
      dismiss: () => {
        if (editingInside()) {
          const active = popupEl?.ownerDocument.activeElement;
          if (isHTMLElementNode(active) && popupEl?.contains(active)) active.blur(); // commits
          endEdit("blur");
          popupEl?.focus();
          return true;
        }
        props.onDismiss();
        return true;
      },
    });
    onCleanup(unregister);
  });
  // While editing, an outside press only blurs (commits) the editor, and the
  // popup closes after the usual grace period. The editor's own portals
  // (autocomplete) live outside the popup and must stay clickable.
  dismissOnOutsidePointer({ open: () => !ownsMenu() && !editingInside(), inside: () => [popupEl, props.anchor()], dismiss: props.onDismiss });

  onMount(() => {
    const anchor = props.anchor();
    if (anchor) setStyle({ "max-height": `${peekMaxHeight(viewportOf(anchor).height)}px` });
    // Measure once after the initial children mount. Later folds, live refreshes
    // and future editing must not move the popup underneath the pointer.
    queueMicrotask(updatePosition);
    const close = () => { if (!editingInside()) props.onDismiss(); };
    // Dismiss when the PAGE BEHIND scrolls, but NOT when the user scrolls the
    // popup's own overflow body (or drags its scrollbar) — capture-phase scroll
    // fires for every scroller, so filter out events originating inside the popup.
    const onScroll = (e: Event) => {
      if (ownsMenu()) return;
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
        onMouseEnter={() => { pointerInside = true; props.onPointerEnter(); }}
        onMouseLeave={() => {
          pointerInside = false;
          if (editingInside()) leftWhileEditing = true;
          else if (!ownsMenu()) props.onPointerLeave();
        }}
        onFocusIn={(e) => { if (isHTMLElementNode(e.target) && e.target.tagName === "TEXTAREA") { setEditorFocused(true); props.onPointerEnter(); } }}
        onFocusOut={(e) => {
          const next = e.relatedTarget;
          if (!(isNodeValue(next) && popupEl?.contains(next))) setEditorFocused(false);
        }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <Show when={props.title}>
          <div class="peek-popup-title">
            <button type="button" class="peek-source-title" onClick={() => openSource()}>{props.title}</button>
            <Show when={props.page}>
              <button type="button" class="peek-open-source" aria-label={t("preview-open-source")} onClick={() => openSource(props.blockId)}>{t("preview-open-source")}</button>
              {props.actions}
              <button type="button" class="peek-page-actions" aria-label={t("preview-page-actions")} aria-haspopup="menu"
                onClick={(event) => {
                  const rect = event.currentTarget.getBoundingClientRect();
                  const page = props.page!;
                  const kind = props.pageKind ?? "page";
                  openPageContextMenu(rect.left, rect.bottom, page, kind, false, event.currentTarget);
                }}>…</button>
            </Show>
          </div>
        </Show>
        <PeekContext.Provider value={true}>
          <Show when={visiblePageProperties(props.properties ?? []).length}>
            <div class="peek-page-properties">
              <For each={visiblePageProperties(props.properties ?? [])}>{([key, value]) => <div><span class="prop-key"><PropertyKey name={key} /></span>{": "}<PagePropertyValue propertyKey={key} value={value} format={props.format ?? "md"} /></div>}</For>
            </div>
          </Show>
          <Show when={props.page} fallback={
            <RefBlocks blocks={props.blocks()} page={props.page} pageKind={props.pageKind} path={props.path} readOnly pageBody onOpenBlock={openSource} />
          }>
            {(page) => <LiveRefGroup page={page()} kind={props.pageKind ?? "page"} path={props.path} blocks={props.blocks()}
              surface="preview" surfaceKey={surfaceKey} eager onOpenBlock={openSource} />}
          </Show>
        </PeekContext.Provider>
        <Show when={(props.truncatedCount?.() ?? 0) > 0}>
          <div class="peek-popup-more">{props.truncatedCount!() > MAX_PEEK_COUNT ? t("render-more-blocks", { count: MAX_PEEK_COUNT }) : t("render-more-blocks-exact", { count: props.truncatedCount!() })}</div>
        </Show>
      </div>
    </FloatingPortal>
  );
}
