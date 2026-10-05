import {
  logbook_apply_marker_transition,
  logbook_info_json,
} from "./render/wasm/lsdoc_wasm.js";
import { parserReady } from "./render/parse";
import { leadingMarker } from "./editor/marker";
import { reportUiFailure } from "./uiFailure";
import type { Format } from "./types";

export interface LogbookRow {
  type: string;
  start: string;
  end: string | null;
  span: string | null;
}

export interface LogbookInfo {
  seconds: number;
  summary: string;
  rows: LogbookRow[];
}

const EMPTY_INFO: LogbookInfo = { seconds: 0, summary: "0s", rows: [] };

export function applyMarkerTransition(
  oldRaw: string,
  nextRaw: string,
  format: Format,
  enabled: boolean,
  withSeconds: boolean,
): string {
  if (!enabled || !parserReady()) return nextRaw;
  try {
    return logbook_apply_marker_transition(
      nextRaw,
      format === "org",
      leadingMarker(oldRaw, format) ?? "",
      leadingMarker(nextRaw, format) ?? "",
      enabled,
      withSeconds,
    );
  } catch (error) {
    // The edit goes through without its LOGBOOK entry; say so (I-9).
    reportUiFailure("logbook", error);
    return nextRaw;
  }
}

export function logbookInfo(raw: string, format: Format): LogbookInfo {
  try {
    const parsed = JSON.parse(logbook_info_json(raw, format === "org")) as LogbookInfo;
    return {
      seconds: Number(parsed.seconds) || 0,
      summary: parsed.summary || "0s",
      rows: Array.isArray(parsed.rows) ? parsed.rows : [],
    };
  } catch (error) {
    reportUiFailure("logbook", error);
    return EMPTY_INFO;
  }
}
