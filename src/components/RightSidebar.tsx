import { t } from "../i18n";
import { BlockList } from "./BlockList";
import { resizeSidebar, commitSidebarWidth, setLeftSidebarReserve } from "../sidebarSizing";
import { For, Show, createEffect, createSignal, createUniqueId, onCleanup, type JSX } from "solid-js";
import { rightSidebar, rightSidebarOpen, toggleRightSidebar, closeRightSidebarItem, closeOtherRightSidebarItems, collapseOtherRightSidebarItems, moveRightSidebarItem, closeAllRightSidebarItems, setRightSidebarItemCollapsed, setAllRightSidebarItemsCollapsed, rightSidebarWidth, sidebarOpen, sidebarWidth, sidebarItemKey, adoptResolvedPageName, registerRightSidebarClosePreparation, replaceSidebarBlock, openBlockInSidebar, type SidebarBlock, type SidebarItem } from "../ui";
import { replaceSidebarItem } from "../sidebarReplace";
import { beginRowReorderDrag, rowReorderClickSuppressed, type RowDropTarget } from "./rowReorder";
import "../styles/rightSidebarReorder.css";
import { graphEpoch } from "../graphSession";
import { mobileDrawerMode } from "../mobileDrawers";
import { dismissOnOutsidePointer, registerTransientLayer } from "../transientLayers";
import { FloatingPortal } from "./FloatingPortal";
import { MobileDrawerPanel, dismissDrawerAndRestore } from "./MobileDrawerShell";
import { openPageTarget, openPageAtBlock, openPageTargetInNewTab, openInNewTab } from "../router";
import { InlineText } from "../render/inline";
import { visibleBody } from "../render/block";
import { nearestAncestors } from "../render/blockTrail";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { EmojiText } from "../render/emoji";
import { pageIconPrefix } from "../pageIconBatch";
import { backend } from "../backend";
import { emptyPage, ensurePageLoaded, pageByName, pageLoadRefusalMessage, resolveBlockRef, settleBlockRef, whenPageReplaceable, formatForBlock, blockRef, blockPositionRef, node as docNode } from "../document";
import { Block, OutlineScopeContext, SurfaceContext } from "./Block";
import { TaggedPages } from "./TaggedPages";
import { LinkedReferences } from "./LinkedReferences";
import { PageTypingTarget } from "./Page";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { endEditForSurface } from "../editorController";
import { FailureBoundary } from "./FailureBoundary";
import { isHTMLElementNode, mainWindow } from "../windowRealm";
import { rightSidebarFocusRequest } from "../ui";

function surfaceKey(item: SidebarItem): string {
  return `sidebar:${sidebarItemKey(item)}`;
}

// Live drop target while a row reorder drag is in progress (GH #211).
const [rsDropTarget, setRsDropTarget] = createSignal<RowDropTarget | null>(null);
/** Pointerdown on a row head starts a reorder drag, unless it landed on an
 *  interactive child (toggle/close button, title link). The drop lands before
 *  or after the target row, within the list only. */
function startRowDrag(from: number, event: PointerEvent) {
  if ((event.target as HTMLElement | null)?.closest("button, a, input, textarea, [contenteditable=\"true\"]")) return;
  beginRowReorderDrag(event, ".right-sidebar-body .rs-item", setRsDropTarget, ({ index, before }) => {
    const at = index + (before ? 0 : 1);
    moveRightSidebarItem(from, at > from ? at - 1 : at);
  });
}
/** The reorder attributes every sidebar row carries. */
function rowAttrs(index: number) {
  const drop = () => rsDropTarget()?.index === index ? rsDropTarget() : null;
  return { index, before: () => drop()?.before === true, after: () => drop()?.before === false };
}
type Row = ReturnType<typeof rowAttrs>;

/** Commit the active textarea synchronously through its blur handler before a
 * disclosure removes the owning surface. Then clear any remaining edit owner
 * (for example when the window-focus preservation path kept edit mode alive). */
