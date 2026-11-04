// CM step 2 (S14, EMPTY-ALIAS, D-20261103-CMA): with `Machine Learning`
// declaring `alias:: ML` and `pages/ML.md` having no real content, a click on a
// rendered [[ML]] opens Machine Learning, as Logseq does
// (`page-empty-or-dummy?` + `get-redirect-page-name`). The alias group's
// "Related pages" row still opens the empty ML page itself; typing its first
// character makes [[ML]] open it, and clearing it again makes [[ML]] open
// Machine Learning, without a restart. UI-CM-EMPTY-ALIAS-LINK-OPENS-EMPTY-PAGE.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const TMP = fs.mkdtempSync("/tmp/tine-cm-empty-alias-");
const GRAPH = path.join(TMP, "graph");
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");
const port = Number(process.env.E2E_DRIVER_PORT || 4738);
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.mkdirSync(ARTIFACT, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{:tine/guide-announced? true}\n");
const files = {
  "pages/Machine Learning.md": "alias:: ML\n\n- Machine Learning body\n",
  "pages/ML.md": "",
  "pages/Source.md": "- See [[ML]]\n",
};
for (const [rel, text] of Object.entries(files)) fs.writeFileSync(path.join(GRAPH, rel), text);
const ML = path.join(GRAPH, "pages/ML.md");
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(ARTIFACT, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, graph: GRAPH, steps: [] };
const bodyText = () => browser.execute(() => document.querySelector(".main-content .page-blocks")?.textContent ?? "");

/** Native click on the link whose text is `text`, found by `selector`. */
async function clickLink(selector, text) {
  const point = await browser.waitUntil(() => browser.execute((sel, wanted) => {
    const undecorate = (value) => (value ?? "").trim().replace(/^\[\[/, "").replace(/\]\]$/, "").trim();
    const link = [...document.querySelectorAll(sel)].find((node) => undecorate(node.textContent) === wanted);
    if (!link) return false;
    link.scrollIntoView({ block: "center" });
    const rect = link.getClientRects()[0];
    const x = Math.round(rect.x + rect.width / 2), y = Math.round(rect.y + rect.height / 2);
    if (!link.contains(document.elementFromPoint(x, y))) return false;
    return { x, y };
  }, selector, text), { timeout: 15_000, interval: 150, timeoutMsg: `no clickable link ${JSON.stringify(text)} at ${selector}` });
  await browser.performActions([
    { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", ...point }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }] },
  ]);
  await browser.releaseActions();
}

async function expectPage(title, body, what) {
  try {
    await browser.waitUntil(async () => (await currentPageTitle(browser)) === title && (body === null || (await bodyText()).includes(body)),
      { timeout: 15_000, interval: 150 });
  } catch {
    throw new Error(`${what}: opened ${JSON.stringify(await currentPageTitle(browser))} showing ${JSON.stringify((await bodyText()).slice(0, 200))}; expected ${JSON.stringify(title)}${body === null ? "" : ` with ${JSON.stringify(body)}`}`);
  }
  proof.steps.push(`${what}: ${title}`);
}

/** Open Source and click its rendered [[ML]]. */
async function clickMlFromSource() {
  await openPageByName(browser, "Source");
  await clickLink(".main-content .page-blocks a.page-ref", "ML");
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });

  await clickMlFromSource();
  await expectPage("Machine Learning", "Machine Learning body", "[[ML]] with an empty ML.md opens the alias owner");

  // The alias group's Related pages row opens the empty file on purpose.
  await clickLink(".main-content [data-alias-group-pages] a.page-ref", "ML");
  await expectPage("ML", null, "the alias group's Related pages row opens the empty ML page itself");

  // Typing its first text: [[ML]] now opens ML, no restart.
  await browser.$(".main-content [data-block-id] .block-content-wrapper").click();
  await browser.waitUntil(() => browser.execute(() => {
    const active = document.activeElement;
    return Boolean(active && active.closest(".main-content") && (active instanceof HTMLTextAreaElement || active.isContentEditable));
  }), { timeout: 10_000, timeoutMsg: "no editor took focus on the empty ML page" });
  await browser.keys("my own notes");
  await browser.keys("Escape");
  await waitForFileText(ML, (text) => text.includes("my own notes"), { timeoutMs: 15_000 });
  proof.steps.push(`typed into ML: file ${JSON.stringify(fs.readFileSync(ML, "utf8"))}`);
  await clickMlFromSource();
  await expectPage("ML", "my own notes", "[[ML]] after typing into ML.md opens ML");

  // Clearing it again: [[ML]] opens Machine Learning again.
  await browser.$(".main-content [data-block-id] .block-content-wrapper").click();
  await browser.waitUntil(() => browser.execute(() => {
    const active = document.activeElement;
    return Boolean(active && active.closest(".main-content") && (active instanceof HTMLTextAreaElement || active.isContentEditable));
  }), { timeout: 10_000, timeoutMsg: "no editor took focus on the ML page" });
  await browser.keys(["Control", "a"]);
  await browser.keys("Backspace");
  await browser.keys("Escape");
  await waitForFileText(ML, (text) => !text.includes("my own notes"), { timeoutMs: 15_000 });
  proof.steps.push(`cleared ML: file ${JSON.stringify(fs.readFileSync(ML, "utf8"))}`);
  await clickMlFromSource();
  await expectPage("Machine Learning", "Machine Learning body", "[[ML]] after clearing ML.md opens the alias owner again");

  const listed = fs.readdirSync(path.join(GRAPH, "pages")).sort();
  if (JSON.stringify(listed) !== JSON.stringify(["ML.md", "Machine Learning.md", "Source.md"])) throw new Error(`navigation created pages: ${listed}`);
  for (const rel of ["pages/Machine Learning.md", "pages/Source.md"]) {
    if (fs.readFileSync(path.join(GRAPH, rel), "utf8") !== files[rel]) throw new Error(`navigation changed ${rel}`);
  }
  proof.steps.push("no page file created; the owner and the linking page keep their bytes");
  console.log(`PASS CM empty alias ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  if (browser) {
    await browser.saveScreenshot(path.join(ARTIFACT, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(ARTIFACT, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(ARTIFACT, "proof.json"), JSON.stringify(proof, null, 2));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
