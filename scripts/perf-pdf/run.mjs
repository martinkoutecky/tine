// PDF viewer perf harness (evidence branch batch/og-pdf-perf-review; never integrate).
//
// One session = one fresh Tine launch on a fresh graph. In it, every scenario
// runs first in Tine's own viewer, then the same scenarios in the stock pdf.js
// 4.10.38 viewer loaded into the SAME webview (WebDriver navigation to a local
// HTTP server), at the same CSS viewport size and the same zoom (CSS px per
// PDF point). Raw dumps + one JSONL row per trial go to --out.
//
//   LANG=C.UTF-8 LC_ALL=C.UTF-8 xvfb-run -a -s "-screen 0 1920x1080x24" \
//     node scripts/perf-pdf/run.mjs --dpr 1 --scenarios S0,S1a,S1b,S1c,S2,S3 --out <dir> [--theme dark] [--viewers ours,stock]
//   (DPR 2: -screen 0 3840x2160x24 and --dpr 2; the harness sets GDK_SCALE=2.)
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { launchApp, sizeWindow, COLLECTOR, startStaticServer, xdo, calibratePointer, rssSampler, loadavg } from "./app.mjs";
import { windowMetrics, firstSharp, firstChange, ms, rssPeak, F } from "./metrics.mjs";

const arg = (name, dflt) => { const i = process.argv.indexOf(`--${name}`); return i >= 0 ? process.argv[i + 1] : dflt; };
const DPR = Number(arg("dpr", "1"));
const THEME = arg("theme", "light");
const SCEN = new Set(arg("scenarios", "S0,S1a,S1b,S1c,S2,S3").split(","));
const VIEWERS = arg("viewers", "ours,stock").split(",");
const OUT = path.resolve(arg("out", "/aux/koutecky/logseq/tine-agents/evidence/pdf-perf-review/raw"));
const TAG = arg("tag", `dpr${DPR}-${THEME}-${Date.now()}`);
const CPUS = arg("cpus", process.env.PERF_CPUS || "");
const CORPUS = process.env.PDF_CORPUS || "/aux/koutecky/logseq/tine-agents/runtime/pdf-perf-corpus";
const DIST = process.env.PDFJS_DIST || "/aux/koutecky/logseq/tine-agents/runtime/pdfjs-dist-4.10.38";
const TMP = process.env.PERF_TMP || path.join("/tmp", `tine-pdf-perf-${process.pid}`);
const S1 = "arikan-0807.3917v5.pdf", S2 = "bv_cvxbook.pdf", S3 = "sourcebookinmath1929smit.pdf";
const PDFS = [S1, S2, S3].filter((p) => fs.existsSync(path.join(CORPUS, p)));
const FAST = 7000, MODERATE = 2000;
fs.mkdirSync(OUT, { recursive: true });
const rowsFile = path.join(OUT, `${TAG}.jsonl`);
const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

const { server, port } = await startStaticServer(DIST, CORPUS);
const app = await launchApp({ corpus: CORPUS, pdfs: PDFS, dpr: DPR, tmp: TMP, driverPort: 4720 + DPR, nativePort: 4730 + DPR, cpus: CPUS });
const b = app.browser;
let viewer = "ours";
let cal = null;
const ourScales = {}; // scale (CSS px / pt) reached in ours, reused for stock

const exec = (fn, ...args) => b.execute(fn, ...args);
const P = (code, ...args) => b.execute(`return (async (P, args) => { ${code} })(window.__perf, arguments)`, ...args);
async function install() { return b.execute(`return (${COLLECTOR})(arguments[0])`, viewer); }
async function dump() { return exec(() => window.__perf.dump()); }

