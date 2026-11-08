// GH #875 (sync copies): Syncthing-style copies identical to their page are
// discarded; a resolved copy's identical twin goes too; an identical-looking
// copy offers Discard copy. Scratch data only.
import "./lib/e2e-auto-account-home.mjs";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/fix-beta-sync-copy-875");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const page = path.join(graph, "pages/Copies 875.md");
fs.writeFileSync(page, "- alpha\n- beta\n");
const copyPath = stamp => path.join(graph, `pages/Copies 875.sync-conflict-20261107-${stamp}-ABCDEFG.md`);
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const env = { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4924);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("/usr/bin/git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex") };
let browser, stage = "startup";
const box = () => browser.execute(() => Boolean(document.querySelector('.main-content .page-conflict[data-source="sync-copy"]')));
const trashed = () => {
  const out = [];
  const walk = dir => { for (const e of fs.existsSync(dir) ? fs.readdirSync(dir, { withFileTypes: true }) : []) {
    const p = path.join(dir, e.name); if (e.isDirectory()) walk(p); else out.push(fs.readFileSync(p, "utf8")); } };
  walk(path.join(graph, "logseq/.tine-trash/conflicts"));
  return out;
};
const gone = (file, what) => browser.waitUntil(() => !fs.existsSync(file), { timeout: 15000, timeoutMsg: `${what} was not discarded` });
const shot = name => {
  try { execFileSync("import", ["-window", "root", path.join(out, name)], { env, timeout: 10000 }); } catch {}
};
const button = text => browser.execute(text => {
  const b = [...document.querySelectorAll(".main-content .page-conflict button")].find(b => b.textContent?.includes(text));
  b?.click(); return Boolean(b);
}, text);
try {
  const deadline = Date.now() + 15000;
  await new Promise((resolve, reject) => {
    const probe = () => {
      const socket = net.connect(port, "127.0.0.1");
      socket.once("connect", () => { socket.destroy(); resolve(); });
      socket.once("error", error => {
        socket.destroy();
        if (Date.now() >= deadline || driver.exitCode !== null) reject(error);
        else setImmediate(probe);
      });
    };
    probe();
  });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app, "beta-sync-copy-875") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Copies 875");

  stage = "two byte-identical copies arrive together";
  const [a, b] = [copyPath("101010"), copyPath("101012")];
  fs.writeFileSync(a, "- alpha\n- beta\n");
  fs.writeFileSync(b, "- alpha\n- beta\n");
  await gone(a, "the first identical copy");
  await gone(b, "the second identical copy");
  assert.equal(trashed().filter(t => t === "- alpha\n- beta\n").length, 2, "both copies are recoverable from the trash");
  await browser.waitUntil(async () => !await box(), { timeout: 10000, timeoutMsg: "an identical copy left its box" });
  assert.equal(fs.readFileSync(page, "utf8"), "- alpha\n- beta\n");

  stage = "Keep sync copy, then the copy's identical twin goes";
  const [c, d] = [copyPath("111010"), copyPath("111012")];
  fs.writeFileSync(c, "- alpha\n- gamma\n");
  fs.writeFileSync(d, "- alpha\n- gamma\n");
  await browser.waitUntil(box, { timeout: 15000, timeoutMsg: "a differing copy showed no review" });
  shot("differing.png");
  await browser.waitUntil(() => button("Keep Sync copy"), { timeout: 10000, timeoutMsg: "no Keep Sync copy" });
  await browser.waitUntil(() => button("Apply resolution"), { timeout: 10000, timeoutMsg: "no Apply resolution" });
  await waitForFileText(page, text => text === "- alpha\n- gamma\n");
  await gone(c, "the resolved copy");
  await gone(d, "the resolved copy's identical twin");
  await browser.waitUntil(async () => !await box(), { timeout: 10000, timeoutMsg: "the twin's box stayed (GH #875)" });

  stage = "an identical-looking copy offers Discard copy";
  const e = copyPath("121010");
  fs.writeFileSync(e, "- alpha\n- gamma"); // no final newline: same blocks, other bytes
  await browser.waitUntil(box, { timeout: 15000, timeoutMsg: "a byte-different copy showed no review" });
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content .page-conflict")?.textContent.includes("nothing to decide")),
    { timeout: 10000, timeoutMsg: "the copy did not read as identical" });
  shot("identical-box.png");
  assert.ok(fs.existsSync(e), "a copy differing in a byte is never discarded automatically");
  assert.ok(await button("Discard copy"), "the identical box offers Discard copy");
  await gone(e, "the discarded copy");
  await browser.waitUntil(async () => !await box(), { timeout: 10000, timeoutMsg: "Discard copy left the box" });
  assert.equal(fs.readFileSync(page, "utf8"), "- alpha\n- gamma\n");
  shot("done.png");
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify({ ...proof, outcome: "PASS" }, null, 2));
  console.log(`PASS BETA-SYNC-COPY-875: ${out}`);
} catch (error) {
  shot("failure.png");
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Identical sync copies are discarded; differing ones are reviewed; identical-looking ones offer Discard copy",
    observed: String(error.stack), classification: "ambiguous; inspect stage and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
