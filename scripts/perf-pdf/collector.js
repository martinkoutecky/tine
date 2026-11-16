// PDF viewer perf collector (PDF perf harness, docs/pdf-perf-harness.md).
// Injected into the page. Installs window.__perf with a per-rAF sampler that
// classifies every visible page region as BLANK (no completed raster), STALE
// (a completed raster at an older scale, or under-resolved while high-zoom
// tiles are pending) or SHARP (rasterized at the current scale at the viewer's
// own target resolution). Works for Tine's viewer (via the __tinePdfProbe
// events) and the stock pdf.js viewer (via PDFViewerApplication state + the
// eventBus "pagerendered" event).
(function install(mode) {
  if (window.__perf && window.__perf.mode === mode) return "already";
  const P = {
    mode,
    running: false,
    frames: [],
    events: [],
    longtasks: [],
    marks: [],
    pages: new Map(), // ours: page -> { complete, scale, ratio }
    tiles: new Map(), // ours: key -> { page, scale, x, y, w, h, ratio, done }
    rendered: new WeakMap(), // stock: canvas -> { scale }
    curScale: null,
    gesture: null,
    longtaskSupported: false,
    pointer: null,
  };
  window.__perf = P;
  const now = () => performance.now();

  // ---- ours: probe events -> state ---------------------------------------
  if (mode === "ours") {
    window.__tinePdfProbe = (e) => {
      P.events.push(e);
      const st = P.pages.get(e.page) || { complete: false, scale: null, ratio: 0 };
      switch (e.type) {
        case "render-start": st.complete = false; st.scale = e.scale; st.ratio = e.ratio; P.pages.set(e.page, st);
          if (P.curScale == null) P.curScale = e.scale; break;
        case "render-end": st.complete = true; st.scale = e.scale; st.ratio = e.ratio; P.pages.set(e.page, st); break;
        case "free": P.pages.delete(e.page); break;
        case "zoom-input": P.curScale = e.scale; break;
        case "tile-start": P.tiles.set(e.key, { page: e.page, scale: e.scale, x: e.x, y: e.y, w: e.w, h: e.h, ratio: e.ratio, done: false }); break;
        case "tile-end": { const t = P.tiles.get(e.key); if (t) t.done = true; break; }
        case "tile-remove": P.tiles.delete(e.key); break;
      }
    };
  }

  // ---- stock: eventBus hooks ---------------------------------------------
  function hookStock() {
    const app = window.PDFViewerApplication;
    if (!app || !app.eventBus || P.hooked === app.eventBus) return;
    P.hooked = app.eventBus;
    const bus = app.eventBus;
    bus._on("pagerendered", (e) => {
      const pv = e.source;
      if (!e.cssTransform && pv && pv.canvas) P.rendered.set(pv.canvas, { scale: pv.scale, sx: pv.outputScale && pv.outputScale.sx });
      P.events.push({ type: e.cssTransform ? "pagerendered-css" : "pagerendered", page: e.pageNumber, t: now(), scale: pv && pv.scale * 4 / 3, error: !!e.error });
    });
    bus._on("pagerender", (e) => P.events.push({ type: "pagerender", page: e.pageNumber, t: now() }));
    bus._on("textlayerrendered", (e) => P.events.push({ type: "text-end", page: e.pageNumber, t: now() }));
    bus._on("scalechanging", (e) => P.events.push({ type: "scalechanging", page: 0, t: now(), scale: e.scale * 4 / 3 }));
    bus._on("pagesinit", () => P.events.push({ type: "pagesinit", page: 0, t: now() }));
    bus._on("documentloaded", () => P.events.push({ type: "documentloaded", page: 0, t: now() }));
  }
  P.hookStock = hookStock;
  if (mode === "stock") hookStock();

  // ---- longtask ------------------------------------------------------------
  try {
    const types = (PerformanceObserver.supportedEntryTypes || []);
    P.supportedEntryTypes = [...types];
    if (types.includes("longtask")) {
      P.longtaskSupported = true;
      new PerformanceObserver((list) => {
        for (const e of list.getEntries()) if (P.running) P.longtasks.push([e.startTime, e.duration]);
      }).observe({ type: "longtask", buffered: false });
    }
  } catch (err) { P.longtaskError = String(err); }

  // ---- input timestamps (wheel / key), capture phase -----------------------
  window.addEventListener("wheel", (e) => {
    if (P.running) P.marks.push(["wheel", now(), e.ctrlKey ? 1 : 0, e.deltaY, e.clientX, e.clientY]);
  }, { capture: true, passive: true });
  window.addEventListener("keydown", (e) => { if (P.running) P.marks.push(["key", now(), e.key]); }, { capture: true });
  window.addEventListener("click", (e) => { if (P.running) P.marks.push(["click", now(), String(e.target && e.target.className || "")]); }, { capture: true });
  window.addEventListener("mousemove", (e) => { P.pointer = [e.clientX, e.clientY, e.screenX, e.screenY, now()]; }, { capture: true, passive: true });

  // ---- geometry ---------------------------------------------------------------
  function scroller() {
    return mode === "ours" ? document.querySelector(".pdf-scroll") : document.getElementById("viewerContainer");
  }
  P.scroller = scroller;
  // [{ n, el, view? }] in document order
  function pageList() {
    if (mode === "ours") {
      const sc = scroller();
      if (!sc) return [];
      if (P._pl && P._plRoot === sc && P._plLen === sc.childElementCount) return P._pl;
      const list = [...sc.querySelectorAll(":scope > .pdf-page")].map((el) => ({ n: Number(el.dataset.page), el }));
      P._pl = list; P._plRoot = sc; P._plLen = sc.childElementCount;
      return list;
    }
    const v = window.PDFViewerApplication && window.PDFViewerApplication.pdfViewer;
    if (!v || !v._pages) return [];
    if (P._pl && P._plSrc === v._pages && P._pl.length === v._pages.length) return P._pl;
    P._pl = v._pages.map((pv) => ({ n: pv.id, el: pv.div, view: pv }));
    P._plSrc = v._pages;
    return P._pl;
  }
  P.pageList = pageList;
  function currentScale() {
    if (mode === "ours") return P.curScale;
    const v = window.PDFViewerApplication && window.PDFViewerApplication.pdfViewer;
    return v ? v.currentScale * 4 / 3 : null; // CSS px per PDF pt
  }
  P.currentScale = currentScale;
  // Binary-search the first page whose bottom is below the scroller top.
  function visiblePages(sc, list) {
    if (!list.length) return [];
    const top = sc.scrollTop, bottom = top + sc.clientHeight;
    // Offsets in scroll-content coordinates, anchored on the first page.
    const first = list[0].el;
    const firstTop = first.getBoundingClientRect().top - (sc.getBoundingClientRect().top + sc.clientTop) + sc.scrollTop;
    const off = (el) => el.offsetTop - first.offsetTop + firstTop;
    let lo = 0, hi = list.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      const el = list[mid].el;
      if (off(el) + el.offsetHeight < top) lo = mid + 1; else hi = mid;
    }
    const out = [];
    for (let i = Math.max(0, lo - 1); i < list.length; i++) {
      const el = list[i].el;
      if (off(el) > bottom) break;
      out.push(list[i]);
    }
    return out;
  }
  const dpr = () => window.devicePixelRatio || 1;

  // Classify one page's visible rectangle. Returns [blank, stale, sharp] areas (CSS px^2).
  function classifyOurs(p, vis, pageRect, k) {
    const st = P.pages.get(p.n);
    const area = (vis.right - vis.left) * (vis.bottom - vis.top);
    // No canvas, or a re-raster in flight (renderPage assigns canvas.width
    // first, which clears the old bitmap): no completed raster -> blank,
    // except where finished current-scale tiles cover it.
    if (!st || !st.complete) return tileSplit(p, vis, pageRect, k, area, "blank");
    const atScale = Math.abs(st.scale - k) < 1e-6;
    const target = Math.min(dpr(), 2) * 0.98;
    if (atScale && (k <= 3 || st.ratio >= target)) return [0, 0, area];
    if (!atScale && k <= 3) return [0, area, 0];
    return tileSplit(p, vis, pageRect, k, area, "stale");
  }
  // At high zoom: cells covered by a finished tile at the current scale are sharp.
  function tileSplit(p, vis, pageRect, k, area, fallback) {
    if (k == null || k <= 3) return fallback === "blank" ? [area, 0, 0] : [0, area, 0];
    let sharp = 0;
    for (const t of P.tiles.values()) {
      if (t.page !== p.n || !t.done || Math.abs(t.scale - k) > 1e-6) continue;
      const l = Math.max(vis.left, pageRect.left + t.x), r = Math.min(vis.right, pageRect.left + t.x + t.w);
      const tp = Math.max(vis.top, pageRect.top + t.y), b = Math.min(vis.bottom, pageRect.top + t.y + t.h);
      if (r > l && b > tp) sharp += (r - l) * (b - tp);
    }
    sharp = Math.min(sharp, area);
    return fallback === "blank" ? [area - sharp, 0, sharp] : [0, area - sharp, sharp];
  }
  function classifyStock(p, vis) {
    const area = (vis.right - vis.left) * (vis.bottom - vis.top);
    const pv = p.view;
    const domCanvas = pv.div.querySelector(".canvasWrapper canvas");
    if (!domCanvas || !domCanvas.isConnected) return [area, 0, 0];
    const rec = P.rendered.get(domCanvas);
    if (!rec) return [area, 0, 0]; // first raster still drawing progressively
    if (Math.abs(rec.scale - pv.scale) < 1e-6 && pv.scale === window.PDFViewerApplication.pdfViewer.currentScale) return [0, 0, area];
    return [0, area, 0];
  }

  function sample(t) {
    const sc = scroller();
    if (!sc) return null;
    const R = sc.getBoundingClientRect();
    const vp = { left: R.left + sc.clientLeft, top: R.top + sc.clientTop };
    vp.right = vp.left + sc.clientWidth; vp.bottom = vp.top + sc.clientHeight;
    const k = currentScale();
    let blank = 0, stale = 0, sharp = 0, visArea = 0;
    const list = pageList();
    const vps = visiblePages(sc, list);
    const blankPages = [];
    for (const p of vps) {
      const r = p.el.getBoundingClientRect();
      const pr = { left: r.left + p.el.clientLeft, top: r.top + p.el.clientTop };
      pr.right = pr.left + p.el.clientWidth; pr.bottom = pr.top + p.el.clientHeight;
      const vis = { left: Math.max(pr.left, vp.left), top: Math.max(pr.top, vp.top), right: Math.min(pr.right, vp.right), bottom: Math.min(pr.bottom, vp.bottom) };
      if (vis.right <= vis.left || vis.bottom <= vis.top) continue;
      const c = mode === "ours" ? classifyOurs(p, vis, pr, k) : classifyStock(p, vis);
      blank += c[0]; stale += c[1]; sharp += c[2];
      visArea += c[0] + c[1] + c[2];
      if (c[0] > 0) blankPages.push(p.n);
    }
    let canvasPx = 0, canvases = 0;
    for (const c of document.getElementsByTagName("canvas")) { canvasPx += c.width * c.height; canvases++; }
    const vpArea = (vp.right - vp.left) * (vp.bottom - vp.top);
    return [t, blank, stale, sharp, visArea, vpArea, canvasPx, canvases, sc.scrollTop, sc.scrollLeft, k, blankPages.join(" ")];
  }
  P.sample = () => sample(now());

  function loop(t) {
    if (!P.running) return;
    if (P.gesture) {
      try { P.gesture(t); } catch (err) { P.gestureError = String(err); P.gesture = null; }
    }
    const c0 = now();
    const s = sample(t);
    if (s) { s.push(Math.round((now() - c0) * 100) / 100); P.frames.push(s); }
    requestAnimationFrame(loop);
  }
  P.start = () => {
    P.frames = []; P.events = []; P.longtasks = []; P.marks = [];
    if (mode === "stock") hookStock();
    P.running = true; P.t0 = now();
    requestAnimationFrame(loop);
    return P.t0;
  };
  P.stop = () => { P.running = false; P.gesture = null; return P.frames.length; };
  P.dump = () => ({ frames: P.frames, events: P.events, longtasks: P.longtasks, marks: P.marks, t0: P.t0, gestureError: P.gestureError });
  // Whole visible area sharp right now?
  P.settled = () => { const s = sample(now()); return !!s && s[4] > 0 && s[1] === 0 && s[2] === 0; };

  // ---- in-page deterministic gestures --------------------------------------
  // Scroll the scroller along `axis` at v px/s for `dist` px (sign = direction).
  P.scrollGesture = (v, dist, axis = "y") => new Promise((resolve) => {
    const sc = scroller();
    const prop = axis === "y" ? "scrollTop" : "scrollLeft";
    const start = sc[prop];
    const t0 = now();
    const dur = Math.abs(dist) / v * 1000;
    P.marks.push(["gesture-start", t0, axis, v, dist]);
    P.gesture = () => {
      const f = Math.min(1, (now() - t0) / dur);
      sc[prop] = start + dist * f;
      if (f >= 1) { P.gesture = null; P.marks.push(["gesture-end", now(), sc[prop]]); resolve({ start, end: sc[prop], ms: now() - t0 }); }
    };
  });
  P.waitSettled = (timeoutMs) => new Promise((resolve) => {
    const t0 = now();
    const tick = () => {
      if (P.settled()) { P.marks.push(["settled", now()]); return resolve(now() - t0); }
      if (now() - t0 > timeoutMs) return resolve(null);
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  return "installed";
})
