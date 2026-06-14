import { For, Show, type JSX } from "solid-js";
import { contextMenu, closeContextMenu, zoomInto } from "../ui";
import { backend } from "../backend";
import { ensureBlockId, blockSubtreeMarkdown, deleteBlock } from "../store";

// Right-click block context menu (copy ref / copy / cut / delete / zoom).
export function ContextMenu(): JSX.Element {
  const items = (id: string) => [
    { label: "Zoom into block", run: () => zoomInto(id) },
    {
      label: "Copy block ref",
      run: () => void backend().writeText(`((${ensureBlockId(id)}))`),
    },
    {
      label: "Copy block",
      run: () => void backend().writeText(blockSubtreeMarkdown(id)),
    },
    {
      label: "Cut block",
      run: () => {
        void backend().writeText(blockSubtreeMarkdown(id));
        deleteBlock(id);
      },
    },
    { label: "Delete block", run: () => deleteBlock(id), danger: true },
  ];

  return (
    <Show when={contextMenu()}>
      <div class="ctx-overlay" onClick={closeContextMenu} onContextMenu={(e) => { e.preventDefault(); closeContextMenu(); }}>
        <div
          class="ctx-menu"
          style={{ left: `${contextMenu()!.x}px`, top: `${contextMenu()!.y}px` }}
          onClick={(e) => e.stopPropagation()}
        >
          <For each={items(contextMenu()!.blockId)}>
            {(it) => (
              <div
                class="ctx-item"
                classList={{ danger: !!(it as any).danger }}
                onClick={() => {
                  it.run();
                  closeContextMenu();
                }}
              >
                {it.label}
              </div>
            )}
          </For>
        </div>
      </div>
    </Show>
  );
}
