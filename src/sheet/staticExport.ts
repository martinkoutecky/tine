// Static-export sheets (family 7). The Rust publisher hands the app one
// `SheetInput` per `tine.view` block; this module answers it with a pure DATA
// fragment (`SheetExport`) computed by the SAME functions the live views call
// (sheetConfig, filterFormulaRows, computeFormulaResults, buildBoardColumns,
// cellView, aggregate). Rust only lays the data out and escapes every string; it
// never re-derives grouping, formulas or aggregates (I-12). A sheet that throws
// exports as `{view: "error"}` and Rust falls back to the plain outline plus a note.
import { appNow } from "../journal";
import { facetsFromDto } from "../render/facets";
import { visibleBody } from "../render/block";
import { blockBackgroundColor } from "../blockColors";
import type { BlockDto } from "../types";
import { aggregate, AGGREGATE_LABELS, collectAggregateColumns } from "./aggregate";
import { boardCardChips, boardGroupField, boardRowTitle, buildBoardColumns } from "./boardColumns";
import { cellView, displayFieldValue, type CellView } from "./cellPresentation";
import { sheetConfig, type FieldType } from "./config";
import { fieldLabel, isFormulaField, type FieldId } from "./fields";
import {
  computeFormulaResults,
  filterFormulaRows,
  formulaResultKey,
  formulaValueToFieldValue,
  readFormulaRowField,
  type FormulaEvalRow,
} from "./formulaEval";
import { formulaFieldId, formulaNameFromField, formulasOf, mergeFormulas } from "./formulaFields";
import { fieldIdsForRecords, tableFieldOrder, tableRowTitle } from "./tableFields";

/** What the Rust publisher sends per candidate sheet block. */
export interface SheetInput {
  page: string;
  /** Child-index path from the page root to the owner block. */
  path: number[];
  /** Fingerprint of the owner's subtree; echoed back so Rust can refuse stale data. */
  fp: string;
  owner: BlockDto;
  /** The owner's children, each with shallow-DTO children (their count is the grid width). */
  rows: BlockDto[];
  /** Children Rust left out to bound the export. */
  omitted: number;
}

export interface Aggregated { label: string; text: string }
export interface TableRowExport { ix: number; title: string; bg: string | null; cells: CellView[] }
export interface BoardCardExport {
  ix: number;
  title: string;
  bg: string | null;
  chips: { priority: string; scheduled: string; deadline: string; tags: string[] };
}
export type SheetBody =
  | {
      view: "table";
      columns: { label: string; formula: boolean }[];
      rows: TableRowExport[];
      footer: (Aggregated | null)[] | null;
      filterError: string | null;
      omitted: number;
    }
  | {
      view: "board";
      columns: { label: string; cards: BoardCardExport[] }[];
      filterError: string | null;
      omitted: number;
    }
  | { view: "grid"; cols: number; header: boolean; footer: (Aggregated | null)[] | null; omitted: number }
  | { view: "error"; message: string };

export type SheetExport = { page: string; path: number[]; fp: string } & SheetBody;

export interface ExportEnv {
  now: Date;
  /** The user's task workflow, which orders the "state" board columns. */
  workflow: "todo" | "now";
}

const detach = (page: string, dto: BlockDto, ix: number): FormulaEvalRow => ({ id: String(ix), page, dto, detached: true });
const bgOf = (dto: BlockDto): string | null => blockBackgroundColor(facetsFromDto(dto).properties) ?? null;

