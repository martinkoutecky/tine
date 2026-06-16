// Read-only rendering of a BlockDto tree. Shared by Linked References, query
// results, and embeds. Mirrors the rendered (non-editing) block look.

import { For, Show, type JSX } from "solid-js";
import type { BlockDto } from "../types";
import { blockView } from "../render/block";
import { InlineText } from "../render/inline";
import { openBlockInSidebar } from "../ui";

// `page`/`pageKind` (where these blocks live) are threaded through so a
// shift-click can open the block live in the sidebar.
export function RefBlocks(props: {
  blocks: BlockDto[];
  page?: string;
  pageKind?: "journal" | "page";
}): JSX.Element {
  return (
    <For each={props.blocks}>
      {(b) => <RefBlock block={b} page={props.page} pageKind={props.pageKind} />}
    </For>
  );
}

function RefBlock(props: {
  block: BlockDto;
  page?: string;
  pageKind?: "journal" | "page";
}): JSX.Element {
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
                openBlockInSidebar({
                  uuid: props.block.id,
                  page: props.page ?? "",
                  pageKind: props.pageKind ?? "page",
                });
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
            <RefBlocks blocks={props.block.children} page={props.page} pageKind={props.pageKind} />
          </div>
        </div>
      </Show>
    </div>
  );
}
