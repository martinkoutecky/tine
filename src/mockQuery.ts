// The dev-preview mock's six query commands. Deliberately NOT a second query
// engine (I-12, D-14): the real parser, printer and walk live in
// `crates/tine-core/src/query/`. These answer the SHAPE of each command and say
// so when they cannot answer the substance.
import type {
  ExplainEmptyResult,
  ParsedQuery,
  Query,
  QueryPrintDialect,
  QueryResult,
  QueryTextDialect,
  RegistrySnapshot,
  Source,
  ViewSettings,
  ViewKind,
} from "./editor/queryIr";
import { sourceOptions, sourceOriginal } from "./editor/queryIr";

function refusal(reason: "not_applicable" | "syntax", message: string): Error {
  return new Error(`query-print-refused:${reason}:${JSON.stringify({ kind: reason, message, suggestions: [], disabled: false })}`);
}

export const mockQueryCommands = {
  async parseQuery(text: string, dialect: QueryTextDialect, blockProperties?: [string, string][]): Promise<ParsedQuery> {
    // The whole argument is retained verbatim: splitting the options map is Rust's
    // one answer, so a `preserveForm` print re-emits these exact bytes.
    const kind: Source["kind"] = dialect === "macro_tql" || dialect === "tql" ? "tql" : dialect === "advanced" ? "advanced" : "og";
    const view: ViewSettings = {};
    for (const [key, value] of blockProperties ?? []) {
      if (key === "tine.view" && ["search", "list", "table", "board"].includes(value)) view.view = value as ViewKind;
    }
    const query: Query = {
      anchor: "block",
      filter: { kind: "raw", text, diagnostic_kind: "not_applicable" },
      diagnostics: [{ kind: "not_applicable", message: "The browser dev preview does not run the query engine.", suggestions: [], disabled: false }],
      source: { kind, original: text, og_options: "" } as Source,
    };
    return { query, view };
  },
  async printQuery(query: Query, _view: ViewSettings, dialect: QueryPrintDialect, preserveForm = false): Promise<string> {
    const original = sourceOriginal(query.source);
    if (preserveForm) {
      if (original === null) throw refusal("not_applicable", "A builder query has no source form to preserve.");
      const options = sourceOptions(query.source);
      return options ? `${original} ${options}` : original;
    }
    if (dialect === "og") throw refusal("not_applicable", "The browser dev preview cannot print the OG DSL from an IR.");
    throw refusal("syntax", "The browser dev preview cannot print a query from an IR.");
  },
  async queryOgExpressible(): Promise<boolean> {
    return false; // the mock's parse only ever yields a `raw` capsule
  },
  async queryRegistry(): Promise<RegistrySnapshot> {
    return { rows: [], generation: 0 };
  },
  async queryRun(query: Query): Promise<QueryResult> {
    return { anchor: "block", groups: [], diagnostics: query.diagnostics ?? [], report: { ran: [], ignored: [], supported: false }, total: 0, exceeded: false };
  },
  async queryExplainEmpty(query: Query): Promise<ExplainEmptyResult> {
    return { rows: [], diagnostics: query.diagnostics ?? [], report: { ran: [], ignored: [], supported: false } };
  },
};
