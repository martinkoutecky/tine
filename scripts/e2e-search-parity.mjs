// Focused Linux real-app proof for the catalog-derived Ctrl-K parity packet.
// User-facing gestures are literal WebDriver keyboard/mouse input; DOM reads
// below are waits/assertions only and never substitute for activation.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  startWebdriverApplication,
  stopWebdriverApplication,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME
  ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver")
  : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4690);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4691);
const TMP = `/tmp/tine-search-parity-${process.pid}`;
const GRAPH = path.join(TMP, "graph");
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const MAIN_PAGE = "Search Parity Main";
const PAGE_TARGET = "E2E Sidebar Target";
const BLOCK_QUERY = "SIDEBAR-BLOCK-WITNESS-731";
const BLOCK_ID = "73173173-1731-4731-8731-731731731731";
const CURRENT_QUERY = "CURRENT-PAGE-NEEDLE-884";
const CURRENT_BLOCK_ID = "88488488-4884-4884-8884-884884884884";
const COMPOSED_PAGE = "Caf\u00e9 Exact";
const DECOMPOSED_ALIAS = "Cafe\u0301 Alias";
const DECOMPOSED_BLOCK = "Unicode Cafe\u0301 block witness";

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) {
  fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
}
for (const dir of ["data", "config", "cache"]) {
  fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
}
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, `pages/${MAIN_PAGE}.md`), [
  "- Collapsed search owner",
  "  collapsed:: true",
  `  - ${CURRENT_QUERY} on the routed page`,
  `    id:: ${CURRENT_BLOCK_ID}`,
  `- ${DECOMPOSED_BLOCK}`,
  "  id:: 50505050-5050-4050-8050-505050505050",
  "",
].join("\n"));
fs.writeFileSync(path.join(GRAPH, "pages/Search Parity Other.md"), [
  `- ${CURRENT_QUERY} on another page`,
  "  id:: 99499499-4994-4994-8994-994994994994",
  "",
].join("\n"));
fs.writeFileSync(path.join(GRAPH, `pages/${PAGE_TARGET}.md`), "- Sidebar page body\n");
fs.writeFileSync(path.join(GRAPH, "pages/Block Host.md"), [
  `- ${BLOCK_QUERY}`,
  `  id:: ${BLOCK_ID}`,
  "",
].join("\n"));
fs.writeFileSync(path.join(GRAPH, `pages/${COMPOSED_PAGE}.md`), [
  `alias:: ${DECOMPOSED_ALIAS}`,
  "",
  "- Canonical page body",
  "",
].join("\n"));
fs.writeFileSync(path.join(GRAPH, "pages/Cafe Plain Control.md"), "- backend-settle witness\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, `journals/${journal}.md`), `- Open [[${MAIN_PAGE}]]\n`);

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg/data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg/config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg/cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};

const typeKeys = async (browser, text) => {
  for (const key of text) await browser.keys([key]);
};

const switcherSnapshot = (browser) => browser.execute(() => {
  const rows = [...document.querySelectorAll(".switcher-row[role='option']")].map((row) => ({
    active: row.classList.contains("active"),
    kind: row.querySelector(".switcher-kind")?.textContent?.trim() ?? "",
    name: row.querySelector(".switcher-name")?.textContent?.trim() ?? "",
    context: row.querySelector(".search-result-context")?.textContent?.trim() ?? "",
    excerpt: row.querySelector(".search-result-excerpt")?.textContent ?? "",
    marks: [...row.querySelectorAll("mark")].map((mark) => mark.textContent ?? ""),
  }));
  return {
    placeholder: document.querySelector(".switcher-input")?.getAttribute("placeholder") ?? "",
    value: document.querySelector(".switcher-input")?.value ?? "",
    headers: [...document.querySelectorAll(".switcher-group-header > span:first-child")]
      .map((node) => node.textContent?.trim() ?? ""),
    rows,
    openSearchTab: Boolean(document.querySelector("[data-open-search-tab]")),
  };
});

