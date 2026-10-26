import { Show, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphOwner, readOwned } from "../owned";
import { openInNewTab, makeQueryRoute } from "../router";
import { sourcePrintDialect, type ParsedQuery } from "../editor/queryIr";
import type { PageEntry, RefGroup, QueryPublicationRequest } from "../types";
import { openQueryExport } from "../ui";
import { pushToastUnique } from "../toasts";
import { t } from "../i18n";
import { QueryActions } from "./QueryActions";
import { ShareIcon } from "./ShareIcon";
import { openQueryResultsCopy } from "./queryResultsCopy";
import { queryResultsHaveCollapsed } from "./QueryCollapse";
import { QuickShareExportActions } from "./QuickShareExportActions";

/** Graph-owned macro actions. Editing, export and folding enter the host's
 * existing guarded paths; this component introduces no writer. */
export function QueryMacroActions(props: {
  hostBlockId?: string;
  current: () => ParsedQuery | undefined;
  groups: () => RefGroup[];
  pages: () => Pick<PageEntry, "name">[];
  editable: boolean;
  results: boolean;
  edit: (tab: "filter" | "display" | "text") => void;
  fold?: (collapsed: boolean) => void;
  export: () => void;
  exportRequest?: () => QueryPublicationRequest;
  /** The block asks for "Pages and blocks" (`tine.result-kinds`); Search then shows both families. */
  both?: () => boolean;
  /** Open the share dialog seeded with this block (enabled with zero results: the dialog shows 0). */
  share: () => void;
}): JSX.Element {
  const search = () => {
    const current = props.current();
    if (!current) return;
    const owner = graphOwner();
    void readOwned(owner, backend().printQuery(current.query, current.view, sourcePrintDialect(current.query.source))).then(
      source => {
        if (source.kind !== "current") return;
        const { view: presentation, ...display } = current.view;
        const macro = current.query.source.kind === "tql" ? "tine-query" : "query";
        // GH #619 c6 (Q2): the query's own view travels. The block's default is the list, not Search; the
        // scoped Pages/Blocks views and displays and the "Pages and blocks" choice go with it.
        const view = presentation ?? (current.legacy_table ? "table" : "list");
        openInNewTab({
          ...makeQueryRoute(`{{${macro} ${source.value}}}`, view, "dsl"),
          ...(current.page_presentation ? { pagePresentation: current.page_presentation } : {}),
          ...(current.block_presentation ? { blockPresentation: current.block_presentation } : {}),
          pageDisplay: current.page_display ?? display,
          blockDisplay: current.block_display ?? display,
          ...(props.both?.() ? { resultKinds: "pages-and-blocks" as const } : {}),
        }, true);
      },
      error => { if (owner()) pushToastUnique(t("query-calm-search-failed", { error: error instanceof Error ? error.message : String(error) }), "error"); },
    );
  };
  return <>
    <Show when={props.hostBlockId}>{id => <QuickShareExportActions hostBlockId={id()} reshare={props.share}
      reexport={folder => { const request = props.exportRequest?.(); if (request) openQueryExport({ ...request, folder, replace: true }); }} />}</Show>
    <button type="button" class="qc-share" title={t("share-query-icon")} aria-label={t("share-query-icon")}
      onMouseDown={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); props.share(); }}>
      <ShareIcon class="share-icon" />
    </button>
    {/* GH #619 c6 (D-20261018-Q619): the most frequent action, one click beside the menu. */}
    <Show when={props.editable}>
      <button type="button" class="qc-edit" title={t("query-calm-edit-query")} aria-label={t("query-calm-edit-query")}
        onMouseDown={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); props.edit("filter"); }}>
        <span aria-hidden="true">{"\u2699\uFE0E"}</span>
      </button>
    </Show>
    <QueryActions editable={props.editable} results={props.results} edit={props.edit} fold={props.fold}
      folded={queryResultsHaveCollapsed(props.groups())}
      copy={() => { void openQueryResultsCopy(props.groups(), props.pages()); }} search={search} export={props.export} share={props.share} />
  </>;
}