let trialNo = 0;
async function trial(scenario, name, fn) {
  trialNo += 1;
  const load0 = loadavg();
  const rss = rssSampler(app.appPid, 100);
  const t0Wall = Date.now();
  let result, error = null, raw = null;
  try {
    result = await fn();
    await sleep(150); // let the sampler record the frames that follow the last settle
    raw = await dump();
  } catch (e) { error = String(e && e.stack || e); log("TRIAL ERROR", scenario, name, error.slice(0, 400)); }
  finally { await P("P.stop()").catch(() => {}); rss.stop(); }
  const row = { tag: TAG, dpr: DPR, theme: THEME, viewer, scenario, name, trial: trialNo, wallMs: Date.now() - t0Wall,
    load: [load0, loadavg()], rss: rssPeak(rss.samples), error, ...(result?.row || {}) };
  if (raw && result?.analyze) Object.assign(row, result.analyze(raw));
  if (raw) {
    const sampleCost = raw.frames.map((f) => f[12]).filter((x) => x != null);
    row.samplerMsP50 = sampleCost.length ? sampleCost.sort((a, c) => a - c)[sampleCost.length >> 1] : null;
    row.longtaskCount = raw.longtasks.length;
  }
  fs.appendFileSync(rowsFile, JSON.stringify(row) + "\n");
  fs.writeFileSync(path.join(OUT, `${TAG}-${String(trialNo).padStart(2, "0")}-${viewer}-${scenario}-${name}.json`),
    JSON.stringify({ row, raw, rss: rss.samples }));
  log(viewer, scenario, name, JSON.stringify(Object.fromEntries(Object.entries(row).filter(([k]) =>
    /Ms|ms|p95|max|Pct|Sharp|open|drift|speed|Mpx|MPx|error|scaleAfter/i.test(k)))).slice(0, 600));
  return row;
}

// ------------------------------------------------------------ viewer control
async function settle(timeout = 15000) { return P(`return await P.waitSettled(${timeout})`); }

async function openDoc(name) {
  if (viewer === "ours") {
    await P("P.start()");
    const t0 = await exec((n) => {
      const link = [...document.querySelectorAll(".pdf-link")].find((a) => (a.textContent || "").includes(n));
      const t = performance.now();
      link.click();
      return t;
    }, name);
    await b.waitUntil(() => exec(() => !!document.querySelector(".pdf-scroll .pdf-page")), { timeout: 60000 });
    const settled = await settle(60000);
    await waitTextEvent();
    return { t0, settled };
  }
  await P("P.start()");
  const t0 = await exec((url, zoom) => {
    const app = window.PDFViewerApplication;
    try { localStorage.clear(); } catch {}
    window.PDFViewerApplicationOptions.set("defaultZoomValue", zoom);
    window.PDFViewerApplicationOptions.set("sidebarViewOnLoad", 0);
    window.PDFViewerApplicationOptions.set("disableHistory", true);
    const t = performance.now();
    app.open({ url });
    return t;
  }, `/pdfs/${name}`, String((ourScales[`fit:${name}`] ?? 1.3153) * 75));
  await b.waitUntil(() => exec(() => (window.PDFViewerApplication.pdfViewer?.pagesCount ?? 0) > 0
    && window.__perf.pageList().length > 0), { timeout: 60000 });
  await P("P.hookStock()");
  const settled = await settle(60000);
  await waitTextEvent();
  const want = ourScales[`fit:${name}`];
  if (want && Math.abs((await currentScale()) - want) > 0.01) {
    log("stock open scale mismatch", await currentScale(), "->", want);
    await exec((k) => { window.PDFViewerApplication.pdfViewer.currentScale = k * 0.75; }, want);
    await settle(20000);
  }
  return { t0, settled };
}
async function waitTextEvent() {
  await b.waitUntil(() => exec(() => window.__perf.events.some((e) => e.type === "text-end")), { timeout: 10000 }).catch(() => {});
}
async function closeDoc() {
  if (viewer === "ours") {
    await exec(() => document.querySelector(".pdf-close-btn")?.click());
    await b.waitUntil(() => exec(() => !document.querySelector(".pdf-scroll")), { timeout: 20000 });
  } else {
    await exec(() => window.PDFViewerApplication.close());
    await sleep(300);
  }
}
async function currentScale() { return P("return P.currentScale()"); }
/** Set zoom: ours by toolbar Zoom-in clicks from fit-width (the UI path);
 * stock directly to the same CSS-px-per-pt value ours reached. */