function prepareSurfaceForUnmount(key: string) {
  const active = mainWindow.document.activeElement;
  if (isHTMLElementNode(active)) {
    const surface = active.closest<HTMLElement>("[data-sidebar-surface]");
    if (surface?.dataset.sidebarSurface === key) active.blur();
  }
  endEditForSurface("sidebar-collapse", key);
}

function restoreDisclosureFocus(key: string) {
  queueMicrotask(() => {
    const surface = [...mainWindow.document.querySelectorAll<HTMLElement>("[data-sidebar-surface]")]
      .find((element) => element.dataset.sidebarSurface === key);
    surface?.querySelector<HTMLButtonElement>("[data-right-sidebar-item-toggle]")?.focus();
  });
}

/** Render open sidebar pages and blocks as live editable surfaces. Opening an
 * item may load its page into the shared working set; edits change the same
 * nodes as the main pane. Hidden state renders nothing. O(visible sidebar
 * blocks) plus page-load latency. */
export function RightSidebar(): JSX.Element {
  let body: HTMLDivElement | undefined;
  createEffect(() => {
    const request = rightSidebarFocusRequest();
    if (!request) return;
    queueMicrotask(() => {
      if (rightSidebarFocusRequest() !== request || !rightSidebarOpen()) return;
      if (body) body.scrollTop = 0;
      const card = body?.querySelector<HTMLElement>("[data-sidebar-surface]");
      if (card?.dataset.sidebarSurface === `sidebar:${request.key}`) {
        card.querySelector<HTMLButtonElement>("[data-right-sidebar-item-toggle]")?.focus();
      }
    });
  });
  const [actionsOpen, setActionsOpen] = createSignal(false);
  let actionsButton: HTMLButtonElement | undefined;
  let actionsMenu: HTMLDivElement | undefined;
  createEffect(() => {
    if (actionsOpen()) queueMicrotask(() => actionsMenu?.querySelector<HTMLButtonElement>("button")?.focus());
  });
  const prepareAll = () => {
    for (const item of rightSidebar()) prepareSurfaceForUnmount(surfaceKey(item));
  };
  onCleanup(registerRightSidebarClosePreparation(prepareAll));
  // GH #521: the right sidebar's max width keeps the main pane usable beside the
  // left sidebar; a drawer-mode left sidebar overlays instead of taking room.
  createEffect(() => setLeftSidebarReserve(sidebarOpen() && !mobileDrawerMode() ? sidebarWidth() : 0));
  createEffect(() => {
    if (!actionsOpen()) return;
    const unregister = registerTransientLayer({
      id: "right-sidebar-actions",
      root: () => actionsMenu ?? null,
      trigger: () => actionsButton ?? null,
      dismiss: () => { setActionsOpen(false); actionsButton?.focus(); return true; },
    });
    onCleanup(unregister);
  });
  const runBulk = (action: "collapse" | "expand" | "close") => {
    if (action !== "expand") prepareAll();
    if (action === "collapse") setAllRightSidebarItemsCollapsed(true);
    else if (action === "expand") setAllRightSidebarItemsCollapsed(false);
    else closeAllRightSidebarItems();
    setActionsOpen(false);
    actionsButton?.focus();
  };
  const onMenuKeyDown: JSX.EventHandlerUnion<HTMLDivElement, KeyboardEvent> = (event) => {
    const buttons = [...(actionsMenu?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    const index = buttons.indexOf(mainWindow.document.activeElement as HTMLButtonElement);
    let next = index;
    if (event.key === "ArrowDown") next = (index + 1 + buttons.length) % buttons.length;
    else if (event.key === "ArrowUp") next = (index - 1 + buttons.length) % buttons.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = buttons.length - 1;
    else if (event.key === "Escape") return; // global transient registry owns it
    else return;
    event.preventDefault();
    buttons[next]?.focus();
  };
  return (
    <Show when={rightSidebarOpen()}>
      <MobileDrawerPanel
        side="right"
        label={t("right-sidebar-reference-sidebar")}
        class="right-sidebar"
        style={{
          flex: `0 0 ${rightSidebarWidth()}px`,
          width: `${rightSidebarWidth()}px`,
          "--mobile-drawer-width": `${rightSidebarWidth()}px`,
        }}
      >
        <div
          class="rs-resizer"
          onMouseDown={(e) => {
            e.preventDefault();
            const onMove = (ev: MouseEvent) =>
              resizeSidebar("right", mainWindow.innerWidth - ev.clientX);
            const onUp = () => {
              mainWindow.removeEventListener("mousemove", onMove);
              mainWindow.removeEventListener("mouseup", onUp);
              commitSidebarWidth("right");
            };
            mainWindow.addEventListener("mousemove", onMove);
            mainWindow.addEventListener("mouseup", onUp);
          }}
        />
        <div class="right-sidebar-header">
          <span>{t("right-sidebar-sidebar")}</span>
          <div class="rs-header-actions">
            <button
              ref={actionsButton}
              class="rs-actions-button"
              type="button"
              title={t("right-sidebar-sidebar-item-actions")}
              aria-label={t("right-sidebar-sidebar-item-actions")}
              aria-haspopup="menu"
              aria-expanded={actionsOpen()}
              data-right-sidebar-actions
              onClick={() => setActionsOpen((open) => !open)}
            >⋯</button>
            <Show when={actionsOpen()}>
              <div ref={actionsMenu} class="rs-actions-menu" role="menu" onKeyDown={onMenuKeyDown}>
                <button type="button" role="menuitem" data-right-sidebar-action="collapse-all" onClick={() => runBulk("collapse")}>{t("right-sidebar-collapse-all")}</button>
                <button type="button" role="menuitem" data-right-sidebar-action="expand-all" onClick={() => runBulk("expand")}>{t("right-sidebar-expand-all")}</button>
                <button type="button" role="menuitem" data-right-sidebar-action="close-all" onClick={() => runBulk("close")}>{t("right-sidebar-close-all")}</button>
              </div>
            </Show>
            <button class="rs-close" title={t("right-sidebar-close-sidebar-t-r")} onClick={() => {
              if (mobileDrawerMode()) dismissDrawerAndRestore("explicit");
              else toggleRightSidebar();
            }}>✕</button>
          </div>
        </div>
        <div class="right-sidebar-body" ref={body}>
          <Show
            when={rightSidebar().length > 0}
            fallback={
              <div class="rs-empty">{t("right-sidebar-nothing-open-shift-click-a-page-or-block")}</div>
            }
          >
            <For each={rightSidebar()}>
              {(item, i) => {
                const key = surfaceKey(item);
                const collapse = (control: HTMLButtonElement) => {
                  const keepFocus = mainWindow.document.activeElement === control;
                  const next = !item.collapsed;
                  if (next) prepareSurfaceForUnmount(key);
                  setRightSidebarItemCollapsed(i(), next);
                  if (keepFocus) restoreDisclosureFocus(key);
                };
                const close = () => {
                  prepareSurfaceForUnmount(key);
                  closeRightSidebarItem(i());
                };
                return (
                // Each sidebar item is its own editing surface, so a block that
                // also shows in the main pane doesn't fight it for the caret.
                <SurfaceContext.Provider value={key}>
                  <FailureBoundary region="This sidebar item"><SidebarItemView item={item} surfaceKey={key} collapsed={!!item.collapsed} onToggle={collapse} onClose={close} row={rowAttrs(i())} /></FailureBoundary>
                </SurfaceContext.Provider>
                );
              }}
            </For>
          </Show>
        </div>
      </MobileDrawerPanel>
    </Show>
  );
}

// Ensure the item's page is loaded into the working set. Return an error signal
// so a failed load is visible while the item stays available for retry.
// Re-runs on graphEpoch so a sidebar restored *before* the graph is open
// retries once it opens. A load refused because another file holding the name
// has unsaved work says so and re-runs once that page is replaceable, instead
// of leaving an empty body observing nothing (GH #254 family, master 7bd793bd0).
function useEnsurePage(
  name: () => string,
  kind: () => "journal" | "page",
  path: () => string | undefined,
  enabled: () => boolean,
) {
  const uid = createUniqueId();
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [retry, setRetry] = createSignal(0);
  createEffect(() => {
    if (!enabled()) return;
    retry();
    setLoadError(null);
    const epoch = graphEpoch();
    const n = name();
    const k = kind();
    const p = path();
    const loaded = pageByName(n);
    if (n && (!loaded || (p && loaded.id !== p))) {
      let active = true;
      let stopWaiting: (() => void) | undefined;
      onCleanup(() => { active = false; stopWaiting?.(); });
      const request = p ? backend().getPageByPath(p) : backend().getPage(n, k);
      void request
        .then((dto) => {
          // Drop a load that resolved after a graph switch — otherwise it would
          // insert an old-graph page into the new graph's working set.
          if (!active || epoch !== graphEpoch()) return;
          // Like the main pane, a name with no file is an editable empty page.
          // An exact file path that disappeared must not become a new page.
          const pageDto = dto ?? (!p ? emptyPage(n, k) : null);
          if (pageDto) {
            // Alias-map warmup usually canonicalizes before the item is created.
            // A restored/early mixed-case item can race it; adopt the backend's
            // canonical page name before the exact-keyed store renders the body.
            if (!p && k === "page" && pageDto.name !== n) adoptResolvedPageName(n, pageDto.name);
            const refusal = ensurePageLoaded(pageDto);
            if (!refusal) return;
            setLoadError(pageLoadRefusalMessage(refusal));
            stopWaiting = whenPageReplaceable(refusal.page, `sidebar:${uid}`, () => setRetry((count) => count + 1));
          }
        })
        .catch(() => {
          if (active && epoch === graphEpoch()) setLoadError("Could not load this sidebar page. Collapse and expand to retry.");
        });
    }
  });
  return loadError;
}

function SidebarItemView(props: {
  item: SidebarItem;
  surfaceKey: string;
  collapsed: boolean;
  onToggle: (control: HTMLButtonElement) => void;
  onClose: () => void;
  row: Row;
}): JSX.Element {
  return (
    <Show
      when={props.item.kind === "page"}
      fallback={<BlockItem item={props.item as Extract<SidebarItem, { kind: "block" }>} surfaceKey={props.surfaceKey} collapsed={props.collapsed} onToggle={props.onToggle} onClose={props.onClose} row={props.row} />}
    >
      <PageItem item={props.item as Extract<SidebarItem, { kind: "page" }>} surfaceKey={props.surfaceKey} collapsed={props.collapsed} onToggle={props.onToggle} onClose={props.onClose} row={props.row} />
    </Show>
  );
}

/** The per-item "more actions" menu with the item's close button (GH #462).
 * Close others and Collapse others need a second item; Open as page is page-only.
 * Item toggles and header bulk actions retain the other operations.
 * The menu floats out of the item (the
 * item clips its overflow) and closes on an outside press or Escape. */
function ItemActions(props: { index: number; surfaceKey: string; page?: { name: string; pageKind: "journal" | "page"; path?: string }; collapsed: boolean; onClose: () => void }): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const [pos, setPos] = createSignal({ top: 0, right: 0 });
  const menuId = `rs-item-menu-${createUniqueId()}`;
  let button: HTMLButtonElement | undefined;
  let menu: HTMLDivElement | undefined;
  const count = () => rightSidebar().length;
  const prepareOthers = () => rightSidebar().forEach((it, i) => { if (i !== props.index) prepareSurfaceForUnmount(surfaceKey(it)); });
  const finish = () => {
    setOpen(false);
    queueMicrotask(() => { if (button?.isConnected) button.focus(); });
  };
  const run = (action: () => void) => () => { action(); finish(); };
  const toggleOpen = () => {
    if (!open() && button) {
      const rect = button.getBoundingClientRect();
      setPos({ top: rect.bottom + 4, right: Math.max(8, mainWindow.innerWidth - rect.right) });
    }
    setOpen(!open());
  };
  createEffect(() => {
    if (!open()) return;
    queueMicrotask(() => menu?.querySelector<HTMLButtonElement>("button")?.focus());
    const unregister = registerTransientLayer({
      id: menuId,
      root: () => menu ?? null,
      trigger: () => button ?? null,
      dismiss: () => { setOpen(false); button?.focus(); return true; },
    });
    onCleanup(unregister);
  });
  dismissOnOutsidePointer({ open, inside: () => [button, menu], dismiss: () => setOpen(false) });
  const onKeyDown: JSX.EventHandlerUnion<HTMLDivElement, KeyboardEvent> = (event) => {
    const buttons = [...(menu?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    const index = buttons.indexOf(mainWindow.document.activeElement as HTMLButtonElement);
    let next = index;
    if (event.key === "ArrowDown") next = (index + 1 + buttons.length) % buttons.length;
    else if (event.key === "ArrowUp") next = (index - 1 + buttons.length) % buttons.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = buttons.length - 1;
    else return; // Escape: the global transient registry owns it
    event.preventDefault();
    buttons[next]?.focus();
  };
  return (
    <div class="rs-item-actions">
      <Show when={count() > 1 || props.page}>
      <button
        ref={button}
        class="rs-actions-button"
        type="button"
        title={t("right-sidebar-more-actions")}
        aria-label={t("right-sidebar-more-actions")}
        aria-haspopup="menu"
        aria-expanded={open()}
        aria-controls={open() ? menuId : undefined}
        data-right-sidebar-item-actions
        onClick={toggleOpen}
      >⋯</button>
      </Show>
      <button class="rs-close" onClick={props.onClose} title={t("right-sidebar-close")}>✕</button>
      <Show when={open()}>
        <FloatingPortal>
          <div
            ref={menu}
            id={menuId}
            class="rs-actions-menu rs-item-menu"
            role="menu"
            style={{ top: `${pos().top}px`, right: `${pos().right}px` }}
            onKeyDown={onKeyDown}
          >
            <Show when={count() > 1}>
              <button type="button" role="menuitem" data-right-sidebar-item-action="close-others" onClick={run(() => { prepareOthers(); closeOtherRightSidebarItems(props.index); })}>{t("right-sidebar-close-others")}</button>
              <button type="button" role="menuitem" data-right-sidebar-item-action="collapse-others" onClick={run(() => { prepareOthers(); collapseOtherRightSidebarItems(props.index); })}>{t("right-sidebar-collapse-others")}</button>
            </Show>
            <Show when={props.page}>
              {(p) => <button type="button" role="menuitem" data-right-sidebar-item-action="open-as-page" onClick={run(() => openPageTarget({ name: p().name, pageKind: p().pageKind, path: p().path }))}>{t("right-sidebar-open-as-page")}</button>}
            </Show>
          </div>
        </FloatingPortal>
      </Show>
    </div>
  );
}

function PageItem(props: {
  item: { name: string; pageKind: "journal" | "page"; path?: string };
  surfaceKey: string;
  collapsed: boolean;
  onToggle: (control: HTMLButtonElement) => void;
  onClose: () => void;
  row: Row;
}): JSX.Element {
  const loadError = useEnsurePage(
    () => props.item.name,
    () => props.item.pageKind,
    () => props.item.path,
    () => !props.collapsed,
  );
  const page = () => {
    const loaded = pageByName(props.item.name);
    return props.item.path && loaded?.id !== props.item.path ? undefined : loaded;
  };
  const bodyId = `rs-item-body-${createUniqueId()}`;
  return (
    <div class="rs-item" data-sidebar-surface={props.surfaceKey} data-row-index={props.row.index} classList={{ collapsed: props.collapsed, "row-drop-before": props.row.before(), "row-drop-after": props.row.after() }}>
      <div class="rs-item-head" onPointerDown={(event) => startRowDrag(props.row.index, event)}>
        <button class="rs-item-toggle" type="button" aria-label={props.collapsed ? t("right-sidebar-expand-sidebar-item") : t("right-sidebar-collapse-sidebar-item")} aria-expanded={!props.collapsed} aria-controls={bodyId} data-right-sidebar-item-toggle onClick={(event) => props.onToggle(event.currentTarget)}>
          <span aria-hidden="true">▸</span>
        </button>
        <a class="rs-item-title" onMouseDown={internalLinkMouseDown} onClick={(e) => {
          if (rowReorderClickSuppressed()) return;
          const target = { name: props.item.name, pageKind: props.item.pageKind, path: props.item.path };
          // The shift destination (right sidebar) is meaningless for a title
          // already IN the sidebar, so it keeps the ordinary navigation.
          const dest = internalLinkDest(e);
          if (dest === "background") openPageTargetInNewTab(target);
          else if (dest === "pane") openRouteInOtherPane({ kind: "page", ...target });
          else openPageTarget(target);
        }} onAuxClick={(e) => internalLinkAuxClick(e, () =>
          openPageTargetInNewTab({ name: props.item.name, pageKind: props.item.pageKind, path: props.item.path }))}>
          <EmojiText text={pageIconPrefix(props.item.name) + props.item.name} />
        </a>
        <ItemActions index={props.row.index} surfaceKey={props.surfaceKey} page={props.item} collapsed={props.collapsed} onClose={props.onClose} />
      </div>
      <Show when={!props.collapsed}>
        <Show when={page()} fallback={<div id={bodyId} class="rs-item-body rs-item-loading">{loadError() ?? ""}</div>}>
          <div id={bodyId} class="rs-item-body">
            <BlockList ids={page()!.roots} />
            {/* The same producer the main pane uses: a page opened only here still
                gets its phantom empty bullet and a trailing target (GH #483). */}
            <PageTypingTarget page={page} surface={props.surfaceKey} />
            {/* OG shows a page's Linked/Unlinked References in the sidebar view too,
                not just the main pane. Same lazy components, so this stays cheap. */}
            <Show when={props.item.pageKind !== "journal"}>
              <FailureBoundary region="Tagged Pages"><TaggedPages name={props.item.name} /></FailureBoundary>
            </Show>
            <FailureBoundary region="Linked References">
              <LinkedReferences name={props.item.name} />
            </FailureBoundary>
            <FailureBoundary region="Unlinked References">
              <UnlinkedReferences name={props.item.name} />
            </FailureBoundary>
          </div>
        </Show>
      </Show>
    </div>
  );
}

function BlockItem(props: {
  item: { uuid: string; page: string; pageKind: "journal" | "page"; path?: string; blockPos?: number[] };
  surfaceKey: string;
  collapsed: boolean;
  onToggle: (control: HTMLButtonElement) => void;
  onClose: () => void;
  row: Row;
}): JSX.Element {
  const loadError = useEnsurePage(
    () => props.item.page,
    () => props.item.pageKind,
    () => props.item.path,
    () => !props.collapsed,
  );
  // Resolve the durable sidebar identity back to the current live store node so
  // edits stay propagated even while its store key is still transient.
  const node = () => {
    const id = resolveBlockRef(props.item, { navigation: true });
    return id ? docNode(id) : undefined;
  };
  // A restored item names its ID-less block by position (navigation never writes an
  // `id::`); once its page loads, swap the position for the block's live key.
  createEffect(() => {
    const item = props.item;
    if (!item.blockPos) return;
    const settled = settleBlockRef(item);
    if (!settled || settled.blockPos) return;
    const { blockPos: _drop, ...rest } = item;
    replaceSidebarBlock(item as SidebarBlock, { ...rest, kind: "block", uuid: settled.uuid } as SidebarBlock);
  });
  const pageLoaded = () => {
    const loaded = pageByName(props.item.page);
    return !!loaded && (!props.item.path || loaded.id === props.item.path);
  };
  const title = () => {
    const n = node();
    return n?.page ?? props.item.page;
  };
  // GH #640, OG `breadcrumb` (block.cljs) as the sidebar uses it: the page, then
  // the block's ancestors (not the block itself), at most the nearest three after
  // a "⋯" (src/render/blockTrail.ts).
  const trail = () => { const n = node(); return n ? nearestAncestors(n.id) : null; };
  const shownAncestors = () => trail()?.ids ?? [];
  const openCrumb = (aid: string, event: MouseEvent) => {
    if (rowReorderClickSuppressed()) return;
    const ref = blockPositionRef(blockRef(aid));
    const dest = internalLinkDest(event);
    const where = { ...(ref.path ? { path: ref.path } : {}), ...(ref.blockPos ? { blockPos: [...ref.blockPos] } : {}) };
    const route = { kind: "page" as const, name: ref.page, pageKind: ref.pageKind, block: ref.uuid, ...where };
    if (dest === "sidebar") openBlockInSidebar(ref);
    else if (dest === "background") openInNewTab(route);
    else if (dest === "pane") openRouteInOtherPane(route);
    else replaceSidebarItem(props.item as SidebarBlock, { kind: "block", uuid: ref.uuid, page: ref.page, pageKind: ref.pageKind, ...where, collapsed: false });
  };
  const bodyId = `rs-item-body-${createUniqueId()}`;
  return (
    <div class="rs-item" data-sidebar-surface={props.surfaceKey} data-row-index={props.row.index} classList={{ collapsed: props.collapsed, "row-drop-before": props.row.before(), "row-drop-after": props.row.after() }}>
      <div class="rs-item-head" onPointerDown={(event) => startRowDrag(props.row.index, event)}>
        <button class="rs-item-toggle" type="button" aria-label={props.collapsed ? t("right-sidebar-expand-sidebar-item") : t("right-sidebar-collapse-sidebar-item")} aria-expanded={!props.collapsed} aria-controls={bodyId} data-right-sidebar-item-toggle onClick={(event) => props.onToggle(event.currentTarget)}>
          <span aria-hidden="true">▸</span>
        </button>
        <a
          class="rs-item-title"
          onClick={() => rowReorderClickSuppressed() || openPageAtBlock({
            name: props.item.page,
            pageKind: props.item.pageKind,
            block: props.item.uuid,
            path: props.item.path,
          })}
          title={t("right-sidebar-on", { page: String(props.item.page) })}
        >
          {title()}
        </a>
        <Show when={shownAncestors().length}>
          <span class="rs-item-crumbs">
            <Show when={trail()?.more}>
              <span class="crumb-sep">›</span><span class="rs-crumb-more">⋯</span>
            </Show>
            <For each={shownAncestors()}>
              {(aid) => (
                <>
                  <span class="crumb-sep">›</span>
                  <a class="rs-crumb" onMouseDown={internalLinkMouseDown} onClick={(e) => openCrumb(aid, e)}
                    onAuxClick={(e) => internalLinkAuxClick(e, () => openCrumb(aid, e))}>
                    <InlineText text={visibleBody(docNode(aid)?.raw ?? "")[0] || "…"} format={formatForBlock(aid)} />
                  </a>
                </>
              )}
            </For>
          </span>
        </Show>
        <ItemActions index={props.row.index} surfaceKey={props.surfaceKey} collapsed={props.collapsed} onClose={props.onClose} />
      </div>
      <Show when={!props.collapsed}>
        <Show
          when={node()}
          fallback={
            <Show
              when={pageLoaded()}
              fallback={<div id={bodyId} class="rs-item-body rs-item-loading">{loadError() ?? ""}</div>}
            >
              <div id={bodyId} class="rs-item-body rs-item-missing">{t("right-sidebar-this-block-is-no-longer-available")}</div>
            </Show>
          }
        >
          {(n) => (
            <div id={bodyId} class="rs-item-body">
              {/* GH #358: a block parked here is the root of this view, like a
                  zoom root: its children render regardless of the source
                  outline's collapsed flag (collapsed:: is not mutated), while
                  descendants keep their own collapse state. */}
              <OutlineScopeContext.Provider value={{ roots: [n().id], forceExpandedRoot: n().id }}>
                <Block id={n().id} forceExpanded />
              </OutlineScopeContext.Provider>
            </div>
          )}
        </Show>
      </Show>
    </div>
  );
}
