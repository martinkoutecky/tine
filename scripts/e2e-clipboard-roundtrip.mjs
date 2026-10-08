// Native WebKitGTK clipboard text round-trip gate for CB1 §3g.
//
// Run with the native environment, for example:
//   source scripts/env.sh && xvfb-run -a node scripts/e2e-clipboard-roundtrip.mjs
//
// `backend().writeRich` falls through to the clipboard-manager plugin's
// `write_text` command on this non-secure WebKit webview. The backend module is
// deliberately not a window global, so this WebDriver probe calls that exposed
// Tauri command from the real webview. It then performs a native Ctrl+V into a
// block editor and captures the actual ClipboardEvent before application paste
// handling, keeping the evidence independent of CB1's paste association code.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TAURI_DRIVER =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4510);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4511);
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "tine-clipboard-roundtrip-"));
const GRAPH = path.join(TMP, "graph");
const PAGE_NAME = "Clipboard Probe";
const PAGE = path.join(GRAPH, "pages", `${PAGE_NAME}.md`);

const fixtures = [
  { name: "multiline-lf", text: "first line\nsecond line\nthird line" },
  { name: "crlf-input", text: "first line\r\nsecond line\r\nthird line" },
  { name: "zero-trailing-newlines", text: "no final newline" },
  { name: "one-trailing-newline", text: "one final newline\n" },
  { name: "multiple-trailing-newlines", text: "several final newlines\n\n\n" },
  { name: "non-ascii", text: "Příliš žluťoučký kůň 🦊\n漢字とかな 😀" },
];

function normalize(text) {
  const lf = text.replace(/\r\n/g, "\n");
  return lf.endsWith("\n") ? lf.slice(0, -1) : lf;
}

function seedGraph() {
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
  fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
  fs.writeFileSync(PAGE, "- paste target\n");
  const now = new Date();
  const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
  fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), `- open [[${PAGE_NAME}]]\n`);
}

if (!fs.existsSync(APP)) {
  throw new Error(`HARNESS UNAVAILABLE: release app binary is missing at ${APP}; this probe does not build it.`);
}

seedGraph();
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const log = fs.openSync(path.join(TMP, "tauri-driver.log"), "w");
const td = spawn(
  TAURI_DRIVER,
  [
    "--port", String(DRIVER_PORT),
    "--native-port", String(NATIVE_PORT),
    "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
  ],
  { env, stdio: ["ignore", log, log], detached: true },
);

let browser;
const results = [];
try {
  await sleep(2500);
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
  });

  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  let opened = false;
  for (const selector of [`a.page-ref=${PAGE_NAME}`, `span.page-ref=${PAGE_NAME}`, `*=${PAGE_NAME}`]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) {
      await link.click();
      opened = true;
      break;
    }
  }
  if (!opened) throw new Error(`could not open seeded ${PAGE_NAME} page`);
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === PAGE_NAME, {
    timeout: 10_000,
    timeoutMsg: "clipboard probe page did not open",
  });

  const target = await browser.$(".page-blocks .ls-block .block-content, .ls-block .block-content");
  await target.click();
  const editor = await browser.$("textarea.block-editor");
  await editor.waitForExist({ timeout: 5_000 });
  const instrumentation = await browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor");
    if (!editor) return { ok: false, error: "block editor textarea not found" };
    globalThis.__tineClipboardRoundtripCapture = null;
    const listener = (event) => {
      if (!(event.target instanceof HTMLTextAreaElement) || !event.target.matches("textarea.block-editor")) return;
      globalThis.__tineClipboardRoundtripCapture = {
        observed: event.clipboardData?.getData("text/plain") ?? null,
        types: event.clipboardData ? [...event.clipboardData.types] : [],
        trusted: event.isTrusted,
      };
      // This is an observation-only probe. Do not let the seeded graph or
      // stage-B paste branch change the next fixture's target editor.
      event.preventDefault();
      event.stopImmediatePropagation();
    };
    document.addEventListener("paste", listener, true);
    return { ok: true };
  });
  if (!instrumentation.ok) throw new Error(instrumentation.error);

  for (const fixture of fixtures) {
    await browser.execute(() => {
      globalThis.__tineClipboardRoundtripCapture = null;
    });
    const written = await browser.executeAsync((text, done) => {
      const invoke = globalThis.__TAURI_INTERNALS__?.invoke;
      if (typeof invoke !== "function") {
        done({ ok: false, error: "Tauri invoke is unavailable in the real webview" });
        return;
      }
      invoke("plugin:clipboard-manager|write_text", { text })
        .then(() => done({ ok: true }))
        .catch((error) => done({ ok: false, error: String(error) }));
    }, fixture.text);
    if (!written.ok) throw new Error(`native writeRich transport failed for ${fixture.name}: ${written.error}`);

    await editor.click();
    await browser.keys(["Control", "v"]);
    await browser.waitUntil(
      () => browser.execute(() => globalThis.__tineClipboardRoundtripCapture),
      { timeout: 5_000, timeoutMsg: `native paste event was not captured for ${fixture.name}` },
    );
    const capture = await browser.execute(() => globalThis.__tineClipboardRoundtripCapture);
    const observed = capture?.observed;
    const pass = typeof observed === "string" && normalize(fixture.text) === normalize(observed);
    const result = {
      fixture: fixture.name,
      written: fixture.text,
      observed,
      normalizedWritten: normalize(fixture.text),
      normalizedObserved: typeof observed === "string" ? normalize(observed) : null,
      types: capture?.types ?? [],
      trusted: capture?.trusted ?? false,
      pass,
    };
    results.push(result);
    console.log(`ROUNDTRIP ${JSON.stringify(result)}`);
    if (!pass) throw new Error(`round-trip mismatch for ${fixture.name}: written=${JSON.stringify(fixture.text)} observed=${JSON.stringify(observed)}`);
  }

  console.log(`PASS: ${results.length} native clipboard text round-trip fixtures normalized as specified`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