function tableBody(input: SheetInput, cfg: ReturnType<typeof sheetConfig>, rows: FormulaEvalRow[], env: ExportEnv): SheetBody {
  const formulas = mergeFormulas(new Map(), formulasOf(facetsFromDto(input.owner).properties));
  const filtered = filterFormulaRows(rows, cfg.filter, formulas, env.now);
  const kept = filtered.rows;
  const { results } = computeFormulaResults(kept, formulas, env.now);
  const types = new Map<FieldId, FieldType>(cfg.fields.map((s) => [s.field, s.type] as const));
  const fields = tableFieldOrder(
    fieldIdsForRecords(kept, false),
    [],
    cfg.fields.map((s) => s.field),
    [...formulas.keys()].map(formulaFieldId)
  );
  const formulaValue = (row: FormulaEvalRow, field: FieldId) => {
    const name = formulaNameFromField(field);
    return name ? results.get(formulaResultKey(row, name)) ?? null : null;
  };
  const fieldValue = (row: FormulaEvalRow, field: FieldId) =>
    isFormulaField(field) ? formulaValueToFieldValue(formulaValue(row, field)) : readFormulaRowField(row, field);
  return {
    view: "table",
    columns: [{ label: "Block", formula: false }, ...fields.map((f) => ({ label: fieldLabel(f), formula: isFormulaField(f) }))],
    rows: kept.map((row) => ({
      ix: Number(row.id),
      title: tableRowTitle(row),
      bg: bgOf(row.dto!),
      cells: fields.map((f) =>
        cellView(f, types.get(f), displayFieldValue(f, types.get(f), fieldValue(row, f)), formulaValue(row, f))
      ),
    })),
    footer: cfg.colAggregates.size === 0
      ? null
      : fields.map((f) => {
          const fn = cfg.colAggregates.get(f);
          return fn ? { label: AGGREGATE_LABELS[fn], text: aggregate(fn, kept.map((r) => fieldValue(r, f))) } : null;
        }),
    filterError: filtered.error,
    omitted: input.omitted,
  };
}

function boardBody(input: SheetInput, cfg: ReturnType<typeof sheetConfig>, rows: FormulaEvalRow[], env: ExportEnv): SheetBody {
  const formulas = mergeFormulas(new Map(), formulasOf(facetsFromDto(input.owner).properties));
  const filtered = filterFormulaRows(rows, cfg.filter, formulas, env.now);
  const groupBy = boardGroupField(cfg.groupBy);
  const columns = buildBoardColumns(filtered.rows, groupBy, cfg.fields, { formulas, now: env.now, workflow: env.workflow });
  return {
    view: "board",
    columns: columns.map((col) => ({
      label: col.label,
      cards: col.rows.map((row) => ({
        ix: Number(row.id),
        title: boardRowTitle(row),
        bg: bgOf(row.dto!),
        chips: boardCardChips(row, groupBy),
      })),
    })),
    filterError: filtered.error,
    omitted: input.omitted,
  };
}

function gridBody(input: SheetInput, cfg: ReturnType<typeof sheetConfig>): SheetBody {
  const cols = Math.max(1, ...input.rows.map((r) => r.children.length));
  const raws = new Map<string, string>();
  const body = input.rows.map((row, r) => ({
    cellIds: row.children.map((cell, c) => {
      const id = `${r}.${c}`;
      raws.set(id, visibleBody(cell.raw).join(" "));
      return id;
    }),
  }));
  const configured = Array.from({ length: cols }, (_, c) => c).filter((c) => cfg.colAggregates.has(`${c}`));
  const columns = collectAggregateColumns(cfg.header ? body.slice(1) : body, configured, (id) => (id ? raws.get(id) ?? "" : ""));
  return {
    view: "grid",
    cols,
    header: cfg.header,
    footer: cfg.colAggregates.size === 0
      ? null
      : Array.from({ length: cols }, (_, c) => {
          const fn = cfg.colAggregates.get(`${c}`);
          return fn ? { label: AGGREGATE_LABELS[fn], text: aggregate(fn, columns.get(c) ?? []) } : null;
        }),
    omitted: input.omitted,
  };
}

/** One sheet's export, or null when the block's `tine.view` is not a sheet view.
 *  Never throws: a failing sheet exports as `{view: "error"}`. */
export function computeSheetExport(input: SheetInput, env: ExportEnv): SheetExport | null {
  try {
    const cfg = sheetConfig(facetsFromDto(input.owner).properties);
    if (!cfg.view) return null;
    const rows = input.rows.map((dto, ix) => detach(input.page, dto, ix));
    const body = cfg.view === "table"
      ? tableBody(input, cfg, rows, env)
      : cfg.view === "board"
        ? boardBody(input, cfg, rows, env)
        : gridBody(input, cfg);
    return { page: input.page, path: input.path, fp: input.fp, ...body };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return { page: input.page, path: input.path, fp: input.fp, view: "error", message: message.slice(0, 300) };
  }
}

/** Every sheet answer for a batch of inputs, at the current app clock. */
export function computeSheetExports(inputs: readonly SheetInput[], workflow: "todo" | "now", now: Date = appNow()): SheetExport[] {
  const out: SheetExport[] = [];
  for (const input of inputs) {
    const exported = computeSheetExport(input, { now, workflow });
    if (exported) out.push(exported);
  }
  return out;
}
