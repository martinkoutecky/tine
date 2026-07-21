// Read-only rendering of a BlockDto tree. Shared by Linked References, query
// results, and embeds. Mirrors the rendered (non-editing) block look.

import { For, Show, createMemo, type JSX } from "solid-js";
import type { BlockDto } from "../types";
import { pageProperties, visibleBody } from "../render/block";
import { effectiveHeadingLevel, facetsFromDto } from "../render/facets";
import { taskCheckboxState } from "../markers";
import { InlineText } from "../render/inline";
import { formatForPage } from "../store";
import { openBlockInSidebar } from "../ui";
import { PagePropertyValue } from "./PagePropertyValue";
import { BeginQuery, inspectBeginQuery } from "./BeginQuery";

// `page`/`pageKind` (where these blocks live) are threaded through so a
// shift-click can open the block live in the sidebar.
export function RefBlocks(props: {
  blocks: BlockDto[];
  page?: string;
  pageKind?: "journal" | "page";
  depth?: number;
}): JSX.Element {
  return (
    <For each={props.blocks}>
      {(b) => (
        <RefBlock
          block={b}
          page={props.page}
          pageKind={props.pageKind}
          depth={props.depth ?? b.breadcrumb?.length ?? 0}
        />
      )}
    </For>
  );
}

function RefBlock(props: {
  block: BlockDto;
  page?: string;
  pageKind?: "journal" | "page";
  depth: number;
}): JSX.Element {
  // Header facts (marker/done) off the one lsdoc parse (cache hit if the panel's
  // DTOs were seeded); the visible body lines via the shared body-text extractor.
  // Memoized — read several times in the markup, and the ref panels render hundreds
  // of rows.
  // Off the DTO's shipped facet fields — NO frontend parse (the backend already
  // computed these). Ref/linked/unlinked panels render hundreds of rows; parsing each
  // was a real hot-path cost and churned the facet cache (audit P3).
  const facets = createMemo(() => facetsFromDto(props.block));
  const headingLevel = createMemo(() => effectiveHeadingLevel(facets(), props.depth));
  const format = createMemo(() => formatForPage(props.page));
  const lines = createMemo(() => visibleBody(props.block.raw));
  // Keep the DTO hot path parse-free for ordinary references. Only an exact
  // whole-block candidate pays for lsdoc confirmation before execution.
  const beginQuery = createMemo(() =>
    /^\s*#\+BEGIN_QUERY\b/i.test(props.block.raw)
      ? inspectBeginQuery(props.block.raw, format())
      : null
  );
  const properties = createMemo(() =>
    props.block.page_property ? pageProperties(props.block.raw, format()) : []
  );
  return (
    <div class="ls-block ref-block" data-block-id={props.block.id} classList={{ "page-property-reference": !!props.block.page_property }}>
      <div class="block-main" classList={{ [`bullet-h${headingLevel()}`]: headingLevel() != null }}>
        <div class="block-controls">
          <span
            class="bullet-container"
            title={props.block.page_property ? "Page property reference" : "Shift-click to open in sidebar"}
            onClick={(e) => {
              if (!props.block.page_property && e.shiftKey) {
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
          <div
            class="block-content"
            classList={{ done: facets().done, [`heading h${headingLevel() ?? ""}`]: headingLevel() != null }}
          >
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
            <Show
              when={props.block.page_property}
              fallback={
                <Show
                  when={beginQuery()}
                  fallback={
                    <For each={lines()}>
                      {(line, i) => (
                        <>
                          <Show when={i() > 0}>
                            <br />
                          </Show>
                          <Show
                            when={i() === 0 && headingLevel() != null}
                            fallback={<InlineText text={line} format={format()} />}
                          >
                            <span class={`heading-text h${headingLevel()}`}>
                              <InlineText text={line} format={format()} />
                            </span>
                          </Show>
                        </>
                      )}
                    </For>
                  }
                >
                  {(match) => <BeginQuery match={match()} currentPage={props.page} />}
                </Show>
              }
            >
              <For each={properties()}>
                {([key, value], i) => (
                  <>
                    <Show when={i() > 0}>
                      <br />
                    </Show>
                    <span class="page-property-reference-row">
                      <span class="prop-key">{key}</span>{" "}
                      <span class="prop-value">
                        <PagePropertyValue propertyKey={key} value={value} format={format()} />
                      </span>
                    </span>
                  </>
                )}
              </For>
            </Show>
          </div>
        </div>
      </div>
      <Show when={props.block.children.length}>
        <div class="block-children-container">
          <div class="block-children">
            <RefBlocks blocks={props.block.children} page={props.page} pageKind={props.pageKind} depth={props.depth + 1} />
          </div>
        </div>
      </Show>
    </div>
  );
}
