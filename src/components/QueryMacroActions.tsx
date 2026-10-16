import type { JSX } from "solid-js";
import { backend } from "../backend";
import { graphOwner, readOwned } from "../owned";
import { openInNewTab, makeQueryRoute } from "../router";
import { sourcePrintDialect, type ParsedQuery } from "../editor/queryIr";
import type { PageEntry, RefGroup } from "../types";
import { pushToastUnique } from "../toasts";
import { t } from "../i18n";
import { QueryActions } from "./QueryActions";
import { openQueryResultsCopy } from "./queryResultsCopy";
import { queryResultsHaveCollapsed } from "./QueryCollapse";

/** Graph-owned macro actions. Editing, export and folding enter the host's
 * existing guarded paths; this component introduces no writer. */
export function QueryMacroActions(props: {
  current: () => ParsedQuery | undefined;
  groups: () => RefGroup[];
  pages: () => Pick<PageEntry, "name">[];
  editable: boolean;
  results: boolean;
  edit: (tab: "filter" | "display" | "text") => void;
  fold?: (collapsed: boolean) => void;
  export: () => void;
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
        openInNewTab({ ...makeQueryRoute(`{{${macro} ${source.value}}}`, presentation ?? "search", "dsl"), blockDisplay: display, pageDisplay: display }, true);
      },
      error => { if (owner()) pushToastUnique(t("query-calm-search-failed", { error: error instanceof Error ? error.message : String(error) }), "error"); },
    );
  };
  return <QueryActions editable={props.editable} results={props.results} edit={props.edit} fold={props.fold}
    folded={queryResultsHaveCollapsed(props.groups())}
    copy={() => { void openQueryResultsCopy(props.groups(), props.pages()); }} search={search} export={props.export} />;
}
