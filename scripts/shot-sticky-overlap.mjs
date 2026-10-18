// GH #717/#718: real Block/StickyAncestor layout, without a native backend.
import { chromium } from "playwright";
import { createServer } from "vite";
import { mkdirSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";
const out = "artifacts/fix-sticky-overlap";
const tested = `${execFileSync("rtk", ["git", "rev-parse", "HEAD"], {encoding:"utf8"}).trim()} + working diff`;
mkdirSync(out, { recursive: true });
writeFileSync(`${out}/fixture.html`, `<html data-theme="light"><meta name="viewport" content="width=device-width,initial-scale=1"><body><div id="root"></div><script type="module" src="/${out}/fixture.tsx"></script></body></html>`);
writeFileSync(`${out}/fixture.tsx`, `
import { render } from "solid-js/web";
import "/src/styles/theme.css";
import "/src/styles/app.css";
import { initParser } from "/src/render/parse";
import { loadSingle } from "/src/document/workingSet";
import { Block } from "/src/components/Block";
await initParser();
let dispose;
window.fixture = ({ depth, multiline, sidebar, scale, zoom, short }) => {
  dispose?.();
  let child = { id: "branch", raw: "Last parent", collapsed: false, children: Array.from({length:short ? 1 : 50}, (_,i)=>({ id: "leaf-"+i, raw: "Readable child "+i, collapsed:false, children:[] })) };
  for (let i=depth-2;i>=0;i--) child = {id:"parent-"+i,raw:"Parent "+i+(multiline ? " with a long title that wraps on phones and keeps its following lines readable\\nSecond line of the parent\\nThird line of the parent" : ""),collapsed:false,children:[child]};
  const tail = short ? Array.from({length:50},(_,i)=>({id:"sibling-"+i,raw:"Following root sibling "+i,collapsed:false,children:[]})) : [];
  loadSingle({name:"Sticky overlap",title:"Sticky overlap",kind:"page",format:"md",pre_block:null,blocks:[child,...tail]});
  document.documentElement.style.fontSize = scale+"px";
  document.documentElement.style.zoom = zoom;
  dispose = render(() => <main data-probe-scroller class={sidebar ? "right-sidebar" : "main-content pane-main-content"} style="position:relative;margin:80px 12px 0;height:450px;overflow:auto;padding:0;width:calc(100% - 24px);max-width:none;flex-shrink:0"><section style="padding:0 12px"><h1>Sticky overlap</h1><Block id={child.id} forceExpanded />{tail.map(block=><Block id={block.id}/>)}</section></main>, document.getElementById("root"));
};
`);
const server = await createServer({ server: { port: 5247, strictPort: true } });
await server.listen();
let browser, page;
const failures = [], results = [];
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  page = await browser.newPage();
  await page.goto(`http://localhost:5247/${out}/fixture.html`);
  await page.waitForFunction(() => !!window.fixture);
  const shells = [
    {width:1100, sidebar:false, scale:16, zoom:1},
    {width:390, sidebar:false, scale:16, zoom:1},
    {width:1100, sidebar:true, scale:20, zoom:1},
    {width:390, sidebar:true, scale:16, zoom:1.25},
    {width:1100, sidebar:false, scale:16, zoom:1, short:true},
    {width:390, sidebar:false, scale:16, zoom:1, short:true},
  ];
  for (const shell of shells) for (const depth of [2,3,4]) for (const multiline of [false,true]) {
    if (process.env.STICKY_SHORT_ONLY && !shell.short) continue;
    if (shell.short && multiline) continue;
    const {width} = shell;
    await page.setViewportSize({ width, height: 700 });
    await page.evaluate(options => window.fixture(options), { depth, multiline, ...shell });
    await page.waitForTimeout(100);
    const label = `${width}-${depth}-${multiline ? "multi" : "single"}-${shell.sidebar ? "sidebar" : "pane"}${shell.short ? "-short" : ""}`;
    const initial = await page.evaluate(() => {
      const root=document.querySelector("[data-probe-scroller]");
      const branch=root.querySelector('[data-block-id="branch"] > .block-main');
      return (branch.getBoundingClientRect().bottom-root.getBoundingClientRect().top)/Number(document.documentElement.style.zoom);
    });
    let previousPins = new Set();
    const positions = Array.from({length:Math.ceil((initial+100)/8)},(_,i)=>i*8);
    positions.push(...positions.slice().reverse());
    let activations = 0;
    const transitions = [];
    for (const scroll of positions) {
      await page.evaluate(scroll => document.querySelector("[data-probe-scroller]").scrollTop=scroll, scroll);
      await page.waitForTimeout(20);
      const geometry = await page.evaluate(() => {
        const root=document.querySelector("[data-probe-scroller]"), top=root.getBoundingClientRect().top;
        const pins=[...root.querySelectorAll('.sticky-ancestor button:not([hidden])')].map(el=>({id:el.parentElement.dataset.stickyBlock,text:el.textContent,top:el.getBoundingClientRect().top,bottom:el.getBoundingClientRect().bottom}));
        const rows=[...root.querySelectorAll('.block-main')].map(el=>({id:el.parentElement.dataset.blockId,text:el.textContent,top:el.getBoundingClientRect().top,bottom:el.getBoundingClientRect().bottom}));
        return {scroll:root.scrollTop,top,pins,rows:rows.slice(0,8)};
      });
      // A new pin must not suddenly cover the child below it. Once present,
      // content can scroll into the established sticky area as usual.
      // Allow only the distance scrolled since the preceding observation.
      if (geometry.pins.some(pin=>!previousPins.has(pin.id))) transitions.push(geometry);
      for (const pin of geometry.pins) {
        if (previousPins.has(pin.id)) continue;
        activations++;
        const ownIndex = geometry.rows.findIndex(row => row.id===pin.id);
        const next = geometry.rows.slice(ownIndex+1).find(row => !geometry.pins.some(other=>other.id===row.id));
        if (next && next.top < pin.bottom-8*shell.zoom-0.5) {
          failures.push({label, geometry, pin, next});
          await page.screenshot({ path: `${out}/${process.env.STICKY_LABEL || "current"}-${label}.png` });
          break;
        }
      }
      if (geometry.pins.length === depth && !previousPins.has("branch")) {
        await page.screenshot({ path: `${out}/${process.env.STICKY_LABEL || "current"}-${label}-stack.png` });
      }
      assert.ok(Math.abs(geometry.scroll-scroll)<1, `pinning moved the scroll position: ${label}`);
      previousPins = new Set(geometry.pins.map(pin=>pin.id));
      if (failures.at(-1)?.label === label) break;
    }
    assert.ok(activations>=depth, `fixture never pinned every parent: ${label}`);
    results.push({label,activations,transitions,failed:failures.at(-1)?.label===label});
  }
  writeFileSync(`${out}/${process.env.STICKY_LABEL || "current"}-geometry.json`, JSON.stringify({tested,results,failures},null,2));
  assert.equal(failures.length,0, `sticky parents overlap readable rows in ${failures.length} cases; see geometry artifact`);
  console.log(`PASS: ${results.length} real-browser sticky geometry cases`);
} catch (error) {
  const screenshot = `${out}/${process.env.STICKY_LABEL || "current"}-failure.png`;
  await page?.screenshot({path:screenshot}).catch(() => {});
  writeFileSync(`${out}/${process.env.STICKY_LABEL || "current"}-failure.json`, JSON.stringify({
    tested, journey:"Scroll nested outlines forward and backward until every ancestor pins",
    expected:"A newly appearing pin leaves the following child readable and does not move the scroll position",
    observed:String(error.stack), screenshot,
    classification:failures.length ? "product" : "ambiguous; inspect browser failure and harness log",
  },null,2));
  throw error;
} finally {
  await browser?.close();
  await server.close();
}
