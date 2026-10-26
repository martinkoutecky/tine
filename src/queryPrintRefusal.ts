import type { Diagnostic } from "./editor/queryIr";

/** `query_print` refused this IR in the requested dialect. `isNotApplicable` is
 *  the expected "OG cannot say this" answer the save path turns into
 *  `{{tine-query}}`; every other refusal is surfaced, never swallowed (I-9). */
export class QueryPrintRefusedError extends Error {
  constructor(readonly reasonCode: string, readonly diagnostic: Diagnostic | null) {
    super(diagnostic?.message ?? `The query could not be printed (reason code: ${reasonCode}).`);
    this.name = "QueryPrintRefusedError";
  }
  get isNotApplicable(): boolean {
    return this.reasonCode === "not_applicable";
  }
}

/** Decode og's `query-print-refused:<reason>:<diagnostic JSON>` envelope. */
export function queryPrintRefusal(error: unknown): QueryPrintRefusedError | null {
  const text = error instanceof Error ? error.message : String(error);
  const match = /^query-print-refused:([a-z_]+):([\s\S]*)$/.exec(text);
  if (!match) return null;
  let diagnostic: Diagnostic | null = null;
  try {
    const value = JSON.parse(match[2]) as Record<string, unknown> | null;
    if (value && typeof value["message"] === "string" && typeof value["kind"] === "string") diagnostic = value as unknown as Diagnostic;
  } catch {
    diagnostic = null; // a malformed detail still refuses; the reason code carries it
  }
  return new QueryPrintRefusedError(match[1], diagnostic);
}
