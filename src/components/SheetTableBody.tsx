import { Show, type JSX } from "solid-js";
import { clearOnBindingInvalidated } from "../binding";
import { InlineText } from "../render/inline";
import { AstBody } from "../render/body";
import { BlockRenderBoundary } from "./BlockRenderBoundary";
import { blockDirAttr } from "../render/textDirection";
import { rowRaw, tableRowTitle } from "../sheet/tableFields";
import type { FormulaEvalRow } from "../sheet/formulaEval";

// Render-once row bodies survive remounts, and reset when the graph changes.
export const renderedSheetRows = new Set<string>();
clearOnBindingInvalidated(() => renderedSheetRows.clear());
export function resetSheetRowVirtualizationForTests() { renderedSheetRows.clear(); }

/** Render the authored block body; the cell's CSS bounds tall content. */
export function TitleBody(props: { row: FormulaEvalRow; format: "md" | "org" }): JSX.Element {
  const placeholder = () => {
    const title = tableRowTitle(props.row);
    return title.trim() === "" || title === "—" ? title : null;
  };
  return (
    <Show when={placeholder() === null} fallback={<InlineText text={placeholder() ?? ""} format={props.format} />}>
      <div class="sheet-title-body" dir={blockDirAttr(rowRaw(props.row), props.format)}>
        <BlockRenderBoundary raw={rowRaw(props.row)}>
          <AstBody raw={rowRaw(props.row)} format={props.format} />
        </BlockRenderBoundary>
      </div>
    </Show>
  );
}
