import { TableWrap } from "./TableWrap";
import { For, Match, Show, Switch, createMemo, type JSX } from "solid-js";
import { pageRowFieldValue, type PageRow, type QueryStatistics } from "../editor/queryIr";
import { openPageTarget, openPageTargetInNewTab } from "../router";
import { openRouteInOtherPane } from "../panes";
import { internalLinkDest } from "../linkGesture";
import { openPageInSidebar } from "../ui";
import { fieldLabel, isFieldId } from "../sheet/fields";
import { querySummary } from "../editor/queryAggregate";
import { openPagePropertiesFromRow } from "../queryPageProps";

// Presentation parts of a query block's answer: page rows and the engine's
// statistics. Neither decides membership or computes an answer (I-12).

export type QueryView = "search" | "list" | "table" | "board";

/** A page-anchored answer (K16): pages, not degenerate empty block groups.
 *  Table columns come off the rows' own page properties via `query_run`, never
 *  a name-only row. */
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
  // GH #619 item 8: OG lists a page result with its page properties, as plain selectable
  // text; the pencil edits them. The row only carries the query's answer, so the pencil first
  // loads the page (a read) and then opens the existing properties panel, whose write is the
  // guarded `setPageProperty` path (see ../queryPageProps).
  const propertyStrip = (row: PageRow) => (
    <>
      <Show when={row.properties.length > 0}>
        <span class="query-page-props" data-selectable="text">
          <For each={row.properties}>{([key, val]) => (
            <span class="query-page-prop"><span class="query-page-prop-key">{key}:</span> {val}</span>
          )}</For>
        </span>
      </Show>
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
    </>
  );
  return (
    <Switch
      fallback={
        <ul class="query-results-list" aria-label="Page results">
          <For each={props.rows}>{(row) => <li>{link(row)}{propertyStrip(row)}</li>}</For>
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
 *  answerer (I-12) instead of a frontend fold over the returned rows. Markup and
 *  wording are master's summary panel (Macro.tsx, `querySummary`). An absent
 *  answer is never a numeric zero. */
export function QueryStatisticsSummary(props: { statistics: QueryStatistics }): JSX.Element {
  const stop = (e: MouseEvent) => e.stopPropagation();
  const summary = createMemo(() => querySummary({ statistics: props.statistics })!);
  const groupLabel = () => {
    const field = props.statistics.group_by;
    return field && isFieldId(field) ? fieldLabel(field) : field;
  };
  return (
    <>
      <Show when={summary().notice}>
        <p class="query-summary-note">{summary().notice}</p>
      </Show>
      <Show
        when={summary().groups}
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