async function waitFor(browser, predicate, message, timeout = 12_000) {
  let last = null;
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    last = await switcherSnapshot(browser);
    if (predicate(last)) return last;
    await sleep(50);
  }
  throw new Error(`${message}; last=${JSON.stringify(last)}`);
}

async function openSwitcher(browser, chord, placeholder) {
  await browser.keys(chord);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5000 });
  await browser.waitUntil(async () => (await browser.execute(() => document.activeElement?.classList.contains("switcher-input"))) === true, {
    timeout: 5000,
    timeoutMsg: "switcher input did not receive focus from the literal shortcut",
  });
  const actual = await input.getAttribute("placeholder");
  if (actual !== placeholder) throw new Error(`switcher placeholder ${JSON.stringify(actual)} != ${JSON.stringify(placeholder)}`);
  return input;
}

async function waitPage(browser, title) {
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === title, {
    timeout: 10_000,
    timeoutMsg: `main route did not become ${title}`,
  });
}

async function expectMainRoute(browser, title, message) {
  const actual = (await browser.$("h1.page-title").getText()).trim();
  if (actual !== title) throw new Error(`${message}: main title ${JSON.stringify(actual)}`);
}

async function closeSwitcher(browser) {
  await browser.keys(["Escape"]);
  await browser.$(".switcher").waitForExist({ reverse: true, timeout: 5000 });
}

const log = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
const driverArgs = webdriverServerArgs(
  DRIVER_PORT,
  NATIVE_PORT,
  process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
);
const webviewTarget = await startWebdriverApplication(APP, env, NATIVE_PORT, "search-parity");
const td = spawn(TD, driverArgs, {
  env: webviewTarget.env,
  stdio: ["ignore", log, log],
  detached: process.platform !== "win32",
});
await sleep(2500);

