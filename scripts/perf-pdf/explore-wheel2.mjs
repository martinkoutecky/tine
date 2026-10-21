// Diagnose the fast-wheel stall seen in S1a (ours): which conditions make it
// appear. Evidence only; prints one line per variant.
//   xvfb-run -a -s "-screen 0 1920x1080x24" node scripts/perf-pdf/explore-wheel2.mjs
import { setTimeout as sleep } from "node:timers/promises";
import { launchApp, xdo } from "./app.mjs";
const CORPUS = "/aux/koutecky/logseq/tine-agents/runtime/pdf-perf-corpus";
const TMP = process.env.PERF_TMP || `/tmp/tine-pdf-wheel2-${process.pid}`;
const base = Number(process.env.PERF_PORT_BASE || 4760);
const app = await launchApp({ corpus: CORPUS, pdfs: ["arikan-0807.3917v5.pdf", "bv_cvxbook.pdf"], dpr: 1, tmp: TMP, driverPort: base, nativePort: base + 1 });
const b = app.browser;
const DELAY = process.env.DELAY || "15";
async function open(name) {
  await b.execute((n) => [...document.querySelectorAll(".pdf-link")].find((a) => a.textContent.includes(n)).click(), name);
  await b.waitUntil(() => b.execute(() => document.querySelectorAll(".pdf-page canvas").length > 0), { timeout: 30000 });
  await sleep(2000);
}
async function variant(label, { top = 0, where = "center", clicks = 200 } = {}) {
  const pos = await b.execute((top, where) => {
    const sc = document.querySelector(".pdf-scroll");
    sc.scrollTop = top;
    window.__w = { wheel: [], scroll: [], writes: [] };
    if (!window.__wHooked) {
      window.__wHooked = true;
      window.addEventListener("wheel", (e) => window.__w.wheel.push([performance.now(), e.defaultPrevented, e.target.className || e.target.tagName]), { passive: true });
      sc.addEventListener("scroll", () => window.__w.scroll.push([performance.now(), sc.scrollTop]), { passive: true });
      const d = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop");
      Object.defineProperty(Element.prototype, "scrollTop", { get: d.get, set(v) { window.__w.writes.push([performance.now(), String(this.className), v]); d.set.call(this, v); }, configurable: true });
    }
    const r = sc.getBoundingClientRect();
    return where === "gutter" ? [r.left + 6, r.top + r.height / 2] : [r.left + r.width / 2, r.top + r.height / 2];
  }, top, where);
  xdo("mousemove", String(Math.round(pos[0])), String(Math.round(pos[1])));
  await sleep(1200);
  await b.execute(() => { window.__w.wheel = []; window.__w.scroll = []; window.__w.writes = []; });
  xdo("click", "--repeat", String(clicks), "--delay", DELAY, "5");
  const atEnd = await b.execute(() => document.querySelector(".pdf-scroll").scrollTop);
  await sleep(2500);
  const r = await b.execute(() => {
    const w = window.__w, W = w.wheel, S = w.scroll;
    const t0 = W[0]?.[0] ?? 0, t1 = W.at(-1)?.[0] ?? 0;
    const during = S.filter((x) => x[0] <= t1);
    return { wheels: W.length, prevented: W.filter((x) => x[1]).length, targets: [...new Set(W.map((x) => x[2]))].slice(0, 4),
      spanMs: Math.round(t1 - t0), scrollEvents: S.length, movedDuringInput: during.length ? Math.round(during.at(-1)[1] - (S[0]?.[1] ?? 0)) : 0,
      final: document.querySelector(".pdf-scroll").scrollTop, writes: w.writes.length, writeOwners: [...new Set(w.writes.map((x) => x[1]))].slice(0, 4) };
  });
  console.log(label, JSON.stringify({ ...r, topAtInputEnd: atEnd }));
}
try {
  await b.setWindowRect(0, 0, 1920, 1080);
  await b.$(".pdf-link").waitForExist({ timeout: 60000 });
  await open("arikan");
  await variant("S1 top center");
  await variant("S1 top center again");
  await variant("S1 top gutter", { where: "gutter" });
  await variant("S1 from 9000 center", { top: 9000, clicks: 100 });
  await variant("S1 top center 100 clicks", { clicks: 100 });
  await b.execute(() => document.querySelector(".pdf-close-btn")?.click());
  await sleep(1000);
  await open("bv_cvxbook");
  await variant("S2 top center");
  await variant("S2 top center again");
} catch (e) { console.error("ERR", e.message); }
finally { await app.stop(); }