async function zoomTo(key, clicks) {
  if (viewer === "ours") {
    for (let i = 0; i < clicks; i++) await exec(() => document.querySelector('button[title="Zoom in"]').click());
    await settle(20000);
    ourScales[key] = await currentScale();
  } else {
    await exec((k) => { window.PDFViewerApplication.pdfViewer.currentScale = k * 0.75; }, ourScales[key]);
    await settle(20000);
  }
  return currentScale();
}
async function fitWidth(name) {
  if (viewer === "ours") {
    await exec(() => document.querySelector('button[title="Fit width"]')?.click());
    await settle(20000);
    ourScales[`fit:${name}`] = await currentScale();
  } else {
    await exec((k) => { window.PDFViewerApplication.pdfViewer.currentScale = k * 0.75; }, ourScales[`fit:${name}`]);
    await settle(20000);
  }
}
/** Page geometry in scroll-content coordinates. */
async function pageGeom(n) {
  return P(`const list = P.pageList(); const sc = P.scroller(); const p = list.find((x) => x.n === args[0]);
    const r = p.el.getBoundingClientRect(), s = sc.getBoundingClientRect();
    return { top: r.top - s.top - sc.clientTop + sc.scrollTop + p.el.clientTop, left: r.left - s.left - sc.clientLeft + sc.scrollLeft + p.el.clientLeft,
      w: p.el.clientWidth, h: p.el.clientHeight, vw: sc.clientWidth, vh: sc.clientHeight, sh: sc.scrollHeight, sw: sc.scrollWidth,
      vx: s.left + sc.clientLeft, vy: s.top + sc.clientTop }`, n);
}
async function scrollToXY(x, y) { await P("const sc = P.scroller(); sc.scrollLeft = args[0]; sc.scrollTop = args[1];", x, y); }

// ------------------------------------------------------------ scenarios
function gestureAnalyze(extra = {}) {
  return (raw) => {
    const gs = raw.marks.find((m) => m[0] === "gesture-start" || m[0] === "wheel");
    const ge = [...raw.marks].reverse().find((m) => m[0] === "gesture-end" || m[0] === "wheel");
    const ta = gs ? gs[1] : raw.t0, tb = ge ? ge[1] : raw.frames.at(-1)?.[0];
    const sharp = firstSharp(raw.frames, tb);
    const during = windowMetrics(raw.frames, ta, tb);
    const after = windowMetrics(raw.frames, tb, sharp ?? Infinity);
    const fa = raw.frames.find((f) => f[F.t] >= ta), fb = [...raw.frames].reverse().find((f) => f[F.t] <= tb);
    return {
      during, after, toSharpAfterEndMs: ms(sharp, tb),
      speedPxS: fa && fb && fb[F.t] > fa[F.t] ? Math.round((fb[F.top] - fa[F.top]) / ((fb[F.t] - fa[F.t]) / 1000)) : null,
      ...extra,
    };
  };
}

async function scrollTrial(scenario, name, v, distFn, driver) {
  return trial(scenario, name, async () => {
    await scrollToXY(0, 0);
    await settle(20000);
    const g = await pageGeom(1);
    const dist = await distFn(g);
    await P("P.start()");
    await sleep(300);
    if (driver === "inpage") {
      await P(`return await P.scrollGesture(args[0], args[1], "y")`, v, dist);
    } else {
      const pxPerClick = await wheelStep(g);
      const clicks = Math.round(dist / pxPerClick);
      const delay = Math.max(1, Math.round((pxPerClick / v) * 1000));
      const [x, y] = cal.toScreen(g.vx + g.vw / 2, g.vy + g.vh / 2);
      xdo("mousemove", String(x), String(y));
      xdo("click", "--repeat", String(clicks), "--delay", String(delay), "5");
    }
    await sleep(200);
    await settle(20000);
    return { row: { driver, targetPxS: v, distPx: Math.round(dist) }, analyze: gestureAnalyze() };
  });
}
let wheelPx = null;
async function wheelStep(g) {
  if (wheelPx) return wheelPx;
  const [x, y] = cal.toScreen(g.vx + g.vw / 2, g.vy + g.vh / 2);
  xdo("mousemove", String(x), String(y));
  const before = await P("return P.scroller().scrollTop");
  xdo("click", "--repeat", "4", "--delay", "80", "5");
  await sleep(1000);
  const after = await P("return P.scroller().scrollTop");
  wheelPx = (after - before) / 4 || 100;
  log("wheel px/click", wheelPx);
  return wheelPx;
}

