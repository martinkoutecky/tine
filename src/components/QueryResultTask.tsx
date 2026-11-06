import type { JSX } from "solid-js";
import { blockExternalId, pageByName } from "../document";
import { blockDtoExternalId } from "../blockIdentity";
import { recordFacets } from "../sheet/tableFields";
import type { FormulaEvalRow } from "../sheet/formulaEval";
import { SourceTaskCheckbox } from "./SourceTaskCheckbox";
import { openPageAtBlock, openPageTarget, openInNewTab } from "../router";
import { openBlockInSidebar, openPageInSidebar } from "../ui";
import { internalLinkDest } from "../linkGesture";
import { openRouteInOtherPane } from "../panes";

export function queryResultTarget(row: FormulaEvalRow) {
  return { uuid: row.dto ? blockDtoExternalId(row.dto) : blockExternalId(row.id) ?? row.id,
    page: row.page, pageKind: row.kind ?? pageByName(row.page)?.kind ?? "page",
    ...(row.path ? { path: row.path } : {}) };
}

export function openQueryResult(row: FormulaEvalRow, event: Pick<MouseEvent, "shiftKey" | "ctrlKey" | "metaKey" | "altKey" | "stopPropagation">): void {
  event.stopPropagation();
  const target = queryResultTarget(row);
  const page = { name: target.page, pageKind: target.pageKind, ...(target.path ? { path: target.path } : {}) };
  const route = { kind: "page" as const, ...page, ...(row.dto?.page_preamble ? {} : { block: target.uuid }),
    ...(target.path ? { path: target.path } : {}) };
  const dest = internalLinkDest(event);
  if (dest === "sidebar") {
    if (row.dto?.page_preamble) openPageInSidebar(page);
    else openBlockInSidebar(target);
  }
  else if (dest === "background") openInNewTab(route);
  else if (dest === "pane") void openRouteInOtherPane(route);
  else if (row.dto?.page_preamble) openPageTarget(page);
  else openPageAtBlock({ ...route, block: target.uuid });
}

export function QueryResultTask(props: { row: FormulaEvalRow }): JSX.Element {
  return <span onPointerDown={(event) => event.stopPropagation()}>
    <SourceTaskCheckbox marker={recordFacets(props.row)?.marker} target={() => queryResultTarget(props.row)} result={props.row} />
  </span>;
}
