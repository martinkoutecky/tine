// Browser component fixture: production math, swipe listeners and movable cards.
import { render } from "solid-js/web";
import { MathView } from "../../src/render/inline";
import { SheetBoard } from "../../src/components/SheetBoard";
import { setDoc } from "../../src/document/model";
import { initParser } from "../../src/render/parse";
import { attachBlockSwipe } from "../../src/blockSwipe";
import "../../src/styles/app.css";
import "katex/dist/katex.css";

await initParser();
document.documentElement.dataset.touchGestures = "android";
const tex = String.raw`\mathrm{K_2Cr_2O_7 + 6FeSO_4 + 7H_2SO_4 \rightarrow K_2SO_4 + Cr_2(SO_4)_3 + 3Fe_2(SO_4)_3 + 7H_2O}`;
const ids = ["inline", "display"];
setDoc({
  byId: {
    board: { id: "board", raw: "Board", parent: null, page: "Math", children: ids, collapsed: false },
    ...Object.fromEntries(ids.map((id, i) => [id, {
      id, raw: `TODO Formula ${i ? "$$" : "$"}${tex}${i ? "$$" : "$"}`,
      parent: "board", page: "Math", children: [], collapsed: false,
    }])),
  },
  pages: [{ name: "Math", title: "Math", kind: "page", preBlock: null, roots: ["board"], format: "md", readOnly: false, guide: false }],
  feed: ["Math"], loaded: true,
});
render(() => <>
  {ids.map((id, i) => <div class="block-main" data-probe={id}>
    <span class="swipe-start">Swipe here</span><MathView tex={tex} display={!!i} />
  </div>)}
  <div class="block-main" data-probe="code"><span class="swipe-start">Swipe here</span><pre class="code-block"><code>{"wide code ".repeat(80)}</code></pre></div>
  <div class="block-main" data-probe="table"><span class="swipe-start">Swipe here</span><div class="md-table-wrap"><table class="md-table"><tbody><tr>{ids.map(id => <td>{id.repeat(100)}</td>)}</tr></tbody></table></div></div>
  <div class="block-main" data-probe="sheet"><span class="swipe-start">Swipe here</span><div class="sheet-scroll"><table><tbody><tr><td>{"sheet ".repeat(100)}</td></tr></tbody></table></div></div>
  <SheetBoard ownerId="board" rowSource="children" groupBy="state" />
</>, document.body);
for (const row of document.querySelectorAll<HTMLElement>("[data-probe]")) {
  attachBlockSwipe(row, { enabled: true, editing: () => false, run: action => { row.dataset.committed = action; } });
}