async function s0(name) {
  // Sanity: idle (blank must be ~0) and an instant jump to an unrendered page (blank must register).
  await trial("S0", "idle", async () => {
    await scrollToXY(0, 0); await settle(20000);
    await P("P.start()"); await sleep(2000);
    return { analyze: (raw) => ({ idle: windowMetrics(raw.frames, raw.t0, Infinity) }) };
  });
  await trial("S0", "jump-p15", async () => {
    const g = await pageGeom(15);
    await P("P.start()"); await sleep(300);
    const tj = await P("P.marks.push(['jump', performance.now()]); P.scroller().scrollTop = args[0]; return performance.now();", g.top);
    await settle(20000);
    return { row: {}, analyze: (raw) => ({ jumpToSharpMs: ms(firstSharp(raw.frames, tj), tj), whole: windowMetrics(raw.frames, tj, Infinity) }) };
  });
}

async function s1a(name) {
  for (const [label, v] of [["fast", FAST], ["moderate", MODERATE]]) {
    await scrollTrial("S1a", `inpage-${label}`, v, (g) => g.sh - g.vh, "inpage");
    if (THEME === "light") await scrollTrial("S1a", `wheel-${label}`, v, (g) => g.sh - g.vh, "xdotool");
  }
}

async function s1b(name) {
  // Zoomed column reading: zoom in, then for 5 column visits: jump to a column
  // top (time to sharp), pan across 300 px and down 1 viewport, back, down again.
  for (const [key, clicks] of [["z257", 7], ["z342", 10]]) {
    await fitWidth(name);
    const k = await zoomTo(key, clicks);
    await trial("S1b", `zoompan-${key}`, async () => {
      const visits = [[3, 0.06], [3, 0.5], [4, 0.06], [4, 0.5], [5, 0.06]];
      const jumps = [];
      await P("P.start()");
      for (const [pg, fx] of visits) {
        const g = await pageGeom(pg);
        const tj = await P("P.marks.push(['jump', performance.now()]); const sc = P.scroller(); sc.scrollLeft = args[0]; sc.scrollTop = args[1]; return performance.now();",
          g.left + fx * g.w - 20, g.top + 0.06 * g.h);
        await settle(5000);
        jumps.push(tj);
        await P("return await P.scrollGesture(1500, 300, 'x')");
        await P("return await P.scrollGesture(1500, args[0], 'y')", g.vh);
        await P("return await P.scrollGesture(1500, -300, 'x')");
        await P("return await P.scrollGesture(1500, args[0], 'y')", g.vh);
      }
      await settle(10000);
      return { row: { scale: k, cssPxPerPt: k },
        analyze: (raw) => {
          const toSharp = jumps.map((tj) => ms(firstSharp(raw.frames, tj), tj));
          const pans = raw.marks.filter((m) => m[0] === "gesture-start").map((m, i) => {
            const end = raw.marks.filter((x) => x[0] === "gesture-end")[i];
            return end ? windowMetrics(raw.frames, m[1], end[1]) : null;
          }).filter(Boolean);
          const sum = (k2) => Math.round(pans.reduce((a, p) => a + (p[k2] || 0), 0) * 10) / 10;
          return { jumpToSharpMs: toSharp, panBlankMs: sum("blankMs"), panStaleMs: sum("staleMs"), panOver50: sum("over50"),
            panP95: Math.max(...pans.map((p) => p.p95 || 0)), panMax: Math.max(...pans.map((p) => p.max || 0)),
            whole: windowMetrics(raw.frames, jumps[0], Infinity) };
        } };
    });
  }
  await fitWidth(name);
}

