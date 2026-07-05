// Read-only rendering of a BlockDto tree. Shared by Linked References, query
// results, and embeds. Mirrors the rendered (non-editing) block look.

import { For, Show, createMemo, type JSX } from "solid-js";
import type { BlockDto } from "../types";
import { visibleBody } from "../render/block";
import { facetsFromDto } from "../render/facets";
import { taskCheckboxState } from "../markers";
import { InlineText } from "../render/inline";
import { formatForPage } from "../store";
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
  // Header facts (marker/done) off the one lsdoc parse (cache hit if the panel's
  // DTOs were seeded); the visible body lines via the shared body-text extractor.
  // Memoized — read several times in the markup, and the ref panels render hundreds
  // of rows.
  // Off the DTO's shipped facet fields — NO frontend parse (the backend already
  // computed these). Ref/linked/unlinked panels render hundreds of rows; parsing each
  // was a real hot-path cost and churned the facet cache (audit P3).
  const facets = createMemo(() => facetsFromDto(props.block));
  const lines = createMemo(() => visibleBody(props.block.raw));
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
          <div class="block-content" classList={{ done: facets().done }}>
            <Show when={taskCheckboxState(facets().marker) !== null}>
              {/* Read-only here (references/embeds/query results aren't edited in
                  place); it mirrors the live block's checkbox look. */}
              <span
                class="block-task-checkbox"
                classList={{ checked: taskCheckboxState(facets().marker) === true }}
                role="checkbox"
                aria-checked={taskCheckboxState(facets().marker) === true}
              />{" "}
            </Show>
            <Show when={facets().marker}>
              <span class={`block-marker marker-${facets().marker?.toLowerCase()}`}>
                {facets().marker}
              </span>{" "}
            </Show>
            <Show when={facets().priority}>
              <span class={`block-priority priority-${facets().priority}`}>[#{facets().priority}]</span>{" "}
            </Show>
            <For each={lines()}>
              {(line, i) => (
                <>
                  <Show when={i() > 0}>
                    <br />
                  </Show>
                  <InlineText text={line} format={formatForPage(props.page)} />
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
