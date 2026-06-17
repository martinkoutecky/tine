import { For, Show, createResource, type JSX } from "solid-js";
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
import { Block } from "./Block";

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
              {(item, i) => <SidebarItemView item={item} onClose={() => closeRightSidebarItem(i())} />}
            </For>
          </Show>
        </div>
      </div>
    </Show>
  );
}

// Load the item's page into the working set; returns a signal that flips true
// once the page (and thus the live nodes) are available.
function useLoadedPage(name: () => string, kind: () => "journal" | "page") {
  // Key on graphEpoch so this re-runs once the graph is open: a restored
  // sidebar mounts before loadGraphPath finishes, so the first attempt can hit
  // a not-yet-open graph; bumping the epoch retries it.
  const [ready] = createResource(
    () => ({ n: name(), k: kind(), e: graphEpoch() }),
    async ({ n, k }) => {
      if (!pageByName(n)) {
        const dto = await backend().getPage(n, k);
        if (dto) ensurePageLoaded(dto);
      }
      return true;
    }
  );
  return ready;
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
  const ready = useLoadedPage(() => props.item.name, () => props.item.pageKind);
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
      <Show when={ready() && page()}>
        <div class="rs-item-body">
          <For each={page()!.roots}>{(id) => <Block id={id} />}</For>
        </div>
      </Show>
    </div>
  );
}

function BlockItem(props: {
  item: { uuid: string; page: string; pageKind: "journal" | "page" };
  onClose: () => void;
}): JSX.Element {
  const ready = useLoadedPage(() => props.item.page, () => props.item.pageKind);
  // Resolve by store key first; fall back to finding the loaded node whose
  // persisted id:: matches (the in-memory key can differ from the id:: it was
  // parked under). Returns the live store node so edits stay propagated.
  const node = () => {
    const direct = doc.byId[props.item.uuid];
    if (direct) return direct;
    const re = new RegExp(`(?:^|\\n)id:: *${props.item.uuid}(?:\\s|$)`);
    return Object.values(doc.byId).find((n) => n.page === props.item.page && re.test(n.raw));
  };
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
      <Show when={ready()} fallback={<div class="rs-item-body rs-item-loading" />}>
        <Show
          when={node()}
          fallback={<div class="rs-item-body rs-item-missing">This block is no longer available.</div>}
        >
          {(n) => (
            <div class="rs-item-body">
              <Block id={n().id} />
            </div>
          )}
        </Show>
      </Show>
    </div>
  );
}
