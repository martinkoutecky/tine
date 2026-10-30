/* GH #783 probe: temporary instrumentation, injected into index.html on the probe
 * branch only. Plain ES2017, no imports, so it applies unchanged to any Tine
 * version. Everything goes to console.log (logcat) with the prefix "GH783 <ms> ".
 * It also drives the setup (create the demo graph, open a tall page) and, per
 * trial, announces a tap target; the host driver (drive.sh) does the real tap. */
(function () {
  if (window.__gh783) return;
  window.__gh783 = 1;
  var T0 = performance.now();
  function ts() { return (performance.now() - T0).toFixed(1); }
  function log() {
    var a = [].slice.call(arguments).map(function (x) { return typeof x === "string" ? x : JSON.stringify(x); });
    console.log("GH783 " + ts() + " " + a.join(" "));
  }
  function r1(x) { return Math.round(x * 10) / 10; }
  function rect(el) {
    if (!el) return null;
    var r = el.getBoundingClientRect();
    return { t: r1(r.top), b: r1(r.bottom), h: r1(r.height), l: r1(r.left), w: r1(r.width) };
  }
  function scroller() { return document.querySelector(".main-content"); }
  function stack() {
    var s = (new Error().stack || "").split("\n").slice(2, 7).map(function (l) {
      return l.trim().replace(/https?:\/\/[^/]+/, "").replace(/^at /, "");
    });
    return s.join(" < ");
  }

  var sc0 = scroller();
  log("BOOT ua=" + navigator.userAgent, "dpr=" + devicePixelRatio, "inner=" + innerWidth + "x" + innerHeight,
    "vv=" + (window.visualViewport ? window.visualViewport.height : "none"), "href=" + location.href);

  // ---- every JS scroll write on .main-content / window, with its caller ----
  var lastWrite = { t: -1e9, who: "" };
  var quiet = false; // the probe's own writes
  var desc = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop");
  Object.defineProperty(Element.prototype, "scrollTop", {
    configurable: true,
    enumerable: desc.enumerable,
    get: function () { return desc.get.call(this); },
    set: function (v) {
      var before = desc.get.call(this);
      desc.set.call(this, v);
      var after = desc.get.call(this);
      if (!quiet && this.classList && this.classList.contains("main-content")) {
        var who = stack();
        lastWrite = { t: performance.now(), who: who };
        log("JSWRITE scrollTop", r1(before), "->", r1(after), "req", r1(v), "|", who);
      }
    },
  });
  ["scrollBy", "scrollTo", "scrollIntoView"].forEach(function (name) {
    var orig = Element.prototype[name];
    if (!orig) return;
    Element.prototype[name] = function () {
      var before = this.scrollTop;
      var res = orig.apply(this, arguments);
      if (!quiet) {
        lastWrite = { t: performance.now(), who: name };
        log("JSCALL el." + name, "on", (this.className || this.tagName) + "", "args", [].slice.call(arguments).map(function (a) { return typeof a === "object" ? JSON.stringify(a) : a; }), "st", r1(before), "->", r1(scroller() ? scroller().scrollTop : -1), "|", stack());
      }
      return res;
    };
  });
  ["scrollBy", "scrollTo"].forEach(function (name) {
    var orig = window[name];
    window[name] = function () {
      lastWrite = { t: performance.now(), who: "window." + name };
      log("JSCALL window." + name, [].slice.call(arguments).map(function (a) { return typeof a === "object" ? JSON.stringify(a) : a; }), "|", stack());
      return orig.apply(window, arguments);
    };
  });

  // ---- geometry snapshot ----
  var trial = 0;
  function tb0() { var t = document.querySelector("[data-mobile-keyboard-toolbar]"); return t && !t.hidden && t.style.display !== "none" ? t.getBoundingClientRect().top : null; }
  function snap(label) {
    var m = scroller();
    var vv = window.visualViewport;
    var ed = document.activeElement;
    var isEd = ed && ed.tagName === "TEXTAREA";
    var root = document.documentElement;
    var o = {
      l: label, tr: trial, ih: innerHeight, vvh: vv ? r1(vv.height) : null, vvt: vv ? r1(vv.offsetTop) : null,
      dock: root.hasAttribute("data-mobile-editing-dock"),
      lift: root.style.getPropertyValue("--mobile-kb-toolbar-lift") || null,
    };
    if (m) {
      var r = m.getBoundingClientRect();
      o.mc = { top: r1(r.top), bot: r1(r.bottom), ch: m.clientHeight, st: r1(m.scrollTop), sh: m.scrollHeight };
    }
    if (isEd) {
      var er = ed.getBoundingClientRect();
      o.ed = { t: r1(er.top), b: r1(er.bottom), h: r1(er.height) };
      if (m) o.edAboveTop = r1(er.top - m.getBoundingClientRect().top); // < 0 = above the scroller top
      var ct = caretTop(ed);
      if (ct !== null && m) { o.caretY = r1(ct); o.caretAboveTop = r1(ct - m.getBoundingClientRect().top); o.caretToTb = tb0() === null ? null : r1(tb0() - ct); }
    }
    if (m) {
      var mt = m.getBoundingClientRect().top, ml = [];
      [].forEach.call(m.querySelectorAll("video, img.media-embed, .asset-img, img"), function (el) {
        var rr = el.getBoundingClientRect();
        if (rr.height > 20 && rr.bottom > mt - 50 && rr.top < mt + m.clientHeight + 50 && ml.length < 4) ml.push({ g: el.tagName[0], t: r1(rr.top - mt), h: r1(rr.height), w: r1(rr.width) });
      });
      o.media = ml; o.nb = m.querySelectorAll(".block-content").length;
      var pins = document.querySelectorAll("[data-sticky-parent]"), pn = 0;
      o.pinsN = pins.length;
    }
    var tb = document.querySelector("[data-mobile-keyboard-toolbar]");
    if (tb) { var tr = tb.getBoundingClientRect(); o.tb = { t: r1(tr.top), h: r1(tr.height), vis: !tb.hidden && tb.style.display !== "none" }; }
    var nav = document.querySelector(".mobile-navigation-toolbar");
    if (nav) o.nav = { h: r1(nav.getBoundingClientRect().height), disp: getComputedStyle(nav).display };
    log("SNAP", o);
  }
  window.__gh783snap = snap;
  // Viewport y of the caret row of a focused textarea, by a hidden mirror (the app's own helper is not reachable from here).
  function caretTop(ed) {
    try {
      var cs = getComputedStyle(ed);
      var d = document.createElement("div");
      ["fontFamily","fontSize","fontWeight","fontStyle","letterSpacing","lineHeight","textTransform","wordSpacing","textIndent","paddingTop","paddingRight","paddingBottom","paddingLeft","borderTopWidth","borderRightWidth","borderBottomWidth","borderLeftWidth","boxSizing","tabSize","wordBreak","overflowWrap"].forEach(function (k) { d.style[k] = cs[k]; });
      d.style.position = "absolute"; d.style.visibility = "hidden"; d.style.left = "-9999px"; d.style.top = "0";
      d.style.whiteSpace = "pre-wrap"; d.style.width = ed.getBoundingClientRect().width + "px";
      d.textContent = ed.value.slice(0, ed.selectionEnd);
      var mk = document.createElement("span"); mk.textContent = "\u200b"; d.appendChild(mk);
      document.body.appendChild(d);
      var y = mk.offsetTop;
      document.body.removeChild(d);
      return ed.getBoundingClientRect().top + y - ed.scrollTop;
    } catch (e) { return null; }
  }

  // ---- events ----
  var tFocus = null;
  function ev(name) { return function (e) { log("EV " + name, e && e.detail !== undefined ? "detail=" + e.detail : ""); snap("after-" + name); }; }
  window.addEventListener("tine-ime", ev("tine-ime"), true);
  window.addEventListener("resize", ev("window-resize"), true);
  if (window.visualViewport) {
    window.visualViewport.addEventListener("resize", ev("vv-resize"));
    window.visualViewport.addEventListener("scroll", ev("vv-scroll"));
  }
  document.addEventListener("pointerdown", function (e) {
    log("EV pointerdown", "x=" + r1(e.clientX), "y=" + r1(e.clientY), "target=" + (e.target.className || e.target.tagName));
  }, true);
  document.addEventListener("focusin", function (e) {
    var t = e.target;
    log("EV focusin", t.tagName, (t.className || "") + "");
    if (t.tagName === "TEXTAREA") {
      tFocus = performance.now();
      snap("focusin");
      [60, 200, 450, 800, 1400, 2500, 4000].forEach(function (ms) {
        setTimeout(function () { snap("focus+" + ms); }, ms);
      });
    }
  }, true);
  document.addEventListener("focusout", function (e) { log("EV focusout", e.target.tagName); }, true);
  document.addEventListener("scroll", function (e) {
    var t = e.target;
    if (!t || !t.classList || !t.classList.contains("main-content")) return;
    var age = performance.now() - lastWrite.t;
    log("EV scroll", "st=" + r1(t.scrollTop), "sinceJsWrite=" + r1(age) + "ms", age < 40 ? "(js)" : "(NATIVE?)");
  }, true);
  new MutationObserver(function (list) {
    list.forEach(function (m) {
      if (m.attributeName === "data-mobile-editing-dock") { log("EV dock", document.documentElement.hasAttribute("data-mobile-editing-dock") ? "SET" : "CLEARED"); snap("after-dock"); }
    });
  }).observe(document.documentElement, { attributes: true, attributeFilter: ["data-mobile-editing-dock"] });
  var roTarget = null;
  var ro = new ResizeObserver(function (entries) {
    entries.forEach(function (en) { log("EV RO", (en.target.className || en.target.tagName) + "", "h=" + r1(en.contentRect.height), "st=" + (scroller() ? r1(scroller().scrollTop) : "-")); });
  });

  // ---- driver: setup and trials ----
  var plan = [
    { f: 0.80, top: true },
    { f: 0.55, top: true },
    { f: 0.90, top: true },
    { f: 0.70, top: false },
    { tall: 0, f: 0.55, within: 0.0 },
    { tall: 0, f: 0.25, within: 0.4 },
    { tall: 1, f: 0.60, within: 0.0 },
  ];
  var mediaPlan = [
    { find: "Kitchen", f: 0.12 },
    { find: "Bedroom", jump: 0.55, f: 0.12 },
    { find: "Between the dishwasher", jump: 0.55, f: 0.15 },
    { find: "Balcony", jump: 0.8, f: 0.12 },
    { find: "Between the dishwasher", jump: 0.8, f: 0.35 },
    { find: "Study", jump: 0.95, f: 0.15 },
    { find: "Cabinets", jump: 0.3, f: 0.2 },
  ];
  var state = "boot";
  var created = false;
  var confirmed = false;
  var navTries = 0;
  var lastLog = 0;

  function clickByText(re, tags) {
    var els = [].slice.call(document.querySelectorAll(tags || "button, a, [role=button]"));
    for (var i = 0; i < els.length; i++) {
      if (re.test(els[i].textContent || "")) { els[i].click(); return els[i]; }
    }
    return null;
  }
  function pickTarget(f) {
    var m = scroller();
    var mr = m.getBoundingClientRect();
    var want = mr.top + f * m.clientHeight;
    var blocks = [].slice.call(m.querySelectorAll(".block-content, .block-content-wrapper"));
    var best = null, bestD = 1e9;
    blocks.forEach(function (b) {
      var r = b.getBoundingClientRect();
      if (r.height < 14 || r.bottom > mr.bottom - 20 || r.top < mr.top + 10) return;
      var y = r.top + Math.min(12, r.height / 2);
      var d = Math.abs(y - want);
      if (d >= bestD) return;
      var xs = [r.left + 40, r.left + 80, r.left + 120, r.left + r.width * 0.5];
      for (var i = 0; i < xs.length; i++) {
        var hit = document.elementFromPoint(xs[i], y);
        if (!hit || hit.closest("a, button, input, textarea, svg, img, .page-ref, .tag, [data-ref], .block-control, .bullet-container, .bullet")) continue;
        if (!hit.closest(".block-content, .block-content-wrapper")) continue;
        best = { cx: xs[i], cy: y, text: (hit.textContent || "").slice(0, 30).replace(/\s+/g, " ") };
        bestD = d;
        break;
      }
    });
    return best;
  }

  function pickTall(p) {
    var m = scroller();
    var mr = m.getBoundingClientRect();
    var tall = [].filter.call(m.querySelectorAll(".block-content"), function (b) { return b.getBoundingClientRect().height >= 250; });
    var b = tall[p.tall];
    if (!b) return null;
    var r = b.getBoundingClientRect();
    quiet = true; m.scrollTop += r.top - (mr.top + p.f * m.clientHeight); quiet = false;
    r = b.getBoundingClientRect();
    var y = r.top + Math.max(14, p.within * r.height);
    var xs = [r.left + 60, r.left + 120, r.left + 40];
    for (var i = 0; i < xs.length; i++) {
      var hit = document.elementFromPoint(xs[i], y);
      if (hit && hit.closest(".block-content") === b && !hit.closest("a, button, svg, img")) return { cx: xs[i], cy: y, text: "tall" + p.tall + " h=" + r1(r.height) };
    }
    return null;
  }

  function pickFind(p) {
    var m = scroller();
    var mr = m.getBoundingClientRect();
    var blocks = [].filter.call(m.querySelectorAll(".block-content"), function (b) { return (b.textContent || "").trim().indexOf(p.find) === 0; });
    var inView = blocks.filter(function (b) { var r0 = b.getBoundingClientRect(); return r0.top >= mr.top && r0.bottom <= mr.bottom; });
    var b = inView[0] || blocks[0];
    if (!b) return null;
    var r = b.getBoundingClientRect();
    quiet = true; m.scrollTop += r.top - (mr.top + p.f * m.clientHeight); quiet = false;
    r = b.getBoundingClientRect();
    var y = r.top + Math.min(12, r.height / 2);
    var xs = [r.left + 60, r.left + 30, r.left + 100];
    for (var i = 0; i < xs.length; i++) {
      var hit = document.elementFromPoint(xs[i], y);
      if (hit && hit.closest(".block-content") === b && !hit.closest("a, button, svg, img, video")) return { cx: xs[i], cy: y, text: p.find + " f=" + p.f };
    }
    return null;
  }

  function startTrial() {
    if (!plan.mediaChecked) {
      plan.mediaChecked = true;
      var hasK = [].some.call(document.querySelectorAll(".block-content"), function (b) { return (b.textContent || "").trim().indexOf("Kitchen") === 0; });
      if (hasK) { plan = mediaPlan; log("MEDIA-PLAN"); }
    }
    var p = plan[trial];
    if (!p) { state = "finished"; log("ALLDONE"); return; }
    trial += 1;
    var m = scroller();
    if (p.jump !== undefined) { quiet = true; m.scrollTop = p.jump * (m.scrollHeight - m.clientHeight); quiet = false; }
    if (p.top) { quiet = true; m.scrollTop = 0; quiet = false; }
    setTimeout(function () {
      var tg = p.find ? pickFind(p) : (p.tall !== undefined ? pickTall(p) : pickTarget(p.f));
      var mr = scroller().getBoundingClientRect();
      if (!tg) { log("TARGET-NONE trial=" + trial); snap("no-target"); setTimeout(startTrial, 1000); return; }
      snap("pre-tap");
      log("TARGET trial=" + trial, "f=" + p.f, "cx=" + r1(tg.cx), "cy=" + r1(tg.cy), "dpr=" + devicePixelRatio, "mcTop=" + r1(mr.top), "text=" + JSON.stringify(tg.text));
      var waited = 0;
      var iv = setInterval(function () {
        waited += 250;
        var a = document.activeElement;
        if (a && a.tagName === "TEXTAREA" && tFocus && performance.now() - tFocus < 60000) {
          clearInterval(iv);
          // let the snapshots (up to +4000) run, then leave the editor
          setTimeout(function () {
            log("TRIAL-DONE trial=" + trial);
            tFocus = null;
            try { document.activeElement.blur(); } catch (e) {}
            setTimeout(startTrial, 3500);
          }, 6500);
        } else if (waited > 25000) {
          clearInterval(iv); log("TRIAL-NOEDIT trial=" + trial); snap("noedit"); setTimeout(startTrial, 500);
        }
      }, 250);
    }, 1200);
  }

  setInterval(function () {
    var now = performance.now();
    [].forEach.call(document.querySelectorAll(".toast-close"), function (b) { b.click(); });
    if (now - lastLog > 5000) { lastLog = now; log("STATE", state, "main=" + !!scroller(), "blocks=" + document.querySelectorAll(".block-content").length); }
    if (state === "boot") {
      if (!created) {
        var b = clickByText(/Create a new graph/i);
        if (b) { created = true; log("CLICKED create-graph"); return; }
      } else if (!confirmed) {
        var b2 = clickByText(/^\s*Create graph\s*$/i);
        if (b2) { confirmed = true; log("CLICKED confirm-create"); return; }
      }
      var m = scroller();
      if (m && document.querySelectorAll(".block-content").length > 5) {
        if (!roTarget) { roTarget = m; ro.observe(m); }
        state = "settle";
        setTimeout(function () { state = "tall"; }, 3000);
      }
    } else if (state === "tall") {
      var m2 = scroller();
      var tall = m2 && (m2.scrollHeight - m2.clientHeight);
      if (tall >= 900) { state = "running"; log("TALL-OK", "scrollable=" + tall, "ch=" + m2.clientHeight); startTrial(); return; }
      if (navTries++ > 6) { state = "running"; log("TALL-FAIL scrollable=" + tall); startTrial(); return; }
      var l = clickByText(/Tips|Queries|Roadmap|Guide|showcase/i, "a");
      log("NAV try " + navTries, l ? "clicked " + (l.textContent || "").slice(0, 40) : "no link", "scrollable=" + tall);
      state = "navwait";
      setTimeout(function () { state = "tall"; }, 3500);
    }
  }, 500);
})();
