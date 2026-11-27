// tine#1054: an Org page whose only content is a table with a #+TBLFM line (no headline) shows
// its table on the routed page, and is listed among the graph's pages; so do other Org table shapes.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver");
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/org-table-file/native"));
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
// The reported file, byte for byte (tine#1054).
const TABLE = [
  "- | Item   | Quantity | Unit Price | Total | Tax (21%) |",
  "  |--------+----------+------------+-------+-----------|",
  "  | Apple  |       10 |       0.50 |  5.00 |      1.05 |",
  "  | Banana |        5 |       0.30 |  1.50 |      0.32 |",
  "  | Orange |        8 |       0.40 |  3.20 |      0.67 |",
  "  |--------+----------+------------+-------+-----------|",
  "  | Total  |       23 |            |  9.70 |      2.04 |",
  "  #+TBLFM: $4=$2*$3::$5=$4*0.21::@>$2=vsum(@2..@-1)::$4=vsum(@2..@-1)::$5=vsum(@2..@-1)",
].join("\n");
const REPORTED = ["Item", "Banana", "0.32", "Total", "9.70"];
const small = (word) => `| ${word} | b |\n|---+---|\n| ${word}-cell | 2 |`;
// The page name, its file's bytes and cells the routed page must show. The reported file in each
// line ending a Windows or Linux editor saves (LF, CRLF, no final line break), then other Org table
// shapes: no list bullet, two formula lines, a #+NAME: line above, a table in a headline's body.
const CASES = [
  ["Table lf", `${TABLE}\n`, REPORTED],
  ["Table crlf", `${TABLE.replace(/\n/g, "\r\n")}\r\n`, REPORTED],
  ["Table no-final-eol", TABLE, REPORTED],
  ["Table bare", `${small("bare")}\n#+TBLFM: $2=$1\n`, ["bare", "bare-cell"]],
  ["Table two formulas", `${small("twofm")}\n#+TBLFM: $2=$1\n#+TBLFM: $1=3\n`, ["twofm", "twofm-cell"]],
  ["Table named", `#+NAME: prices\n${small("named")}\n#+TBLFM: $2=$1\n`, ["named", "named-cell"]],
  ["Table in heading", `* Prices\n${small("heading")}\n#+TBLFM: $2=$1\n`, ["heading", "heading-cell"]],
];
for (const [name, bytes] of CASES) fs.writeFileSync(path.join(GRAPH, `pages/${name}.org`), bytes);
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${day}.md`), "- today\n");
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const port = Number(process.env.E2E_DRIVER_PORT || 4888);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), graph: GRAPH, steps: [] };
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  for (const [name, bytes, expected] of CASES) {
    await openPageByName(browser, name);
    proof.steps.push(`${name}: the Org page opens by name (it is in the page list)`);
    // The table's cells, as a table, on the routed page.
    await browser.waitUntil(() => browser.execute((want) => {
      const cells = [...document.querySelectorAll(".main-content .page-blocks table td, .main-content .page-blocks table th")]
        .map((cell) => cell.textContent.trim());
      return want.every((text) => cells.includes(text));
    }, expected), { timeout: 30_000, interval: 200, timeoutMsg: `${name}: the Org table's cells are not shown on the routed page` });
    proof.steps.push(`${name}: the routed page shows the table's header and body cells`);
    if (fs.readFileSync(path.join(GRAPH, `pages/${name}.org`), "utf8") !== bytes) throw new Error(`${name}: opening the page changed the file`);
    proof.steps.push(`${name}: opening the page leaves the file unchanged`);
  }
  fs.writeFileSync(path.join(TMP, "final-dom.html"), await browser.execute(() => document.body.innerHTML));
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = "ambiguous; inspect DOM and driver log";
  if (browser) {
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
