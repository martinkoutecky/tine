// BqbeQ: cold-launch time until the configured daily template fills an empty
// today on the realistic 10k graph, on a first launch (no launch checkpoint)
// and on a second launch (checkpoint present, today's file removed between).
// Run under xvfb-run. E2E_HOLD_LOAD=1 needs a tine-store/test-faults binary:
// the initial graph parse is held, so the template must appear before Ready.
// E2E_BASELINE=1 only records timing (an unchanged app may wait for Ready).
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { generateRealisticGraph } from "./generate-realistic-graph.mjs";

const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || "artifacts/journal-template-cold/native");
fs.mkdirSync(out, { recursive: true });
const trial = fs.mkdtempSync(path.join(out, "trial-"));
const graph = path.join(trial, "graph");
const manifest = await generateRealisticGraph({ root: graph, pages: 7000, journals: 3000, seed: 543 });
const now = new Date();
const stem = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const today = path.join(graph, "journals", `${stem}.md`);
fs.rmSync(today);
const SENTINEL = "BQBEQ-TEMPLATE-SENTINEL";
fs.writeFileSync(path.join(graph, "pages", "Daily templates.md"),
  `- Daily\n  template:: BqbeQ Daily\n  template-including-parent:: false\n  - ${SENTINEL} <%today%>\n  - Second row\n`);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:tine/guide-announced? true :default-templates {:journals "BqbeQ Daily"}}\n');
const hold = process.env.E2E_HOLD_LOAD === "1";
const pause = path.join(graph, ".tine-test-pause-load");
for (const name of ["data", "config", "cache"]) fs.mkdirSync(path.join(trial, name));
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(trial, "data"), XDG_CONFIG_HOME: path.join(trial, "config"), XDG_CACHE_HOME: path.join(trial, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4891);
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), manifest, hold, loadavg: os.loadavg(),
  journey: "cold Journals with a configured daily template on an empty today",
  expected: "template rows fill today; today's file is written once; nothing else changes", launches: [] };

async function launch(label) {
  const log = fs.openSync(path.join(trial, `driver-${label}.log`), "w");
  const driver = spawn("dbus-run-session", ["--", process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
    "--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
    { env, stdio: ["ignore", log, log], detached: true });
  await sleep(500);
  if (hold) fs.writeFileSync(pause, "");
  let browser;
  const started = performance.now();
  const step = { label };
  try {
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
    step.sessionMs = performance.now() - started;
    await browser.waitUntil(() => browser.execute((needle) => [...document.querySelectorAll(".main-content .block-content-wrapper")]
      .some((el) => el.textContent.includes(needle)), SENTINEL), { timeout: hold ? 15000 : 180000, interval: 50,
      timeoutMsg: "the daily template did not fill today" });
    step.templateMs = performance.now() - started;
    await browser.saveScreenshot(path.join(trial, `${label}-template.png`));
    if (hold) {
      step.heldWhenShown = fs.existsSync(pause);
      fs.unlinkSync(pause);
    }
    // The save is the audited host write; wait for its bytes, never a sleep.
    const deadline = Date.now() + 60000;
    while (!(fs.existsSync(today) && fs.readFileSync(today, "utf8").includes(SENTINEL))) {
      if (Date.now() > deadline) throw new Error("today's template was not saved");
      await sleep(100);
    }
    const text = fs.readFileSync(today, "utf8");
    if (text.split(SENTINEL).length !== 2) throw new Error(`template applied more than once: ${JSON.stringify(text)}`);
    if (text.includes("<%")) throw new Error("template variables were not resolved");
    step.savedBytes = text.length;
    if (label === "first") {
      // Let the first launch leave its launch checkpoint (written a few
      // seconds after the cold build), so the second launch is warm.
      const dir = path.join(trial, "data");
      const found = () => fs.readdirSync(dir, { recursive: true }).some((rel) => String(rel).includes("launch-checkpoints") && String(rel).endsWith(".bin"));
      const by = Date.now() + 180000;
      while (!found()) {
        if (Date.now() > by) throw new Error("the first launch left no launch checkpoint");
        await sleep(250);
      }
      step.checkpoint = true;
    }
    proof.launches.push(step);
  } catch (error) {
    step.observed = String(error);
    proof.launches.push(step);
    if (browser) {
      await browser.saveScreenshot(path.join(trial, `${label}-failure.png`)).catch(() => {});
      fs.writeFileSync(path.join(trial, `${label}-failure-dom.html`), await browser.execute(() => document.body.innerHTML).catch(() => ""));
    }
    throw error;
  } finally {
    if (hold && fs.existsSync(pause)) fs.unlinkSync(pause);
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGTERM"); } catch {}
    fs.closeSync(log);
    await sleep(1000);
  }
}

try {
  await launch("first");
  // The second launch reopens with the launch checkpoint the first one left.
  fs.rmSync(today);
  await launch("second");
  console.log("PASS", JSON.stringify(proof));
} finally {
  fs.writeFileSync(path.join(trial, "proof.json"), JSON.stringify(proof, null, 2));
}
