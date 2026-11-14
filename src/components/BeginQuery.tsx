import { t } from "../i18n";
import { type JSX } from "solid-js";
import { QueryMacro } from "./Macro";
import type { QueryListOptions } from "../editor/queryListOptions";
import type { BeginQueryMatch } from "../render/beginQuery";

/** Forward authored presentation alongside the engine's child-row option.
 * Tine intentionally retains its existing table default; explicit false allows
 * OG's list options to apply (GH #881; query.cljs/custom-query-inner). */
function beginQueryOptions(match: { keeps_block_children?: boolean; listOptions?: QueryListOptions }): string {
  const options = match.listOptions;
  return `{${[
    `:table-view? ${options?.table !== false}`,
    ...(match.keeps_block_children ? [":remove-block-children? false"] : []),
    ...(options?.breadcrumb !== undefined ? [`:breadcrumb-show? ${options.breadcrumb}`] : []),
    ...(options?.groupByPage !== undefined ? [`:group-by-page? ${options.groupByPage}`] : []),
  ].join(" ")}}`;
}

/** Read-only BEGIN_QUERY presentation. OG presents the authored title and query
 * result table in its normal custom-query shell (og/src/main/frontend/components/query.cljs:184-247).
 * Tine reuses QueryMacro so execution retains the existing native result bounds. */
export function BeginQuery(props: { match: BeginQueryMatch; currentPage?: string; hideWhenEmpty?: boolean; findHost?: string }): JSX.Element {
  if (props.match.kind === "unsupported") {
    return (
      <div class="query-unsupported begin-query-unsupported" role="alert">{t("begin-query-unsupported", { reason: props.match.reason })}
      </div>
    );
  }
  return (
    <QueryMacro
      body={`query ${props.match.query} ${beginQueryOptions(props.match)}`}
      title={props.match.title}
      currentPage={props.currentPage}
      findHost={props.findHost}
      hideWhenEmpty={props.hideWhenEmpty}
      strictAdvanced
      unsupportedLabel="Unsupported BEGIN_QUERY"
    />
  );
}
