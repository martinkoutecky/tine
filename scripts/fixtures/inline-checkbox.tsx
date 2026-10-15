import { render } from "solid-js/web";
import { initParser } from "../../src/render/parse";
import { loadSingle } from "../../src/document/workingSet";
import { Block } from "../../src/components/Block";
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/ls-shim.css";

await initParser();
loadSingle({ name: "Checklist", title: "Checklist", kind: "page", pre_block: null, format: "md",
  blocks: [{ id: "list", raw: "Checklist:\n* [ ] Parent item\n   * [ ] Child item", collapsed: false,
    children: [{ id: "child", raw: "Child block", collapsed: false, children: [] }] }] });
render(() => <main class="main-content" style={{ padding: "40px", width: "640px" }}><Block id="list" /></main>, document.getElementById("root")!);
