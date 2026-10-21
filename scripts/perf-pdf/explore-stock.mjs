import { setTimeout as sleep } from "node:timers/promises";
import { launchApp, startStaticServer } from "./app.mjs";
const CORPUS = "/aux/koutecky/logseq/tine-agents/runtime/pdf-perf-corpus";
const DIST = "/aux/koutecky/logseq/tine-agents/runtime/pdfjs-dist-4.10.38";
const { server, port } = await startStaticServer(DIST, CORPUS);
const app = await launchApp({ corpus: CORPUS, pdfs: ["arikan-0807.3917v5.pdf"], dpr: 1, tmp: "/tmp/claude-3042/-aux-koutecky-logseq/054754e6-ffb1-4723-99f8-0c9658056a4c/scratchpad/explore-s", driverPort: 4640, nativePort: 4641 });
const b = app.browser;
try {
  await b.url(`http://127.0.0.1:${port}/web/viewer.html?file=`);
  await sleep(3000);
  console.log(await b.execute(() => ({ a: !!window.PDFViewerApplication, o: !!window.PDFViewerApplicationOptions, v: !!window.PDFViewerApplication?.pdfViewer, init: window.PDFViewerApplication?.initialized })));
  const r = await b.execute(async () => {
    const errs = [];
    window.addEventListener("error", (e) => errs.push(String(e.message)));
    window.addEventListener("unhandledrejection", (e) => errs.push(String(e.reason)));
    try { window.PDFViewerApplicationOptions.set("defaultZoomValue", "98.6"); window.PDFViewerApplicationOptions.set("sidebarViewOnLoad", 0);
      window.PDFViewerApplicationOptions.set("disableHistory", true); } catch (e) { errs.push("opt " + e); }
    try { await window.PDFViewerApplication.open({ url: "/pdfs/arikan-0807.3917v5.pdf" }); } catch (e) { errs.push("open " + e); }
    await new Promise((r) => setTimeout(r, 3000));
    const v = window.PDFViewerApplication.pdfViewer;
    return { errs, pages: v.pagesCount, scale: v.currentScale, n: v._pages?.length };
  });
  console.log(r);
} catch (e) { console.error("ERR", e.message); }
finally { await app.stop(); server.close(); }
