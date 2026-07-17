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
  type SidebarItem,
} from "../ui";
import { mobileDrawerMode } from "../mobileDrawers";
import { registerTransientLayer } from "../transientLayers";
import { MobileDrawerPanel, dismissDrawerAndRestore } from "./MobileDrawerShell";
import { openPageTarget, openPageAtBlock } from "../router";
import { EmojiText } from "../render/emoji";
import { backend } from "../backend";
import { doc, ensurePageLoaded, pageByName } from "../store";
import { visibleBody } from "../render/block";
import { Block, SurfaceContext } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { endEditForSurface } from "../editorController";

function surfaceKey(item: SidebarItem): string {
  return `sidebar:${sidebarItemKey(item)}`;
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
                  <SidebarItemView item={item} surfaceKey={key} collapsed={!!item.collapsed} onToggle={collapse} onClose={close} />
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
      onCleanup(() => { active = false; });
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
            ensurePageLoaded(dto);
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
}): JSX.Element {
  return (
    <Show
      when={props.item.kind === "page"}
      fallback={<BlockItem item={props.item as Extract<SidebarItem, { kind: "block" }>} surfaceKey={props.surfaceKey} collapsed={props.collapsed} onToggle={props.onToggle} onClose={props.onClose} />}
    >
      <PageItem item={props.item as Extract<SidebarItem, { kind: "page" }>} surfaceKey={props.surfaceKey} collapsed={props.collapsed} onToggle={props.onToggle} onClose={props.onClose} />
    </Show>
  );
}

function PageItem(props: {
  item: { name: string; pageKind: "journal" | "page"; path?: string };
  surfaceKey: string;
  collapsed: boolean;
  onToggle: (control: HTMLButtonElement) => void;
  onClose: () => void;
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
    <div class="rs-item" data-sidebar-surface={props.surfaceKey} classList={{ collapsed: props.collapsed }}>
      <div class="rs-item-head">
        <button class="rs-item-toggle" type="button" aria-label={props.collapsed ? "Expand sidebar item" : "Collapse sidebar item"} aria-expanded={!props.collapsed} aria-controls={bodyId} data-right-sidebar-item-toggle onClick={(event) => props.onToggle(event.currentTarget)}>
          <span aria-hidden="true">▸</span>
        </button>
        <a class="rs-item-title" onClick={() => {
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
}): JSX.Element {
  useEnsurePage(
    () => props.item.page,
    () => props.item.pageKind,
    () => props.item.path,
    () => !props.collapsed,
  );
  // Resolve by store key first; fall back to finding the loaded node whose
  // persisted id:: matches (the in-memory key can differ from the id:: it was
  // parked under). Returns the live store node so edits stay propagated.
  const node = () => {
    const owner = pageByName(props.item.page);
    if (props.item.path && owner?.path !== props.item.path) return undefined;
    const direct = doc.byId[props.item.uuid];
    if (direct) return direct;
    const re = new RegExp(`(?:^|\\n)id:: *${props.item.uuid}(?:\\s|$)`);
    return Object.values(doc.byId).find((n) => n.page === props.item.page && re.test(n.raw));
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
    <div class="rs-item" data-sidebar-surface={props.surfaceKey} classList={{ collapsed: props.collapsed }}>
      <div class="rs-item-head">
        <button class="rs-item-toggle" type="button" aria-label={props.collapsed ? "Expand sidebar item" : "Collapse sidebar item"} aria-expanded={!props.collapsed} aria-controls={bodyId} data-right-sidebar-item-toggle onClick={(event) => props.onToggle(event.currentTarget)}>
          <span aria-hidden="true">▸</span>
        </button>
        <a
          class="rs-item-title"
          onClick={() => openPageAtBlock({
            name: props.item.page,
            pageKind: props.item.pageKind,
            block: props.item.uuid,
            path: props.item.path,
          })}
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
