// Real-app probe for the bundled in-app Guide.
// Spawns tauri-driver in its own process group and kills the group on teardown.
// Usage: DISPLAY=:99 node scripts/probe-guide.mjs
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/tine-guide-probe";
const APP = process.env.TINE_APP || `${repo}/target/release/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME && fs.existsSync(`${process.env.CARGO_HOME}/bin/tauri-driver`)
    ? `${process.env.CARGO_HOME}/bin/tauri-driver`
    : fs.existsSync(`${repo}/../.toolchain/cargo/bin/tauri-driver`)
      ? `${repo}/../.toolchain/cargo/bin/tauri-driver`
      : "tauri-driver");
const PORT = Number(process.env.TINE_PROBE_PORT || 4890 + Math.floor(Math.random() * 200));
const NATIVE_PORT = PORT + 1;
const today = new Date();
const JNAME = `${today.getFullYear()}_${String(today.getMonth() + 1).padStart(2, "0")}_${String(today.getDate()).padStart(2, "0")}`;
const JFILE = `${G}/journals/${JNAME}.md`;

fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.writeFileSync(JFILE, "- Probe home\n");
fs.rmSync("/tmp/tine-guide-probe-xdg", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/tine-guide-probe-xdg/${d}`, { recursive: true });

const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/tine-guide-probe-xdg/data",
  XDG_CONFIG_HOME: "/tmp/tine-guide-probe-xdg/config",
  XDG_CACHE_HOME: "/tmp/tine-guide-probe-xdg/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/tine-guide-probe-td.log", "w");
const td = spawn(TD, ["--port", String(PORT), "--native-port", String(NATIVE_PORT), "--native-driver", "/usr/bin/WebKitWebDriver"], {
  env,
  stdio: ["ignore", tdLog, tdLog],
  detached: true,
});
const killTree = () => {
  try {
    process.kill(-td.pid, "SIGKILL");
  } catch {
    /* already gone */
  }
};
await sleep(3000);

