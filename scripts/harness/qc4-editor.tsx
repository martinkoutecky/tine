import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import { render } from "solid-js/web";
import { initParser } from "../../src/render/parse";
import { setDoc } from "../../src/document/model";
import { openPage } from "../../src/router";
import { Block } from "../../src/components/Block";
import { MobileKeyboardToolbar } from "../../src/components/MobileKeyboardToolbar";

await initParser();
setDoc({ byId: { selection: { id: "selection", raw: "alpha selected omega", page: "QC4 selection",
  parent: null, children: [], collapsed: false } }, pages: [{ name: "QC4 selection", kind: "page",
  title: "QC4 selection", preBlock: null, roots: ["selection"], format: "md", readOnly: false, guide: false }],
  feed: [], loaded: true });
openPage("QC4 selection");
render(() => <><main style="padding:100px;max-width:800px"><h1>QC4 selection</h1><Block id="selection" /></main><MobileKeyboardToolbar /></>, document.getElementById("root")!);