async function pointUnder(x, y) {
  // PDF-space point under client (x, y): page number + coordinates in PDF pt.
  return P(`const k = P.currentScale(); for (const p of P.pageList()) { const r = p.el.getBoundingClientRect();
      const L = r.left + p.el.clientLeft, T = r.top + p.el.clientTop;
      if (args[0] >= L && args[0] <= L + p.el.clientWidth && args[1] >= T && args[1] <= T + p.el.clientHeight)
        return { n: p.n, px: (args[0] - L) / k, py: (args[1] - T) / k, k }; } return null;`, x, y);
}
async function whereIs(pt) {
  return P(`const k = P.currentScale(); const p = P.pageList().find((x) => x.n === args[0]); const r = p.el.getBoundingClientRect();
    return { x: r.left + p.el.clientLeft + args[1] * k, y: r.top + p.el.clientTop + args[2] * k, k };`, pt.n, pt.px, pt.py);
}

async function s1c(name) {
  // Ctrl+wheel bursts (real X input), pointer mid-viewport over page 3 middle.
  await fitWidth(name);
  const g = await pageGeom(3);
  await scrollToXY(0, g.top + g.h / 2 - g.vh / 2);
  await settle(20000);
  const cx = g.vx + g.vw / 2, cy = g.vy + g.vh / 2;
  const [sx, sy] = cal.toScreen(cx, cy);
  // Two inputs: real Ctrl+wheel (each viewer's own wheel->zoom mapping) and
  // real clicks on the toolbar Zoom in/out buttons (both step by x1.1).
  // Keyboard Ctrl+= was dropped: in Tine it goes to interface zoom unless the
  // PDF pane owns focus, which changed the whole webview's zoom mid-session.
  const buttonCenter = (dir) => exec((ours, d) => {
    const el = ours ? document.querySelector(`button[title="${d === "in" ? "Zoom in" : "Zoom out"}"]`)
      : document.getElementById(d === "in" ? "zoomInButton" : "zoomOutButton");
    const r = el.getBoundingClientRect();
    return [r.left + r.width / 2, r.top + r.height / 2];
  }, viewer === "ours", dir);
  for (const input of ["wheel", "button"]) {
    for (let cycle = 1; cycle <= 3; cycle++) {
      for (const [dir, button] of [["in", "4"], ["out", "5"]]) {
        await trial("S1c", `ctrl${input}-${dir}-${cycle}`, async () => {
          xdo("mousemove", String(sx), String(sy));
          await sleep(200);
          const before = await pointUnder(cx, cy);
          await P("P.start()");
          await sleep(300);
          const tIn = await exec(() => performance.now());
          if (input === "wheel") {
            xdo("keydown", "ctrl");
            xdo("click", "--repeat", "5", "--delay", "60", button);
            xdo("keyup", "ctrl");
          } else {
            const [bx, by] = cal.toScreen(...(await buttonCenter(dir)));
            xdo("mousemove", String(bx), String(by));
            xdo("click", "--repeat", "5", "--delay", "60", "1");
            xdo("mousemove", String(sx), String(sy));
          }
          await sleep(300);
          await settle(10000);
          const afterPos = before ? await whereIs(before) : null;
          const drift = afterPos ? Math.round(Math.hypot(afterPos.x - cx, afterPos.y - cy)) : null;
          return { row: { input, anchorDriftPx: drift, scaleBefore: before?.k, scaleAfter: afterPos?.k },
            analyze: (raw) => {
              const inputs = raw.marks.filter((m) => (input === "wheel" ? m[0] === "wheel" : m[0] === "click"));
              const tFirst = inputs[0]?.[1] ?? tIn, tLast = inputs.at(-1)?.[1] ?? tIn;
              return { inputEvents: inputs.length, firstChangeMs: ms(firstChange(raw.frames, tFirst), tFirst),
                sharpFromFirstMs: ms(firstSharp(raw.frames, tLast), tFirst), sharpFromLastMs: ms(firstSharp(raw.frames, tLast), tLast),
                whole: windowMetrics(raw.frames, tFirst, Infinity) };
            } };
        });
      }
    }
  }
  await fitWidth(name);
}

