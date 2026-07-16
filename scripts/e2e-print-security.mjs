// Native Tauri/WebKit proof that the literal page-menu → PDF-export path creates
// only a static, script-disabled print frame. The observer removes the frame
// before load so CI never opens a platform print dialog.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4510);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4511);
const TMP = path.join(os.tmpdir(), "tine-print-security-e2e");
const GRAPH = path.join(TMP, "graph");

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
const oversizedAsset = path.join(GRAPH, "assets", "oversized.png");
fs.writeFileSync(oversizedAsset, "");
fs.truncateSync(oversizedAsset, 12 * 1024 * 1024 + 1);
fs.writeFileSync(path.join(GRAPH, "pages", "Print proof.md"), [
  "- Math $x^2$ stays typeset",
  "- ```rust",
  "  fn main() {}",
  "  ```",
  "- ![large](../assets/oversized.png)",
  "",
].join("\n"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), "- Open [[Print proof]]\n");

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
const driverArgs = webdriverServerArgs(
  DRIVER_PORT,
  NATIVE_PORT,
  process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
);
const td = spawn(TD, driverArgs, {
  env, stdio: ["ignore", log, log], detached: process.platform !== "win32",
});
await sleep(2500);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP),
  });
  await browser.$(".page-ref").waitForExist({ timeout: 20_000 });
  const routed = await browser.execute(() => {
    const ref = [...document.querySelectorAll(".page-ref")]
      .find((element) => element.textContent?.includes("Print proof"));
    if (!ref) return {
      ok: false,
      refs: [...document.querySelectorAll(".page-ref")].map((element) => element.textContent?.trim()),
      title: document.querySelector("h1.page-title")?.textContent?.trim(),
    };
    for (const type of ["mousedown", "mouseup", "click"]) {
      ref.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
    }
    return { ok: true, refs: [], title: "" };
  });
  if (!routed.ok) throw new Error(`print fixture page-ref is missing: ${JSON.stringify(routed)}`);
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Print proof", {
    timeout: 10_000, timeoutMsg: "could not route to print fixture",
  });

  await browser.execute(() => {
    window.__tinePrintSecurityProof = null;
    const observer = new MutationObserver((records) => {
      for (const record of records) {
        for (const node of record.addedNodes) {
          if (!(node instanceof HTMLIFrameElement) || !node.hasAttribute("srcdoc")) continue;
          window.__tinePrintSecurityProof = {
            sandbox: node.getAttribute("sandbox"),
            srcdoc: node.srcdoc,
          };
          node.remove();
          observer.disconnect();
          return;
        }
      }
    });
    observer.observe(document.body, { childList: true });
  });

  await browser.execute(() => document.querySelector("h1.page-title")?.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 120, clientY: 100 })
  ));
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".ctx-item")].some((item) => item.textContent?.trim() === "Export to PDF…")), {
    timeout: 5_000, timeoutMsg: "page PDF menu item did not appear",
  });
  await browser.execute(() => {
    const item = [...document.querySelectorAll(".ctx-item")]
      .find((candidate) => candidate.textContent?.trim() === "Export to PDF…");
    item?.click();
  });
  await browser.$(".pdf-export-modal").waitForExist({ timeout: 5_000 });
  await browser.$(".pdf-export-modal .export-btn-primary").click();
  await browser.waitUntil(() => browser.execute(() => Boolean(window.__tinePrintSecurityProof)), {
    timeout: 15_000, timeoutMsg: "PDF export did not create its print frame",
  });
  const proof = await browser.execute(() => window.__tinePrintSecurityProof);
  const sandbox = new Set((proof.sandbox ?? "").split(/\s+/).filter(Boolean));
  if (sandbox.has("allow-scripts") || !sandbox.has("allow-same-origin") || !sandbox.has("allow-modals")) {
    throw new Error(`unsafe print sandbox: ${JSON.stringify(proof.sandbox)}`);
  }
  if (/<script\b/i.test(proof.srcdoc) || /cdn\.jsdelivr\.net/i.test(proof.srcdoc)
    || !/script-src 'none'/.test(proof.srcdoc) || !/class="katex/.test(proof.srcdoc)
    || !/hljs-keyword/.test(proof.srcdoc) || !/print-asset-omitted/.test(proof.srcdoc)
    || /oversized\.png/.test(proof.srcdoc)) {
    throw new Error(`print document privilege boundary failed: ${proof.srcdoc.slice(0, 2_000)}`);
  }
  console.log("PASS: native page-to-PDF path locally renders a bounded, script-disabled sandboxed print document");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try {
    if (process.platform === "win32") td.kill("SIGKILL");
    else process.kill(-td.pid, "SIGKILL");
  } catch {}
  fs.closeSync(log);
}
