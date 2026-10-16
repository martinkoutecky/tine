// Real menu/reference components with bounded, read-only browser data.
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../../src/backend";
import { initParser } from "../../src/render/parse";
import { setDoc } from "../../src/document/model";
import { ContextMenu } from "../../src/components/ContextMenu";
import { LinkedReferences } from "../../src/components/LinkedReferences";
import { UnlinkedReferences } from "../../src/components/UnlinkedReferences";
import { QueryActions } from "../../src/components/QueryActions";
import { openContextMenu, openPageContextMenu } from "../../src/ui";
import { setSectionOverride } from "../../src/referenceSectionState";
import { setActiveLocale } from "../../src/i18n";
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/query-calm.css";
import "../../src/styles/ls-shim.css";

await initParser();
await setActiveLocale(new URLSearchParams(location.search).get("locale") ?? "en");
const groups = ["Alpha", "Beta"].map(page => ({ page, kind: "page" as const,
  blocks: [{ id: page, raw: "Planning [[Target]]", collapsed: false, children: [] }] }));
backend().getBacklinks = async () => groups;
backend().getUnlinkedRefs = async () => groups;
backend().getBlockRefCounts = async () => ({});
const [copied, setCopied] = createSignal("");
const [folded, setFolded] = createSignal(false);
backend().writeText = async text => { setCopied(text); };
setDoc({ byId: Object.fromEntries(["Target", "Alpha", "Beta"].map(id => [id,
  { id, raw: id === "Target" ? "A block" : "Planning [[Target]]", page: id, parent: null, collapsed: false, children: [] }])),
  pages: ["Target", "Alpha", "Beta"].map(name => ({ name, kind: "page", title: name, roots: [name], preBlock: null, format: "md", readOnly: false, guide: false })),
  feed: ["Target"], loaded: true });
setSectionOverride("linked", "Target", false);
setSectionOverride("unlinked", "Target", true);
render(() => <main style={{ padding: "24px", "max-width": "640px" }}>
  <h1>Menu and reference controls</h1>
  <button id="page-menu" onClick={e => { const b = e.currentTarget.getBoundingClientRect(); openPageContextMenu(b.left, b.bottom, "Target", "page", true); }}>Page actions</button>{" "}
  <button id="block-menu" onClick={e => { const b = e.currentTarget.getBoundingClientRect(); openContextMenu(b.left, b.bottom, "Target"); }}>Block actions</button>
  <output id="copied">{copied()}</output>
  <LinkedReferences name="Target" />
  <UnlinkedReferences name="Target" />
  <div class="query-block"><div class="query-header">Query results
    <QueryActions editable results folded={folded()} fold={setFolded} edit={() => {}} copy={() => {}} search={() => {}} export={() => {}} />
  </div></div>
  <ContextMenu />
</main>, document.getElementById("root")!);
