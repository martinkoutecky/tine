import { For, Show, createResource, type JSX } from "solid-js";
import {
  rightSidebar,
  closeRightSidebarItem,
  rightSidebarWidth,
  setRightSidebarWidth,
  persistRightSidebarWidth,
} from "../ui";
import { openPage } from "../router";
import { backend } from "../backend";
import { RefBlocks } from "./RefBlocks";

// Read-only right sidebar: pages/blocks opened via shift-click for reference
// while editing the main pane.
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
            {(item, i) => <SidebarItem item={item} onClose={() => closeRightSidebarItem(i())} />}
          </For>
        </div>
      </div>
    </Show>
  );
}

function SidebarItem(props: {
  item: { kind: "page" | "block"; ref: string };
  onClose: () => void;
}): JSX.Element {
  const [data] = createResource(
    () => props.item,
    async (it) => {
      if (it.kind === "page") {
        const p = await backend().getPage(it.ref, "page");
        return p ? { title: p.name, blocks: p.blocks } : null;
      }
      const g = await backend().resolveBlock(it.ref);
      return g ? { title: g.page, blocks: g.blocks } : null;
    }
  );

  return (
    <div class="rs-item">
      <div class="rs-item-head">
        <a
          class="rs-item-title"
          onClick={() => props.item.kind === "page" && openPage(props.item.ref, "page")}
        >
          {data()?.title ?? props.item.ref}
        </a>
        <button class="rs-close" onClick={props.onClose} title="Close">
          ✕
        </button>
      </div>
      <Show when={data()}>
        <div class="rs-item-body">
          <RefBlocks blocks={data()!.blocks} />
        </div>
      </Show>
    </div>
  );
}
