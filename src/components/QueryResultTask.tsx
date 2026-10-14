import type { JSX } from "solid-js";
import { blockExternalId, pageByName } from "../document";
import { blockDtoExternalId } from "../blockIdentity";
import { recordFacets } from "../sheet/tableFields";
import type { FormulaEvalRow } from "../sheet/formulaEval";
import { SourceTaskCheckbox } from "./SourceTaskCheckbox";
import { openPageAtBlock, openInNewTab } from "../router";
import { openBlockInSidebar } from "../ui";
import { internalLinkDest } from "../linkGesture";
import { openRouteInOtherPane } from "../panes";

export function queryResultTarget(row: FormulaEvalRow) {
  return { uuid: row.dto ? blockDtoExternalId(row.dto) : blockExternalId(row.id) ?? row.id,
    page: row.page, pageKind: row.kind ?? pageByName(row.page)?.kind ?? "page",
    ...(row.path ? { path: row.path } : {}) };
}

export function openQueryResult(row: FormulaEvalRow, event: MouseEvent): void {
  event.stopPropagation();
  const target = queryResultTarget(row);
  const route = { kind: "page" as const, name: target.page, pageKind: target.pageKind, block: target.uuid,
    ...(target.path ? { path: target.path } : {}) };
  const dest = internalLinkDest(event);
  if (dest === "sidebar") openBlockInSidebar(target);
  else if (dest === "background") openInNewTab(route);
  else if (dest === "pane") void openRouteInOtherPane(route);
  else openPageAtBlock(route);
}

export function QueryResultTask(props: { row: FormulaEvalRow }): JSX.Element {
  return <span onPointerDown={(event) => event.stopPropagation()}>
    <SourceTaskCheckbox marker={recordFacets(props.row)?.marker} target={() => queryResultTarget(props.row)} result={props.row} />
  </span>;
}
