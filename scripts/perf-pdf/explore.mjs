// Exploration: launch, open S1 in ours, print geometry; try stock navigation; calibrate wheel.
import { setTimeout as sleep } from "node:timers/promises";
import path from "node:path";
import { launchApp, sizeWindow, COLLECTOR, startStaticServer, xdo, calibratePointer } from "./app.mjs";
const CORPUS = "/aux/koutecky/logseq/tine-agents/runtime/pdf-perf-corpus";
const DIST = "/aux/koutecky/logseq/tine-agents/runtime/pdfjs-dist-4.10.38";
const dpr = Number(process.env.DPR || 1);
const { server, port } = await startStaticServer(DIST, CORPUS);
const app = await launchApp({ corpus: CORPUS, pdfs: ["arikan-0807.3917v5.pdf"], dpr, tmp: "/tmp/claude-3042/-aux-koutecky-logseq/054754e6-ffb1-4723-99f8-0c9658056a4c/scratchpad/explore", driverPort: 4620, nativePort: 4621 });
const b = app.browser;
try {
  console.log("pid", app.appPid);
  console.log("win", await sizeWindow(b, 1920, 1080));
  console.log(xdo("search", "--onlyvisible", "--name", "."));
  await b.$(".pdf-link").waitForExist({ timeout: 30000 });
  console.log(await b.execute(`return (${COLLECTOR})("ours")`));
  await b.$(".pdf-link").click();
  await b.$(".pdf-page canvas").waitForExist({ timeout: 20000 });
  await sleep(2000);
  console.log(await b.execute(() => {
    const sc = document.querySelector(".pdf-scroll"); const r = sc.getBoundingClientRect();
    const pg = document.querySelector(".pdf-page"); const pr = pg.getBoundingClientRect();
    return { sc: [r.left, r.top, r.width, r.height, sc.clientWidth, sc.clientHeight, sc.scrollHeight], page: [pr.left, pr.top, pr.width, pr.height, pg.clientLeft],
      zoom: document.querySelector(".pdf-zoom-level")?.textContent, dpr: devicePixelRatio, curScale: window.__perf.curScale,
      ev: window.__perf.events.slice(0, 12).map(e => `${e.type}:${e.page}:${Math.round(e.t)}`), sample: window.__perf.sample() };
  }));
  await b.saveScreenshot(path.join("/tmp/claude-3042/-aux-koutecky-logseq/054754e6-ffb1-4723-99f8-0c9658056a4c/scratchpad", `explore-ours-${dpr}.png`));
  const cal = await calibratePointer(b);
  console.log("cal", cal && { offX: cal.offX, offY: cal.offY, sx: cal.scaleX, sy: cal.scaleY });
  if (cal) {
    const [x, y] = cal.toScreen(1500, 600);
    xdo("mousemove", String(x), String(y));
    const before = await b.execute(() => document.querySelector(".pdf-scroll").scrollTop);
    xdo("click", "--repeat", "10", "--delay", "30", "5");
    await sleep(800);
    const after = await b.execute(() => document.querySelector(".pdf-scroll").scrollTop);
    console.log("wheel 10 clicks", before, after);
  }
  await b.url(`http://127.0.0.1:${port}/web/viewer.html?file=/pdfs/arikan-0807.3917v5.pdf`);
  await sleep(4000);
  console.log(await b.execute(() => ({ href: location.href, app: !!window.PDFViewerApplication, pages: window.PDFViewerApplication?.pdfViewer?.pagesCount,
    scale: window.PDFViewerApplication?.pdfViewer?.currentScale, dpr: devicePixelRatio, ua: navigator.userAgent })));
  await b.saveScreenshot(path.join("/tmp/claude-3042/-aux-koutecky-logseq/054754e6-ffb1-4723-99f8-0c9658056a4c/scratchpad", `explore-stock-${dpr}.png`));
  await b.url("tauri://localhost/");
  await sleep(3000);
  console.log("back", await b.execute(() => ({ href: location.href, links: document.querySelectorAll(".pdf-link").length })));
} catch (e) { console.error("ERR", e); }
finally { await app.stop(); server.close(); }