const receipt = {
  schemaVersion: 1,
  scenario: "search-parity",
  app: path.resolve(APP),
  appSha256: crypto.createHash("sha256").update(fs.readFileSync(APP)).digest("hex"),
  sourceRevision: process.env.TINE_SOURCE_REVISION || null,
  literalInput: true,
  observations: {},
  omitted: {
    restartPersistence: "Existing right-sidebar native coverage owns restart persistence; this packet stays on search activation and scope.",
  },
};

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "search-parity", process.platform, webviewTarget.debuggerAddress),
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });

  // Reach the routed fixture through the ordinary literal Ctrl-K + Enter path.
  // Besides avoiding journal-feed timing as a setup dependency, this is the
  // packet's representative proof that plain Enter keeps its old navigation.
  await openSwitcher(browser, ["Control", "k"], "Jump to page, search, or run a command…");
  await typeKeys(browser, MAIN_PAGE);
  await waitFor(browser, (state) => {
    const active = state.rows.find((row) => row.active);
    return active?.kind === "page" && active.name === MAIN_PAGE
      && !state.rows.some((row) => row.kind === "new");
  }, "main fixture page did not settle as the exact active result");
  await browser.keys(["Enter"]);
  await waitPage(browser, MAIN_PAGE);

  // 1. Exact global page result + Shift-only Enter parks the page without
  // changing the focused main route.
  await openSwitcher(browser, ["Control", "k"], "Jump to page, search, or run a command…");
  await typeKeys(browser, PAGE_TARGET);
  const pageResult = await waitFor(browser, (state) => {
    const active = state.rows.find((row) => row.active);
    return active?.kind === "page" && active.name === PAGE_TARGET
      && !state.rows.some((row) => row.kind === "new");
  }, "exact page result did not settle as the active non-create row");
  await browser.keys(["Shift", "Enter"]);
  await browser.$(".switcher").waitForExist({ reverse: true, timeout: 5000 });
  await expectMainRoute(browser, MAIN_PAGE, "Shift-Enter page activation navigated the main route");
  const pageSurface = `[data-sidebar-surface="sidebar:page:page:${PAGE_TARGET}"]`;
  await browser.$(pageSurface).waitForExist({ timeout: 10_000 });
  receipt.observations.pageSidebar = {
    query: PAGE_TARGET,
    active: pageResult.rows.find((row) => row.active),
    surface: pageSurface,
    mainRoute: MAIN_PAGE,
  };

  // 2. A global block result follows the same literal Shift-only activation.
  // The Create row is initially active, so ArrowDown is part of the user flow.
  await openSwitcher(browser, ["Control", "k"], "Jump to page, search, or run a command…");
  await typeKeys(browser, BLOCK_QUERY);
  const blockReady = await waitFor(browser, (state) => state.rows.length === 2
    && state.rows[0]?.active && state.rows[0]?.kind === "new"
    && state.rows[1]?.kind === "block" && state.rows[1]?.excerpt.includes(BLOCK_QUERY),
  "global block query did not settle to Create + exact block");
  await browser.keys(["ArrowDown"]);
  const selectedBlock = await waitFor(browser, (state) => state.rows.some((row) => row.active
    && row.kind === "block" && row.excerpt.includes(BLOCK_QUERY)),
  "literal ArrowDown did not activate the block row");
  await browser.keys(["Shift", "Enter"]);
  await browser.$(".switcher").waitForExist({ reverse: true, timeout: 5000 });
  await expectMainRoute(browser, MAIN_PAGE, "Shift-Enter block activation navigated the main route");
  const blockSurface = `[data-sidebar-surface="sidebar:block:${BLOCK_ID}"]`;
  await browser.$(blockSurface).waitForExist({ timeout: 10_000 });
  await browser.$(`${blockSurface} .rs-item-body [data-block-id="${BLOCK_ID}"]`).waitForExist({ timeout: 10_000 });
  const blockSidebarText = await browser.$(`${blockSurface} .rs-item-body`).getText();
  if (!blockSidebarText.includes(BLOCK_QUERY)) throw new Error(`sidebar block body was ${JSON.stringify(blockSidebarText)}`);
  receipt.observations.blockSidebar = {
    beforeArrow: blockReady.rows,
    active: selectedBlock.rows.find((row) => row.active),
    surface: blockSurface,
    mainRoute: MAIN_PAGE,
  };

  // 3. Current-page search includes a descendant hidden by source collapse but
  // excludes the identical block on another page and all global providers.
  if (await browser.$(`[data-block-id="${CURRENT_BLOCK_ID}"]`).isExisting()) {
    throw new Error("collapsed descendant was already mounted before current-page search");
  }
  await openSwitcher(browser, ["Control", "Shift", "k"], "Search blocks in current page…");
  await typeKeys(browser, CURRENT_QUERY);
  const scoped = await waitFor(browser, (state) => state.headers.length === 1
    && state.headers[0] === "Current page"
    && state.rows.length === 1
    && state.rows[0]?.kind === "block"
    && state.rows[0]?.context.startsWith(MAIN_PAGE)
    && state.rows[0]?.excerpt.includes(CURRENT_QUERY),
  "current-page search did not settle to the one collapsed descendant");
  if (scoped.rows.some((row) => ["page", "journal", "new", "cmd"].includes(row.kind))) {
    throw new Error(`current-page search leaked a global provider: ${JSON.stringify(scoped)}`);
  }
  if (scoped.openSearchTab) throw new Error("current-page search exposed Open search tab");
  await expectMainRoute(browser, MAIN_PAGE, "current-page search changed the focused route");
  receipt.observations.currentPage = scoped;
  await closeSwitcher(browser);

  // 5a. Canonically equivalent page-name and alias queries are exact: the
  // existing page is offered and the Create provider is suppressed.
  await openSwitcher(browser, ["Control", "k"], "Jump to page, search, or run a command…");
  await typeKeys(browser, "Cafe\u0301 Exact");
  const unicodePage = await waitFor(browser, (state) => state.rows.some((row) => row.kind === "page" && row.name === COMPOSED_PAGE)
    && !state.rows.some((row) => row.kind === "new"),
  "NFD query did not exact-match the NFC page name");
  receipt.observations.unicodePage = unicodePage;
  await closeSwitcher(browser);

  await openSwitcher(browser, ["Control", "k"], "Jump to page, search, or run a command…");
  await typeKeys(browser, "Caf\u00e9 Alias");
  const unicodeAlias = await waitFor(browser, (state) => state.rows.some((row) => row.kind === "page" && row.name === COMPOSED_PAGE)
    && !state.rows.some((row) => row.kind === "new"),
  "NFC query did not exact-match the NFD alias");
  receipt.observations.unicodeAlias = unicodeAlias;
  await closeSwitcher(browser);

  // 5b. A composed scalar matches decomposed source text and the highlight
  // retains the entire original grapheme/UTF-16 source span.
  await openSwitcher(browser, ["Control", "Shift", "k"], "Search blocks in current page…");
  await typeKeys(browser, "\u00e9");
  const unicodeBlock = await waitFor(browser, (state) => state.rows.some((row) => row.kind === "block"
    && row.excerpt.includes(DECOMPOSED_BLOCK)
    && row.marks.includes("e\u0301")),
  "NFC block query did not highlight the complete NFD source grapheme");
  receipt.observations.unicodeBlock = unicodeBlock;
  await closeSwitcher(browser);

  // 5c. Canonical equivalence is not accent folding. Wait for an ordinary
  // backend page hit as the settlement witness, then prove the accent-bearing
  // block is absent and Create remains available (no false exact page match).
  await openSwitcher(browser, ["Control", "k"], "Jump to page, search, or run a command…");
  await typeKeys(browser, "cafe");
  const accentNegative = await waitFor(browser, (state) => state.rows.some((row) => row.kind === "page" && row.name === "Cafe Plain Control")
    && state.rows.some((row) => row.kind === "new"),
  "ASCII negative control did not reach a settled backend result");
  if (accentNegative.rows.some((row) => row.kind === "block" && row.excerpt.includes(DECOMPOSED_BLOCK))) {
    throw new Error(`ASCII cafe accent-folded into the decomposed block: ${JSON.stringify(accentNegative)}`);
  }
  receipt.observations.accentNegative = accentNegative;
  await closeSwitcher(browser);

  // 4. A route without a single page uses the ordinary global provider set.
  const journalsNav = await browser.$("//div[contains(concat(' ', normalize-space(@class), ' '), ' nav-item ')][.//span[normalize-space(.)='Journals']]");
  await journalsNav.click();
  await browser.waitUntil(() => browser.execute((mainPage) => {
    const titles = [...document.querySelectorAll("h1.page-title")].map((title) => title.textContent?.trim() ?? "");
    return titles.length > 0 && !titles.includes(mainPage);
  }, MAIN_PAGE), {
    timeout: 10_000,
    timeoutMsg: "Journals route did not open",
  });
  await openSwitcher(browser, ["Control", "Shift", "k"], "Jump to page, search, or run a command…");
  await typeKeys(browser, PAGE_TARGET);
  const fallback = await waitFor(browser, (state) => state.headers.includes("Pages")
    && state.rows.some((row) => row.kind === "page" && row.name === PAGE_TARGET)
    && state.openSearchTab,
  "no-single-page Ctrl-Shift-K did not fall back to global search");
  receipt.observations.noPageFallback = fallback;
  await closeSwitcher(browser);

  fs.writeFileSync(path.join(ARTIFACTS, "receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log("PASS: literal search gestures preserve routes, scope blocks, and compare canonical Unicode without accent folding");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try {
    if (process.platform === "win32") td.kill("SIGKILL");
    else process.kill(-td.pid, "SIGKILL");
  } catch {}
  stopWebdriverApplication(webviewTarget);
  fs.closeSync(log);
}
