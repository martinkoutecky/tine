import { t as translate } from "../i18n";
import { For, Show, createEffect, createSignal, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { FloatingPortal } from "./FloatingPortal";
import { hoverIntent } from "./hoverIntent";
import { resolveRouteBlock, routeTitle, type PaneRouter, type Route, type Tab } from "../router";
import { formatForBlock, node as docNode } from "../document";
import { splitProps, BUILTIN_HIDE, type PropFormat } from "../editor/properties";
import { EmojiText } from "../render/emoji";
import { parseBlock, parserReady } from "../render/parse";
import { inlineText } from "../render/facets";
import { moveTabToPane, moveTabToSeamSplit, moveTabToSplitPane, layoutHasMultiplePanes, layoutRoot, windowOfPane } from "../panes";
import { dismissOnOutsidePointer, registerTransientLayer } from "../transientLayers";
import { MAIN_WINDOW_ID, isHTMLElementNode, newResizeObserver, useOwnerWindow } from "../windowRealm";

/** One predicate for the strip's ✕ and the overview's close button: closing is
 *  effective when another tab remains, or when this lone non-feed tab can close
 *  its whole split pane (the router's closeTab hands it to the pane close). */
function closeOffered(router: PaneRouter, t: Tab): boolean {
  // A workspace window's last pane closes the window itself (src/panes.ts closePane).
  const windowId = windowOfPane(router.paneId);
  return router.tabs().length > 1 || (router.tabRoute(t).kind !== "journals"
    && (layoutHasMultiplePanes(layoutRoot(windowId)) || windowId !== MAIN_WINDOW_ID));
}

const MAX_TITLE = 32;
const DRAG_THRESHOLD_PX = 4;

// A short, plain-text summary of a zoomed-into block, for the tab label. Drops
// the hidden id::/collapsed:: lines, takes the first non-empty line, and reads it
// as the PARSER does (I-12): links show their label, page refs their name, emphasis
// and code spans their text, and a heading its title - no regex over the markup.
export function blockSummary(raw: string, format: PropFormat, truncate = true): string {
  const { visible } = splitProps(raw, BUILTIN_HIDE, format);
  const line =
    visible
      .split("\n")
      .map((s) => s.trim())
      .find((s) => s.length > 0) ?? "";
  const first = line && parserReady() ? parseBlock(line, format === "org")[0] : undefined;
  const text = first && "inline" in first && Array.isArray(first.inline) ? inlineText(first.inline) : line;
  const marker = first && "marker" in first && first.marker ? `${first.marker} ` : "";
  const plain = (marker + text).replace(/\s+/g, " ").trim();
  return truncate && plain.length > MAX_TITLE ? plain.slice(0, MAX_TITLE - 1).trimEnd() + "…" : plain;
}

// The loaded block a zoomed tab shows. A restored (position-saved) zoom has no
// trustworthy key until its page loads, so it resolves by position.
function tabBlockId(r: Route): string | undefined {
  if (r.kind !== "page" || !r.block) return undefined;
  return r.blockPos ? resolveRouteBlock(r) ?? undefined : r.block;
}

// Tab label: a zoomed-into block shows its (shortened) content; everything else
// shows the page name. A zoomed tab whose block is not loaded (a background tab
// after a session restore, or an evicted page) must still differ from the plain tab
// of the same page, so it reads "Page › …" until the block is known (GH #528).
function zoomedLabel(r: Route, truncate: boolean): string {
  if (r.kind === "page" && r.block) {
    const id = tabBlockId(r);
    const n = id ? docNode(id) : undefined;
    if (n) {
      const s = blockSummary(n.raw, formatForBlock(id!), truncate);
      if (s) return s;
    }
    return `${routeTitle(r)} › …`;
  }
  return routeTitle(r);
}

export function tabTitle(r: Route): string {
  return zoomedLabel(r, true);
}

export function tabFullTitle(r: Route): string {
  return zoomedLabel(r, false);
}

type SplitSide = "left" | "right" | "top" | "bottom";

export type TabDropTarget =
  | { kind: "tab"; paneId: string; tabId: string; index: number; before: boolean }
  | { kind: "pane"; paneId: string }
  | { kind: "split"; paneId: string; side: SplitSide }
  | { kind: "seam"; path: number[]; previewPaneId: string | null; side: SplitSide };

const [activeTabDrag, setActiveTabDrag] = createSignal<{ paneId: string; tabId: string } | null>(null);
const [currentTabDropTarget, setCurrentTabDropTarget] = createSignal<TabDropTarget | null>(null);
type ActiveStripDragSession = { cancel: () => void };
let activeStripDragSession: ActiveStripDragSession | null = null;

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

function edgeSideAt(x: number, y: number, rect: DOMRect, contentTop = rect.top): SplitSide | null {
  const zone = (extent: number) => Math.max(24, Math.min(extent * 0.2, 96));
  const horizontalZone = zone(rect.width);
  const verticalZone = zone(rect.height);
  const hits = [
    { side: "left" as const, dist: x - rect.left, zone: horizontalZone },
    { side: "right" as const, dist: rect.right - x, zone: horizontalZone },
    { side: "top" as const, dist: y - contentTop, zone: verticalZone },
    { side: "bottom" as const, dist: rect.bottom - y, zone: verticalZone },
  ].filter((h) => h.dist >= 0 && h.dist <= h.zone);
  hits.sort((a, b) => a.dist - b.dist);
  return hits[0]?.side ?? null;
}

export function tabSplitPreviewSideForPane(paneId: string): SplitSide | null {
  const target = currentTabDropTarget();
  if (target?.kind === "split" && target.paneId === paneId) return target.side;
  if (target?.kind === "seam" && target.previewPaneId === paneId) return target.side;
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

  // GH #527: anywhere else in a tab bar (the empty space after the last tab, the
  // overflow button) drops the tab LAST in that bar's pane. The visual cue is the
  // "after" mark on the last tab.
  const bar = el.closest(".tab-bar") as HTMLElement | null;
  if (bar) {
    const paneId = stripPaneId(bar);
    const tabs = Array.from(bar.querySelectorAll<HTMLElement>(".tab[data-tab-id]"));
    const last = tabs[tabs.length - 1]?.dataset.tabId;
    if (!paneId || !last) return null;
    return { kind: "tab", paneId, tabId: last, index: tabs.length, before: false };
  }
  const pane = el.closest("[data-pane-id]") as HTMLElement | null;
  const paneId = pane?.dataset.paneId;
  if (!pane || !paneId || paneId === "pdf") return null;
  const rect = pane.getBoundingClientRect();
  // The strip keeps its reorder/append meaning. Start the top split band
  // directly below it, so every pane has a reachable top edge (GH #499).
  const strip = pane.querySelector<HTMLElement>(":scope > .tab-bar");
  const contentTop = Math.max(rect.top, strip?.getBoundingClientRect().bottom ?? rect.top);
  const side = edgeSideAt(x, y, rect, contentTop);
  if (side) {
    // The preview occupies this pane, including at an outer layout edge.
    // GH #499: use that same region for the drop rather than splitting root.
    return { kind: "split", paneId, side };
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
  // The window this tab bar renders into (OG-MULTIWINDOW P1).
  const ownerWindow = useOwnerWindow();
  let suppressClick = false;
  let root: HTMLDivElement | undefined;
  let strip: HTMLDivElement | undefined;
  let overviewTrigger: HTMLButtonElement | undefined;
  const router = props.router;
  const overviewId = `tab-overview-${createUniqueId()}`;
  const stripDragLayerId = `tab-strip-drag-${createUniqueId()}`;
  const [overflowing, setOverflowing] = createSignal(false);
  const [overviewOpen, setOverviewOpen] = createSignal(false);
  let hoverOverview = false;
  const overviewHover = hoverIntent(() => { if (hoverOverview && !overviewDragId()) dismissOverview(false); });
  const [overviewPosition, setOverviewPosition] = createSignal({ left: 8, top: 48, width: 360 });
  const [overviewDragId, setOverviewDragId] = createSignal<string | null>(null);
  const [overviewDrop, setOverviewDrop] = createSignal<{ tabId: string; before: boolean } | null>(null);
  let cancelOverviewReorder: (() => void) | undefined;
  let cancelStripDrag: (() => void) | undefined;
  let stripDragTabId: string | null = null;

  onCleanup(() => cancelStripDrag?.());
  createEffect(() => {
    const ids = router.tabs().map((tab) => tab.id);
    if (stripDragTabId && !ids.includes(stripDragTabId)) cancelStripDrag?.();
  });

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
    overviewHover.enter();
    hoverOverview = false;
    cancelOverviewReorder?.();
    setOverviewOpen(false);
    if (restoreFocus) queueMicrotask(() => overviewTrigger?.focus());
  };
  const positionOverview = () => {
    const rect = overviewTrigger?.getBoundingClientRect();
    if (!rect) return;
    const width = Math.min(360, Math.max(240, ownerWindow.innerWidth - 16));
    setOverviewPosition({
      left: Math.max(8, Math.min(rect.right - width, ownerWindow.innerWidth - width - 8)),
      top: rect.bottom + 5,
      width,
    });
  };
  const focusOverviewRow = (index: number) => {
    const rows = [...(ownerWindow.document.getElementById(overviewId)?.querySelectorAll<HTMLElement>("[data-tab-overview-row]") ?? [])];
    rows[Math.min(Math.max(index, 0), rows.length - 1)]?.focus();
  };
  const focusOverviewTab = (tabId: string) => {
    ownerWindow.document.getElementById(overviewId)
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
    const rows = [...(ownerWindow.document.getElementById(overviewId)?.querySelectorAll<HTMLElement>("[data-tab-overview-row]") ?? [])];
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
    } else return;
    event.preventDefault();
    focusOverviewRow(next);
  };

  const beginOverviewDrag = (tabId: string, event: PointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    cancelOverviewReorder?.();
    const startX = event.clientX;
    const startY = event.clientY;
    const pointerId = event.pointerId;
    let dragging = false;
    let active = true;

    const cleanup = () => {
      if (!active) return;
      active = false;
      ownerWindow.removeEventListener("pointermove", onMove);
      ownerWindow.removeEventListener("pointerup", onUp);
      ownerWindow.removeEventListener("pointercancel", onCancel);
      if (cancelOverviewReorder === cleanup) {
        cancelOverviewReorder = undefined;
        setOverviewDragId(null);
        setOverviewDrop(null);
      }
    };
    const ownsPointer = (pointer: PointerEvent) => pointer.pointerId === pointerId;
    const onMove = (move: PointerEvent) => {
      if (!ownsPointer(move)) return;
      if (!dragging && Math.hypot(move.clientX - startX, move.clientY - startY) < DRAG_THRESHOLD_PX) return;
      if (!dragging) {
        dragging = true;
        setOverviewDragId(tabId);
      }
      move.preventDefault();
      const row = ownerWindow.document.elementFromPoint(move.clientX, move.clientY)
        ?.closest<HTMLElement>("[data-tab-overview-row]");
      const targetId = row?.dataset.tabId;
      if (!row || !targetId || targetId === tabId) {
        setOverviewDrop(null);
        return;
      }
      const rect = row.getBoundingClientRect();
      setOverviewDrop({ tabId: targetId, before: move.clientY < rect.top + rect.height / 2 });
    };
    const onCancel = (cancel: PointerEvent) => {
      if (ownsPointer(cancel)) cleanup();
    };
    const onUp = (up: PointerEvent) => {
      if (!ownsPointer(up)) return;
      if (!active) return;
      const drop = overviewDrop();
      const didDrag = dragging;
      cleanup();
      if (didDrag && drop) {
        const index = router.tabs().findIndex((tab) => tab.id === drop.tabId);
        if (index >= 0) router.moveTabToIndex(tabId, index + (drop.before ? 0 : 1));
      }
      if (didDrag) queueMicrotask(() => focusOverviewTab(tabId));
    };
    cancelOverviewReorder = cleanup;
    ownerWindow.addEventListener("pointermove", onMove);
    ownerWindow.addEventListener("pointerup", onUp);
    ownerWindow.addEventListener("pointercancel", onCancel);
  };

  createEffect(() => {
    router.activeId();
    router.tabs().map((tab) => `${tab.id}:${tab.pos}:${tab.pinned}`).join("|");
    queueMicrotask(revealActive);
  });
  createEffect(() => {
    if (!overflowing() && overviewOpen()) dismissOverview(false);
  });
  createEffect(() => {
    if (!overviewOpen()) return;
    const unregister = registerTransientLayer({
      id: overviewId,
      root: () => ownerWindow.document.getElementById(overviewId),
      trigger: () => overviewTrigger ?? null,
      dismiss: () => {
        // The capture dispatcher owns ordinary Escape.  Let its registry
        // restoration focus the real trigger after this exact row session has
        // been retired, rather than competing with a target-local handler.
        dismissOverview(false);
        return true;
      },
    });
    queueMicrotask(() => {
      positionOverview();
      const activeIndex = Math.max(0, router.tabs().findIndex((tab) => tab.id === router.activeId()));
      if (!hoverOverview) focusOverviewRow(activeIndex);
    });
    ownerWindow.addEventListener("resize", positionOverview);
    ownerWindow.addEventListener("scroll", positionOverview, true);
    onCleanup(() => {
      cancelOverviewReorder?.();
      unregister();
      ownerWindow.removeEventListener("resize", positionOverview);
      ownerWindow.removeEventListener("scroll", positionOverview, true);
    });
  });
  // The overview panel is portalled out of `root`, so both count as inside.
  dismissOnOutsidePointer({
    open: overviewOpen,
    inside: () => [root, ownerWindow.document.getElementById(overviewId)],
    dismiss: () => dismissOverview(),
  });
  onMount(() => {
    const observer = newResizeObserver(ownerWindow, revealActive);
    if (strip) observer?.observe(strip);
    ownerWindow.addEventListener("resize", revealActive);
    queueMicrotask(revealActive);
    onCleanup(() => {
      observer?.disconnect();
      ownerWindow.removeEventListener("resize", revealActive);
    });
  });

  function beginDrag(tabId: string, e: PointerEvent) {
    if (e.button !== 0) return;
    // Interactive children keep their native pointer sequence. Capturing their
    // pointer on the tab card retargets the later click to the card in WebView2,
    // so the visible close control only activates the tab instead of closing it.
    if ((e.target as Element | null)?.closest("[data-tab-drag-exempt]")) return;
    e.preventDefault();
    activeStripDragSession?.cancel();
    const card = e.currentTarget as HTMLElement;
    const pointerId = e.pointerId;
    const focused = card.ownerDocument.activeElement;
    const restoreTarget = isHTMLElementNode(focused) && focused !== card.ownerDocument.body
      ? focused
      : card;
    const sourcePaneId = router.paneId;
    const startX = e.clientX;
    const startY = e.clientY;
    let dragging = false;
    let finished = false;
    let unregister = () => {};
    let session!: ActiveStripDragSession;

    const markMoved = () => {
      suppressClick = true;
      setTimeout(() => (suppressClick = false), 0);
    };
    const finish = (cancelled: boolean, restoreFocus: boolean) => {
      if (finished) return;
      finished = true;
      ownerWindow.removeEventListener("pointermove", onMove);
      ownerWindow.removeEventListener("pointerup", onUp);
      ownerWindow.removeEventListener("pointercancel", onCancel);
      ownerWindow.removeEventListener("blur", onBlur);
      card.removeEventListener("lostpointercapture", onLostPointerCapture, true);
      try {
        if (typeof card.hasPointerCapture === "function" &&
            typeof card.releasePointerCapture === "function" &&
            card.hasPointerCapture(pointerId)) card.releasePointerCapture(pointerId);
      } catch {
        // The platform may already have revoked capture during disposal.
      }
      unregister();
      if (activeStripDragSession === session) activeStripDragSession = null;
      if (cancelStripDrag === session.cancel) {
        cancelStripDrag = undefined;
        stripDragTabId = null;
      }
      if (activeTabDrag()?.paneId === sourcePaneId && activeTabDrag()?.tabId === tabId) {
        setActiveTabDrag(null);
        setCurrentTabDropTarget(null);
      }
      if (cancelled) markMoved();
      if (restoreFocus && restoreTarget.isConnected && !restoreTarget.inert) {
        queueMicrotask(() => restoreTarget.isConnected && restoreTarget.focus());
      }
    };
    session = { cancel: () => finish(true, true) };
    const ownsPointer = (event: PointerEvent) => event.pointerId === pointerId && activeStripDragSession === session;
    const onCancel = (event: PointerEvent) => {
      if (ownsPointer(event)) finish(true, true);
    };
    const onMove = (ev: PointerEvent) => {
      if (!ownsPointer(ev)) return;
      if (!dragging && Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_THRESHOLD_PX) return;
      if (!dragging) {
        dragging = true;
        setActiveTabDrag({ paneId: sourcePaneId, tabId });
      }
      ev.preventDefault();
      const el = ownerWindow.document.elementFromPoint(ev.clientX, ev.clientY);
      setCurrentTabDropTarget(tabDropTargetAt(el, ev.clientX, ev.clientY, sourcePaneId));
    };
    const onUp = (event: PointerEvent) => {
      if (!ownsPointer(event)) return;
      const target = currentTabDropTarget();
      const shouldDrop = dragging && !!target;
      if (dragging) markMoved();
      finish(false, false);
      if (shouldDrop) applyTabDrop(sourcePaneId, tabId, target!);
    };
    const onBlur = () => finish(true, true);
    const onLostPointerCapture = (event: PointerEvent) => {
      if (ownsPointer(event)) finish(true, true);
    };

    activeStripDragSession = session;
    cancelStripDrag = session.cancel;
    stripDragTabId = tabId;
    unregister = registerTransientLayer({
      id: stripDragLayerId,
      root: () => card,
      trigger: () => restoreTarget,
      dismiss: () => {
        if (finished || activeStripDragSession !== session) return false;
        // Registry focus restoration owns the Escape/Back path.
        finish(true, false);
        return true;
      },
    });
    try { card.setPointerCapture?.(pointerId); } catch { /* optional platform support */ }

    ownerWindow.addEventListener("pointermove", onMove);
    ownerWindow.addEventListener("pointerup", onUp);
    ownerWindow.addEventListener("pointercancel", onCancel);
    // Editor-to-tab focus transfer must not cancel the armed drag.
    ownerWindow.addEventListener("blur", onBlur);
    card.addEventListener("lostpointercapture", onLostPointerCapture, true);
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
              t.pinned ? translate("tab-bar-pinned-sticky-links-open-in-a-new-tab") : translate("tab-bar-double-click-to-pin-sticky")
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
            {/* Offer ✕ only when closing actually works: another tab remains, OR
                this lone tab can close its whole split pane (GH #207 — closeTab
                hands a lone non-feed tab to the pane's last-tab handler). The
                window's very last tab and a feed pane's last tab stay unclosable.
                A real button with its own title: keyboard-operable, and its hover
                tooltip can't fall back to the tab's pin hint (GH #340). */}
            <Show when={closeOffered(router, t)}>
              <button
                class="tab-close"
                type="button"
                title={translate("tab-bar-close-tab")}
                aria-label={translate("tab-bar-close-tab")}
                data-tab-drag-exempt
                onClick={(e) => {
                  e.stopPropagation();
                  router.closeTab(t.id);
                }}
              >
                ✕
              </button>
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
          title={translate("tab-bar-show-all-tabs")}
          aria-label={translate("tab-bar-show-all-tabs")}
          aria-haspopup="listbox"
          aria-expanded={overviewOpen()}
          aria-controls={overviewId}
          data-tab-overview-trigger
          onMouseEnter={() => {
            overviewHover.enter();
            if (!overviewOpen()) { hoverOverview = true; setOverviewOpen(true); }
          }}
          onMouseLeave={overviewHover.leave}
          onClick={() => {
            overviewHover.enter();
            if (hoverOverview) {
              hoverOverview = false;
              focusOverviewTab(router.activeId());
            }
            else if (overviewOpen()) dismissOverview();
            else setOverviewOpen(true);
          }}
        >⌄</button>
        <Show when={overviewOpen()}>
          <FloatingPortal>
          <div
            id={overviewId}
            class="tab-overview"
            role="listbox"
            aria-label={translate("tab-bar-open-tabs")}
            style={{
              left: `${overviewPosition().left}px`,
              top: `${overviewPosition().top}px`,
              width: `${overviewPosition().width}px`,
            }}
            onKeyDown={onOverviewKeyDown}
            onMouseEnter={overviewHover.enter}
            onMouseLeave={overviewHover.leave}
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
                    aria-label={translate("tab-bar-reorder", { title: String(tabFullTitle(router.tabRoute(tab))) })}
                    title={translate("tab-bar-drag-to-reorder-alt-up-down-moves-the")}
                    onPointerDown={(event) => beginOverviewDrag(tab.id, event)}
                    onClick={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                    }}
                  >⠿</button>
                  <span class="tab-overview-active" aria-hidden="true">{tab.id === router.activeId() ? "✓" : ""}</span>
                  <Show when={tab.pinned}><span class="tab-overview-pin" title={translate("tab-bar-pinned")}><EmojiText text="📌" /></span></Show>
                  <span class="tab-overview-title"><EmojiText text={tabFullTitle(router.tabRoute(tab))} /></span>
                  <Show when={closeOffered(router, tab)}>
                    <button
                      class="tab-overview-close"
                      type="button"
                      aria-label={translate("tab-bar-close", { title: String(tabFullTitle(router.tabRoute(tab))) })}
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
          </FloatingPortal>
        </Show>
      </Show>
    </div>
  );
}
