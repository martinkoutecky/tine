import { TableWrap } from "./TableWrap";
import { t, msg, render } from "../i18n";
import { Rich } from "../i18n/Rich";
import { For, Match, Show, Switch, createMemo, type JSX } from "solid-js";
import { pageRowFieldValue, type OgQueryHint, type PageRow, type Query, type QueryStatistics, type ViewKind } from "../editor/queryIr";
import { openPageTarget, openPageTargetInNewTab } from "../router";
import { openRouteInOtherPane } from "../panes";
import { internalLinkDest } from "../linkGesture";
import { openPageInSidebar } from "../ui";
import { fieldLabel, isFieldId } from "../sheet/fields";
import { querySummary } from "../editor/queryAggregate";
import { openPagePropertiesFromRow } from "../queryPageProps";

// Presentation parts of a query block's answer: page rows, the engine's
// statistics and the engine's Logseq-form hint. None decides membership or
// computes an answer (I-12).

/** GH #726: the results draw OTHER blocks; a right click in here never opens the
 *  host block's menu (contextMenuPolicy reads `data-block-surface`). A real box,
 *  not `display: contents`: TableWrap measures its parent, and a contents box has
 *  no geometry, which pushed wide result tables off the pane (GH #749). */
export function QueryResultsRegion(props: { children: JSX.Element }): JSX.Element {
  return <div class="query-results-region" data-block-surface="query">{props.children}</div>;
}

export type QueryView = ViewKind;

/** A page-anchored answer (K16): pages, not degenerate empty block groups.
 *  Table columns come off the rows' own page properties via `query_run`, never
 *  a name-only row. Search/List show titles and a one-click properties editor;
 *  columns only affect Table. Cost O(returned rows and displayed cells). */
export function QueryPageRows(props: { rows: PageRow[]; view: QueryView; groupBy?: string; columns?: string[] }): JSX.Element {
  const target = (row: PageRow) => ({ name: row.name, pageKind: row.kind, path: row.path });
  const open = (row: PageRow, event: MouseEvent) => {
    event.stopPropagation();
    const dest = internalLinkDest(event);
    if (dest === "sidebar") openPageInSidebar(target(row));
    else if (dest === "background") openPageTargetInNewTab(target(row));
    else if (dest === "pane") openRouteInOtherPane({ kind: "page", ...target(row) });
    else openPageTarget(target(row));
  };
  const fieldName = (field: string) => field.replace(/^prop:/, "");
  const value = (row: PageRow, field: string): string => pageRowFieldValue(row, row, field);
  const columns = createMemo(() => props.columns !== undefined
    ? props.columns
    : [...new Set(props.rows.flatMap((row) => row.properties.map(([key]) => key)))]);
  const board = createMemo(() => {
    const out: [string, PageRow[]][] = [];
    for (const row of props.rows) {
      const key = props.groupBy ? value(row, props.groupBy) : "";
      const last = out[out.length - 1];
      if (last && last[0] === key) last[1].push(row);
      else out.push([key, [row]]);
    }
    return out;
  });
  const link = (row: PageRow) => (
    <button
      type="button"
      class="query-page-link"
      data-page-path={row.path}
      data-page-kind={row.kind}
      onClick={(event) => open(row, event)}
    >
      {row.name}
    </button>
  );
  // Search/List use master's title-only page rows; the existing pencil keeps
  // the full properties one click away through the guarded page editor.
  const propertyEditor = (row: PageRow) => (
    <button
      type="button"
      class="query-page-props-edit"
      aria-label={t("query-query-result-parts-edit-properties-of", { value1: row.name })}
      title={t("query-query-result-parts-edit-page-properties")}
      onClick={(event) => {
        event.stopPropagation();
        const rect = event.currentTarget.getBoundingClientRect();
        void openPagePropertiesFromRow(row, rect.left, rect.bottom + 4);
      }}
    >✎</button>
  );
  return (
    <Switch
      fallback={
        <ul class="query-results-list" aria-label={t("query-query-result-parts-page-results")}>
          <For each={props.rows}>{(row) => <li>{link(row)}{propertyEditor(row)}</li>}</For>
        </ul>
      }
    >
      <Match when={props.view === "table"}>
        <TableWrap><table class="md-table query-table query-page-table">
          <thead>
            <tr>
              <th>{t("query-query-workspace-page")}</th>
              <For each={columns()}>{(column) => <th>{fieldName(column)}</th>}</For>
            </tr>
          </thead>
          <tbody>
            <For each={props.rows}>{(row) => (
              <tr>
                <td>{link(row)}</td>
                <For each={columns()}>{(column) => <td>{value(row, column)}</td>}</For>
              </tr>
            )}</For>
          </tbody>
        </table></TableWrap>
      </Match>
      <Match when={props.view === "board"}>
        <div class="query-results-board" aria-label={t("query-query-result-parts-page-results-grouped")}>
          <For each={board()}>{([key, rows]) => (
            <section class="query-board-column" aria-label={key || t("query-query-result-parts-no-value")}>
              <h4>{key || t("query-query-result-parts-no-value")}<span class="query-board-count">{rows.length}</span></h4>
              <For each={rows}>{(row) => <div class="query-board-card">{link(row)}</div>}</For>
            </section>
          )}</For>
        </div>
      </Match>
    </Switch>
  );
}

