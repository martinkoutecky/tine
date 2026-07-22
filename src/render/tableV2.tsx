import { For, Show, type JSX } from "solid-js";
import type { Block, Inline } from "./ast";
import { propertyKeyNorm } from "./block";
import type { SpanDomAttrs } from "./spans";

type TableBlock = Extract<Block, { kind: "table" }>;

export type TableHeaderMode = "uppercase" | "capitalize" | "lowercase" | "none" | "capitalize-first";

export interface TableV2Options {
  version: number;
  compact: boolean;
  headers: TableHeaderMode;
}

const DEFAULT_OPTIONS: TableV2Options = { version: 1, compact: false, headers: "none" };
const HEADER_MODES = new Set<TableHeaderMode>(["uppercase", "capitalize", "lowercase", "none", "capitalize-first"]);

/**
 * Reads the `logseq.table.*` values from the one parsed property AST. The OG
 * view reads block properties before config
 * (`og/deps/shui/src/logseq/shui/table/v2.cljs:37-50`), while its version
 * dispatcher reads block then graph config then v1
 * (`og/src/main/frontend/shui.cljs:12-23`).
 * Tine currently has no graph-config payload in GraphMeta, so this deliberately
 * implements the available block-property layer and keeps v1 as the fallback.
 */
export function tableV2Options(properties: readonly [string, string][]): TableV2Options {
  const values = new Map<string, string>();
  for (const [key, value] of properties) values.set(propertyKeyNorm(key), value.trim());

  const version = Number.parseFloat(values.get("logseq.table.version") ?? "1");
  const headerValue = (values.get("logseq.table.headers") ?? "none").toLowerCase();
  const headers: TableHeaderMode = HEADER_MODES.has(headerValue as TableHeaderMode)
    ? headerValue as TableHeaderMode
    : "none";
  return {
    // OG uses `js/parseFloat`; non-2 values stay on the existing v1 renderer.
    version: Number.isFinite(version) ? version : DEFAULT_OPTIONS.version,
    compact: (values.get("logseq.table.compact") ?? "false").toLowerCase() === "true",
    headers,
  };
}

function headerTransform(mode: TableHeaderMode): string | undefined {
  switch (mode) {
    case "uppercase":
    case "capitalize":
    case "lowercase":
      return mode;
    case "capitalize-first":
      // OG combines its custom `capitalize-first` class with lowercase
      // (`og/deps/shui/src/logseq/shui/table/v2.cljs:166-169`); keep both
      // semantic classes for stylesheet parity and use lowercase as the
      // portable base in this render-only grid.
      return "lowercase";
    default:
      return undefined;
  }
}

/**
 * The deliberately non-editable v2 presentation. OG's v2 root is a grid with
 * `data-testid="v2-table-container"`
 * (`og/deps/shui/src/logseq/shui/table/v2.cljs:364-372`); its cells use compact
 * padding and header text transforms
 * (`og/deps/shui/src/logseq/shui/table/v2.cljs:166-189`). Width measurement,
 * colours, hover, and borders remain out of scope here.
 */
export function TableV2(props: {
  table: TableBlock;
  options: TableV2Options;
  renderCell: (cell: Inline[]) => JSX.Element;
  spanAttrs?: SpanDomAttrs;
}): JSX.Element {
  const columnCount = Math.max(1, props.table.header?.length ?? 0, ...props.table.rows.map((row) => row.length));
  const alignment = (index: number) => props.table.aligns[index] ?? undefined;
  const cellStyle = (index: number, header: boolean): JSX.CSSProperties => ({
    "text-align": alignment(index),
    "padding": props.options.compact
      ? "0.125rem 0.25rem"
      : header ? "0.375rem 0.75rem" : "0.5rem 0.75rem",
    "text-transform": header ? headerTransform(props.options.headers) : undefined,
  });
  const cellClass = (header: boolean) => [
    "v2-table-cell",
    header && "v2-table-header",
    props.options.compact && "v2-table-compact",
    header && `v2-table-headers-${props.options.headers}`,
  ].filter(Boolean).join(" ");

  return (
    <div
      class="v2-table-container"
      data-testid="v2-table-container"
      role="grid"
      style={{ "display": "grid", "grid-template-columns": `repeat(${columnCount}, minmax(0, 1fr))` }}
      {...(props.spanAttrs ?? {})}
    >
      <Show when={props.table.header}>
        <For each={props.table.header!}>
          {(cell, index) => (
            <div class={cellClass(true)} role="columnheader" style={cellStyle(index(), true)}>
              {props.renderCell(cell)}
            </div>
          )}
        </For>
      </Show>
      <For each={props.table.rows}>
        {(row) => (
          <For each={row}>
            {(cell, index) => (
              <div class={cellClass(false)} role="gridcell" style={cellStyle(index(), false)}>
                {props.renderCell(cell)}
              </div>
            )}
          </For>
        )}
      </For>
    </div>
  );
}
