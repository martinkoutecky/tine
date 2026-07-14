import { For, Show, createEffect, createSignal, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { routeTitle, type PaneRouter, type Route } from "../router";
import { doc, formatForBlock } from "../store";
import { splitProps, isBuiltinHidden, type PropFormat } from "../editor/properties";
import { EmojiText } from "../render/emoji";
import { moveTabToPane, moveTabToRootEdge, moveTabToSeamSplit, moveTabToSplitPane } from "../panes";

const MAX_TITLE = 32;
const DRAG_THRESHOLD_PX = 4;
const EDGE_ZONE_PX = 24;

// A short, plain-text summary of a zoomed-into block, for the tab label. Drops
// the hidden id::/collapsed:: lines, takes the first non-empty line, and strips
// the common markdown decorations so the pill reads like the block's text.
function blockSummary(raw: string, format: PropFormat, truncate = true): string {
  const { visible } = splitProps(raw, isBuiltinHidden, format);
  const line =
    visible
      .split("\n")
      .map((s) => s.trim())
      .find((s) => s.length > 0) ?? "";
  const plain = line
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1") // image → alt text
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1") // link → label
    .replace(/\[\[([^\]]+)\]\]/g, "$1") // page ref → name
    .replace(/\(\(([^)]+)\)\)/g, "$1") // block ref → inner
    .replace(/==/g, "") // highlight markers
    .replace(/[*_~`]{1,3}/g, "") // bold / italic / strike / code
    .replace(/^#{1,6}\s+/, "") // markdown heading
    .replace(/\s+/g, " ")
    .trim();
  return truncate && plain.length > MAX_TITLE ? plain.slice(0, MAX_TITLE - 1).trimEnd() + "…" : plain;
}

// Tab label: a zoomed-into block shows its (shortened) content; everything else
// shows the page name (falling back to it when the block isn't loaded or empty).
function tabTitle(r: Route): string {
  if (r.kind === "page" && r.block) {
    const n = doc.byId[r.block];
    if (n) {
      const s = blockSummary(n.raw, formatForBlock(r.block));
      if (s) return s;
    }
  }
  return routeTitle(r);
}

function tabFullTitle(r: Route): string {
  if (r.kind === "page" && r.block) {
    const n = doc.byId[r.block];
    if (n) {
      const summary = blockSummary(n.raw, formatForBlock(r.block), false);
      if (summary) return summary;
    }
  }
  return routeTitle(r);
}

type SplitSide = "left" | "right" | "top" | "bottom";

export type TabDropTarget =
  | { kind: "tab"; paneId: string; tabId: string; index: number; before: boolean }
  | { kind: "pane"; paneId: string }
  | { kind: "split"; paneId: string; side: SplitSide }
  | { kind: "seam"; path: number[]; previewPaneId: string | null; side: SplitSide }
  | { kind: "root-edge"; side: SplitSide; previewPaneId: string | null };

const [activeTabDrag, setActiveTabDrag] = createSignal<{ paneId: string; tabId: string } | null>(null);
const [currentTabDropTarget, setCurrentTabDropTarget] = createSignal<TabDropTarget | null>(null);

export const tabDragState = activeTabDrag;
export const tabDropTarget = currentTabDropTarget;

function cssEsc(s: string): string {
  return typeof CSS !== "undefined" && CSS.escape ? CSS.escape(s) : s.replace(/["\\]/g, "\\$&");
}

function stripPaneId(strip: HTMLElement | null): string | null {
  return strip?.dataset.tabStripPaneId ?? strip?.closest("[data-pane-id]")?.getAttribute("data-pane-id") ?? null;
}

function tabIndexInStrip(tab: HTMLElement, strip: HTMLElement): number {
  return Array.from(strip.querySelectorAll<HTMLElement>(".tab[data-tab-id]")).indexOf(tab);
}

function firstPaneIdIn(el: Element | null): string | null {
  return (el?.querySelector("[data-pane-id]") as HTMLElement | null)?.dataset.paneId ?? null;
}

function branchContainsPane(el: Element | null, paneId: string): boolean {
  return !!el?.querySelector(`[data-pane-id="${cssEsc(paneId)}"]`);
}

function seamPreview(
  seam: HTMLElement,
  sourcePaneId: string | null,
  dir: "row" | "col",
  x: number,
  y: number
): { paneId: string | null; side: SplitSide } {
  const beforeBranch = seam.previousElementSibling;
  const afterBranch = seam.nextElementSibling;
  const sourceBefore = !!sourcePaneId && branchContainsPane(beforeBranch, sourcePaneId);
  const sourceAfter = !!sourcePaneId && branchContainsPane(afterBranch, sourcePaneId);
  const box = seam.getBoundingClientRect();
  const useBefore =
    sourceBefore || (!sourceAfter && (dir === "row" ? x <= box.left + box.width / 2 : y <= box.top + box.height / 2));
  return {
    paneId: useBefore ? (sourceBefore ? sourcePaneId : firstPaneIdIn(beforeBranch)) : (sourceAfter ? sourcePaneId : firstPaneIdIn(afterBranch)),
    side: dir === "row" ? (useBefore ? "right" : "left") : (useBefore ? "bottom" : "top"),
  };
}

function edgeSideAt(x: number, y: number, rect: DOMRect): SplitSide | null {
  const hits = [
    { side: "left" as const, dist: x - rect.left },
    { side: "right" as const, dist: rect.right - x },
    { side: "top" as const, dist: y - rect.top },
    { side: "bottom" as const, dist: rect.bottom - y },
  ].filter((h) => h.dist >= 0 && h.dist <= EDGE_ZONE_PX);
  hits.sort((a, b) => a.dist - b.dist);
  return hits[0]?.side ?? null;
}

function isRootEdge(pane: HTMLElement, side: SplitSide): boolean {
  const row = document.querySelector(".content-row") as HTMLElement | null;
  if (!row) return false;
  const paneRect = pane.getBoundingClientRect();
  const rowRect = row.getBoundingClientRect();
  const close = (a: number, b: number) => Math.abs(a - b) <= 2;
  switch (side) {
    case "left": return close(paneRect.left, rowRect.left);
    case "right": return close(paneRect.right, rowRect.right);
    case "top": return close(paneRect.top, rowRect.top);
    case "bottom": return close(paneRect.bottom, rowRect.bottom);
  }
}

export function tabSplitPreviewSideForPane(paneId: string): SplitSide | null {
  const target = currentTabDropTarget();
  if (target?.kind === "split" && target.paneId === paneId) return target.side;
  if (target?.kind === "seam" && target.previewPaneId === paneId) return target.side;
  if (target?.kind === "root-edge" && target.previewPaneId === paneId) return target.side;
  return null;
}

export function tabDropHighlightsPane(paneId: string): boolean {
  const target = currentTabDropTarget();
  return target?.kind === "pane" && target.paneId === paneId;
}

function tabDropBefore(paneId: string, tabId: string): boolean {
  const target = currentTabDropTarget();
  return target?.kind === "tab" && target.paneId === paneId && target.tabId === tabId && target.before;
}

function tabDropAfter(paneId: string, tabId: string): boolean {
  const target = currentTabDropTarget();
  return target?.kind === "tab" && target.paneId === paneId && target.tabId === tabId && !target.before;
}

export function tabDropTargetAt(
  el: Element | null,
  x: number,
  y: number,
  sourcePaneId: string | null = null
): TabDropTarget | null {
  if (!el) return null;
  const tab = el.closest(".tab[data-tab-id]") as HTMLElement | null;
  if (tab) {
    const strip = tab.closest(".tab-bar") as HTMLElement | null;
    const paneId = stripPaneId(strip);
    const tabId = tab.dataset.tabId;
    if (!strip || !paneId || !tabId) return null;
    const rect = tab.getBoundingClientRect();
    const before = x < rect.left + rect.width / 2;
    const tabIndex = tabIndexInStrip(tab, strip);
    return { kind: "tab", paneId, tabId, index: tabIndex + (before ? 0 : 1), before };
  }

  const seam = el.closest(".pane-resizer") as HTMLElement | null;
  if (seam) {
    const rawPath = seam.dataset.paneSeamPath ?? "";
    const path = rawPath ? rawPath.split(".").map((n) => Number(n)).filter((n) => Number.isFinite(n)) : [];
    const dir = seam.dataset.paneSeamDir === "col" ? "col" : "row";
    const preview = seamPreview(seam, sourcePaneId, dir, x, y);
    return { kind: "seam", path, previewPaneId: preview.paneId, side: preview.side };
  }

  if (el.closest(".tab-bar")) return null;
  const pane = el.closest("[data-pane-id]") as HTMLElement | null;
  const paneId = pane?.dataset.paneId;
  if (!pane || !paneId || paneId === "pdf") return null;
  const side = edgeSideAt(x, y, pane.getBoundingClientRect());
  if (side) {
    return isRootEdge(pane, side)
      ? { kind: "root-edge", side, previewPaneId: paneId }
      : { kind: "split", paneId, side };
  }
  return { kind: "pane", paneId };
}

function applyTabDrop(sourcePaneId: string, tabId: string, target: TabDropTarget): boolean {
  switch (target.kind) {
    case "tab":
      return moveTabToPane(sourcePaneId, tabId, target.paneId, target.index);
    case "pane":
      return moveTabToPane(sourcePaneId, tabId, target.paneId);
    case "split":
      return !!moveTabToSplitPane(sourcePaneId, tabId, target.paneId, target.side);
    case "seam":
      return !!moveTabToSeamSplit(sourcePaneId, tabId, target.path);
    case "root-edge":
      return !!moveTabToRootEdge(sourcePaneId, tabId, target.side);
  }
}

// Tab strip: click to activate, middle-click to close, double-click to pin,
// pointer-drag to reorder or move between panes. Pinned tabs are "sticky": they
// sort to the left and stay on their content — navigating from one (clicking a page/link, Ctrl-K) opens the
// destination in a new foreground tab instead of leaving the pinned view (zoom
// stays in place; middle-click still opens in the background). Closing a pinned
// tab asks first. The whole session persists across launches (see router
// persist/restoreSession). Always shown — even a single tab is rendered, so the
// pill signals that tabs exist (a feature OG Logseq lacks) without costing
// extra vertical space.
export function TabBar(props: { router: PaneRouter; dragRegion?: boolean; paneStrip?: boolean; focused?: boolean }): JSX.Element {
  let suppressClick = false;
  let root: HTMLDivElement | undefined;
  let strip: HTMLDivElement | undefined;
  let overviewTrigger: HTMLButtonElement | undefined;
  const router = props.router;
  const overviewId = `tab-overview-${createUniqueId()}`;
  const [overflowing, setOverflowing] = createSignal(false);
  const [overviewOpen, setOverviewOpen] = createSignal(false);
  const [overviewPosition, setOverviewPosition] = createSignal({ left: 8, top: 48, width: 360 });
  const [overviewDragId, setOverviewDragId] = createSignal<string | null>(null);
  const [overviewDrop, setOverviewDrop] = createSignal<{ tabId: string; before: boolean } | null>(null);

  const measureOverflow = () => {
    if (!strip) return;
    setOverflowing(strip.scrollWidth > strip.clientWidth + 1);
  };
  const revealActive = () => {
    if (!strip) return;
    measureOverflow();
    const active = strip.querySelector<HTMLElement>(`.tab[data-tab-id="${cssEsc(router.activeId())}"]`);
    if (!active) return;
    const left = active.offsetLeft;
    const right = left + active.offsetWidth;
    if (left < strip.scrollLeft) strip.scrollLeft = left;
    else if (right > strip.scrollLeft + strip.clientWidth) strip.scrollLeft = right - strip.clientWidth;
  };
  const dismissOverview = (restoreFocus = true) => {
    setOverviewOpen(false);
    if (restoreFocus) queueMicrotask(() => overviewTrigger?.focus());
  };
  const positionOverview = () => {
    const rect = overviewTrigger?.getBoundingClientRect();
    if (!rect) return;
    const width = Math.min(360, Math.max(240, window.innerWidth - 16));
    setOverviewPosition({
      left: Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8)),
      top: rect.bottom + 5,
      width,
    });
  };
  const focusOverviewRow = (index: number) => {
    const rows = [...(document.getElementById(overviewId)?.querySelectorAll<HTMLElement>("[data-tab-overview-row]") ?? [])];
    rows[Math.min(Math.max(index, 0), rows.length - 1)]?.focus();
  };
  const focusOverviewTab = (tabId: string) => {
    document.getElementById(overviewId)
      ?.querySelector<HTMLElement>(`[data-tab-overview-row][data-tab-id="${cssEsc(tabId)}"]`)
      ?.focus();
  };
  const moveOverviewTab = (tabId: string, direction: -1 | 1) => {
    const tabs = router.tabs();
    const from = tabs.findIndex((tab) => tab.id === tabId);
    if (from < 0) return;
    const target = Math.min(Math.max(0, from + direction), tabs.length - 1);
    if (target === from) return;
    // moveTabToIndex takes an insertion boundary in the pre-removal list.
    router.moveTabToIndex(tabId, direction > 0 ? target + 1 : target);
    queueMicrotask(() => focusOverviewTab(tabId));
  };
  const activateOverviewTab = (id: string) => {
    router.setActiveTab(id);
    dismissOverview();
  };
  const closeOverviewTab = async (id: string, index: number) => {
    await router.closeTab(id);
    queueMicrotask(() => {
      measureOverflow();
      if (overviewOpen()) focusOverviewRow(Math.min(index, router.tabs().length - 1));
    });
  };
  const onOverviewKeyDown: JSX.EventHandlerUnion<HTMLDivElement, KeyboardEvent> = (event) => {
    const rows = [...(document.getElementById(overviewId)?.querySelectorAll<HTMLElement>("[data-tab-overview-row]") ?? [])];
    if (!rows.length) return;
    const row = (event.target as HTMLElement).closest<HTMLElement>("[data-tab-overview-row]");
    const index = Math.max(0, rows.indexOf(row ?? rows[0]));
    if (event.altKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
      event.preventDefault();
      const id = rows[index].dataset.tabId;
      if (id) moveOverviewTab(id, event.key === "ArrowUp" ? -1 : 1);
      return;
    }
    let next = index;
    if (event.key === "ArrowDown") next = (index + 1) % rows.length;
    else if (event.key === "ArrowUp") next = (index - 1 + rows.length) % rows.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = rows.length - 1;
    else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      rows[index].click();
      return;
    } else if (event.key === "Delete") {
      event.preventDefault();
      const id = rows[index].dataset.tabId;
      if (id) void closeOverviewTab(id, index);
      return;
    } else if (event.key === "Escape") {
      event.preventDefault();
      dismissOverview();
      return;
    } else return;
    event.preventDefault();
    focusOverviewRow(next);
  };

  const beginOverviewDrag = (tabId: string, event: PointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const startX = event.clientX;
    const startY = event.clientY;
    let dragging = false;

    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      setOverviewDragId(null);
      setOverviewDrop(null);
    };
    const onMove = (move: PointerEvent) => {
      if (!dragging && Math.hypot(move.clientX - startX, move.clientY - startY) < DRAG_THRESHOLD_PX) return;
      if (!dragging) {
        dragging = true;
        setOverviewDragId(tabId);
      }
      move.preventDefault();
      const row = document.elementFromPoint(move.clientX, move.clientY)
        ?.closest<HTMLElement>("[data-tab-overview-row]");
      const targetId = row?.dataset.tabId;
      if (!row || !targetId || targetId === tabId) {
        setOverviewDrop(null);
        return;
      }
      const rect = row.getBoundingClientRect();
      setOverviewDrop({ tabId: targetId, before: move.clientY < rect.top + rect.height / 2 });
    };
    const onCancel = () => cleanup();
    const onUp = () => {
      const drop = overviewDrop();
      if (dragging && drop) {
        const index = router.tabs().findIndex((tab) => tab.id === drop.tabId);
        if (index >= 0) router.moveTabToIndex(tabId, index + (drop.before ? 0 : 1));
      }
      cleanup();
      if (dragging) queueMicrotask(() => focusOverviewTab(tabId));
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
  };

  createEffect(() => {
    router.activeId();
    router.tabs().map((tab) => `${tab.id}:${tab.pos}:${tab.pinned}`).join("|");
    queueMicrotask(revealActive);
  });
  createEffect(() => {
    if (!overflowing() && overviewOpen()) setOverviewOpen(false);
  });
  createEffect(() => {
    if (!overviewOpen()) return;
    queueMicrotask(() => {
      positionOverview();
      const activeIndex = Math.max(0, router.tabs().findIndex((tab) => tab.id === router.activeId()));
      focusOverviewRow(activeIndex);
    });
    const outside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!root?.contains(target) && !document.getElementById(overviewId)?.contains(target)) dismissOverview();
    };
    document.addEventListener("pointerdown", outside, true);
    window.addEventListener("resize", positionOverview);
    window.addEventListener("scroll", positionOverview, true);
    onCleanup(() => {
      document.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("resize", positionOverview);
      window.removeEventListener("scroll", positionOverview, true);
    });
  });
  onMount(() => {
    const observer = typeof ResizeObserver !== "undefined" ? new ResizeObserver(revealActive) : null;
    if (strip) observer?.observe(strip);
    window.addEventListener("resize", revealActive);
    queueMicrotask(revealActive);
    onCleanup(() => {
      observer?.disconnect();
      window.removeEventListener("resize", revealActive);
    });
  });

  function beginDrag(tabId: string, e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    const sourcePaneId = router.paneId;
    const startX = e.clientX;
    const startY = e.clientY;
    let dragging = false;
    let cancelled = false;

    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("keydown", onKeyDown, true);
      setActiveTabDrag(null);
      setCurrentTabDropTarget(null);
    };
    const markMoved = () => {
      suppressClick = true;
      setTimeout(() => (suppressClick = false), 0);
    };
    const onCancel = () => {
      cancelled = true;
      if (dragging) markMoved();
      cleanup();
    };
    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      ev.preventDefault();
      onCancel();
    };
    const onMove = (ev: PointerEvent) => {
      if (!dragging && Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_THRESHOLD_PX) return;
      if (!dragging) {
        dragging = true;
        setActiveTabDrag({ paneId: sourcePaneId, tabId });
      }
      ev.preventDefault();
      const el = document.elementFromPoint(ev.clientX, ev.clientY);
      setCurrentTabDropTarget(tabDropTargetAt(el, ev.clientX, ev.clientY, sourcePaneId));
    };
    const onUp = () => {
      const target = currentTabDropTarget();
      const shouldDrop = dragging && !cancelled && !!target;
      if (dragging) markMoved();
      cleanup();
      if (shouldDrop) applyTabDrop(sourcePaneId, tabId, target!);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("keydown", onKeyDown, true);
  }

  return (
    // The tab strip fills the toolbar's middle, so its empty space is the main
    // window-drag handle (the tabs, being children, still click/drag-to-reorder).
    <div
      ref={root}
      class="tab-bar"
      classList={{ "pane-tab-bar": !!props.paneStrip, "pane-tab-focused": !!props.focused }}
      data-tauri-drag-region={props.dragRegion === false ? undefined : ""}
      data-tab-strip-pane-id={router.paneId}
    >
      <div
        ref={strip}
        class="tab-strip-scroll"
        data-tauri-drag-region={props.dragRegion === false ? undefined : ""}
      >
      <For each={router.tabs()}>
        {(t) => (
          <div
            class="tab"
            classList={{
              active: t.id === router.activeId(),
              pinned: t.pinned,
              "tab-dragging": activeTabDrag()?.paneId === router.paneId && activeTabDrag()?.tabId === t.id,
              "tab-drop-before": tabDropBefore(router.paneId, t.id),
              "tab-drop-after": tabDropAfter(router.paneId, t.id),
            }}
            data-tab-id={t.id}
            onMouseDown={(e) => {
              // Stop the double-click (pin) gesture from word-selecting the label
              // — user-select:none alone still flashes a selection in WebKitGTK.
              if (e.detail >= 2) e.preventDefault();
            }}
            onPointerDown={(e) => beginDrag(t.id, e)}
            onClick={() => {
              if (suppressClick) return;
              router.setActiveTab(t.id);
            }}
            onDblClick={() => router.togglePin(t.id)}
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                router.closeTab(t.id);
              }
            }}
            title={
              t.pinned
                ? "Pinned (sticky) — links open in a new tab. Double-click to unpin"
                : "Double-click to pin (sticky)"
            }
          >
            <Show when={t.pinned}>
              {/* The red 📌 — now a Twemoji SVG <img> (render/emoji.tsx), so it
                  shows everywhere, including WebKitGTK where the emoji *font*
                  painted it blank. */}
              <span class="tab-pin">
                <EmojiText text="📌" />
              </span>
            </Show>
            <span class="tab-title"><EmojiText text={tabTitle(router.tabRoute(t))} /></span>
            {/* The last tab can't be closed (closeTab keeps one), so hide its ✕. */}
            <Show when={router.tabs().length > 1}>
              <span
                class="tab-close"
                onClick={(e) => {
                  e.stopPropagation();
                  router.closeTab(t.id);
                }}
              >
                ✕
              </span>
            </Show>
          </div>
        )}
      </For>
      </div>
      <Show when={overflowing()}>
        <button
          ref={overviewTrigger}
          class="tab-overview-trigger"
          type="button"
          title="Show all tabs"
          aria-label="Show all tabs"
          aria-haspopup="listbox"
          aria-expanded={overviewOpen()}
          aria-controls={overviewId}
          data-tab-overview-trigger
          onClick={() => setOverviewOpen((open) => !open)}
        >⌄</button>
        <Show when={overviewOpen()}>
          <Portal>
          <div
            id={overviewId}
            class="tab-overview"
            role="listbox"
            aria-label="Open tabs"
            style={{
              left: `${overviewPosition().left}px`,
              top: `${overviewPosition().top}px`,
              width: `${overviewPosition().width}px`,
            }}
            onKeyDown={onOverviewKeyDown}
          >
            <For each={router.tabs()}>
              {(tab, index) => (
                <div
                  class="tab-overview-row"
                  classList={{
                    active: tab.id === router.activeId(),
                    "tab-overview-dragging": overviewDragId() === tab.id,
                    "tab-overview-drop-before": overviewDrop()?.tabId === tab.id && overviewDrop()?.before,
                    "tab-overview-drop-after": overviewDrop()?.tabId === tab.id && !overviewDrop()?.before,
                  }}
                  role="option"
                  aria-selected={tab.id === router.activeId()}
                  aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
                  tabIndex={-1}
                  data-tab-overview-row
                  data-tab-id={tab.id}
                  onClick={() => activateOverviewTab(tab.id)}
                  onAuxClick={(event) => {
                    if (event.button !== 1) return;
                    event.preventDefault();
                    void closeOverviewTab(tab.id, index());
                  }}
                >
                  <button
                    class="tab-overview-drag-handle"
                    type="button"
                    tabIndex={-1}
                    aria-label={`Reorder ${tabFullTitle(router.tabRoute(tab))}`}
                    title="Drag to reorder; Alt+Up/Down moves the focused tab"
                    onPointerDown={(event) => beginOverviewDrag(tab.id, event)}
                    onClick={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                    }}
                  >⠿</button>
                  <span class="tab-overview-active" aria-hidden="true">{tab.id === router.activeId() ? "✓" : ""}</span>
                  <Show when={tab.pinned}><span class="tab-overview-pin" title="Pinned"><EmojiText text="📌" /></span></Show>
                  <span class="tab-overview-title"><EmojiText text={tabFullTitle(router.tabRoute(tab))} /></span>
                  <Show when={router.tabs().length > 1}>
                    <button
                      class="tab-overview-close"
                      type="button"
                      aria-label={`Close ${tabFullTitle(router.tabRoute(tab))}`}
                      onClick={(event) => {
                        event.stopPropagation();
                        void closeOverviewTab(tab.id, index());
                      }}
                    >✕</button>
                  </Show>
                </div>
              )}
            </For>
          </div>
          </Portal>
        </Show>
      </Show>
    </div>
  );
}
