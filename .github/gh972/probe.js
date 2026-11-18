/* GH #972 probe: temporary instrumentation injected into index.html on the probe
 * branch only. Plain ES2017. Creates the demo graph, then walks the ways a mobile
 * drawer can close (both panels) and logs, after each, the scroll offsets of the
 * app shells and the top bar position. Console output goes to logcat with the
 * prefix "GH972 <ms> ". The host driver (drive.sh) performs the real taps. */
(function () {
  if (window.__gh972) return;
  window.__gh972 = 1;
  var T0 = performance.now();
  function ts() { return (performance.now() - T0).toFixed(1); }
  function log() {
    var a = [].slice.call(arguments).map(function (x) { return typeof x === "string" ? x : JSON.stringify(x); });
    console.log("GH972 " + ts() + " " + a.join(" "));
  }
  function r1(x) { return Math.round(x * 10) / 10; }
  function q(s) { return document.querySelector(s); }
  function cls(el) { return el ? ((el.className && el.className.baseVal === undefined ? el.className : el.tagName) + "").slice(0, 60) : "none"; }
  function stack() {
    return (new Error().stack || "").split("\n").slice(2, 7).map(function (l) {
      return l.trim().replace(/https?:\/\/[^/]+/, "").replace(/^at /, "");
    }).join(" < ");
  }
  var SHELLS = [".app-container", ".main-container", ".content-row", ".drawer-workspace", ".main-content"];
  function snap(label) {
    var o = { doc: r1(document.scrollingElement.scrollTop) + "," + r1(document.scrollingElement.scrollLeft) };
    SHELLS.forEach(function (s) {
      var e = q(s);
      o[s] = e ? r1(e.scrollTop) + "," + r1(e.scrollLeft) + " sh=" + e.scrollHeight + " ch=" + e.clientHeight : null;
    });
    var tb = q(".topbar");
    var vv = window.visualViewport;
    log("SNAP " + label, JSON.stringify({
      topbarTop: tb ? r1(tb.getBoundingClientRect().top) : null,
      mainTop: q(".main-content") ? r1(q(".main-content").getBoundingClientRect().top) : null,
      inner: innerWidth + "x" + innerHeight,
      vv: vv ? r1(vv.width) + "x" + r1(vv.height) + "@" + r1(vv.offsetTop) : null,
      active: cls(document.activeElement),
      drawer: (q(".app-container") || {}).dataset ? q(".app-container").dataset.activeDrawer : null,
      sc: o,
    }));
  }
  // Every focus() call, with its options and caller (the suspected trigger).
  var focus0 = HTMLElement.prototype.focus;
  HTMLElement.prototype.focus = function (opts) {
    log("FOCUS", cls(this), "opts=" + JSON.stringify(opts || null), "|", stack());
    var before = SHELLS.map(function (s) { var e = q(s); return e ? e.scrollTop : null; });
    var res = focus0.apply(this, arguments);
    var after = SHELLS.map(function (s) { var e = q(s); return e ? e.scrollTop : null; });
    if (before.join() !== after.join()) log("FOCUS-SCROLLED", cls(this), before.join("/"), "->", after.join("/"));
    return res;
  };
  document.addEventListener("scroll", function (e) {
    var t = e.target;
    if (t === document) { log("SCROLL document", r1(document.scrollingElement.scrollTop)); return; }
    if (t.matches && t.matches(SHELLS.slice(0, 4).join(","))) log("SCROLL shell", cls(t), "top=" + r1(t.scrollTop), "active=" + cls(document.activeElement));
  }, true);

  log("BOOT ua=" + navigator.userAgent, "dpr=" + devicePixelRatio, "inner=" + innerWidth + "x" + innerHeight, "vv=" + (window.visualViewport ? window.visualViewport.height : "none"), "href=" + location.href);

  function clickByText(re, tags, root) {
    var els = [].slice.call((root || document).querySelectorAll(tags || "button, a, [role=button]"));
    for (var i = 0; i < els.length; i++) if (re.test(els[i].textContent || "")) { els[i].click(); return els[i]; }
    return null;
  }
  function center(el) {
    if (!el) return null;
    var r = el.getBoundingClientRect();
    return r.width > 0 ? { cx: r.left + r.width / 2, cy: r.top + r.height / 2 } : null;
  }
  function byText(re, root, tags) {
    var els = [].slice.call(document.querySelectorAll(root + " " + (tags || "button, a, [role=button], .sidebar-item, li, div")));
    for (var i = 0; i < els.length; i++) if (re.test((els[i].textContent || "").trim()) && els[i].children.length < 4) { var c = center(els[i]); if (c) return c; }
    return null;
  }
  var isOpen = function (side) { return !!q('[data-mobile-drawer-panel="' + side + '"]') && (q(".app-container") || { dataset: {} }).dataset.activeDrawer === side; };
  var isClosed = function () { return !q("[data-mobile-drawer-panel][data-drawer-motion]") || (q(".app-container").dataset.activeDrawer === "" && !q('[data-mobile-drawer-scrim]')); };
  var closedNow = function () { return q(".app-container") && q(".app-container").dataset.activeDrawer === "" && !q("[data-mobile-drawer-scrim]"); };
  function openL() { return center(q('header.topbar [title="Toggle sidebar (t l)"]')); }
  function openR() { return center(q('header.topbar [title^="Toggle right sidebar"]')); }
  function scrimL() { return { cx: innerWidth - 8, cy: Math.round(innerHeight * 0.5) }; }
  function scrimR() { return { cx: 8, cy: Math.round(innerHeight * 0.5) }; }
  function closeL() { return center(q('[aria-label^="Close navigation"]')); }
  function closeR() { return center(q('.right-sidebar [aria-label*="lose"]')) || center(q(".right-sidebar .rs-close")); }
  function navL() { return byText(/^All pages$/, ".left-sidebar") || byText(/^Journals$/, ".left-sidebar"); }
  var BACK = "BACK";
  var steps = [
    { name: "L-open1", find: openL, until: function () { return isOpen("left"); } },
    { name: "L-scrim", find: scrimL, until: closedNow },
    { name: "L-open2", find: openL, until: function () { return isOpen("left"); } },
    { name: "L-button", find: closeL, until: closedNow },
    { name: "L-open3", find: openL, until: function () { return isOpen("left"); } },
    { name: "L-back", find: BACK, until: closedNow },
    { name: "L-open4", find: openL, until: function () { return isOpen("left"); } },
    { name: "L-nav", find: navL, until: closedNow },
    { name: "R-open1", find: openR, until: function () { return isOpen("right"); } },
    { name: "R-scrim", find: scrimR, until: closedNow },
    { name: "R-open2", find: openR, until: function () { return isOpen("right"); } },
    { name: "R-button", find: closeR, until: closedNow },
    { name: "R-open3", find: openR, until: function () { return isOpen("right"); } },
    { name: "R-back", find: BACK, until: closedNow },
  ];
  var created = false, confirmed = false, state = "boot", idx = 0, lastLog = 0;

  function runStep() {
    var s = steps[idx];
    if (!s) { state = "finished"; snap("final"); log("ALLDONE"); return; }
    var n = idx + 1;
    // dismiss toasts that would intercept taps
    [].forEach.call(document.querySelectorAll(".toast-close"), function (b) { b.click(); });
    var tg = s.find === BACK ? BACK : s.find();
    if (!tg) { log("TARGET-NONE step=" + n + " name=" + s.name); snap("no-target-" + s.name); idx++; setTimeout(runStep, 800); return; }
    snap("pre-" + s.name);
    if (tg === BACK) log("KEY step=" + n + " name=" + s.name + " code=4");
    else log("TARGET step=" + n + " name=" + s.name, "cx=" + r1(tg.cx), "cy=" + r1(tg.cy), "dpr=" + devicePixelRatio);
    var waited = 0;
    var iv = setInterval(function () {
      waited += 250;
      if (s.until()) {
        clearInterval(iv);
        // settle past the 300/400 ms slide and fade, then measure
        [50, 400, 1200].forEach(function (ms) { setTimeout(function () { snap("post-" + s.name + "+" + ms); }, ms); });
        setTimeout(function () { log("STEP-DONE step=" + n + " name=" + s.name); idx++; runStep(); }, 1500);
      } else if (waited > 15000) {
        clearInterval(iv); log("STEP-TIMEOUT step=" + n + " name=" + s.name); snap("timeout-" + s.name);
        // force a clean state for the next step
        setTimeout(function () { idx++; runStep(); }, 500);
      }
    }, 250);
  }

  setInterval(function () {
    var now = performance.now();
    [].forEach.call(document.querySelectorAll(".toast-close"), function (b) { b.click(); });
    if (now - lastLog > 5000) { lastLog = now; log("STATE", state, "topbar=" + !!q(".topbar"), "blocks=" + document.querySelectorAll(".block-content").length); }
    if (state === "boot") {
      if (!created) {
        var b = clickByText(/Create a new graph/i);
        if (b) { created = true; log("CLICKED create-graph"); return; }
      } else if (!confirmed) {
        var b2 = clickByText(/^\s*Create graph\s*$/i);
        if (b2) { confirmed = true; log("CLICKED confirm-create"); return; }
      }
      if (created && confirmed && q(".topbar") && q(".main-content")) {
        // phase A: the graph exists; the driver now seeds a long journal and relaunches
        state = "idle"; setTimeout(function () { log("GRAPH-CREATED"); }, 6000); return;
      }
      if (!created && q(".topbar") && q(".main-content") && document.querySelectorAll(".block-content").length > 5) {
        state = "settle";
        setTimeout(function () { state = "running"; log("READY"); snap("ready"); runStep(); }, 4000);
      }
    }
  }, 500);
})();
