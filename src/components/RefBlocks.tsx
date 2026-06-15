// Read-only rendering of a BlockDto tree. Shared by Linked References, query
// results, and embeds. Mirrors the rendered (non-editing) block look.

import { For, Show, type JSX } from "solid-js";
import type { BlockDto } from "../types";
import { blockView } from "../render/block";
import { InlineText } from "../render/inline";
import { openBlockInSidebar } from "../ui";

// `page` (the page these blocks live on) is threaded through so a shift-click
// can open the block in the sidebar with a correct "go to page" link.
export function RefBlocks(props: { blocks: BlockDto[]; page?: string }): JSX.Element {
  return <For each={props.blocks}>{(b) => <RefBlock block={b} page={props.page} />}</For>;
}

// Snapshot a read-only DTO block for the sidebar (same shape as the store's
// blockSnapshot, but the DTO is already self-contained here).
function dtoSnapshot(b: BlockDto, page: string): { key: string; page: string; blocks: BlockDto[] } {
  const idProp = /(?:^|\n)id:: *([0-9a-fA-F-]{8,})/.exec(b.raw);
  return { key: idProp ? idProp[1] : b.id, page, blocks: [b] };
}

function RefBlock(props: { block: BlockDto; page?: string }): JSX.Element {
  const view = () => blockView(props.block.raw);
  return (
    <div class="ls-block ref-block">
      <div class="block-main">
        <div class="block-controls">
          <span
            class="bullet-container"
            title="Shift-click to open in sidebar"
            onClick={(e) => {
              if (e.shiftKey) {
                e.stopPropagation();
                openBlockInSidebar(dtoSnapshot(props.block, props.page ?? ""));
              }
            }}
          >
            <span class="bullet" />
          </span>
        </div>
        <div class="block-content-wrapper">
          <div class="block-content" classList={{ done: view().done }}>
            <Show when={view().marker}>
              <span class={`block-marker marker-${view().marker?.toLowerCase()}`}>
                {view().marker}
              </span>{" "}
            </Show>
            <For each={view().lines}>
              {(line, i) => (
                <>
                  <Show when={i() > 0}>
                    <br />
                  </Show>
                  <InlineText text={line} />
                </>
              )}
            </For>
          </div>
        </div>
      </div>
      <Show when={props.block.children.length}>
        <div class="block-children-container">
          <div class="block-children">
            <RefBlocks blocks={props.block.children} page={props.page} />
          </div>
        </div>
      </Show>
    </div>
  );
}
