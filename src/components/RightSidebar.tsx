import { For, Show, createEffect, createSignal, createUniqueId, onCleanup, type JSX } from "solid-js";
import {
  rightSidebar,
  rightSidebarOpen,
  toggleRightSidebar,
  closeRightSidebarItem,
  closeAllRightSidebarItems,
  setRightSidebarItemCollapsed,
  setAllRightSidebarItemsCollapsed,
  rightSidebarWidth,
  setRightSidebarWidth,
  persistRightSidebarWidth,
  graphEpoch,
  sidebarItemKey,
  renamePageInNavigation,
  registerRightSidebarClosePreparation,
  moveRightSidebarItem,
  pushToast,
  type SidebarItem,
} from "../ui";
import { beginRowReorderDrag, rowReorderClickSuppressed, type RowDropTarget } from "./rowReorder";
import { mobileDrawerMode } from "../mobileDrawers";
import { registerTransientLayer } from "../transientLayers";
import { MobileDrawerPanel, dismissDrawerAndRestore } from "./MobileDrawerShell";
import { openPageTarget, openPageAtBlock } from "../router";
import { EmojiText } from "../render/emoji";
import { backend } from "../backend";
import { doc, ensurePageLoaded, onPageBecameReplaceable, pageByName, resolveBlockRef } from "../store";
import { visibleBody } from "../render/block";
import { Block, SurfaceContext } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { endEditForSurface } from "../editorController";

function surfaceKey(item: SidebarItem): string {
  return `sidebar:${sidebarItemKey(item)}`;
}

// Live drop target while a row reorder drag is in progress (GH #211).
const [rsDropTarget, setRsDropTarget] = createSignal<RowDropTarget | null>(null);
/** Pointerdown on a row head starts a reorder drag, unless it landed on an
 *  interactive child (toggle/close button, title link). */
function startRowDrag(index: number, event: PointerEvent) {
  if ((event.target as HTMLElement | null)?.closest("button, a, input, textarea, [contenteditable=\"true\"]")) return;
  beginRowReorderDrag(event, index, ".right-sidebar-body .rs-item", setRsDropTarget, moveRightSidebarItem);
}

/** Commit the active textarea synchronously through its blur handler before a
 * disclosure removes the owning surface. Then clear any remaining edit owner
 * (for example when the window-focus preservation path kept edit mode alive). */
function prepareSurfaceForUnmount(key: string) {
  const active = document.activeElement;
  if (active instanceof HTMLElement) {
    const surface = active.closest<HTMLElement>("[data-sidebar-surface]");
    if (surface?.dataset.sidebarSurface === key) active.blur();
  }
  endEditForSurface("sidebar-collapse", key);
}

function restoreDisclosureFocus(key: string) {
  queueMicrotask(() => {
    const surface = [...document.querySelectorAll<HTMLElement>("[data-sidebar-surface]")]
      .find((element) => element.dataset.sidebarSurface === key);
    surface?.querySelector<HTMLButtonElement>("[data-right-sidebar-item-toggle]")?.focus();
  });
}

