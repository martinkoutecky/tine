// Diagnose fast-wheel behaviour: scroll events vs wheel events in ours, stock, and a plain div.
import { setTimeout as sleep } from "node:timers/promises";
import { launchApp, startStaticServer, xdo } from "./app.mjs";
const CORPUS = "/aux/koutecky/logseq/tine-agents/runtime/pdf-perf-corpus";
const DIST = "/aux/koutecky/logseq/tine-agents/runtime/pdfjs-dist-4.10.38";
const { server, port } = await startStaticServer(DIST, CORPUS);
const app = await launchApp({ corpus: CORPUS, pdfs: ["arikan-0807.3917v5.pdf"], dpr: 1, tmp: "/tmp/claude-3042/-aux-koutecky-logseq/054754e6-ffb1-4723-99f8-0c9658056a4c/scratchpad/explore-w", driverPort: 4650, nativePort: 4651 });
const b = app.browser;
const delays = (process.env.DELAYS || "15,30,60").split(",");
async function measure(label, sel) {
  for (const d of delays) {
    await b.execute((s) => { const sc = s === "#viewerContainer" ? document.getElementById("viewerContainer") : document.querySelector(s); sc.scrollTop = 0;
      window.__w = { wheel: [], scroll: [], sc };
      window.addEventListener("wheel", (e) => window.__w.wheel.push([performance.now(), e.defaultPrevented]), { passive: true });
      sc.addEventListener("scroll", () => window.__w.scroll.push([performance.now(), sc.scrollTop]), { passive: true }); }, sel);
    await sleep(500);
    const r = await b.execute((s) => { const sc = s === "#viewerContainer" ? document.getElementById("viewerContainer") : document.querySelector(s); const x = sc.getBoundingClientRect(); return [x.left + x.width / 2, x.top + x.height / 2]; }, sel);
    xdo("mousemove", String(Math.round(r[0])), String(Math.round(r[1])));
    await sleep(200);
    const t0 = Date.now();
    xdo("click", "--repeat", "60", "--delay", d, "5");
    const wall = Date.now() - t0;
    await sleep(1500);
    const res = await b.execute(() => { const w = window.__w; const W = w.wheel, S = w.scroll;
      return { wheels: W.length, prevented: W.filter(x => x[1]).length, wheelSpan: Math.round(W.at(-1)?.[0] - W[0]?.[0]), scrolls: S.length,
        top: w.sc.scrollTop, firstScroll: Math.round(S[0]?.[0] - W[0]?.[0]), lastScroll: Math.round(S.at(-1)?.[0] - W.at(-1)?.[0]),
        trace: S.filter((_, i) => i % 6 == 0).map(x => Math.round(x[0] - W[0][0]) + ":" + x[1]).join(" ") }; });
    console.log(label, "delay", d, "xdotool wall", wall, JSON.stringify(res));
  }
}
try {
  await b.setWindowRect(0, 0, 1920, 1080);
  await b.$(".pdf-link").waitForExist({ timeout: 30000 });
  await b.$(".pdf-link").click();
  await b.$(".pdf-page canvas").waitForExist({ timeout: 20000 });
  await sleep(1500);
  await b.execute(() => {
    window.__writes = [];
    const d = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop");
    Object.defineProperty(Element.prototype, "scrollTop", { get: d.get, set(v) { window.__writes.push(["scrollTop", performance.now(), this.className, v, new Error().stack.split("\n").slice(1, 4).join("|")]); d.set.call(this, v); }, configurable: true });
    for (const m of ["scrollTo", "scrollBy", "scrollIntoView"]) { const o = Element.prototype[m]; Element.prototype[m] = function (...a) { window.__writes.push([m, performance.now(), this.className, JSON.stringify(a), new Error().stack.split("\n").slice(1, 4).join("|")]); return o.apply(this, a); }; }
    for (const m of ["replaceState", "pushState"]) { const o = history[m]; history[m] = function (...a) { window.__writes.push([m, performance.now(), "", String(a[2]).slice(0, 80), ""]); return o.apply(this, a); }; }
  });
  await measure("ours", ".pdf-scroll");
  console.log(JSON.stringify(await b.execute(() => { const w = window.__writes; return { n: w.length, kinds: [...new Set(w.map((x) => x[0] + " " + x[2]))], sample: w.slice(-8) }; }), null, 1).slice(0, 3000));
  await b.url(`http://127.0.0.1:${port}/web/viewer.html?file=/pdfs/arikan-0807.3917v5.pdf#zoom=98.6`);
  await sleep(3000);
  await measure("stock", "#viewerContainer");
  await b.url(`http://127.0.0.1:${port}/perf-test/plain.html`);
  await sleep(1000);
  await measure("plain-passive", "#sc");
  await b.url(`http://127.0.0.1:${port}/perf-test/plain.html#nonpassive`);
  await b.refresh();
  await sleep(1000);
  await measure("plain-nonpassive", "#sc");
} catch (e) { console.error("ERR", e.message); }
finally { await app.stop(); server.close(); }