/** The engine's statistics fold (`query_run` `statistics`), rendered — one
 *  answerer (I-12) instead of a frontend fold over the returned rows. An absent
 *  answer is never a numeric zero. Grouping is named; a sole missing-value
 *  group shows overall totals instead of a table. O(returned statistic cells);
 *  the host supplies a clear action only on writable surfaces. */
export function QueryStatisticsSummary(props: { statistics: QueryStatistics; onClearGrouping?: () => void }): JSX.Element {
  const stop = (e: MouseEvent) => e.stopPropagation();
  const summary = createMemo(() => querySummary({ statistics: props.statistics })!);
  const groupLabel = () => {
    const field = props.statistics.group_by;
    return field && isFieldId(field) ? fieldLabel(field) : field;
  };
  const breakdown = createMemo(() => {
    const groups = summary().groups;
    return groups?.length && !(groups.length === 1 && groups[0].key === null) ? groups : null;
  });
  return (
    <>
      <Show when={props.statistics.group_by}>
        <div class="query-summary" onClick={stop}>
          <span class="qs-label">{t("query-grouped-by", { field: groupLabel() ?? "" })}</span>
          <Show when={props.onClearGrouping}>
            <button type="button" class="query-grouping-clear" aria-label={t("query-query-result-parts-clear-grouping")} onClick={() => props.onClearGrouping?.()}>{t("query-query-sheet-parts-clear")}</button>
          </Show>
        </div>
      </Show>
      <Show when={summary().notice}>
        <p class="query-summary-note">{summary().notice}</p>
      </Show>
      <Show
        when={breakdown()}
        fallback={
          <div class="query-summary" onClick={stop}>
            <For each={summary().columns}>{(column, i) => (
              <span class="qs-entry">
                <span class="qs-label">{column.label}:</span>{" "}
                <span class="qs-value">{summary().overall[i()]?.text}</span>
                <Show when={(summary().overall[i()]?.skipped ?? 0) > 0}>
                  <span class="qs-skip">{t("query-skipped-non-numeric", { count: summary().overall[i()]!.skipped })}</span>
                </Show>
              </span>
            )}</For>
          </div>
        }
      >
        {(groups) => (
          <>
            <TableWrap><table class="md-table query-summary-table" onClick={stop}>
              <thead>
                <tr>
                  <th>{groupLabel()}</th>
                  <For each={summary().columns}>{(column) => <th>{column.label}</th>}</For>
                </tr>
              </thead>
              <tbody>
                <For each={groups()}>{(group) => (
                  <tr>
                    <td>{group.label}</td>
                    <For each={group.cells}>{(cell) => (
                      <td>
                        {cell.text}
                        <Show when={cell.skipped > 0}><span class="qs-skip">{t("query-skipped", { count: cell.skipped })}</span></Show>
                      </td>
                    )}</For>
                  </tr>
                )}</For>
              </tbody>
            </table></TableWrap>
            <Show when={summary().multiMembership}>
              <p class="query-summary-note" onClick={stop}>
                {t("query-query-result-parts-a-row-with-several-tags-appears-in-every-matching-group")}</p>
            </Show>
          </>
        )}
      </Show>
    </>
  );
}

const OG_REWRITE_LABEL = {
  open_tasks: msg("query-prose-open-tasks"),
  any_task: msg("query-prose-any-task"),
  priorities: msg("query-prose-a-b-or-c"),
};

/** **The on-query cue for Logseq's "no condition" forms (D-18, GH #422).** A
 *  bare `(task)` / `(priority)` adds no condition, and a query with no
 *  condition shows nothing — in Logseq, and so in Tine. The engine detects it
 *  and supplies each rewrite as a whole query (`wire_parse.rs` `OgQueryHint`);
 *  this only says so and hands the chosen query to the host's ordinary save.
 *  It sits beside the results and never replaces them. A host that cannot
 *  write (read-only page, published export) passes no `onRewrite`, and the
 *  buttons are not offered. Styled as the crossing notice's inline region. */
export function QueryOgHint(props: { hint: OgQueryHint; onRewrite?: (query: Query) => void }): JSX.Element {
  const has = (head: "task" | "priority") => props.hint.bare.includes(head);
  return (
    <div class="query-og-hint" role="note" onClick={(e) => e.stopPropagation()}>
      <p class="query-og-hint-text">
        <Show when={props.hint.date_windows}>{t("query-date-window-refusal")}</Show>
        <Show when={has("task")}><Rich message={msg("query-og-bare-task", { syntax: "(task)" })} /></Show>
        <Show when={has("priority")}><Rich message={msg("query-og-bare-priority", { syntax: "(priority)" })} /></Show>
        <Show
          when={props.hint.bare.length > 0}
          fallback={<Show when={props.hint.no_conditions}>{t("query-query-result-parts-this-query-has-no-conditions-so-it-shows-nothing-as")}</Show>}
        >
          <Show when={props.hint.no_conditions}>{t("query-query-result-parts-on-its-own-it-matches-nothing")}</Show>
        </Show>
      </p>
      <Show when={props.onRewrite && props.hint.rewrites.length > 0}>
        <div class="query-og-hint-actions">
          <For each={props.hint.rewrites}>
            {(rewrite) => (
              <button
                type="button"
                class="query-og-hint-rewrite"
                data-rewrite={rewrite.rewrite}
                onClick={(e) => {
                  e.stopPropagation();
                  props.onRewrite?.(rewrite.query);
                }}
              >
                {rewrite.rewrite === "convert" ? t("query-convert") : render(OG_REWRITE_LABEL[rewrite.rewrite])}
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
