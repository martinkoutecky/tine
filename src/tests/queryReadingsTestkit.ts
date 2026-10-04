// How the ONE engine reads a macro argument — stated explicitly, for tests.
//
// The frontend no longer decides where a trailing options map begins, nor
// whether a `{{query …}}` holds the legacy DSL or advanced datalog. `query_parse`
// answers both, in Rust (SPEC §7.1, X4; I-12). `src/mock.ts` deliberately cannot
// answer them — it has no parser, and a copy of one is exactly the twin this
// packet deleted — so it retains the whole argument verbatim as `source.original`
// with no options.
//
// That is the honest behaviour of a backend with no engine, but it means a test
// that depends on either reading has to SAY what the engine answers, instead of
// relying on a frontend regex it can no longer reach. This helper is that
// statement: a table from macro argument to the reading, with no parsing of any
// kind. An argument the table does not mention is a test asking for a reading it
// never declared, and it throws rather than guessing.
import { vi } from "vitest";
import { backend } from "../backend";
import type {
  Filter,
  ParsedQuery,
  Query,
  QueryReport,
  QueryResult,
  Source,
  ViewSettings,
} from "../editor/queryIr";
import type { RefGroup } from "../types";

/** One declared reading: the form, the opaque options map, and which source
 *  variant the engine recognised. */
export interface QueryReading {
  /** `source.original` — the authored form WITHOUT the options map. */
  form: string;
  /** `source.og_options` — the opaque map INCLUDING its braces, or "". */
  opts?: string;
  /** The source variant. `og` unless stated; `advanced` is what makes the
   *  datalog path run. */
  kind?: Source["kind"];
  /** OG's table? answer (`wire_parse.rs`: options, `query-table::` or a
   *  trailing `table`), which the engine computes and a stub must declare. */
  legacy_table?: boolean;
  /** The view settings the engine lifted out of the text and the block's
   *  `tine.*` properties. */
  view?: ViewSettings;
  /** The filter the engine read, for the few tests that are about what the
   *  query MEANS rather than how the argument was split. Omitted, the reading is
   *  `raw` — the IR's honest spelling for text retained but not interpreted. A
   *  test that needs a real filter states it here; it does not ask a mock with
   *  no parser to derive one. */
  filter?: Filter;
  /** **The scoped display state `query_parse` flattens beside `{query, view}`**
   *  (§7.6, Q3). A reading that mentions none of these is a block with no
   *  scoped drafts at all — which is what every existing caller declares, and
   *  it is DIFFERENT from a block whose drafts are present and empty. Presence
   *  is therefore stated by mentioning the key, never by its value. */
  page_presentation?: ParsedQuery["page_presentation"];
  block_presentation?: ParsedQuery["block_presentation"];
  page_display?: ParsedQuery["page_display"];
  block_display?: ParsedQuery["block_display"];
  page_match_scope?: ParsedQuery["page_match_scope"];
  unreadable_settings?: ParsedQuery["unreadable_settings"];
}

function readingToIr(reading: QueryReading): ParsedQuery {
  const kind = reading.kind ?? "og";
  const original = reading.form;
  const og_options = reading.opts ?? "";
  const source: Source =
    kind === "builder"
      ? { kind: "builder" }
      : kind === "advanced"
        ? { kind: "advanced", original, og_options }
        : kind === "tql"
          ? { kind: "tql", original, og_options }
          : { kind: "og", original, og_options };
  // A stub does not invent a filter: `raw` is the IR's own spelling for text the
  // reader retained but did not interpret (§4.3.2), and it carries the payload
  // losslessly so a `preserveForm` print can still re-emit it. Tests here are
  // about the READING, never about what the filter means.
  const query: Query = {
    anchor: "block",
    filter: reading.filter ?? { kind: "raw", text: original, diagnostic_kind: "not_applicable" },
    diagnostics: [],
    source,
  };
  return {
    query,
    view: reading.view ?? {},
    ...(reading.legacy_table !== undefined ? { legacy_table: reading.legacy_table } : {}),
    // `Object.hasOwn`, never `??`: a present-but-empty scoped draft is `{}`,
    // and `{} ?? x` is `{}` while `undefined ?? x` is `x` — the two readings a
    // truthiness copy cannot tell apart.
    ...(reading.page_presentation !== undefined ? { page_presentation: reading.page_presentation } : {}),
    ...(reading.block_presentation !== undefined ? { block_presentation: reading.block_presentation } : {}),
    ...(Object.hasOwn(reading, "page_display") ? { page_display: reading.page_display } : {}),
    ...(Object.hasOwn(reading, "block_display") ? { block_display: reading.block_display } : {}),
    ...(reading.page_match_scope !== undefined ? { page_match_scope: reading.page_match_scope } : {}),
    ...(reading.unreadable_settings ? { unreadable_settings: reading.unreadable_settings } : {}),
  };
}

/** Install a `query_parse` stub that answers only for the arguments listed.
 *
 *  Keys are the macro argument as the component receives it — the macro body
 *  with the name already stripped. */
export function backendReadsQueries(readings: Record<string, QueryReading>): void {
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text: string) => {
    const reading = readings[text];
    if (!reading) {
      throw new Error(
        `this test did not declare how the engine reads ${JSON.stringify(text)} — `
          + "add it to backendReadsQueries({...}) rather than expecting the mock to parse it",
      );
    }
    return readingToIr(reading);
  });
}

/** What `query_run` answers for a block-anchored query (§7.1).
 *
 *  Execution goes through the ONE evaluator now — `run_query` and
 *  `run_advanced_query` cannot read TQL and are off the render path — so every
 *  query test states its result in this shape rather than as a bare group list.
 *  `report` is the advanced ran/ignored answer, which rides on the result
 *  instead of on a second command (M5). */
export function blockRunResult(groups: RefGroup[], report?: Partial<QueryReport>): QueryResult {
  return {
    anchor: "block",
    groups,
    diagnostics: [],
    report: { ran: [], ignored: [], supported: true, ...report },
    total: groups.reduce((sum, group) => sum + group.blocks.length, 0),
    exceeded: false,
  };
}