// Right sidebar: a stack of pages/blocks opened for reference. Each item is a
// LIVE reference — it loads its page into the shared working set and renders the
// same editable <Block> the main view uses, so edits here are edits to the one
// underlying node and propagate everywhere (OG's model, kept lazy). A parked
// {{query}} also stays live, since it's the real block.
export function RightSidebar(): JSX.Element {
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
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
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
        label="Reference sidebar"
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
              setRightSidebarWidth(Math.min(800, Math.max(220, window.innerWidth - ev.clientX)));
            const onUp = () => {
              window.removeEventListener("mousemove", onMove);
              window.removeEventListener("mouseup", onUp);
              persistRightSidebarWidth();
            };
            window.addEventListener("mousemove", onMove);
            window.addEventListener("mouseup", onUp);
          }}
        />
        <div class="right-sidebar-header">
          <span>Sidebar</span>
          <div class="rs-header-actions">
            <button
              ref={actionsButton}
              class="rs-actions-button"
              type="button"
              title="Sidebar item actions"
              aria-label="Sidebar item actions"
              aria-haspopup="menu"
              aria-expanded={actionsOpen()}
              data-right-sidebar-actions
              onClick={() => setActionsOpen((open) => !open)}
            >⋯</button>
            <Show when={actionsOpen()}>
              <div ref={actionsMenu} class="rs-actions-menu" role="menu" onKeyDown={onMenuKeyDown}>
                <button type="button" role="menuitem" data-right-sidebar-action="collapse-all" onClick={() => runBulk("collapse")}>Collapse all</button>
                <button type="button" role="menuitem" data-right-sidebar-action="expand-all" onClick={() => runBulk("expand")}>Expand all</button>
                <button type="button" role="menuitem" data-right-sidebar-action="close-all" onClick={() => runBulk("close")}>Close all</button>
              </div>
            </Show>
            <button class="rs-close" title="Close sidebar (t r)" onClick={() => {
              if (mobileDrawerMode()) dismissDrawerAndRestore("explicit");
              else toggleRightSidebar();
            }}>✕</button>
          </div>
        </div>
        <div class="right-sidebar-body">
          <Show
            when={rightSidebar().length > 0}
            fallback={
              <div class="rs-empty">
                Nothing open. Shift-click a page or block to open it here.
              </div>
            }
          >
            <For each={rightSidebar()}>
              {(item, i) => {
                const key = surfaceKey(item);
                const collapse = (control: HTMLButtonElement) => {
                  const keepFocus = document.activeElement === control;
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
                  <SidebarItemView item={item} surfaceKey={key} collapsed={!!item.collapsed} onToggle={collapse} onClose={close} rowIndex={i()} onHeadPointerDown={(e) => startRowDrag(i(), e)} />
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

// Ensure the item's page is loaded into the working set. Fire-and-forget side
// effect (NOT a resource whose error state could gate rendering): the body
// renders off actual store presence, so a failed early attempt is harmless.
// Re-runs on graphEpoch so a sidebar restored *before* the graph is open
// retries once it opens.
function useEnsurePage(
  name: () => string,
  kind: () => "journal" | "page",
  path: () => string | undefined,
  enabled: () => boolean,
) {
  createEffect(() => {
    if (!enabled()) return;
    const epoch = graphEpoch();
    const n = name();
    const k = kind();
    const p = path();
    const loaded = pageByName(n);
    if (n && (!loaded || (p && loaded.path !== p))) {
      let active = true;
      // Registered from the SYNCHRONOUS effect body, never from an awaited
      // `.then`: an `onCleanup` created outside the Solid owner never runs, so a
      // disposed refusal would leak a permanently inactive listener.
      let stopRetry: (() => void) | null = null;
      onCleanup(() => {
        active = false;
        stopRetry?.();
      });
      // DURABLE, not one-shot: the retry can itself be refused again — a lease
      // taken during its awaited read is enough — and a listener that
      // unsubscribed before that read would strand the item on an empty body.
      // One read at a time. Two sweeps landing while a retry read is pending
      // otherwise fan out into several reads of the same target; the refusal gate
      // still keeps them SAFE, but they are wasted work on a path that can be
      // driven by any unrelated save.
      let retryInFlight = false;
      const retryWhenFreed = (pageName: string) => {
        stopRetry?.();
        stopRetry = onPageBecameReplaceable(pageName, () => {
          if (retryInFlight) return;
          retryInFlight = true;
          void (p ? backend().getPageByPath(p) : backend().getPage(n, k))
            .then((fresh) => {
              if (!active || epoch !== graphEpoch() || !fresh) return;
              if (!ensurePageLoaded(fresh)) {
                stopRetry?.();
                stopRetry = null;
              }
            })
            .catch(() => {})
            .finally(() => {
              retryInFlight = false;
            });
        });
      };
      const request = p ? backend().getPageByPath(p) : backend().getPage(n, k);
      void request
        .then((dto) => {
          // Drop a load that resolved after a graph switch — otherwise it would
          // insert an old-graph page into the new graph's working set.
          if (!active || epoch !== graphEpoch()) return;
          if (dto) {
            // Alias-map warmup usually canonicalizes before the item is created.
            // A restored/early mixed-case item can race it; adopt the backend's
            // canonical page name before the exact-keyed store renders the body.
            if (!p && k === "page" && dto.name !== n) renamePageInNavigation(n, dto.name);
            // A refusal must not leave this item on an empty loading body with
            // nothing observing the incumbent's save lifecycle — collapsing and
            // re-expanding happening to retrigger it is not a contract. Say what
            // is holding the file, so the user can resolve it and re-open.
            // (GH #254 increment 3.)
            const refusal = ensurePageLoaded(dto);
            if (refusal) {
              // Say why, AND resume automatically. Relying on the user collapsing
              // and re-expanding happened to work but was never a contract; the
              // item otherwise sits on an empty loading body observing nothing.
              // (GH #254 increment 3, acceptance row E2.)
              pushToast(
                `“${refusal.page}” has unsaved changes, so the other file with that name ` +
                  `can't be shown in the sidebar yet. It will appear once that is resolved.`,
                "error",
              );
              // DURABLE, not one-shot: the retry can itself be refused again —
              // a lease taken during its awaited read is enough — and a listener
              // that unsubscribed before that read would strand the item on an
              // empty body. It stops only when the load actually succeeds.
              retryWhenFreed(refusal.page);
            }
          }
        })
        .catch(() => {
          // graph not open yet / page missing — retried on graphEpoch.
        });
    }
  });
}

function SidebarItemView(props: {
  item: SidebarItem;
  surfaceKey: string;
  collapsed: boolean;
  onToggle: (control: HTMLButtonElement) => void;
  onClose: () => void;
  rowIndex: number;
  onHeadPointerDown: (event: PointerEvent) => void;
}): JSX.Element {
  return (
    <Show
      when={props.item.kind === "page"}
      fallback={<BlockItem item={props.item as Extract<SidebarItem, { kind: "block" }>} surfaceKey={props.surfaceKey} collapsed={props.collapsed} onToggle={props.onToggle} onClose={props.onClose} rowIndex={props.rowIndex} onHeadPointerDown={props.onHeadPointerDown} />}
    >
      <PageItem item={props.item as Extract<SidebarItem, { kind: "page" }>} surfaceKey={props.surfaceKey} collapsed={props.collapsed} onToggle={props.onToggle} onClose={props.onClose} rowIndex={props.rowIndex} onHeadPointerDown={props.onHeadPointerDown} />
    </Show>
  );
}

function PageItem(props: {
  item: { name: string; pageKind: "journal" | "page"; path?: string };
  surfaceKey: string;
  collapsed: boolean;
  onToggle: (control: HTMLButtonElement) => void;
  onClose: () => void;
  rowIndex: number;
  onHeadPointerDown: (event: PointerEvent) => void;
}): JSX.Element {
  useEnsurePage(
    () => props.item.name,
    () => props.item.pageKind,
    () => props.item.path,
    () => !props.collapsed,
  );
  const page = () => {
    const loaded = pageByName(props.item.name);
    return props.item.path && loaded?.path !== props.item.path ? undefined : loaded;
  };
  const bodyId = `rs-item-body-${createUniqueId()}`;
  return (
    <div
      class="rs-item"
      data-sidebar-surface={props.surfaceKey}
      data-row-index={props.rowIndex}
      classList={{ collapsed: props.collapsed, "row-drop-before": rsDropTarget()?.index === props.rowIndex && rsDropTarget()!.before, "row-drop-after": rsDropTarget()?.index === props.rowIndex && !rsDropTarget()!.before }}
    >
      <div class="rs-item-head" onPointerDown={props.onHeadPointerDown}>
        <button class="rs-item-toggle" type="button" aria-label={props.collapsed ? "Expand sidebar item" : "Collapse sidebar item"} aria-expanded={!props.collapsed} aria-controls={bodyId} data-right-sidebar-item-toggle onClick={(event) => props.onToggle(event.currentTarget)}>
          <span aria-hidden="true">▸</span>
        </button>
        <a class="rs-item-title" onClick={() => {
          if (rowReorderClickSuppressed()) return;
          openPageTarget({ name: props.item.name, pageKind: props.item.pageKind, path: props.item.path });
        }}>
          <EmojiText text={props.item.name} />
        </a>
        <button class="rs-close" onClick={props.onClose} title="Close">
          ✕
        </button>
      </div>
      <Show when={!props.collapsed}>
        <Show when={page()} fallback={<div id={bodyId} class="rs-item-body rs-item-loading" />}>
          <div id={bodyId} class="rs-item-body">
            <For each={page()!.roots}>{(id) => <Block id={id} />}</For>
            {/* OG shows a page's Linked/Unlinked References in the sidebar view too,
                not just the main pane. Same lazy components, so this stays cheap. */}
            <LinkedReferences name={props.item.name} />
            <UnlinkedReferences name={props.item.name} />
          </div>
        </Show>
      </Show>
    </div>
  );
}

function BlockItem(props: {
  item: { uuid: string; page: string; pageKind: "journal" | "page"; path?: string };
  surfaceKey: string;
  collapsed: boolean;
  onToggle: (control: HTMLButtonElement) => void;
  onClose: () => void;
  rowIndex: number;
  onHeadPointerDown: (event: PointerEvent) => void;
}): JSX.Element {
  useEnsurePage(
    () => props.item.page,
    () => props.item.pageKind,
    () => props.item.path,
    () => !props.collapsed,
  );
  // Resolve the durable sidebar identity back to the current live store node so
  // edits stay propagated even while its store key is still transient.
  const node = () => {
    const id = resolveBlockRef(props.item);
    return id ? doc.byId[id] : undefined;
  };
  const pageLoaded = () => {
    const loaded = pageByName(props.item.page);
    return !!loaded && (!props.item.path || loaded.path === props.item.path);
  };
  const title = () => {
    const n = node();
    return n ? visibleBody(n.raw)[0] || props.item.page : props.item.page;
  };
  const bodyId = `rs-item-body-${createUniqueId()}`;
  return (
    <div
      class="rs-item"
      data-sidebar-surface={props.surfaceKey}
      data-row-index={props.rowIndex}
      classList={{ collapsed: props.collapsed, "row-drop-before": rsDropTarget()?.index === props.rowIndex && rsDropTarget()!.before, "row-drop-after": rsDropTarget()?.index === props.rowIndex && !rsDropTarget()!.before }}
    >
      <div class="rs-item-head" onPointerDown={props.onHeadPointerDown}>
        <button class="rs-item-toggle" type="button" aria-label={props.collapsed ? "Expand sidebar item" : "Collapse sidebar item"} aria-expanded={!props.collapsed} aria-controls={bodyId} data-right-sidebar-item-toggle onClick={(event) => props.onToggle(event.currentTarget)}>
          <span aria-hidden="true">▸</span>
        </button>
        <a
          class="rs-item-title"
          onClick={() => {
            if (rowReorderClickSuppressed()) return;
            openPageAtBlock({
              name: props.item.page,
              pageKind: props.item.pageKind,
              block: props.item.uuid,
              path: props.item.path,
            });
          }}
          title={`On ${props.item.page}`}
        >
          {title()}
        </a>
        <button class="rs-close" onClick={props.onClose} title="Close">
          ✕
        </button>
      </div>
      <Show when={!props.collapsed}>
        <Show
          when={node()}
          fallback={
            <Show
              when={pageLoaded()}
              fallback={<div id={bodyId} class="rs-item-body rs-item-loading" />}
            >
              <div id={bodyId} class="rs-item-body rs-item-missing">This block is no longer available.</div>
            </Show>
          }
        >
          {(n) => (
            <div id={bodyId} class="rs-item-body">
              <Block id={n().id} />
            </div>
          )}
        </Show>
      </Show>
    </div>
  );
}
