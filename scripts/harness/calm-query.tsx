import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/query.css";
import "../../src/styles/query-calm.css";
import { render } from "solid-js/web";
import { For } from "solid-js";
import { backend } from "../../src/backend";
import { initParser } from "../../src/render/parse";
import { setDoc } from "../../src/document/model";
import { openPage } from "../../src/router";
import { Block } from "../../src/components/Block";
import type { ParsedQuery, QueryResult } from "../../src/editor/queryIr";
import { taskFilter } from "../../src/editor/queryBuilder";

await initParser();
const params = new URLSearchParams(location.search);
const queryIds = Array.from({ length: Math.min(40, Math.max(1, Number(params.get("queries") ?? 1))) }, (_, index) => index === 0 ? "query" : `query-${index}`);
const rowCount = Math.min(100, Math.max(3, Number(params.get("rows") ?? 3)));
const started = performance.now();
const blocks = Array.from({ length: rowCount }, (_, index) => ["Plan the trip", "Book the train", "Pack the bag"][index] ?? `Travel task ${index}`).map((text, index) => ({ id: `task-${index}`, raw: `TODO ${text}\nowner:: Martin`, marker: "TODO", collapsed: false, children: [], properties: [["owner", "Martin"]] as [string, string][] }));
setDoc({ byId: Object.fromEntries([
  ...queryIds.map(id => [id, { id, raw: '{{query (task TODO) {:title "Travel tasks"}}}', page: "Travel", parent: null, children: [], collapsed: false }]),
  ...blocks.map(block => [block.id, { ...block, page: "Travel", parent: null }]),
]), pages: [{ name: "Travel", kind: "page", title: "Travel", preBlock: null, roots: [...queryIds, ...blocks.map(block => block.id)], format: "md", readOnly: false, guide: false }], feed: ["Travel"], loaded: true });
openPage("Travel");
const b = backend();
b.parseQuery = async (_text, _dialect, properties = []): Promise<ParsedQuery> => ({
  query: { anchor: "block", filter: taskFilter(["TODO"]), diagnostics: [], source: { kind: "og", original: "(task TODO)", og_options: '{:title "Travel tasks"}' } },
  view: { view: (properties.find(([key]) => key === "tine.view")?.[1] ?? "list") as "list" | "table" | "board" | "search" },
});
b.queryOgExpressible = async () => true;
b.printQuery = async () => '(task TODO) {:title "Travel tasks"}';
b.queryRun = async (): Promise<QueryResult> => ({ anchor: "block", groups: [{ page: "Travel", kind: "page", blocks }], diagnostics: [], report: { ran: [], ignored: [], supported: true }, total: blocks.length, exceeded: false });
b.queryRegistry = async () => ({ generation: 1, rows: [] });
render(() => <main style="max-width:760px;margin:24px auto;padding:16px"><h1>Travel</h1><For each={queryIds}>{id => <Block id={id} />}</For></main>, document.getElementById("root")!);
(window as unknown as { __calmBenchStart: number }).__calmBenchStart = started;