async function s2(name) {
  await scrollTrial("S2", "inpage-fast-50p", FAST, async (g) => 50 * (g.sh / 714), "inpage");
  await scrollTrial("S2", "wheel-fast-50p", FAST, async (g) => 50 * (g.sh / 714), "xdotool");
  // Page-number jump: type 400 + Enter in the page field.
  await scrollToXY(0, 0); await settle(20000);
  await trial("S2", "jump-pagenumber-400", async () => {
    const sel = viewer === "ours" ? ".pdf-page-input" : "#pageNumber";
    await exec((s) => { const i = document.querySelector(s); i.focus(); i.select(); }, sel);
    await P("P.start()"); await sleep(300);
    await b.keys(["4", "0", "0", "Enter"]);
    await sleep(300); await settle(20000);
    return { analyze: (raw) => { const k = raw.marks.find((m) => m[0] === "key" && m[2] === "Enter"); const t = k?.[1];
      return { jumpToSharpMs: ms(firstSharp(raw.frames, t), t), firstChangeMs: ms(firstChange(raw.frames, t), t), whole: windowMetrics(raw.frames, t, Infinity) }; } };
  });
  // Outline jump: "III Algorithms" (page 469).
  await scrollToXY(0, 0); await settle(20000);
  if (viewer === "ours") {
    await exec(() => document.querySelector('button[title="Outline"]').click());
    await b.waitUntil(() => exec(() => !!document.querySelector(".pdf-outline-label")), { timeout: 20000 });
  } else {
    await exec(() => { const a = window.PDFViewerApplication; a.pdfSidebar.open(); a.pdfSidebar.switchView(2); });
    await b.waitUntil(() => exec(() => !!document.querySelector("#outlineView .treeItem a")), { timeout: 20000 });
    await settle(20000);
  }
  await trial("S2", "jump-outline-algorithms", async () => {
    await P("P.start()"); await sleep(300);
    const ok = await exec((ours) => {
      const els = [...document.querySelectorAll(ours ? ".pdf-outline-label" : "#outlineView .treeItem > a")];
      const el = els.find((e) => (e.textContent || "").trim() === "III Algorithms");
      if (!el) return false;
      el.click();
      return true;
    }, viewer === "ours");
    if (!ok) throw new Error("outline entry not found");
    await sleep(300); await settle(20000);
    return { analyze: (raw) => { const c = raw.marks.find((m) => m[0] === "click"); const t = c?.[1];
      return { jumpToSharpMs: ms(firstSharp(raw.frames, t), t), firstChangeMs: ms(firstChange(raw.frames, t), t), whole: windowMetrics(raw.frames, t, Infinity) }; } };
  });
  if (viewer === "stock") await exec(() => window.PDFViewerApplication.pdfSidebar.close());
}

async function s3(name) {
  await scrollTrial("S3", "inpage-moderate-20p", MODERATE, async (g) => 20 * (g.sh / 748), "inpage");
  await scrollTrial("S3", "inpage-fast-50p", FAST, async (g) => 50 * (g.sh / 748), "inpage");
}

const SCENARIOS = [
  ["S0", S1, s0], ["S1a", S1, s1a], ["S1b", S1, s1b], ["S1c", S1, s1c], ["S2", S2, s2], ["S3", S3, s3],
];

