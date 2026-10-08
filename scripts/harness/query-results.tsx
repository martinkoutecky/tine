// QBV shot fixture: real macro/results and CSS over synthetic backend answers.
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/query.css";
import { render } from "solid-js/web";
import { backend } from "../../src/backend";
import { initParser } from "../../src/render/parse";
import { setDoc, doc } from "../../src/document/model";
import { openPage } from "../../src/router";
import { Block } from "../../src/components/Block";
import type { ParsedQuery, QueryResult } from "../../src/editor/queryIr";

await initParser();
const ids = ["default", "missing", "grouped"];
const raw = "{{query (page-property tags gptpro)}}";
setDoc({ byId: Object.fromEntries(ids.map((id) => [id, {
  id, raw: raw + (id === "missing" ? "\ntine.group-field:: prop:anchor" : id === "grouped" ? "\ntine.group-field:: prop:status" : "")
    + "\ntine.view:: " + (id === "grouped" ? "list" : "search"),
  page: "Index", parent: null, children: [], collapsed: false,
}])), pages: [{ name: "Index", kind: "page", title: "Index", preBlock: null, roots: ids,
  format: "md", readOnly: false, guide: false }], feed: ["Index"], loaded: true });
openPage("Index");
const b = backend();
b.parseQuery = async (text, _dialect, properties = []): Promise<ParsedQuery> => ({
  query: { anchor: "page", filter: { kind: "raw", text, diagnostic_kind: "not_applicable" },
    diagnostics: [], source: { kind: "og", original: text, og_options: "" } },
  view: { view: properties.find(([key]) => key === "tine.view")?.[1] as "search" | "list",
    group_by: properties.find(([key]) => key === "tine.group-field")?.[1] },
});
b.queryOgExpressible = async () => true;
b.printQuery = async () => "(page-property tags gptpro)";
b.queryRegistry = async () => ({ generation: 1, rows: [{ normalized_name: "anchor", cardinality: "one",
  observed_type: "text", count_blocks: 0, count_pages: 1, mismatch_count: 0 }] });
b.queryRun = async (_query, view): Promise<QueryResult> => ({
  anchor: "page", pages: ["Notebook A", "Notebook B", "Notebook C"].map((name) => ({
    name, path: `pages/${name}.md`, kind: "page", properties: [
      ["type", "research"], ["status", "open"], ["source", "Synthetic corpus"], ["date", "2026-10-04"], ["tags", "gptpro"],
    ],
  })), diagnostics: [], report: { ran: [], ignored: [], supported: true }, total: 3, exceeded: false,
  statistics: view.group_by ? { count: 3, aggregates: [["", "count"]], group_by: view.group_by,
    overall: [{ kind: "number", value: 3, skipped: 0 }], grouping_status: "exact",
    groups: [{ key: view.group_by === "prop:anchor" ? null : "open", count: 3,
      cells: [{ kind: "number", value: 3, skipped: 0 }] }],
  } : undefined,
});
(window as unknown as { __qbvRaw: () => string[] }).__qbvRaw = () => ids.map((id) => doc.byId[id].raw);
render(() => <main style="max-width:800px;margin:24px auto;padding:16px">
  <h2>Page query results</h2>
  <h3>Default Search</h3><Block id="default" />
  <h3>Search with a missing grouping field</h3><Block id="missing" />
  <h3>List grouped by status</h3><Block id="grouped" />
</main>, document.getElementById("root")!);
