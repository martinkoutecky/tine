// Browser observation boundary: real components and shipped styles, bounded DTO fixture.
import { render } from "solid-js/web";
import { backend } from "../../src/backend";
import { initParser } from "../../src/render/parse";
import { setDoc } from "../../src/document/model";
import { LinkedReferences } from "../../src/components/LinkedReferences";
import { UnlinkedReferences } from "../../src/components/UnlinkedReferences";
import { DatePicker } from "../../src/components/DatePicker";
import { openDatePicker } from "../../src/ui";
import { setSectionOverride } from "../../src/referenceSectionState";
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/ls-shim.css";

await initParser();
const groups = [{ page: "Source", kind: "page" as const,
  blocks: [{ id: "mention", raw: "Target", collapsed: false, children: [] }] }];
backend().getBacklinks = async () => groups;
backend().getUnlinkedRefs = async () => groups;
setDoc({ byId: { task: { id: "task", raw: "TODO Calendar fixture", page: "Source", parent: null, collapsed: false, children: [] } },
  pages: [{ name: "Source", kind: "page", title: "Source", roots: ["task"], preBlock: null, format: "md", readOnly: false, guide: false }], feed: [], loaded: true });
setSectionOverride("linked", "Target", true);
setSectionOverride("unlinked", "Target", false);
render(() => <main style={{ padding: "40px", width: "640px" }}>
  <h1>Reference controls</h1>
  <LinkedReferences name="Target" />
  <UnlinkedReferences name="Target" />
  <button id="open-calendar" onClick={() => openDatePicker("task", "scheduled", 360, 210)}>Schedule task</button>
  <DatePicker />
</main>, document.getElementById("root")!);