// ------------------------------------------------------------ main
try {
  const win = await sizeWindow(b, 1920, 1080);
  log("session", TAG, "window", JSON.stringify(win), "cpus", CPUS || "-", "load", loadavg());
  const scalesFile = path.join(OUT, `scales-dpr${DPR}.json`);
  if (VIEWERS[0] === "stock" && fs.existsSync(scalesFile)) Object.assign(ourScales, JSON.parse(fs.readFileSync(scalesFile, "utf8")));
  for (const v of VIEWERS) {
    viewer = v;
    if (v === "stock") {
      await b.url(`http://127.0.0.1:${port}/web/viewer.html?file=`);
      await b.waitUntil(() => exec(() => !!window.PDFViewerApplication?.pdfViewer && !!window.PDFViewerApplicationOptions), { timeout: 30000 });
      // Same CSS viewport as Tine's PDF pane; dark = same invert filter Tine applies.
      await exec((w, h, dark) => {
        const st = document.createElement("style");
        st.textContent = `#viewerContainer { right: auto !important; bottom: auto !important; width: ${w}px !important; height: ${h}px !important; }`
          + (dark ? " .pdfViewer .page canvas, .pdfViewer .textLayer { filter: invert(1); }" : "");
        document.head.appendChild(st);
      }, ourScales.vw ?? 837, ourScales.vh ?? 957, THEME === "dark");
    } else {
      await b.$(".pdf-link").waitForExist({ timeout: 60000 });
      if (THEME === "dark") await exec(() => localStorage.setItem("ls-pdf-viewer-theme", "dark"));
      // Dismiss the one-time Guide toast so it can't intercept pointer input.
      await exec(() => document.querySelectorAll(".toast button, .toast-close").forEach((x) => (x.textContent || "").includes("×") && x.click()));
    }
    await install();
    if (!cal) cal = await calibratePointer(b);
    const byPdf = new Map();
    for (const [s, pdf, fn] of SCENARIOS) if (SCEN.has(s)) { if (!byPdf.has(pdf)) byPdf.set(pdf, []); byPdf.get(pdf).push([s, fn]); }
    for (const [pdf, list] of byPdf) {
      if (!PDFS.includes(pdf)) { log("missing", pdf); continue; }
      const open = await trial("open", pdf.replace(/\.pdf$/, ""), async () => {
        const o = await openDoc(pdf);
        return { row: { settledAfterStartMs: o.settled },
          analyze: (raw) => {
            const ev = raw.events;
            const firstRaster = ev.find((e) => e.type === "render-end" || e.type === "pagerendered");
            const firstText = ev.find((e) => e.type === "text-end");
            return { openFirstRasterMs: ms(firstRaster?.t, o.t0), openSettledMs: ms(firstSharp(raw.frames, o.t0), o.t0), openFirstTextMs: ms(firstText?.t, o.t0),
              whole: windowMetrics(raw.frames, o.t0, Infinity) };
          } };
      });
      if (viewer === "ours" && pdf === S1) {
        const g = await pageGeom(1); ourScales.vw = g.vw; ourScales.vh = g.vh;
        ourScales[`fit:${pdf}`] = await currentScale();
      }
      if (viewer === "ours" && pdf !== S1) ourScales[`fit:${pdf}`] = await currentScale();
      const g = await pageGeom(1);
      if (!wheelPx) { await wheelStep(g); await scrollToXY(0, 0); await sleep(1000); await settle(20000); }
      log(viewer, pdf, "viewport", g.vw, "x", g.vh, "page", g.w, "x", g.h, "scale", await currentScale(), "open", JSON.stringify(open.openSettledMs),
        "theme", await exec(() => document.querySelector(".pdf-viewer")?.getAttribute("data-theme") ?? (getComputedStyle(document.querySelector(".pdfViewer .page canvas") || document.body).filter || "-")));
      for (const [s, fn] of list) {
        try { await fn(pdf); } catch (e) { log("SCENARIO ERROR", viewer, s, String(e).slice(0, 300)); }
      }
      await closeDoc().catch((e) => log("close error", String(e)));
    }
    if (v === "ours") fs.writeFileSync(scalesFile, JSON.stringify(ourScales));
  }
} catch (e) {
  log("FATAL", e);
  process.exitCode = 1;
} finally {
  await app.stop();
  server.close();
}
