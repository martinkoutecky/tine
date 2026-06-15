import { For, Show, createResource, type JSX } from "solid-js";
import {
  rightSidebar,
  closeRightSidebarItem,
  rightSidebarWidth,
  setRightSidebarWidth,
  persistRightSidebarWidth,
  type SidebarItem,
} from "../ui";
import { openPage } from "../router";
import { backend } from "../backend";
import { blockView } from "../render/block";
import { RefBlocks } from "./RefBlocks";
import type { BlockDto } from "../types";

// Read-only right sidebar: a stack of pages/blocks opened for reference while
// editing the main pane. A page item is resolved live by name; a block item
// renders the snapshot subtree it was opened with (so it survives navigation
// and never round-trips to the backend). Macros inside a snapshot — e.g. a
// parked `{{query}}` TODO list — still re-run live via RefBlocks → InlineText.
export function RightSidebar(): JSX.Element {
  return (
    <Show when={rightSidebar().length > 0}>
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
        <div class="right-sidebar-header">Sidebar</div>
        <div class="right-sidebar-body">
          <For each={rightSidebar()}>
            {(item, i) => <SidebarItemView item={item} onClose={() => closeRightSidebarItem(i())} />}
          </For>
        </div>
      </div>
    </Show>
  );
}

function SidebarItemView(props: { item: SidebarItem; onClose: () => void }): JSX.Element {
  return (
    <Show when={props.item.kind === "page"} fallback={<BlockItem item={props.item as Extract<SidebarItem, { kind: "block" }>} onClose={props.onClose} />}>
      <PageItem item={props.item as Extract<SidebarItem, { kind: "page" }>} onClose={props.onClose} />
    </Show>
  );
}

// A page opened in the sidebar: resolved live by name.
function PageItem(props: {
  item: { name: string; pageKind: "journal" | "page" };
  onClose: () => void;
}): JSX.Element {
  const [data] = createResource(
    () => props.item,
    async (it) => {
      const p = await backend().getPage(it.name, it.pageKind);
      return p ? { title: p.title, blocks: p.blocks } : null;
    }
  );
  return (
    <div class="rs-item">
      <div class="rs-item-head">
        <a class="rs-item-title" onClick={() => openPage(props.item.name, props.item.pageKind)}>
          {data()?.title ?? props.item.name}
        </a>
        <button class="rs-close" onClick={props.onClose} title="Close">
          ✕
        </button>
      </div>
      <Show when={data()}>
        <div class="rs-item-body">
          <RefBlocks blocks={data()!.blocks} page={props.item.name} />
        </div>
      </Show>
    </div>
  );
}

// A block subtree opened in the sidebar: rendered straight from the snapshot.
function BlockItem(props: {
  item: { page: string; blocks: BlockDto[] };
  onClose: () => void;
}): JSX.Element {
  const title = () => blockView(props.item.blocks[0]?.raw ?? "").lines[0] || props.item.page;
  return (
    <div class="rs-item">
      <div class="rs-item-head">
        <a class="rs-item-title" onClick={() => openPage(props.item.page, "page")} title={`On ${props.item.page}`}>
          {title()}
        </a>
        <button class="rs-close" onClick={props.onClose} title="Close">
          ✕
        </button>
      </div>
      <div class="rs-item-body">
        <RefBlocks blocks={props.item.blocks} page={props.item.page} />
      </div>
    </div>
  );
}
