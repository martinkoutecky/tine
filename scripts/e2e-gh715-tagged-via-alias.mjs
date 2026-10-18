// GH #715: "Pages tagged with" on a page lists pages tagged with any of its
// aliases (the Linked References alias group), in the main pane and in the
// right sidebar, while a `(page-tags …)` query macro keeps OG's literal match.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = process.cwd();
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || "artifacts/gh715/native");
fs.mkdirSync(OUT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(OUT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
const files = {
  "page_A": "alias:: my_important_page\n\n- Canonical body\n",
  // The reporter's case: tagged through the alias.
  "page_X": "tags:: my_important_page\n\n- X body\n",
  "page_Y": "tags:: page_A\n\n- Y body\n",
  "page_Z": "tags:: unrelated\n\n- Z body\n",
  "Literal": "- {{query (page-tags page_A)}}\n",
  // A page tagged with its own alias lists itself, as Logseq lists a page
  // tagged with its own name.
  "Selfie": "alias:: selfie_alias\ntags:: selfie_alias\n\n- Selfie body\n",
};
for (const [name, text] of Object.entries(files)) fs.writeFileSync(path.join(GRAPH, `pages/${name}.md`), text);
const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4799);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"), ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: process.env.E2E_TESTED_COMMIT, binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), steps: [] };
const rows = (scope) => browser.execute((selector) => [...document.querySelectorAll(selector)].map(li => li.textContent.trim()).join("|"), `${scope} .tagged-pages li`);
const expectRows = (scope, wanted, what) => browser.waitUntil(async () => (await rows(scope)) === wanted, { timeout: 15_000, timeoutMsg: `${what}: expected tagged rows ${wanted}` }).catch(async (error) => {
  throw new Error(`${error.message}; saw ${JSON.stringify(await rows(scope))}`);
});
let browser;
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });

  await openPageByName(browser, "page_A");
  await expectRows(".main-content", "page_X|page_Y", "main pane, canonical page");
  proof.steps.push("page_A main pane: page_X (tagged via alias) and page_Y (canonical)");

  // Opened through the alias, the routed page is page_A with the same rows.
  await openPageByName(browser, "page_Y");
  await openPageByName(browser, "page_A", { query: "my_important_page" });
  await expectRows(".main-content", "page_X|page_Y", "main pane, opened via alias");
  proof.steps.push("opened via the alias: same rows");

  // The right sidebar mounts the section under the name the link names.
  await openPageByName(browser, "page_X");
  await browser.waitUntil(() => browser.execute(() => {
    const ref = [...document.querySelectorAll(".main-content .prop-row .page-ref")].find(a => a.textContent.trim() === "my_important_page");
    if (!ref) return false;
    ref.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, shiftKey: true }));
    return true;
  }), { timeout: 15_000, timeoutMsg: "page_X: alias tag link to shift-click" });
  await expectRows(".rs-item", "page_X|page_Y", "right sidebar, alias link");
  proof.steps.push("right sidebar via the alias tag link: same rows");

  await openPageByName(browser, "Selfie");
  await expectRows(".main-content", "Selfie", "main pane, page tagged with its own alias");
  proof.steps.push("Selfie, tagged with its own alias, lists itself");

  // `(page-tags page_A)` stays literal: page_Y only.
  await openPageByName(browser, "Literal");
  await browser.waitUntil(() => browser.execute(() => {
    const text = document.querySelector(".main-content .query-block")?.textContent ?? "";
    return text.includes("page_Y");
  }), { timeout: 15_000, timeoutMsg: "Literal: query lists page_Y" });
  const literal = await browser.execute(() => document.querySelector(".main-content .query-block")?.textContent ?? "");
  if (literal.includes("page_X")) throw new Error(`(page-tags page_A) matched through the alias: ${literal}`);
  proof.steps.push("(page-tags page_A) query lists page_Y, not page_X");

  for (const [name, text] of Object.entries(files)) {
    if (fs.readFileSync(path.join(GRAPH, `pages/${name}.md`), "utf8") !== text) throw new Error(`${name}: browsing changed source file`);
  }
  proof.steps.push("all source files remain byte-identical after browsing");
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = "ambiguous; inspect captured DOM and driver log";
  if (browser) {
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), JSON.stringify(proof, null, 2));
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
