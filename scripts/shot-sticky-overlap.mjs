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
// GH #717 (second report): an arbitrary outline, written as the reporter writes it:
// [text, ...children]. Block ids are the texts, so they must be unique.
window.outline = ({ outline, multiline, sidebar, scale, zoom }) => {
  dispose?.();
  const build = ([text, ...kids]) => ({ id: text, raw: text + (multiline && kids.length ? " has a long title that wraps on phones and keeps its following lines readable\\nSecond line of the parent\\nThird line of the parent" : ""), collapsed:false, children: kids.map(build) });
  const roots = outline.map(build);
  const tail = Array.from({length:40},(_,i)=>({id:"tail-"+i,raw:"Tail row "+i,collapsed:false,children:[]}));
  loadSingle({name:"Sticky subtree",title:"Sticky subtree",kind:"page",format:"md",pre_block:null,blocks:[...roots,...tail]});
  document.documentElement.style.fontSize = scale+"px";
  document.documentElement.style.zoom = zoom;
  dispose = render(() => <main data-probe-scroller class={sidebar ? "right-sidebar" : "main-content pane-main-content"} style="position:relative;margin:80px 12px 0;height:450px;overflow:auto;padding:0;width:calc(100% - 24px);max-width:none;flex-shrink:0"><section style="padding:0 12px"><h1>Sticky subtree</h1>{[...roots,...tail].map(block=><Block id={block.id}/>)}</section></main>, document.getElementById("root"));
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
    if (process.env.STICKY_SUBTREE_ONLY) continue;
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
  if (!process.env.STICKY_SUBTREE_ONLY) {
  writeFileSync(`${out}/${process.env.STICKY_LABEL || "current"}-geometry.json`, JSON.stringify({tested,results,failures},null,2));
  assert.equal(failures.length,0, `sticky parents overlap readable rows in ${failures.length} cases; see geometry artifact`);
  console.log(`PASS: ${results.length} real-browser sticky geometry cases`);
  }

  // GH #717 second report: a sticky row stays inside its own subtree. A pin may
  // cover only rows of its own descendants (or its ancestors, which are higher
  // level); a row at its own level or above, outside its subtree, is pushed
  // against, never covered, and the pin is pushed up and out in its place.
  const outlines = {
    reporter1: ["0", ["1", ["1a", ["1aa"]]], ["2", ["2a"]], ["3"]],
    reporter2: ["0", ["1", ["1a", ["1aa"]], ["1b"], ["1c"], ["1d"]], ["2", ["2a"]], ["3"]],
    deep: ["r", ["a", ["b", ["c", ["d", ["d1"], ["d2"]], ["c2"]], ["b2"]], ["a2"]], ["s", ["s1"]]],
    twoRoots: ["p", ["p1", ["p1a", ["p1aa"], ["p1ab"]], ["p1b", ["p1ba"]]], ["p2"]],
  };
  const subtreeShells = [
    {width:1100, sidebar:false, scale:16, zoom:1},
    {width:390, sidebar:false, scale:16, zoom:1},
    {width:1100, sidebar:true, scale:20, zoom:1},
    {width:390, sidebar:true, scale:16, zoom:1.25},
  ];
  const subtreeFailures = [], subtreeResults = [];
  // The reporter's trees are one top-level block each, so wrap the second in
  // the same way: every outline is a list of top-level blocks.
  for (const [name, tree] of Object.entries(outlines)) for (const shell of subtreeShells) for (const multiline of [false, true]) {
    await page.setViewportSize({ width: shell.width, height: 700 });
    await page.evaluate(options => window.outline(options), { outline: [tree], multiline, ...shell });
    await page.waitForTimeout(100);
    const label = `${name}-${shell.width}-${multiline ? "multi" : "single"}-${shell.sidebar ? "sidebar" : "pane"}`;
    // The pin takes its 30 px in flow and the row pulls itself back over it: the
    // page must lay out exactly as it does without any pin.
    const layout = await page.evaluate(() => {
      const root=document.querySelector("[data-probe-scroller]"), tail=root.querySelector('[data-block-id="tail-0"] > .block-main');
      const withPins = tail.getBoundingClientRect().top, height = root.scrollHeight;
      const holders=[...root.querySelectorAll(".sticky-ancestor")], blocks=holders.map(h=>h.parentElement);
      holders.forEach(h=>h.remove()); blocks.forEach(b=>b.removeAttribute("data-sticky-parent"));
      return {withPins, without: tail.getBoundingClientRect().top, height, heightWithout: root.scrollHeight};
    });
    assert.ok(Math.abs(layout.withPins-layout.without)<0.5 && Math.abs(layout.height-layout.heightWithout)<0.5, `pins change the layout: ${label} ${JSON.stringify(layout)}`);
    await page.evaluate(options => window.outline(options), { outline: [tree], multiline, ...shell });
    await page.waitForTimeout(100);
    const scrollMax = await page.evaluate(() => { const r=document.querySelector("[data-probe-scroller]"); return r.scrollHeight - r.clientHeight; });
    const positions = Array.from({length:Math.ceil(scrollMax/8)+1},(_,i)=>Math.min(i*8,scrollMax));
    positions.push(...Array.from({length:Math.ceil(scrollMax/40)+1},(_,i)=>Math.max(scrollMax-i*40,0)));
    let overlapCount = 0, pinnedSeen = new Set(), pushed = new Set(), sawShot = false;
    for (const scroll of positions) {
      await page.evaluate(scroll => document.querySelector("[data-probe-scroller]").scrollTop=scroll, scroll);
      await page.waitForTimeout(15);
      const found = await page.evaluate(() => {
        const root=document.querySelector("[data-probe-scroller]"), rootTop=root.getBoundingClientRect().top;
        const out = {violations:[], pins:[], pushed:[]};
        const blocks=[...root.querySelectorAll("[data-block-id]")];
        for (const button of root.querySelectorAll(".sticky-ancestor button:not([hidden])")) {
          const holder = button.closest(".sticky-ancestor"), id = holder.dataset.stickyBlock;
          const own = holder.parentElement, pin = button.getBoundingClientRect();
          let depth = 0; for (let el = own.parentElement; el && el !== root; el = el.parentElement) if (el.hasAttribute("data-sticky-parent")) depth++;
          out.pins.push(id);
          // pushed: the pin sits above the slot its depth gives it.
          if (pin.top < rootTop + depth * pin.height - 1) out.pushed.push(id);
          for (const other of blocks) {
            if (other === own || own.contains(other) || other.contains(own)) continue;
            const row = other.querySelector(":scope > .block-main").getBoundingClientRect();
            const overlap = Math.min(pin.bottom,row.bottom) - Math.max(pin.top,row.top);
            if (overlap > 1 && row.bottom > rootTop && row.top < root.getBoundingClientRect().bottom) out.violations.push({pin:id, covers:other.dataset.blockId, overlap, pinTop:pin.top, pinBottom:pin.bottom, rowTop:row.top, rowBottom:row.bottom});
          }
        }
        return out;
      });
      found.pins.forEach(id => pinnedSeen.add(id));
      if (found.pushed.includes("1a") && !pushed.has("1a") && label.startsWith("reporter")) await page.screenshot({ path: `${out}/${process.env.STICKY_LABEL || "current"}-pushed-${label}.png` });
      found.pushed.forEach(id => pushed.add(id));
      if (found.violations.length) {
        overlapCount++;
        if (!sawShot) { sawShot = true; await page.screenshot({ path: `${out}/${process.env.STICKY_LABEL || "current"}-subtree-${label}.png` }); }
        subtreeFailures.push({label, scroll, violations: found.violations.slice(0,3)});
        break;
      }
    }
    // The pushed-up behaviour: with a parent whose subtree ends before the next
    // row at its level, that pin slides up out of its own slot as the next row arrives.
    const mustPush = name === "reporter1" ? "1a" : name === "reporter2" ? "1a" : null;
    if (mustPush && !overlapCount) {
      assert.ok(pinnedSeen.has(mustPush), `never pinned ${mustPush}: ${label}`);
      if (!pushed.has(mustPush)) subtreeFailures.push({label, reason: `${mustPush} was never pushed up out of its slot`});
    }
    subtreeResults.push({label, pins:[...pinnedSeen], pushed:[...pushed], overlapped: overlapCount > 0});
  }
  writeFileSync(`${out}/${process.env.STICKY_LABEL || "current"}-subtree.json`, JSON.stringify({tested, subtreeResults, subtreeFailures}, null, 2));
  assert.equal(subtreeFailures.length, 0, `sticky pins leave their subtree in ${subtreeFailures.length} cases: ${JSON.stringify(subtreeFailures.slice(0,2))}`);
  console.log(`PASS: ${subtreeResults.length} real-browser sticky subtree cases`);
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
