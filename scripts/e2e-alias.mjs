// Real-app regression for GH #62/#86/#138/#139. A first bullet becomes syntactically a
// page-property block as soon as the second ':' in `alias::` is typed. The page
// must not hide/unmount its textarea at that intermediate point.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4484);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4485);
const TMP = "/tmp/tine-alias-e2e";
const GRAPH = `${TMP}/graph`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/pages/Books.md`, "- \n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Books]]\n- A singular #book reference\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  const link = await browser.$(".page-ref");
  await link.waitForExist({ timeout: 10_000 });
  await link.click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()) === "Books", { timeout: 10_000 });

  const content = await browser.$(".page-blocks .ls-block .block-content-wrapper");
  await content.click();
  let editor = await browser.$(".page-blocks textarea.block-editor");
  await editor.waitForExist({ timeout: 5000 });
  console.log("alias editor mounted");
  await editor.addValue("alias:");
  console.log("typed alias:");
  await editor.addValue(":");
  console.log("typed second colon");

  // This is the exact old failure boundary: GH #86 hid the property block and
  // destroyed the focused editor before the alias value could be entered.
  editor = await browser.$(".page-blocks textarea.block-editor");
  if (!(await editor.isExisting())) throw new Error("first-bullet editor disappeared after typing alias::");
  console.log("alias editor remained mounted after delimiter");
  await editor.addValue(" book");
  console.log("typed alias value");

  // GH #138: Enter stays inside this special first property block. The next
  // property can be typed without creating a separate outline bullet; a second
  // Enter on the trailing blank line exits to an ordinary body bullet.
  await browser.keys(["Enter"]);
  editor = await browser.$(".page-blocks textarea.block-editor");
  if ((await editor.getValue()) !== "alias:: book\n") throw new Error("Enter left the first page-property block");
  await editor.addValue("tags:: blah，foobar");
  await browser.keys(["Enter"]);
  if ((await editor.getValue()) !== "alias:: book\ntags:: blah，foobar\n") {
    throw new Error(`second property did not stay in one block: ${JSON.stringify(await editor.getValue())}`);
  }
  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => (await browser.$$(".page-blocks textarea.block-editor")).length === 1, {
    timeout: 5_000, timeoutMsg: "double Enter did not create and focus one body bullet",
  });
  const bodyEditor = await browser.$(".page-blocks textarea.block-editor");
  if ((await bodyEditor.getValue()) !== "") throw new Error("double Enter did not focus the empty body bullet");

  await browser.waitUntil(async () => (await browser.$(".page-aliases").getText()).includes("book"), {
    timeout: 5000, timeoutMsg: "completed alias did not render as a page alias",
  });
  const propertyLinks = await browser.execute(() => [...document.querySelectorAll(".prop-row")]
    .find((row) => row.querySelector(".prop-key")?.textContent === "tags")
    ?.querySelectorAll(".page-ref").length ?? 0);
  if (propertyLinks !== 2) throw new Error(`tags property did not expose two page links: ${propertyLinks}`);
  await browser.$("h1.page-title").click();
  await browser.waitUntil(() => fs.readFileSync(`${GRAPH}/pages/Books.md`, "utf8").startsWith("alias:: book\n"), {
    timeout: 5000, timeoutMsg: "completed alias was not saved to disk",
  });
  const saved = fs.readFileSync(`${GRAPH}/pages/Books.md`, "utf8");
  if (/^- alias::/m.test(saved) || !saved.includes("tags:: blah，foobar")) {
    throw new Error(`new page properties did not persist as an unbulleted canonical header: ${JSON.stringify(saved)}`);
  }
  await browser.execute(() => [...document.querySelectorAll(".nav-item")]
    .find((element) => element.textContent?.trim() === "Journals")
    ?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 })));
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll("a.page-ref")]
    .some((element) => element.textContent?.includes("Books"))), {
    timeout: 5_000, timeoutMsg: "Journals did not expose the Books link for a cold reopen",
  });
  await browser.execute(() => [...document.querySelectorAll("a.page-ref")]
    .find((element) => element.textContent?.includes("Books"))
    ?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 })));
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()) === "Books", { timeout: 10_000 });
  if (!(await browser.$(".page-aliases").getText()).includes("book")) {
    throw new Error("reopened page did not parse the authored header as metadata");
  }
  console.log("PASS: first-bullet properties stay in one editor, persist unbulleted, and reopen as page metadata");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
