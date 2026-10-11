import { TableWrap } from "./TableWrap";
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
  const columns = createMemo(() => props.columns?.length
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
      aria-label={`Edit properties of ${row.name}`}
      title="Edit page properties"
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
        <ul class="query-results-list" aria-label="Page results">
          <For each={props.rows}>{(row) => <li>{link(row)}{propertyEditor(row)}</li>}</For>
        </ul>
      }
    >
      <Match when={props.view === "table"}>
        <TableWrap><table class="md-table query-table query-page-table">
          <thead>
            <tr>
              <th>Page</th>
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
        <div class="query-results-board" aria-label="Page results grouped">
          <For each={board()}>{([key, rows]) => (
            <section class="query-board-column" aria-label={key || "No value"}>
              <h4>{key || "No value"}<span class="query-board-count">{rows.length}</span></h4>
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
          <span class="qs-label">Grouped by {groupLabel()}</span>
          <Show when={props.onClearGrouping}>
            <button type="button" class="query-grouping-clear" aria-label="Clear grouping" onClick={() => props.onClearGrouping?.()}>Clear</button>
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
                  <span class="qs-skip"> ({summary().overall[i()]!.skipped} non-numeric skipped)</span>
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
                        <Show when={cell.skipped > 0}><span class="qs-skip"> ({cell.skipped} skipped)</span></Show>
                      </td>
                    )}</For>
                  </tr>
                )}</For>
              </tbody>
            </table></TableWrap>
            <Show when={summary().multiMembership}>
              <p class="query-summary-note" onClick={stop}>
                A row with several tags appears in every matching group, so these counts can add up to more than the result.
              </p>
            </Show>
          </>
        )}
      </Show>
    </>
  );
}

const OG_REWRITE_LABEL: Record<OgQueryHint["rewrites"][number]["rewrite"], string> = {
  open_tasks: "Open tasks",
  any_task: "Any task",
  priorities: "A, B or C",
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
        <Show when={has("task")}><code>(task)</code> without markers adds no condition, as in Logseq. </Show>
        <Show when={has("priority")}><code>(priority)</code> without levels adds no condition, as in Logseq. </Show>
        <Show
          when={props.hint.bare.length > 0}
          fallback={<Show when={props.hint.no_conditions}>This query has no conditions, so it shows nothing, as in Logseq.</Show>}
        >
          <Show when={props.hint.no_conditions}>On its own it matches nothing.</Show>
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
                {OG_REWRITE_LABEL[rewrite.rewrite]}
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
