import { render } from "solid-js/web";
import { Block } from "../../src/components/Block";
import { loadSingle } from "../../src/document/workingSet";
import { initParser } from "../../src/render/parse";
import { clearSeededFacets } from "../../src/render/facets";
import "../../src/styles/theme.css";
import "../../src/styles/inter.css";
import "../../src/styles/app.css";
import { QueryLegacyTable } from "../../src/components/QueryLegacyTable";
import { SheetContainer } from "../../src/components/SheetContainer";
import { SheetTable } from "../../src/components/SheetTable";

export async function mount(raw: string, depth: number, paneWidth: number, format: "md" | "org" = "md", surface = "markdown") {
  await initParser();
  let block = { id: "table", raw, collapsed: false, children: [] as any[] };
  for (let i = 0; i < depth; i++) block = { id: `parent-${i}`, raw: `Parent ${i + 1}`, collapsed: false, children: [block] };
  if (surface === "sheet") block = { id: "table", raw: "Sheet\ntine.fields:: first=text;second=text\ntine.table-widths:: prop:first=450;prop:second=450", collapsed: false, children: [{ id: "row", raw: "Row\nfirst:: editable\nsecond:: stable", collapsed: false, children: [] }] };
  loadSingle({ name: "Table bleed", title: "Table bleed", kind: "page", format, pre_block: null, blocks: [block] });
  clearSeededFacets();
  document.body.innerHTML = '<div id="fixture"></div>';
  return render(() => <div class="pane-leaf" style={{ width: `${paneWidth}px`, height: "650px", flex: "none", margin: "0 auto" }}>
    <div class="main-content"><div class="main-content-inner" style={{ "max-width": "720px", width: "100%" }}><h1>Table bleed</h1>{surface === "query" ? <div class="query-block"><QueryLegacyTable cols={["block", "page"]} rows={[{ text: "a".repeat(65), page: "b".repeat(65), kind: "page", props: {}, byKey: {} }]} sortBy={() => {}} arrow={() => ""} /></div>
      : surface === "sheet" ? <SheetContainer><SheetTable ownerId="table" rowSource="children" /></SheetContainer>
      : surface === "sidebar" ? <div class="right-sidebar-body"><div class="rs-item-body"><Block id={block.id} /></div></div>
      : surface === "embed" ? <div class="embed-block"><Block id={block.id} /></div>
      : <Block id={block.id} />}</div></div>
  </div>, document.getElementById("fixture")!);
}
