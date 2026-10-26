import { render } from "solid-js/web";
import { initParser } from "../../src/render/parse";
import { loadSingle } from "../../src/document/workingSet";
import { doc } from "../../src/document/model";
import { Block } from "../../src/components/Block";
import { setWorkflow } from "../../src/ui";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";

await initParser();
setWorkflow("todo");
const format = new URLSearchParams(location.search).get("format") === "org" ? "org" : "md";
const marker = format === "org" ? "+" : "*";
loadSingle({ name: "Touch", title: "Touch", kind: "page", pre_block: null, format,
  blocks: [
    { id: "list", raw: `Checklist:\n${marker} [ ] same\n${marker} [ ] same\n   ${marker} [ ] nested`, collapsed: false, children: [] },
    { id: "task", raw: "TODO task", marker: "TODO", collapsed: false, children: [] },
    { id: "task-next", raw: "TODO next task", marker: "TODO", collapsed: false, children: [] },
  ] });
Object.assign(window, { checkboxRaw: (id: string) => doc.byId[id].raw });
render(() => <main class="main-content"><div class="main-content-inner">
  <Block id="list" /><Block id="task" /><Block id="task-next" />
</div></main>, document.body);
