import { render } from "solid-js/web";
import { For } from "solid-js";
import { initParser } from "../../src/render/parse";
import { loadSingle } from "../../src/document/workingSet";
import { pageByName } from "../../src/document/model";
import { Block } from "../../src/components/Block";
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/ls-shim.css";

await initParser();
loadSingle({ name: "Editor bugs", title: "Editor bugs", kind: "page", pre_block: null, format: "md",
  blocks: ["before", "---", "after", "Tasks\n* [ ] first\n* [ ] second", "> [!TIP]\n> helpful"].map((raw, i) => ({
    id: `b${i}`, raw, collapsed: false, children: i === 4 ? [{ id: "child", raw: "child", collapsed: false, children: [] }] : [],
  })) });
render(() => <main class="main-content" style={{ padding: "40px", width: "640px" }}>
  <For each={pageByName("Editor bugs")!.roots}>{id => <Block id={id} />}</For>
</main>, document.getElementById("root")!);
