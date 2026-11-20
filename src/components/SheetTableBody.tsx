import { Show, createMemo, type JSX } from "solid-js";
import { clearOnBindingInvalidated } from "../binding";
import { InlineText } from "../render/inline";
import { AstBody } from "../render/body";
import { BlockRenderBoundary } from "./BlockRenderBoundary";
import { blockDirAttr } from "../render/textDirection";
import { sourceFormatForPage } from "../document";
import { rowRaw, tableRowTitle } from "../sheet/tableFields";
import type { FormulaEvalRow } from "../sheet/formulaEval";

// Render-once row bodies survive remounts, and reset when the graph changes.
export const renderedSheetRows = new Set<string>();
clearOnBindingInvalidated(() => renderedSheetRows.clear());
export function resetSheetRowVirtualizationForTests() { renderedSheetRows.clear(); }

/** Render the authored block body; the cell's CSS bounds tall content. */
export function TitleBody(props: { row: FormulaEvalRow }): JSX.Element {
  const format = createMemo(() => sourceFormatForPage(props.row.page, props.row.kind, props.row.path, props.row.format) ?? null);
  const placeholder = () => {
    const title = tableRowTitle(props.row, format());
    return title.trim() === "" || title === "—" ? title : null;
  };
  return (
    <Show when={placeholder() === null} fallback={<InlineText text={placeholder() ?? ""} format={format()} />}>
      <Show when={format()} fallback={<div class="sheet-title-body">{rowRaw(props.row)}</div>}>
        {sourceFormat => <div class="sheet-title-body" dir={blockDirAttr(rowRaw(props.row), sourceFormat())}>
          <BlockRenderBoundary raw={rowRaw(props.row)}>
            <AstBody raw={rowRaw(props.row)} format={sourceFormat()} />
          </BlockRenderBoundary>
        </div>}
      </Show>
    </Show>
  );
}
