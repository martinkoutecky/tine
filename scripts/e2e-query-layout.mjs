// UI-LP-FIX: query tables stay inside their query box (main pane and right sidebar), board counts
// do not touch their labels, page links/cards read as links, Collapse all sits in the Linked
// References header row, an image in a reference breadcrumb is small, comma-separated property
// values are page links, and `:feature/enable-journals? false` hides Journals and routes home to
// the configured page. Synthetic graph only; geometry via getBoundingClientRect, no pixel values.
//
// LAYOUT_SOFT=1 records every failed check in result.json instead of throwing (used to produce the
// fail-before evidence against a pre-fix binary). LAYOUT_TAG labels the screenshots (before/after).
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName, currentPageTitle } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture } from "./lib/e2e-account-home.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = path.join(root, "artifacts/query-layout");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const soft = process.env.LAYOUT_SOFT === "1";
const tag = process.env.LAYOUT_TAG || "run";

// ---- synthetic graph -------------------------------------------------------------------------
const areas = ["Alpha", "Beta", "Gamma"];
const statuses = [
  "Verified against the reference implementation on every supported platform and recorded in the changelog",
  "Pending review by the maintainers after the second round of benchmark measurements finished overnight",
  "Blocked on an upstream release that has not been scheduled; revisit when the dependency publishes",
];
const widgetNames = [];
for (let i = 1; i <= 12; i++) {
  const name = `Widget ${String(i).padStart(2, "0")}`;
  widgetNames.push(name);
  fs.writeFileSync(path.join(graph, `pages/${name}.md`), [
    "kind:: widget",
    `area:: ${areas[i % 3]}`,
    `status:: ${statuses[i % 3]}`,
    "technique:: Time sync, Route framing",
    `serial:: ${1000000 + i * 7919}`,
    `owner:: Maintainer ${i}`,
    `revision:: r${i}-${(i * 7919).toString(16)}`,
    "",
    `- Notes for ${name}`,
  ].join("\n") + "\n");
}
const wideRow = (c) => `| ${c} |`;
const longText = "This cell holds a long sentence of ordinary prose that should wrap onto several lines";
fs.writeFileSync(path.join(graph, "pages/Hub.md"), [
  "- Widget table",
  "  tine.view:: table",
  "  {{query (page-property kind widget)}}",
  "- Widget boards",
  "  tine.view:: board",
  "  tine.group-by:: area",
  "  tine.sort:: area asc",
  "  {{query (page-property kind widget)}}",
  "- | Topic | Verdict | Count |",
  "  | --- | --- | --- |",
  `  | Alpha | ${longText} | 1234567 |`,
  `  | Beta | ${longText}, then a second sentence of a similar length | 7654321 |`,
  "- | A | B | C | D | E | F | G | H | I | J |",
  "  | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
  `  | ${Array.from({ length: 10 }, (_, k) => `column ${k} holds ${"words ".repeat(5)}`).join(" | ")} |`,
  "- block with properties",
  "  technique:: Time sync, Route framing",
  "- trailing block",
].join("\n") + "\n");
fs.writeFileSync(path.join(graph, "pages/Tag.md"), "- Tag page body\n");
for (const n of ["Ref A", "Ref B", "Ref C"]) fs.writeFileSync(path.join(graph, `pages/${n}.md`), `- ${n} mentions [[Tag]] here\n- ${n} also says Tag in plain words\n`);
// breadcrumb with an image: a parent block carrying the image, a child referencing Tag
function png(w, h) {
  const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (buf) => { let c = 0xffffffff; for (const b of buf) c = crcTable[(c ^ b) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => { const len = Buffer.alloc(4); len.writeUInt32BE(data.length); const td = Buffer.concat([Buffer.from(type), data]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([len, td, c]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const o = y * (w * 3 + 1) + 1 + x * 3; raw[o] = 40; raw[o + 1] = 120 + (x % 90); raw[o + 2] = 200; }
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", zlib.deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}
fs.writeFileSync(path.join(graph, "assets/diagram.png"), png(240, 160));
fs.writeFileSync(path.join(graph, "pages/Gallery.md"), "- ![diagram](../assets/diagram.png)\n  - Figure caption cites [[Tag]]\n");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:default-home {:page "Hub"}\n :feature/enable-journals? false\n :property/separated-by-commas #{:technique}}\n');

const account = accountHomeFixture(run, run);
const env = { ...account.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4921);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log] });
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
let browser;
const results = [];
const measured = {};
async function check(name, fn) {
  try { await fn(); results.push({ name, ok: true }); console.log(`PASS: ${name}`); }
  catch (error) {
    results.push({ name, ok: false, error: String(error.message || error).slice(0, 400) });
    console.log(`FAIL: ${name}: ${String(error.message || error).slice(0, 300)}`);
    if (!soft) throw error;
  }
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function settle() {
  await browser.executeAsync((done) => requestAnimationFrame(() => requestAnimationFrame(() => done())));
  await sleep(700);
  await browser.execute(() => document.querySelectorAll(".toast-stack .toast-close").forEach((b) => b.click()));
}
async function shot(label, W, H) {
  const file = path.join(run, `${tag}-${label}.png`);
  try { execFileSync("import", ["-window", "root", "-crop", `${W}x${H}+0+0`, "+repage", file], { env, timeout: 30000, stdio: "ignore" }); }
  catch { await browser.saveScreenshot(file); }
  return file;
}
async function theme(dark) {
  const isDark = await browser.execute(() => document.documentElement.classList.contains("dark") || document.documentElement.dataset.theme === "dark" || document.body.classList.contains("dark"));
  if (Boolean(isDark) !== dark) { await browser.$('button[title^="Toggle theme"]').click(); await sleep(400); }
}
// All geometry is measured in the page, returning plain numbers.
const rect = (sel, scope) => browser.execute((sel, scope) => {
  const el = (scope ? document.querySelector(scope) : document)?.querySelector(sel);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height, scrollW: el.scrollWidth, clientW: el.clientWidth, clientH: el.clientHeight, offsetH: el.offsetHeight };
}, sel, scope);
const TOL = 1.5;
async function tableInsideBox(scope, label) {
  const box = await rect(".query-block:has(table.query-page-table)", scope);
  assert(box, `${label}: query box with a table exists`);
  const table = await rect(".query-block table.query-page-table", scope);
  assert(table, `${label}: table exists`);
  const wrap = await rect(".query-block .md-table-wrap", scope);
  assert(wrap, `${label}: table wrapper exists`);
  measured[`${label}-box`] = box; measured[`${label}-wrap`] = wrap; measured[`${label}-table`] = table;
  assert(wrap.left >= box.left - TOL && wrap.right <= box.right + TOL, `${label}: wrapper ${wrap.left}-${wrap.right} lies inside box ${box.left}-${box.right}`);
}
async function wideTableScrollsInside(scope, label) {
  // The table may be wider than the box, but then the wrapper (the scroll viewport) is not.
  const wrap = await rect(".query-block .md-table-wrap", scope);
  const table = await rect(".query-block table.query-page-table", scope);
  if (table.width > wrap.width + TOL) {
    assert(wrap.scrollW > wrap.clientW, `${label}: overflowing table scrolls inside its wrapper`);
    assert(wrap.offsetH > wrap.clientH, `${label}: a visible scrollbar strip is reserved under the overflowing table`);
  }
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/release/tine") } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1920, 1080);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await settle();

  await check("F9 Journals is hidden and the first navigation item is the configured home page", async () => {
    const nav = await browser.execute(() => [...document.querySelectorAll(".left-sidebar .nav-item, .sidebar .nav-item")].map((el) => ({ text: el.textContent.trim(), home: el.classList.contains("home-nav") })));
    measured.nav = nav;
    assert(!nav.some((n) => n.text === "Journals"), `no Journals item (got ${JSON.stringify(nav)})`);
    assert(nav[0]?.home && nav[0].text === "Hub", `first item is the home page (got ${JSON.stringify(nav[0])})`);
  });
  await check("F9 the app opens on the configured home page, not a journal feed", async () => {
    await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Hub", { timeout: 15000, timeoutMsg: "home route is Hub" });
  });

  await openPageByName(browser, "Hub");
  await settle();
  await browser.$(".query-block table.query-page-table").waitForExist({ timeout: 20000 });
  await settle();

  await check("F1 main pane: query table lies inside its query box", async () => {
    await tableInsideBox(".main-content", "main");
    await wideTableScrollsInside(".main-content", "main");
  });
  await check("F11 table page links read as links, not buttons", async () => {
    const s = await browser.execute(() => {
      const a = document.querySelector(".main-content .query-page-table .query-page-link");
      if (!a) return null;
      const cs = getComputedStyle(a);
      return { border: cs.borderTopWidth, bg: cs.backgroundColor, display: cs.display, deco: cs.textDecorationLine, color: cs.color };
    });
    assert(s, "a page link exists in the query table");
    assert.equal(s.border, "0px", "no button border");
    assert.equal(s.bg, "rgba(0, 0, 0, 0)", "no button background");
  });
  await check("F3 board: the count does not touch its label", async () => {
    const r = await browser.execute(() => {
      const h = document.querySelector(".main-content .query-board-column h4");
      if (!h) return null;
      const label = h.querySelector(".query-board-label")?.getBoundingClientRect();
      const count = h.querySelector(".query-board-count")?.getBoundingClientRect();
      return label && count ? { labelRight: label.right, countLeft: count.left, countText: h.querySelector(".query-board-count").textContent } : null;
    });
    measured.boardHeading = r;
    assert(r, "board heading has label and count elements");
    assert(r.countLeft - r.labelRight >= 6, `count (${r.countLeft}) is at least 6px from the label (${r.labelRight})`);
  });
  await check("F4 board cards: bordered card with a link-styled page link", async () => {
    const r = await browser.execute(() => {
      const card = document.querySelector(".main-content .query-board-card");
      const a = card?.querySelector(".query-page-link");
      if (!card || !a) return null;
      const c = getComputedStyle(card), l = getComputedStyle(a);
      return { cardBorder: c.borderTopWidth, linkBorder: l.borderTopWidth, linkBg: l.backgroundColor };
    });
    assert(r, "a board card with a page link exists");
    assert.notEqual(r.cardBorder, "0px", "card has a border");
    assert.equal(r.linkBorder, "0px", "link has no button border");
  });
  await check("F5 'Grouped by' breakdown is folded by default", async () => {
    const r = await browser.execute(() => {
      const d = document.querySelector(".main-content .query-summary-details");
      const flat = document.querySelector(".main-content .query-board-column")?.closest(".query-block")?.querySelector(".query-summary-table");
      const shown = !!flat && (typeof flat.checkVisibility === "function" ? flat.checkVisibility({ contentVisibilityAuto: true, visibilityProperty: true }) : flat.getBoundingClientRect().height > 0) && !(flat.closest("details") && !flat.closest("details").open);
      return { hasDetails: !!d, open: d?.open ?? null, tableVisible: shown };
    });
    measured.summary = r;
    // A board with a group-by and no aggregate columns has no breakdown table at all: either shape is fine,
    // but a visible always-open summary table above the columns is the defect.
    assert(!r.tableVisible, "no visible group-summary table above the board columns");
  });
  await check("F2 markdown tables: long text wraps, numbers stay whole, overflow shows a scrollbar strip", async () => {
    const r = await browser.execute(() => {
      const tables = [...document.querySelectorAll(".main-content .md-table-wrap > table.md-table:not(.query-page-table):not(.query-summary-table)")];
      return tables.map((t) => {
        const wrap = t.parentElement.getBoundingClientRect();
        const pane = document.querySelector(".main-content").getBoundingClientRect();
        const longCell = t.querySelector("td.md-cell-long");
        const numCell = [...t.querySelectorAll("td")].find((td) => /^\d{7}$/.test(td.textContent.trim()));
        return {
          wrapLeft: wrap.left, wrapRight: wrap.right, paneLeft: pane.left, paneRight: pane.right,
          tableW: t.getBoundingClientRect().width,
          longHeight: longCell?.getBoundingClientRect().height ?? null,
          lineHeight: longCell ? parseFloat(getComputedStyle(longCell).lineHeight) || 20 : null,
          numHeight: numCell ? (() => { const r = document.createRange(); r.selectNodeContents(numCell); return r.getBoundingClientRect().height; })() : null,
          wrapper: { sw: t.parentElement.scrollWidth, cw: t.parentElement.clientWidth, oh: t.parentElement.offsetHeight, ch: t.parentElement.clientHeight },
        };
      });
    });
    measured.mdTables = r;
    assert(r.length >= 2, `found the two markdown tables (${r.length})`);
    const [prose, wide] = r;
    assert(prose.longHeight > prose.lineHeight * 1.5, `long prose cell wraps (height ${prose.longHeight})`);
    assert(prose.numHeight < prose.lineHeight * 2.2, `number cell does not wrap (height ${prose.numHeight})`);
    assert(prose.wrapRight <= prose.paneRight + TOL, "wrapped table stays inside the pane");
    assert(wide.wrapRight <= wide.paneRight + TOL, "wide table stays inside the pane");
    assert(wide.wrapper.sw > wide.wrapper.cw ? wide.wrapper.oh > wide.wrapper.ch : true, "an overflowing table has a visible scrollbar strip");
  });

  // ---- screenshots, 1920 and 1280, light and dark -------------------------------------------
  for (const [W, H] of [[1920, 1080], [1280, 900]]) {
    await browser.setWindowSize(W, H); await settle();
    for (const dark of [false, true]) {
      await theme(dark); await settle();
      await shot(`hub-${W}-${dark ? "dark" : "light"}`, W, H);
      for (const [what, sel] of [["board", ".main-content .query-results-board"], ["tables", ".main-content .md-table-wrap:not(:has(.query-page-table))"]]) {
        await browser.execute((sel) => { (document.querySelector(sel)?.closest(".ls-block") ?? document.querySelector(sel))?.scrollIntoView({ block: "start" }); }, sel);
        await sleep(400);
        await shot(`hub-${W}-${dark ? "dark" : "light"}-${what}`, W, H);
      }
      await browser.execute(() => { document.querySelector(".page-title")?.scrollIntoView({ block: "start" }); });
    }
    await theme(false);
  }
  await browser.setWindowSize(1920, 1080); await settle();

  // ---- right sidebar --------------------------------------------------------------------------
  await check("F1b right sidebar: the query table lies inside its query box", async () => {
    await openPageByName(browser, "Hub"); await settle();
    await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: "" }] }]);
    try { await browser.$("h1.page-title").click(); } finally { await browser.releaseActions(); }
    await browser.$(".right-sidebar-body .rs-item").waitForExist({ timeout: 15000 });
    await browser.$(".right-sidebar-body .query-block table.query-page-table").waitForExist({ timeout: 20000 });
    await settle();
    const box = await rect(".query-block:has(table.query-page-table)", ".right-sidebar-body");
    const item = await rect(".rs-item-body", ".right-sidebar-body");
    const wrap = await rect(".query-block .md-table-wrap", ".right-sidebar-body");
    const table = await rect(".query-block table.query-page-table", ".right-sidebar-body");
    measured.sidebar = { box, item, wrap, table };
    assert(box && item && wrap && table, "sidebar query box, item, wrapper and table exist");
    assert(wrap.right <= box.right + TOL && wrap.left >= box.left - TOL, `sidebar: wrapper ${wrap.left}-${wrap.right} inside box ${box.left}-${box.right}`);
    assert(box.right <= item.right + TOL, "sidebar: query box inside the sidebar item");
    assert(table.width <= wrap.width + TOL || wrap.scrollW > wrap.clientW, "sidebar: a table wider than the box scrolls inside it");
  });
  await shot("sidebar-1920-light", 1920, 1080);
  await theme(true); await settle();
  await shot("sidebar-1920-dark", 1920, 1080);
  await theme(false); await settle();
  await browser.setWindowSize(1280, 900); await settle();
  await shot("sidebar-1280-light", 1280, 900);
  await browser.setWindowSize(1920, 1080); await settle();

  // ---- linked references ------------------------------------------------------------------------
  await openPageByName(browser, "Tag"); await settle();
  await browser.$(".linked-references .references-header").waitForExist({ timeout: 20000 });
  await sleep(1500); await settle();
  await check("F6 Collapse all sits inside the Linked References header row", async () => {
    const r = await browser.execute(() => {
      const header = document.querySelector(".linked-references .references-header");
      const bulk = document.querySelector(".linked-references .reference-bulk-controls");
      if (!header || !bulk) return { header: !!header, bulk: !!bulk };
      const h = header.getBoundingClientRect(), b = bulk.getBoundingClientRect();
      return { header: true, bulk: true, inside: header.contains(bulk), hTop: h.top, hBottom: h.bottom, bTop: b.top, bBottom: b.bottom };
    });
    measured.bulk = r;
    assert(r.header && r.bulk, "header and bulk controls exist");
    assert(r.inside, "bulk controls are children of the header row");
    assert(r.bTop >= r.hTop - TOL && r.bBottom <= r.hBottom + TOL, "bulk controls lie vertically within the header row");
  });
  await check("F8 a stale Unlinked References count shows a mark with a tooltip, not a stray separator", async () => {
    // Unlinked References starts collapsed; an external edit moves the graph revision, which marks its count stale.
    fs.writeFileSync(path.join(graph, "pages/Ref D.md"), "- Ref D says Tag in plain words too\n");
    await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".references-count-stale")), { timeout: 20000, timeoutMsg: "the stale mark appears after an external edit" });
    const r = await browser.execute(() => {
      const m = document.querySelector(".references-count-stale");
      const header = m.closest(".references-header");
      return { text: header.textContent, markText: m.textContent, title: m.getAttribute("title"), hasIcon: !!m.querySelector("svg"), w: m.getBoundingClientRect().width };
    });
    measured.stale = r;
    assert(!/\d\s*·/.test(r.text) && !r.markText.includes("·"), `no ' ·' after the count (got ${JSON.stringify(r.text)})`);
    assert(r.title && r.hasIcon, "the stale mark is an icon with the existing tooltip");
  });
  await check("F7 gap before Linked References is modest", async () => {
    const g = await browser.execute(() => {
      const lr = document.querySelector(".linked-references");
      const blocks = [...document.querySelectorAll(".main-content .ls-block")].filter((b) => b.getBoundingClientRect().bottom <= lr.getBoundingClientRect().top + 1);
      const last = blocks.at(-1)?.getBoundingClientRect().bottom ?? null;
      return { lrTop: lr.getBoundingClientRect().top, lastBottom: last, gap: last === null ? null : lr.getBoundingClientRect().top - last };
    });
    measured.refGap = g;
    assert(g.gap === null || g.gap <= 72, `gap before Linked References is ${g.gap}px`);
  });
  await shot("tag-refs-1920-light", 1920, 1080);

  await openPageByName(browser, "Gallery"); await settle();
  await check("F13 an image in a reference breadcrumb renders small", async () => {
    await openPageByName(browser, "Tag"); await settle(); await sleep(1500);
    const r = await browser.execute(() => {
      const imgs = [...document.querySelectorAll(".linked-references .ref-breadcrumb img, .linked-references .breadcrumb img")];
      return imgs.map((i) => i.getBoundingClientRect().height);
    });
    measured.breadcrumbImgs = r;
    assert(r.length > 0, "the Gallery reference shows an image in its breadcrumb");
    assert(r.every((h) => h <= 22), `breadcrumb images are ~18px tall (got ${r})`);
  });

  // ---- comma-separated property values ----------------------------------------------------------
  await openPageByName(browser, "Widget 01"); await settle();
  await check("F12 each comma item of a separated-by-commas property is a page link (page header)", async () => {
    const r = await browser.execute(() => {
      const links = [...document.querySelectorAll(".main-content a.page-ref, .main-content .page-ref")].map((a) => a.textContent.trim());
      return links;
    });
    measured.commaLinks = r;
    assert(r.includes("Time sync") && r.includes("Route framing"), `both items are links (got ${JSON.stringify(r)})`);
  });
  await check("F12 each comma item of a separated-by-commas property is a page link (block properties)", async () => {
    await openPageByName(browser, "Hub"); await settle();
    const r = await browser.execute(() => [...document.querySelectorAll(".main-content .block-properties a.page-ref, .main-content .block-properties .page-ref")].map((a) => a.textContent.trim()));
    measured.blockPropLinks = r;
    assert(r.includes("Time sync") && r.includes("Route framing"), `both items are links (got ${JSON.stringify(r)})`);
  });
  await openPageByName(browser, "Widget 01"); await settle();
  await shot("widget-1920-light", 1920, 1080);

  account.verify();
  const failed = results.filter((r) => !r.ok);
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ testedCommit, tag, soft, failed: failed.length, results, measured }, null, 2));
  console.log(`Evidence: ${run}; ${results.length - failed.length}/${results.length} checks passed`);
  if (failed.length && !soft) throw new Error(`${failed.length} checks failed`);
} catch (error) {
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ testedCommit, observed: String(error), results, measured }, null, 2));
  throw error;
} finally {
  if (browser) await browser.deleteSession().catch(() => {});
  driver.kill();
  fs.closeSync(log);
}