let browser;
let failures = 0;
let checks = 0;
const check = (name, ok, extra = "") => {
  checks++;
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok ? "" : "  " + extra}`);
  if (!ok) failures++;
};

function graphFiles() {
  const out = [];
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return;
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, ent.name);
      if (ent.isDirectory()) walk(p);
      else out.push(p);
    }
  };
  walk(G);
  return out;
}

function copiedSheetsFile() {
  return graphFiles().find((p) => {
    const rel = path.relative(G, p);
    if (!rel.startsWith("pages/")) return false;
    if (!/\.(md|org)$/.test(p)) return false;
    const body = fs.readFileSync(p, "utf8");
    return body.includes("# Sheets") && body.includes("tine.formula.effort::");
  });
}

function copiedFormulasFile() {
  return graphFiles().find((p) => {
    const rel = path.relative(G, p);
    if (!rel.startsWith("pages/")) return false;
    if (!/\.(md|org)$/.test(p)) return false;
    const body = fs.readFileSync(p, "utf8");
    return body.includes("# Formulas") && body.includes("tine.formula.plan::");
  });
}

function copiedIndexFile() {
  return graphFiles().find((p) => {
    const rel = path.relative(G, p);
    if (!rel.startsWith("pages/")) return false;
    if (!/\.(md|org)$/.test(p)) return false;
    const body = fs.readFileSync(p, "utf8");
    return body.includes("# Tine Guide") && body.includes("[[tine-guide/Features/Sheets]]");
  });
}

function copiedGuidePageFiles() {
  return graphFiles().filter((p) => {
    const rel = path.relative(G, p);
    return rel.startsWith("pages/") && /\.(md|org)$/.test(p) && rel.includes("tine-guide");
  });
}

async function clickButtonByText(selector, pattern) {
  return browser.execute((sel, source) => {
    const re = new RegExp(source);
    const item = [...document.querySelectorAll(sel)].find((el) => re.test((el.textContent || "").trim()));
    item?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    return !!item;
  }, selector, pattern.source);
}

try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: PORT,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });
  await browser.$(".ls-block").waitForExist({ timeout: 20000 });

  const clickedHelp = await browser.execute(() => {
    const btn = document.querySelector(".help-corner-btn");
    btn?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    return !!btn;
  });
  check("clicked help button", clickedHelp);
  await browser.$(".help-menu").waitForExist({ timeout: 5000 });
  const guideFirst = await browser.execute(() => {
    const items = [...document.querySelectorAll(".help-menu-item")];
    const first = items[0];
    return {
      count: items.length,
      firstLabel: first?.querySelector(".help-menu-label")?.textContent?.trim() ?? "",
      firstDetail: first?.querySelector(".help-menu-detail")?.textContent?.trim() ?? "",
    };
  });
  check("help menu lists Guide first", guideFirst.firstLabel === "Guide", JSON.stringify(guideFirst));
  check("help menu Guide detail matches spec", guideFirst.firstDetail === "Open the in-app how-to guide", JSON.stringify(guideFirst));

  const clickedGuide = await clickButtonByText(".help-menu-label", /^Guide$/);
  check("clicked Help -> Guide", clickedGuide);
  await browser.$(".page-guide-banner").waitForExist({ timeout: 10000 });
  const guideIndex = await browser.execute(() => ({
    title: document.querySelector(".page-title")?.textContent?.trim() ?? "",
    tab: document.querySelector(".tab.active .tab-title")?.textContent?.trim() ?? "",
    copyButton: !!document.querySelector(".guide-copy-btn"),
    text: document.body.textContent || "",
  }));
  check("Guide tab opens on Tine Guide index", guideIndex.title === "Tine Guide" && guideIndex.tab === "Guide", JSON.stringify(guideIndex));
  check("Guide index is marked read-only with copy affordance", guideIndex.copyButton && guideIndex.text.includes("Bundled Guide page"), JSON.stringify(guideIndex));

  const pagesAfterOpen = graphFiles().filter((p) => path.relative(G, p).startsWith("pages/"));
  check("opening bundled Guide wrote no graph page files", pagesAfterOpen.length === 0, pagesAfterOpen.join("\n"));

  const copied = await clickButtonByText(".guide-copy-btn", /^Copy the guide into your graph$/);
  check("clicked Copy the guide into your graph on Guide index", copied);

  for (let i = 0; i < 30 && (!copiedSheetsFile() || !copiedIndexFile()); i += 1) await sleep(250);
  const copiedFile = copiedSheetsFile();
  const copiedIndex = copiedIndexFile();
  check("copy writes the whole guide namespace under graph pages", copiedGuidePageFiles().length >= 6, graphFiles().join("\n"));
  check("copy writes a rewritten Tine Guide index", !!copiedIndex, graphFiles().join("\n"));
  check("copy writes a real Sheets markdown page under graph pages", !!copiedFile, graphFiles().join("\n"));
  if (copiedFile) {
    const rel = path.relative(G, copiedFile);
    const body = fs.readFileSync(copiedFile, "utf8");
    check("Sheets copy file is the lowercase tine-guide namespace", rel.includes("tine-guide") || rel.includes("tine_guide"), rel);
    check("Sheets copy file contains Logseq markdown guide content", body.includes("- # Sheets") && body.includes("Create one yourself"), body.slice(0, 400));
  }
  if (copiedIndex) {
    const body = fs.readFileSync(copiedIndex, "utf8");
    check("copied Guide index rewrites Sheets link", body.includes("[[tine-guide/Features/Sheets]]"), body.slice(0, 600));
    check("copied Guide index has no dangling unprefixed Sheets link", !body.includes("[[Features/Sheets]]"), body.slice(0, 600));
  }
  check("copy writes referenced Guide assets", fs.existsSync(`${G}/assets/quick-capture.png`), graphFiles().join("\n"));

  await browser.waitUntil(async () => {
    const state = await browser.execute(() => ({
      title: document.querySelector(".page-title")?.textContent?.trim() ?? "",
      guideBanner: !!document.querySelector(".page-guide-banner"),
      copyButtons: document.querySelectorAll(".guide-copy-btn").length,
    }));
    return state.title.includes("tine-guide/Tine Guide") && !state.guideBanner && state.copyButtons === 0;
  }, { timeout: 10000, timeoutMsg: "Whole-guide copy did not navigate to the real copied Guide index" });
  const copiedIndexState = await browser.execute(() => ({
    title: document.querySelector(".page-title")?.textContent?.trim() ?? "",
    guideBanner: !!document.querySelector(".page-guide-banner"),
    copyButtons: document.querySelectorAll(".guide-copy-btn").length,
    readOnlyBlocks: document.querySelectorAll(".block-content-wrapper.read-only").length,
  }));
  check("copied Guide index is in the user's graph", copiedIndexState.title.includes("tine-guide/Tine Guide") && !copiedIndexState.guideBanner && copiedIndexState.copyButtons === 0, JSON.stringify(copiedIndexState));

  // The copied Guide index links every guide page near the top (reliably mounted).
  // Open the Formulas page first, assert its live computed column, then hop to the
  // Sheets page via the Formulas page's own [[Features/Sheets]] link.
  check("copy writes the Formulas guide page under graph pages", !!copiedFormulasFile(), graphFiles().join("\n"));
  const clickedCopiedFormulas = await browser.execute(() => {
    const link = [...document.querySelectorAll("a.page-ref")]
      .find((el) => (el.textContent || "").includes("tine-guide/Features/Formulas"));
    link?.setAttribute("data-probe", "copied-formulas-link");
    return !!link;
  });
  check("found rewritten Formulas link in copied Guide index", clickedCopiedFormulas);
  if (clickedCopiedFormulas) await browser.$('[data-probe="copied-formulas-link"]').click();
  await browser.waitUntil(async () => {
    const title = await browser.execute(() => document.querySelector(".page-title")?.textContent?.trim() ?? "");
    return title.includes("tine-guide/Features/Formulas");
  }, { timeout: 10000, timeoutMsg: "Copied Guide index link did not open copied Formulas page" });
  await browser.$(".sheet-table").waitForExist({ timeout: 10000 });
  const planState = await browser.execute(() => {
    const table = [...document.querySelectorAll(".sheet-table")].find((t) =>
      [...t.querySelectorAll(".sheet-header-cell")].some((h) => (h.textContent || "").includes("plan"))
    );
    if (!table) return { plan: null, title: document.querySelector(".page-title")?.textContent?.trim() ?? "" };
    const headers = [...table.querySelectorAll(".sheet-header-cell")].map((h) => (h.textContent || "").trim());
    const col = headers.findIndex((h) => h.includes("plan"));
    const plan = [0, 1].map((row) =>
      (table.querySelector(`.sheet-field-cell[data-row="${row}"][data-col="${col}"]`)?.textContent || "").trim()
    );
    return { plan, title: document.querySelector(".page-title")?.textContent?.trim() ?? "" };
  });
  check(
    "Formulas page computes the plan column live",
    Array.isArray(planState.plan) && planState.plan[0] === "quick task" && planState.plan[1] === "focus block",
    JSON.stringify(planState)
  );

  const clickedCopiedSheets = await browser.execute(() => {
    const link = [...document.querySelectorAll("a.page-ref")]
      .find((el) => (el.textContent || "").includes("tine-guide/Features/Sheets"));
    link?.setAttribute("data-probe", "copied-sheets-link");
    return !!link;
  });
  check("found rewritten Sheets link on copied Formulas page", clickedCopiedSheets);
  if (clickedCopiedSheets) await browser.$('[data-probe="copied-sheets-link"]').click();
  await browser.waitUntil(async () => {
    const title = await browser.execute(() => document.querySelector(".page-title")?.textContent?.trim() ?? "");
    return title.includes("tine-guide/Features/Sheets");
  }, { timeout: 10000, timeoutMsg: "Formulas page link did not open copied Sheets page" });

  await browser.$(".sheet-grid").waitForExist({ timeout: 10000 });
  await browser.$(".sheet-table").waitForExist({ timeout: 10000 });
  const sheetState = await browser.execute(() => {
    const tables = [...document.querySelectorAll(".sheet-table")];
    let effort = null;
    for (const table of tables) {
      const headers = [...table.querySelectorAll(".sheet-header-cell")].map((h) => (h.textContent || "").trim());
      const col = headers.findIndex((h) => h.includes("effort"));
      if (col < 0) continue;
      effort = [0, 1].map((row) =>
        (table.querySelector(`.sheet-field-cell[data-row="${row}"][data-col="${col}"]`)?.textContent || "").trim()
      );
      break;
    }
    return {
      title: document.querySelector(".page-title")?.textContent?.trim() ?? "",
      gridCount: document.querySelectorAll(".sheet-grid").length,
      tableCount: document.querySelectorAll(".sheet-table").length,
      effort,
      readOnlyBlocks: document.querySelectorAll(".block-content-wrapper.read-only").length,
      guideBanner: !!document.querySelector(".page-guide-banner"),
    };
  });
  check("rewritten link navigates to copied Sheets page", sheetState.title.includes("tine-guide/Features/Sheets") && !sheetState.guideBanner, JSON.stringify(sheetState));
  check("copied Sheets page shows live grid/table", sheetState.gridCount > 0 && sheetState.tableCount > 0, JSON.stringify(sheetState));
  check("formula column evaluates in copied Sheets page", Array.isArray(sheetState.effort) && sheetState.effort[0] === "10" && sheetState.effort[1] === "6", JSON.stringify(sheetState));
  const editableBlock = await browser.execute(() => {
    const block = [...document.querySelectorAll(".block-content-wrapper:not(.read-only)")]
      .find((el) => (el.textContent || "").includes("Sheets turn ordinary") || (el.textContent || "").includes("Create one yourself"));
    block?.setAttribute("data-probe", "editable-copy-block");
    return !!block;
  });
  check("copied page has a normal editable text block", editableBlock);
  if (editableBlock) await browser.$('[data-probe="editable-copy-block"]').click();
  await browser.$(".block-editor").waitForExist({ timeout: 5000 });
  check("copied Sheets page is editable", true);

  console.log(failures === 0 ? `\nALL PASS (${checks} checks)` : `\n${failures} FAILED of ${checks}`);
} catch (e) {
  console.error("PROBE ERROR:", e);
  failures++;
} finally {
  if (browser) await browser.deleteSession().catch(() => {});
  killTree();
}
process.exit(failures === 0 ? 0 : 1);
