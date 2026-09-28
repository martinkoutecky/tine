// Drives the REAL Tauri app (real backend + frontend) under Xvfb via tauri-driver.
// Three rename journeys on one disposable graph:
//  1. Plain rename (the original regression): open Pokus2 through a linked
//     reference with Tine open in history, rename the title Pokus2 -> Pokus, and
//     check the file moved and every reference was rewritten.
//  2. `title::` identity (og batch 11a/11e): a page whose file is
//     pages/Physical.md but whose `title:: Effective` names it is routed, offered
//     and renamed as "Effective"; the rename rebinds its own title. The written
//     bytes are master's (differential in evidence/og-11/11e/RECEIPT-11E.md).
//  3. Rename onto an existing page merges (og batch 12a, GH #327, OG
//     `merge-pages!`): the native confirmation is declined first (nothing may
//     change), then accepted (content appended, aliases united, references
//     rewritten, source moved to graph trash).
// Usage: node scripts/e2e-rename.mjs  (TINE_E2E_RENAME=ctx uses the context-menu
// rename for journey 1). Needs xdotool for the native confirmation.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { currentPageTitle, openPageByLink, openPageByName, switcherPageRows } from "./lib/e2e-navigation.mjs";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";

const TMP = process.env.E2E_TMP_DIR || `/tmp/tine-rename-e2e-${process.pid}`;
const G = `${TMP}/graph`;
const XDG = `${TMP}/xdg`;
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4444);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4445);
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;

const TITLE_SOURCE = "title:: Effective\n\n- [[Effective]] body\n";
const MERGE_SOURCE = "alias:: Former\n\n- source body\n";
const MERGE_SURVIVOR = "alias:: Kept\n\n- survivor body\n";
const MERGE_REF = "- [[Merge Source]] via [[Former]]\n";

function seed() {
  fs.rmSync(TMP, { recursive: true, force: true });
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${G}/${dir}`, { recursive: true });
  for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${XDG}/${dir}`, { recursive: true });
  fs.mkdirSync(ARTIFACTS, { recursive: true });
  fs.writeFileSync(`${G}/pages/Pokus2.md`, "- Tohle je pokus\n");
  // Mirrors the real Tine.md: a ```calc fenced block on a bullet line, then the
  // ref in a LATER bullet (the case that used to be mis-read as "inside code").
  fs.writeFileSync(
    `${G}/pages/Tine.md`,
    "- ## Tests\n\t- ```calc\n\t  1 + 2\n\t  var = 2+4\n\t  ```\n\t- #+BEGIN_TIP\n\t  a tip\n\t  #+END_TIP\n\t- [[Pokus2]]\n"
  );
  fs.writeFileSync(`${G}/pages/Testtest2.md`, "- This is a test test page\n- [[Pokus2]]\n");
  fs.writeFileSync(`${G}/journals/2026_06_24.md`, "- journal ref [[Pokus2]]\n");
  fs.writeFileSync(`${G}/pages/Physical.md`, TITLE_SOURCE);
  fs.writeFileSync(`${G}/pages/Title Ref.md`, "- [[Effective]]\n");
  fs.writeFileSync(`${G}/pages/Merge Source.md`, MERGE_SOURCE);
  fs.writeFileSync(`${G}/pages/Merge Survivor.md`, MERGE_SURVIVOR);
  fs.writeFileSync(`${G}/pages/Merge Ref.md`, MERGE_REF);
}
const read = (rel) => {
  const p = `${G}/${rel}`;
  return fs.existsSync(p) ? fs.readFileSync(p, "utf8") : null;
};
const dump = (tag) => {
  console.log(`\n===== ${tag} =====`);
  for (const f of ["pages/Pokus.md", "pages/Pokus2.md", "pages/Tine.md", "pages/Testtest2.md", "journals/2026_06_24.md",
    "pages/Physical.md", "pages/Renamed.md", "pages/Title Ref.md",
    "pages/Merge Source.md", "pages/Merge Survivor.md", "pages/Merge Ref.md"]) {
    const body = read(f);
    console.log(`-- ${f}:`, body === null ? "(absent)" : JSON.stringify(body));
  }
};
/** Files anywhere under `dir` (recursive), as graph-relative paths. */
function filesUnder(dir) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir, { withFileTypes: true, recursive: true })
    .filter((entry) => entry.isFile())
    .map((entry) => path.join(entry.parentPath ?? entry.path, entry.name));
}
async function waitForFiles(predicate, message, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await sleep(100);
  }
  throw new Error(message);
}

