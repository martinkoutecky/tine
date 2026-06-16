import { For, Show, type JSX } from "solid-js";
import { contextMenu, closeContextMenu, zoomInto, openBlockInSidebar, openPageInSidebar } from "../ui";
import { openPage, openPageInNewTab } from "../router";
import { backend } from "../backend";
import {
  ensureBlockId,
  blockRef,
  blockSubtreeMarkdown,
  deleteBlock,
  setBlockProperty,
  toggleBlockProperty,
  blockProperty,
  setHeading,
  setCollapsedDeep,
} from "../store";

// Block background colors, matching Logseq's built-in set.
const COLORS = ["yellow", "red", "pink", "green", "blue", "purple", "gray"];
const COLOR_BG: Record<string, string> = {
  yellow: "#fbe69e",
  red: "#f5a3a3",
  pink: "#f3b0d4",
  green: "#a6e3b4",
  blue: "#a8c9f0",
  purple: "#cdb4ee",
  gray: "#d3d6da",
};

// Right-click context menu. Universal over its target: a block (full editing
// menu — colors, headings, open/copy/cut, collapse, numbered list) or a page
// reference (open / open in sidebar / new tab / copy ref). The target is
// whatever you right-clicked, so right-clicking a [[page]] acts on the page,
// not the block that contains it.
export function ContextMenu(): JSX.Element {
  const close = () => closeContextMenu();

  return (
    <Show when={contextMenu()}>
      {(m) => (
        <div class="ctx-overlay" onClick={close} onContextMenu={(e) => { e.preventDefault(); close(); }}>
          <div
            class="ctx-menu"
            style={{ left: `${m().x}px`, top: `${m().y}px` }}
            onClick={(e) => e.stopPropagation()}
          >
            <Show
              when={m().kind === "block"}
              fallback={
                <PageMenu
                  name={(m() as { name: string }).name}
                  pageKind={(m() as { pageKind: "journal" | "page" }).pageKind}
                  close={close}
                />
              }
            >
              <BlockMenu id={(m() as { blockId: string }).blockId} close={close} />
            </Show>
          </div>
        </div>
      )}
    </Show>
  );
}

function BlockMenu(props: { id: string; close: () => void }): JSX.Element {
  return (
    <>
      {/* Color row */}
      <div class="ctx-row ctx-colors">
        <button
          class="ctx-color ctx-color-none"
          title="No background"
          onClick={() => { setBlockProperty(props.id, "background-color", null); props.close(); }}
        >
          ✕
        </button>
        <For each={COLORS}>
          {(c) => (
            <button
              class="ctx-color"
              title={c}
              style={{ background: COLOR_BG[c] }}
              onClick={() => { toggleBlockProperty(props.id, "background-color", c); props.close(); }}
            />
          )}
        </For>
      </div>

      {/* Heading row */}
      <div class="ctx-row ctx-headings">
        <For each={[1, 2, 3, 4, 5, 6]}>
          {(h) => (
            <button class="ctx-h" title={`Heading ${h}`} onClick={() => { setHeading(props.id, h); props.close(); }}>
              H{h}
            </button>
          )}
        </For>
        <button class="ctx-h" title="Remove heading" onClick={() => { setHeading(props.id, null); props.close(); }}>
          ⌫
        </button>
      </div>

      <div class="ctx-sep" />

      <For each={blockActions(props.id)}>
        {(it) => (
          <div
            class="ctx-item"
            classList={{ danger: !!it.danger }}
            onClick={() => { it.run(); props.close(); }}
          >
            {it.label}
          </div>
        )}
      </For>
    </>
  );
}

function PageMenu(props: {
  name: string;
  pageKind: "journal" | "page";
  close: () => void;
}): JSX.Element {
  const items: { label: string; run: () => void }[] = [
    { label: "Open", run: () => openPage(props.name, props.pageKind) },
    { label: "Open in sidebar", run: () => openPageInSidebar(props.name, props.pageKind) },
    { label: "Open in new tab", run: () => openPageInNewTab(props.name, props.pageKind) },
    { label: "Copy page ref", run: () => void backend().writeText(`[[${props.name}]]`) },
  ];
  return (
    <For each={items}>
      {(it) => (
        <div class="ctx-item" onClick={() => { it.run(); props.close(); }}>
          {it.label}
        </div>
      )}
    </For>
  );
}

function blockActions(id: string): { label: string; run: () => void; danger?: boolean }[] {
  const numbered = blockProperty(id, "logseq.order-list-type") === "number";
  return [
    { label: "Open in sidebar", run: () => openBlockInSidebar(blockRef(id)) },
    { label: "Zoom into block", run: () => zoomInto(id) },
    { label: "Copy block ref", run: () => void backend().writeText(`((${ensureBlockId(id)}))`) },
    { label: "Copy block embed", run: () => void backend().writeText(`{{embed ((${ensureBlockId(id)}))}}`) },
    { label: "Copy block", run: () => void backend().writeText(blockSubtreeMarkdown(id)) },
    {
      label: "Cut block",
      run: () => {
        void backend().writeText(blockSubtreeMarkdown(id));
        deleteBlock(id);
      },
    },
    {
      label: numbered ? "Remove numbered list" : "Numbered list",
      run: () => toggleBlockProperty(id, "logseq.order-list-type", "number"),
    },
    { label: "Collapse all", run: () => setCollapsedDeep(id, true) },
    { label: "Expand all", run: () => setCollapsedDeep(id, false) },
    { label: "Delete block", run: () => deleteBlock(id), danger: true },
  ];
}
