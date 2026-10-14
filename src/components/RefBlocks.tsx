import { t } from "../i18n";
// Rendering of a BlockDto tree (read-only text; a task checkbox ticks the source block, GH #518). Shared by Linked References, query
// results, and embeds. Mirrors the rendered (non-editing) block look.
// Visits at most 64 ancestor levels per branch and shows a message below that;
// cost is O(rendered blocks), bounded by the supplied result tree.

import { For, Show, createMemo, type JSX } from "solid-js";
import type { BlockDto } from "../types";
import { pageProperties, visibleBody } from "../render/block";
import { effectiveHeadingLevel, facetsFromDto } from "../render/facets";
import { taskCheckboxState } from "../markers";
import { InlineText } from "../render/inline";
import { formatForPage } from "../document";
import { openBlockInSidebar } from "../ui";
import { openInNewTab, type Route } from "../router";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { PagePropertyValue } from "./PagePropertyValue";
import { BeginQuery, inspectBeginQuery } from "./BeginQuery";
import { blockDtoExternalId } from "../blockIdentity";
import { SourceTaskCheckbox } from "./SourceTaskCheckbox";
import { isElementNode } from "../windowRealm";

const MAX_REF_BLOCK_DEPTH = 64;

// `page`/`pageKind` (where these blocks live) are threaded through so a
// shift-click can open the block live in the sidebar.
export function RefBlocks(props: {
  blocks: BlockDto[];
  page?: string;
  pageKind?: "journal" | "page";
  /** Exact source file, when the caller knows it (a reference group does). */
  path?: string;
  depth?: number;
  /** GH #486: a click on a page-property row edits that page's properties
   *  (the caller owns the source page's edit path). Absent: read-only row. */
  onEditPageProperties?: (event: MouseEvent) => void;
  queryResult?: boolean;
  readOnly?: boolean;
  onOpenBlock?: (id: string) => void;
}): JSX.Element {
  return <RefBlocksAtDepth {...props} treeDepth={1} />;
}

function RefBlocksAtDepth(props: {
  blocks: BlockDto[];
  page?: string;
  pageKind?: "journal" | "page";
  path?: string;
  depth?: number;
  onEditPageProperties?: (event: MouseEvent) => void;
  readOnly?: boolean;
  onOpenBlock?: (id: string) => void;
  treeDepth: number;
  queryResult?: boolean;
}): JSX.Element {
  return (
    <For each={props.blocks}>
      {(b) => (
        <RefBlock
          block={b}
          queryResult={props.queryResult}
          page={props.page}
          pageKind={props.pageKind}
          path={props.path}
          onEditPageProperties={props.onEditPageProperties}
          readOnly={props.readOnly}
          onOpenBlock={props.onOpenBlock}
          depth={props.depth ?? b.breadcrumb?.length ?? 0}
          treeDepth={props.treeDepth}
        />
      )}
    </For>
  );
}

function RefBlock(props: {
  block: BlockDto;
  page?: string;
  pageKind?: "journal" | "page";
  path?: string;
  onEditPageProperties?: (event: MouseEvent) => void;
  readOnly?: boolean;
  onOpenBlock?: (id: string) => void;
  depth: number;
  treeDepth: number;
  queryResult?: boolean;
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
    !props.readOnly && /^\s*#\+BEGIN_QUERY\b/i.test(props.block.raw)
      ? inspectBeginQuery(props.block.raw, format())
      : null
  );
  const properties = createMemo(() =>
    props.block.page_property ? pageProperties(props.block.raw, format()) : []
  );
  const refTarget = () => ({
    uuid: blockDtoExternalId(props.block),
    page: props.page ?? "",
    pageKind: (props.pageKind ?? "page") as "journal" | "page",
    ...(props.path ? { path: props.path } : {}),
  });
  const openRefRoute = (open: (route: Route) => unknown): void => {
    const t = refTarget();
    open({ kind: "page", name: t.page, pageKind: t.pageKind, block: t.uuid });
  };
  return (
    <div
      class="ls-block ref-block"
      data-block-id={props.block.id}
      data-block-ref={blockDtoExternalId(props.block)}
      classList={{ "page-property-reference": !!props.block.page_property }}
    >
      <div class="block-main" classList={{ [`bullet-h${headingLevel()}`]: headingLevel() != null }}>
        <div class="block-controls">
          <span
            class="bullet-container"
            title={props.block.page_property ? t("macro-ref-blocks-page-property-reference") : t("macro-ref-blocks-shift-click-sidebar-ctrl-cmd-click-or-middle-click-new")}
            onMouseDown={(e) => {
              // GH #207's other half: suppress shift-range selection and
              // middle-button autoscroll / PRIMARY-paste for the gestures this
              // bullet answers.
              if (!props.block.page_property) internalLinkMouseDown(e);
            }}
            onClick={(e) => {
              if (props.block.page_property) return;
              // Sibling of the live outline's bullet (GH #456): the same one
              // decision, same destinations, minus the in-place zoom, which a
              // read-only reference has no equivalent of, so a plain click keeps
              // doing nothing.
              const dest = internalLinkDest(e);
              if (dest === "default") {
                if (props.onOpenBlock) { e.stopPropagation(); props.onOpenBlock(blockDtoExternalId(props.block)); }
                return;
              }
              e.stopPropagation();
              if (dest === "sidebar") openBlockInSidebar(refTarget());
              else openRefRoute(dest === "pane" ? openRouteInOtherPane : openInNewTab);
            }}
            onAuxClick={(e) => {
              if (props.block.page_property) return;
              if (internalLinkAuxClick(e, () => openRefRoute(openInNewTab))) e.stopPropagation();
            }}
          >
            <span class="bullet" />
          </span>
        </div>
        <div class="block-content-wrapper">
          <div
            class="block-content"
            classList={{ done: facets().done, [`heading h${headingLevel() ?? ""}`]: headingLevel() != null }}
            onClick={(event) => {
              // GH #486: the same gesture as the source page's properties area
              // (Page.tsx editPageHeader); links and buttons keep their own click.
              if (!props.block.page_property || !props.onEditPageProperties) return;
              if (isElementNode(event.target) && event.target.closest("a, button, .page-ref, .tag")) return;
              event.stopPropagation();
              props.onEditPageProperties(event);
            }}
          >
            <Show when={!props.readOnly && taskCheckboxState(facets().marker) !== null && !props.block.page_property}>
              {/* GH #518: ticks the SOURCE block through its own checkbox path
                  (OG block-checkbox on a referenced block). */}
              <SourceTaskCheckbox marker={facets().marker} target={refTarget}
                result={props.queryResult ? { id: props.block.id, dto: props.block, page: props.page ?? "", kind: props.pageKind, path: props.path } : undefined} />{" "}
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
      <Show when={props.block.children.length && props.treeDepth < MAX_REF_BLOCK_DEPTH}>
        <div class="block-children-container">
          <div class="block-children">
            <RefBlocksAtDepth blocks={props.block.children} page={props.page} pageKind={props.pageKind} path={props.path}
              queryResult={props.queryResult}
              onEditPageProperties={props.onEditPageProperties}
              readOnly={props.readOnly} onOpenBlock={props.onOpenBlock}
              depth={props.depth + 1} treeDepth={props.treeDepth + 1} />
          </div>
        </div>
      </Show>
      <Show when={props.block.children.length && props.treeDepth >= MAX_REF_BLOCK_DEPTH}>
        <div class="ref-block-depth-warning">{t("macro-ref-blocks-more-nested-blocks-are-available-on-the-page")}</div>
      </Show>
    </div>
  );
}
