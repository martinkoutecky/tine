import { For, Show, createEffect, type JSX } from "solid-js";
import {
  rightSidebar,
  rightSidebarOpen,
  toggleRightSidebar,
  closeRightSidebarItem,
  rightSidebarWidth,
  setRightSidebarWidth,
  persistRightSidebarWidth,
  graphEpoch,
  type SidebarItem,
} from "../ui";
import { openPage } from "../router";
import { backend } from "../backend";
import { doc, ensurePageLoaded, pageByName } from "../store";
import { blockView } from "../render/block";
import { Block, SurfaceContext } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";

// Right sidebar: a stack of pages/blocks opened for reference. Each item is a
// LIVE reference — it loads its page into the shared working set and renders the
// same editable <Block> the main view uses, so edits here are edits to the one
// underlying node and propagate everywhere (OG's model, kept lazy). A parked
// {{query}} also stays live, since it's the real block.
export function RightSidebar(): JSX.Element {
  return (
    <Show when={rightSidebarOpen()}>
      <div
        class="right-sidebar"
        style={{ flex: `0 0 ${rightSidebarWidth()}px`, width: `${rightSidebarWidth()}px` }}
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
          <button class="rs-close" title="Close sidebar (t r)" onClick={toggleRightSidebar}>✕</button>
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
              {(item, i) => (
                // Each sidebar item is its own editing surface, so a block that
                // also shows in the main pane doesn't fight it for the caret.
                <SurfaceContext.Provider value={`sidebar:${i()}`}>
                  <SidebarItemView item={item} onClose={() => closeRightSidebarItem(i())} />
                </SurfaceContext.Provider>
              )}
            </For>
          </Show>
        </div>
      </div>
    </Show>
  );
}

// Ensure the item's page is loaded into the working set. Fire-and-forget side
// effect (NOT a resource whose error state could gate rendering): the body
// renders off actual store presence, so a failed early attempt is harmless.
// Re-runs on graphEpoch so a sidebar restored *before* the graph is open
// retries once it opens.
function useEnsurePage(name: () => string, kind: () => "journal" | "page") {
  createEffect(() => {
    const epoch = graphEpoch();
    const n = name();
    const k = kind();
    if (n && !pageByName(n)) {
      void backend()
        .getPage(n, k)
        .then((dto) => {
          // Drop a load that resolved after a graph switch — otherwise it would
          // insert an old-graph page into the new graph's working set.
          if (epoch !== graphEpoch()) return;
          if (dto) ensurePageLoaded(dto);
        })
        .catch(() => {
          // graph not open yet / page missing — retried on graphEpoch.
        });
    }
  });
}

function SidebarItemView(props: { item: SidebarItem; onClose: () => void }): JSX.Element {
  return (
    <Show
      when={props.item.kind === "page"}
      fallback={<BlockItem item={props.item as Extract<SidebarItem, { kind: "block" }>} onClose={props.onClose} />}
    >
      <PageItem item={props.item as Extract<SidebarItem, { kind: "page" }>} onClose={props.onClose} />
    </Show>
  );
}

function PageItem(props: {
  item: { name: string; pageKind: "journal" | "page" };
  onClose: () => void;
}): JSX.Element {
  useEnsurePage(() => props.item.name, () => props.item.pageKind);
  const page = () => pageByName(props.item.name);
  return (
    <div class="rs-item">
      <div class="rs-item-head">
        <a class="rs-item-title" onClick={() => openPage(props.item.name, props.item.pageKind)}>
          {props.item.name}
        </a>
        <button class="rs-close" onClick={props.onClose} title="Close">
          ✕
        </button>
      </div>
      <Show when={page()} fallback={<div class="rs-item-body rs-item-loading" />}>
        <div class="rs-item-body">
          <For each={page()!.roots}>{(id) => <Block id={id} />}</For>
          {/* OG shows a page's Linked/Unlinked References in the sidebar view too,
              not just the main pane. Same lazy components, so this stays cheap. */}
          <LinkedReferences name={props.item.name} />
          <UnlinkedReferences name={props.item.name} />
        </div>
      </Show>
    </div>
  );
}

function BlockItem(props: {
  item: { uuid: string; page: string; pageKind: "journal" | "page" };
  onClose: () => void;
}): JSX.Element {
  useEnsurePage(() => props.item.page, () => props.item.pageKind);
  // Resolve by store key first; fall back to finding the loaded node whose
  // persisted id:: matches (the in-memory key can differ from the id:: it was
  // parked under). Returns the live store node so edits stay propagated.
  const node = () => {
    const direct = doc.byId[props.item.uuid];
    if (direct) return direct;
    const re = new RegExp(`(?:^|\\n)id:: *${props.item.uuid}(?:\\s|$)`);
    return Object.values(doc.byId).find((n) => n.page === props.item.page && re.test(n.raw));
  };
  const pageLoaded = () => !!pageByName(props.item.page);
  const title = () => {
    const n = node();
    return n ? blockView(n.raw).lines[0] || props.item.page : props.item.page;
  };
  return (
    <div class="rs-item">
      <div class="rs-item-head">
        <a
          class="rs-item-title"
          onClick={() => openPage(props.item.page, props.item.pageKind)}
          title={`On ${props.item.page}`}
        >
          {title()}
        </a>
        <button class="rs-close" onClick={props.onClose} title="Close">
          ✕
        </button>
      </div>
      <Show
        when={node()}
        fallback={
          <Show
            when={pageLoaded()}
            fallback={<div class="rs-item-body rs-item-loading" />}
          >
            <div class="rs-item-body rs-item-missing">This block is no longer available.</div>
          </Show>
        }
      >
        {(n) => (
          <div class="rs-item-body">
            <Block id={n().id} />
          </div>
        )}
      </Show>
    </div>
  );
}
