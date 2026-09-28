import { For, Match, Show, Switch, createMemo, type JSX } from "solid-js";
import type { PageRow, QueryStatistics, QueryStatisticsCell } from "../editor/queryIr";
import { openPageTarget } from "../router";
import { openPageInSidebar } from "../ui";
import { fieldLabel, isFieldId } from "../sheet/fields";

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
    if (event.shiftKey) openPageInSidebar(target(row));
    else openPageTarget(target(row));
  };
  const fieldName = (field: string) => field.replace(/^prop:/, "");
  const value = (row: PageRow, field: string): string => {
    const name = fieldName(field);
    if (name === "name") return row.name;
    if (name === "kind") return row.kind === "journal" ? "Journal" : "Page";
    if (name === "day" || name === "journal-day") return row.journal_day != null ? String(row.journal_day) : "";
    const key = name.trim().toLowerCase();
    return row.properties.find(([property]) => property.trim().toLowerCase() === key)?.[1] ?? "";
  };
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
  return (
    <Switch
      fallback={
        <ul class="query-results-list" aria-label="Page results">
          <For each={props.rows}>{(row) => <li>{link(row)}</li>}</For>
        </ul>
      }
    >
      <Match when={props.view === "table"}>
        <table class="md-table query-table query-page-table">
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
        </table>
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
  const label = ([field, fn]: [string, string]) => {
    const verb = fn === "count" ? "Count" : fn === "sum" ? "Sum" : "Avg";
    return field ? `${verb} of ${field.replace(/^prop:/, "")}` : verb;
  };
  const text = (cell: QueryStatisticsCell | undefined) => {
    if (!cell) return "";
    if (cell.kind === "marker") return `Unavailable (${cell.reason.replaceAll("_", " ")})`;
    const scaled = cell.value * 1000;
    return `${Number.isFinite(scaled) ? Math.round(scaled) / 1000 : cell.value}`;
  };
  const groupLabel = () => {
    const field = props.statistics.group_by;
    return field && isFieldId(field) ? fieldLabel(field) : field;
  };
  return (
    <>
      <Show when={props.statistics.grouping_status === "unsupported_formula"}>
        <p class="query-summary-note">Exact statistics by formula are not supported yet. Overall statistics are shown.</p>
      </Show>
      <Show
        when={props.statistics.groups}
        fallback={
          <div class="query-summary" onClick={stop}>
            <For each={props.statistics.aggregates}>{(aggregate, i) => (
              <span class="qs-entry">
                <span class="qs-label">{label(aggregate)}:</span>{" "}
                <span class="qs-value">{text(props.statistics.overall[i()])}</span>
                <Show when={(props.statistics.overall[i()]?.skipped ?? 0) > 0}>
                  <span class="qs-skip"> ({props.statistics.overall[i()]!.skipped} non-numeric skipped)</span>
                </Show>
              </span>
            )}</For>
          </div>
        }
      >
        {(groups) => (
          <>
            <table class="md-table query-summary-table" onClick={stop}>
              <thead>
                <tr>
                  <th>{groupLabel()}</th>
                  <For each={props.statistics.aggregates}>{(aggregate) => <th>{label(aggregate)}</th>}</For>
                </tr>
              </thead>
              <tbody>
                <For each={groups()}>{(group) => (
                  <tr>
                    <td>{group.key ?? "(none)"}</td>
                    <For each={group.cells}>{(cell) => (
                      <td>
                        {text(cell)}
                        <Show when={cell.skipped > 0}><span class="qs-skip"> ({cell.skipped} skipped)</span></Show>
                      </td>
                    )}</For>
                  </tr>
                )}</For>
              </tbody>
            </table>
            <Show when={props.statistics.group_by === "tags"}>
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
