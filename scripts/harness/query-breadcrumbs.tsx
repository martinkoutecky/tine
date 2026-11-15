import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/settingsControls.css";
import "../../src/styles/query.css";
import "../../src/styles/query-calm.css";
import { render } from "solid-js/web";
import { backend } from "../../src/backend";
import { initParser } from "../../src/render/parse";
import { doc, setDoc } from "../../src/document/model";
import { openPage } from "../../src/router";
import { Block } from "../../src/components/Block";
import { taskFilter } from "../../src/editor/queryBuilder";
import type { ParsedQuery, QueryResult } from "../../src/editor/queryIr";

await initParser();
const task = { id: "task", raw: "TODO Buy apples", children: [], collapsed: false };
setDoc({ byId: {
  query: { id: "query", raw: "{{query (task TODO)}}", page: "Shopping", parent: null, children: [], collapsed: false },
  parent: { id: "parent", raw: "Weekly groceries", page: "Shopping", parent: null, children: ["task"], collapsed: false },
  task: { ...task, page: "Shopping", parent: "parent" },
}, pages: [{ name: "Shopping", title: "Shopping", kind: "page", roots: ["query", "parent"], preBlock: null,
  format: "md", readOnly: false, guide: false }], feed: ["Shopping"], loaded: true });
openPage("Shopping");
const b = backend();
b.parseQuery = async (_text, _name, properties = []): Promise<ParsedQuery> => ({
  query: { anchor: "block", filter: taskFilter(["TODO"]), source: { kind: "og", original: "(task TODO)", og_options: "" } },
  view: { view: (properties.find(([key]) => key === "tine.view")?.[1] ?? "list") as "list" | "table" },
});
b.queryRun = async (): Promise<QueryResult> => ({ anchor: "block", groups: [{ page: "Shopping", kind: "page",
  blocks: [{ ...task, breadcrumb: ["Weekly groceries"] }] }], diagnostics: [],
  report: { ran: [], ignored: [], supported: true }, total: 1, exceeded: false });
b.queryRegistry = async () => ({ generation: 1, rows: [] });
b.queryOgExpressible = async () => true;
b.printQuery = async () => "(task TODO)";
render(() => <main style="max-width:760px;margin:24px auto;padding:16px"><h1>Shopping</h1><Block id="query" /></main>, document.getElementById("root")!);
Object.assign(window, { breadcrumbSource: () => doc.byId.query.raw });