seed();
dump("BEFORE");

const env = {
  ...process.env, // DISPLAY inherited
  TINE_GRAPH: G,
  XDG_DATA_HOME: `${XDG}/data`,
  XDG_CONFIG_HOME: `${XDG}/config`,
  XDG_CACHE_HOME: `${XDG}/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};
console.log("DISPLAY=", process.env.DISPLAY);

const tdLog = fs.openSync(`${ARTIFACTS}/tauri-driver.log`, "w");
const td = spawn(
  TD,
  ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", tdLog, tdLog], detached: true }
);
await sleep(3000);

let browser;
const fillAndEnter = (sel, val) =>
  browser.execute((s, v) => {
    const inp = document.querySelector(s);
    inp.focus();
    inp.value = v;
    inp.dispatchEvent(new Event("input", { bubbles: true }));
    inp.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  }, sel, val);
/** Rename the routed page through its title (double-click, type, Enter). */
async function renameByTitle(to) {
  await browser.execute(() => {
    const t = document.querySelector("h1.page-title");
    if (t) t.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window }));
  });
  const input = await browser.$(".page-title-input");
  await input.waitForExist({ timeout: 4000, timeoutMsg: "rename input never appeared (dblclick did not reach startRename)" });
  await fillAndEnter(".page-title-input", to);
}
// "After a rename the user is on the renamed page" is a product outcome that
// og fails when the renamed page was reached from another page (RECEIPT-12E P1).
// It is recorded and the journey continues via the switcher, so every run also
// proves the disk outcomes of the later journeys; any recorded failure still
// fails the run at the end.
const routeFailures = [];
async function expectLandedOn(name, what) {
  try {
    await waitForTitle(name, what, 8_000);
  } catch (error) {
    routeFailures.push(String(error.message ?? error));
    console.log(`ROUTE FAILURE: ${error.message ?? error}`);
    await openPageByName(browser, name);
  }
}
async function waitForTitle(name, what, timeout = 15_000) {
  try {
    await browser.waitUntil(async () => (await currentPageTitle(browser)) === name, { timeout, interval: 100 });
  } catch {
    const seen = await browser.execute(() => ({
      titles: [...document.querySelectorAll(".page-title, .page-title-input")].map((n) => n.textContent || n.value),
      toasts: [...document.querySelectorAll(".toast")].map((n) => n.textContent?.trim()),
      tabs: [...document.querySelectorAll(".tab-title, .tab")].map((n) => n.textContent?.trim()).slice(0, 8),
      blocks: [...document.querySelectorAll(".page-blocks .block-content")].map((n) => n.textContent?.trim()).slice(0, 6),
    }));
    throw new Error(`${what}: page title never became ${JSON.stringify(name)}; saw ${JSON.stringify(seen)}`);
  }
}

try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });

  // Wait for first paint (journals feed).
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });

  // ---- Journey 1: plain rename --------------------------------------------
  // Open Pokus2 by clicking its [[Pokus2]] link in the journal feed.
  await openPageByLink(browser, "Pokus2");
  console.log("PAGE TITLE:", await currentPageTitle(browser));
  // Let Linked References (LiveRefGroup -> loads Tine, Testtest2, journal) hydrate.
  await browser.waitUntil(async () => (await browser.$$(".reference-page")).length >= 3, {
    timeout: 10_000, timeoutMsg: "Pokus2 linked references did not hydrate",
  });
  const refNames = [];
  for (const r of await browser.$$(".reference-page")) refNames.push(await r.getText());
  console.log("LINKED-REF pages before rename:", JSON.stringify(refNames));

  // Replicate "Tine is an open/pinned tab": click through to Tine (active view),
  // then back to Pokus2, before renaming.
  const tineRef = await browser.$(".reference-page=Tine");
  await tineRef.waitForExist({ timeout: 10_000, timeoutMsg: "no Tine linked-reference header to exercise open-page rename state" });
  await tineRef.click();
  await waitForTitle("Tine", "Tine linked reference");
  const back = await browser.$('button[title="Go back"]');
  if (!(await back.isExisting()) || !(await back.isEnabled())) {
    throw new Error("Tine's Go back button was not available after opening the linked reference");
  }
  await back.click();
  await waitForTitle("Pokus2", "history back");

  if (process.env.TINE_E2E_RENAME === "ctx") {
    // Context-menu rename: right-click the title, click "Rename page…", fill, Enter.
    await browser.execute(() => {
      const t = document.querySelector(".page-title");
      const r = t.getBoundingClientRect();
      t.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 6, clientY: r.top + 6, view: window }));
    });
    await sleep(700);
    const clicked = await browser.execute(() => {
      const it = [...document.querySelectorAll(".ctx-item")].find((e) => e.textContent.trim().startsWith("Rename page"));
      if (it) { it.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window })); return true; }
      return false;
    });
    console.log("clicked 'Rename page…' item:", clicked);
    const ri = await browser.$(".ctx-rename-name");
    await ri.waitForExist({ timeout: 4000, timeoutMsg: "context-menu rename input never appeared" });
    await fillAndEnter(".ctx-rename-name", "Pokus");
  } else {
    await renameByTitle("Pokus");
  }
  await waitForTitle("Pokus", "plain rename");
  await waitForFiles(() => read("pages/Pokus.md") !== null && read("pages/Pokus2.md") === null,
    "rename did not move Pokus2.md to Pokus.md");
  if (read("pages/Pokus.md") !== "- Tohle je pokus\n") throw new Error("renamed page content changed");
  for (const file of ["pages/Tine.md", "pages/Testtest2.md", "journals/2026_06_24.md"]) {
    const body = read(file);
    if (!body.includes("[[Pokus]]") || body.includes("[[Pokus2]]")) throw new Error(`references not rewritten in ${file}`);
  }
  console.log("PASS 1: renamed intended page and rewrote every reference");

  // ---- Journey 2: title:: identity ----------------------------------------
  await openPageByName(browser, "Effective");
  await browser.waitUntil(async () => (await browser.$$(".reference-page")).length >= 1, {
    timeout: 10_000, timeoutMsg: "Effective's linked reference from Title Ref did not appear",
  });
  const titleRefs = [];
  for (const r of await browser.$$(".reference-page")) titleRefs.push(await r.getText());
  if (!titleRefs.includes("Title Ref")) throw new Error(`Effective's linked references: ${JSON.stringify(titleRefs)}`);
  // The physical filename is not a page name: the switcher must not offer "Physical".
  await browser.keys(["Control", "k"]);
  const switcher = await browser.$(".switcher-input");
  await switcher.waitForExist({ timeout: 5000 });
  await switcher.setValue("Physical");
  await sleep(800); // let the debounced query settle before reading the offer
  const offered = await switcherPageRows(browser);
  await browser.keys(["Escape"]);
  if (offered.includes("Physical")) throw new Error(`switcher offers the physical filename as a page: ${JSON.stringify(offered)}`);
  await waitForTitle("Effective", "switcher dismissal");

  await renameByTitle("Renamed");
  await expectLandedOn("Renamed", "title-owned rename");
  await waitForFiles(() => read("pages/Physical.md") === null && read("pages/Renamed.md") !== null,
    "title-owned rename did not move pages/Physical.md to pages/Renamed.md");
  // Exact bytes = master's output on the same action (11e written-byte differential).
  const renamedBytes = read("pages/Renamed.md");
  if (renamedBytes !== "title:: Renamed\n\n- [[Renamed]] body\n") throw new Error(`title not rebound: ${JSON.stringify(renamedBytes)}`);
  if (read("pages/Title Ref.md") !== "- [[Renamed]]\n") throw new Error(`title referrer not rewritten: ${JSON.stringify(read("pages/Title Ref.md"))}`);
  console.log("PASS 2: title:: page routed by its effective name and renamed with its title rebound");

  // ---- Journey 3: rename onto an existing page merges ---------------------
  await openPageByName(browser, "Merge Source");
  const before = Object.fromEntries(["pages/Merge Source.md", "pages/Merge Survivor.md", "pages/Merge Ref.md"].map((f) => [f, read(f)]));
  await renameByTitle("Merge Survivor");
  const declined = await answerNativeDialog("no", { env });
  console.log("declined native dialog:", JSON.stringify(declined));
  await sleep(1500); // a declined merge writes nothing; give a wrong write time to land
  for (const [file, bytes] of Object.entries(before)) {
    if (read(file) !== bytes) throw new Error(`declined merge changed ${file}: ${JSON.stringify(read(file))}`);
  }
  await waitForTitle("Merge Source", "declined merge");

  await renameByTitle("Merge Survivor");
  const accepted = await answerNativeDialog("yes", { env });
  console.log("accepted native dialog:", JSON.stringify(accepted));
  await expectLandedOn("Merge Survivor", "accepted merge");
  await waitForFiles(() => read("pages/Merge Source.md") === null, "merge left the source page file in pages/");
  const survivor = read("pages/Merge Survivor.md");
  const aliasLine = survivor.split("\n").find((line) => line.startsWith("alias::")) ?? "";
  if (!/\bKept\b/.test(aliasLine) || !/\bFormer\b/.test(aliasLine)) throw new Error(`aliases not united: ${JSON.stringify(survivor)}`);
  const survivorAt = survivor.indexOf("survivor body");
  const sourceAt = survivor.indexOf("source body");
  if (survivorAt < 0 || sourceAt < survivorAt || survivor.lastIndexOf("source body") !== sourceAt) {
    throw new Error(`merged survivor does not hold survivor then exactly one source block: ${JSON.stringify(survivor)}`);
  }
  const mergeRef = read("pages/Merge Ref.md");
  if (!mergeRef.includes("[[Merge Survivor]]") || mergeRef.includes("[[Merge Source]]")) {
    throw new Error(`merge referrer not rewritten: ${JSON.stringify(mergeRef)}`);
  }
  const trashed = filesUnder(`${G}/logseq`).filter((file) => fs.readFileSync(file, "utf8") === MERGE_SOURCE);
  if (trashed.length !== 1) throw new Error(`merged source was not kept once in graph trash: ${JSON.stringify(trashed)}`);
  // The united alias still resolves: the old alias routes to the survivor.
  await openPageByName(browser, "Merge Ref");
  const clickedAlias = await browser.execute(() => {
    const link = [...document.querySelectorAll(".page-blocks .page-ref")]
      .find((node) => (node.textContent ?? "").replace(/^\[\[|\]\]$/g, "").trim() === "Former");
    if (!link) return false;
    link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  });
  if (!clickedAlias) throw new Error("Merge Ref lost its [[Former]] link");
  await waitForTitle("Merge Survivor", "[[Former]] (alias of the merged source)", 10_000);
  console.log("PASS 3: declined merge wrote nothing; accepted merge united content, aliases and references");
  if (routeFailures.length) throw new Error(`renamed page was not shown after ${routeFailures.length} rename(s): ${routeFailures.join(" / ")}`);
  console.log("PASS: every rename landed on the renamed page");
} catch (e) {
  console.log("E2E ERROR:", String(e).split("\n").slice(0, 4).join(" | "));
  try { await browser?.saveScreenshot(`${ARTIFACTS}/rename-failure.png`); } catch {}
  process.exitCode = 1;
} finally {
  dump("AFTER");
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(tdLog);
}
