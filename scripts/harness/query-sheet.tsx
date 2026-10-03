// Dev-server-only harness for scripts/shot-query-chooser.mjs (GH #619 class 2): the REAL QueryBuilder + QuerySheet
// with the app's real CSS, over a stubbed backend whose property registry has `?keys=N` keys. Never part of the
// production build (not reachable from index.html).
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/topbar.css";
import "../../src/styles/themePresentation.css";
import "../../src/styles/settingsControls.css";
import "../../src/styles/query.css";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../../src/backend";
import { QueryBuilder, type BuilderSession } from "../../src/components/QueryBuilder";
import { taskFilter } from "../../src/editor/queryBuilder";

const params = new URLSearchParams(location.search);
const keys = Number(params.get("keys") ?? 6);
const rows = Array.from({ length: keys }, (_, i) => ({
  normalized_name: `prop-key-${String(i).padStart(4, "0")}`, cardinality: "one" as const, observed_type: "text" as const,
  count_blocks: 3, count_pages: 1, mismatch_count: 0, top_values: [["alpha", 2], ["beta", 1]] as [string, number][],
}));
const b = backend() as unknown as Record<string, unknown>;
b.queryRegistry = async () => ({ generation: 1, rows });
b.queryFacets = async () => [];
b.printQuery = async () => "(and (task TODO))";

const session = (): BuilderSession => ({ query: { anchor: "block", filter: taskFilter(["TODO"]), source: { kind: "builder" } }, view: {} });
const [current, setCurrent] = createSignal<BuilderSession>(session());
document.getElementById("root")!.style.cssText = "padding:120px 40px 40px";
render(() => (
  <div class="query-block" style="max-width:700px">
    <QueryBuilder session={current} onChange={setCurrent} blockId="harness" />
  </div>
), document.getElementById("root")!);
