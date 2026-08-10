// GH #304 / GH #254 increment 3, at the real observation boundary.
//
// Two files legitimately share one page name (the duplicate-day stray of #21, or
// same-titled pages in different folders). Opening the second one while the first
// holds unsaved edits used to replace the loaded instance outright: the edit was
// destroyed, and the dirty mark survived to describe the replacement's content.
//
// The journey deliberately does not stop at "the edit survived". A refusal that
// leaves the user unable to ever reach the file they asked for is a trap, not a
// safeguard, so this also resolves the incumbent and proves the requested file
// then arrives. That second half is what a survival-only test would miss.
//
// Usage: node scripts/e2e-dirty-editor-replacement.mjs
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import { ensureDisplay } from "./lib/e2e-display.mjs";

await ensureDisplay();

const G = "/tmp/tgraph-dirty-replacement";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4464);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4465);

// Two real files, one page name. `pages/Note.md` is the canonical one; the copy
// under `pages/archive/` shares its title and is reachable only by path.
function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages/archive`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  fs.writeFileSync(`${G}/pages/Note.md`, "- canonical body\n");
  fs.writeFileSync(`${G}/pages/archive/Note.md`, "- archived body\n");
  fs.writeFileSync(`${G}/journals/2026_06_24.md`, "- opens [[Note]]\n");
}

const read = (rel) => (fs.existsSync(`${G}/${rel}`) ? fs.readFileSync(`${G}/${rel}`, "utf8") : "(absent)");

seed();

fs.rmSync("/tmp/txdg-dirty", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/txdg-dirty/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg-dirty/data",
  XDG_CONFIG_HOME: "/tmp/txdg-dirty/config",
  XDG_CACHE_HOME: "/tmp/txdg-dirty/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/td-dirty.log", "w");
const td = spawn(
  TD,
  [
    "--port",
    String(DRIVER_PORT),
    "--native-port",
    String(NATIVE_PORT),
    "--native-driver",
    process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
  ],
  { env, stdio: ["ignore", tdLog, tdLog], detached: true },
);
await sleep(3000);

let browser;
let failure = null;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });

  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await sleep(1200);

  // Open the canonical Note and give it an unsaved edit.
  let opened = false;
  for (const sel of ["span.page-ref=Note", "a.page-ref=Note", ".page-ref=Note", "*=Note"]) {
    const el = await browser.$(sel);
    if (await el.isExisting()) {
      await el.click();
      opened = true;
      break;
    }
  }
  if (!opened) throw new Error("no [[Note]] link found in the journal feed");
  await sleep(1500);

  const block = await browser.$(".ls-block");
  await block.click();
  await sleep(400);
  await browser.keys(["End"]);
  await browser.keys(" UNSAVED".split(""));
  await sleep(400);

  const edited = await browser.$(".ls-block").getText();
  if (!edited.includes("UNSAVED")) throw new Error(`the edit did not land in the editor: ${edited}`);

  // Now ask for the OTHER file with the same name, by path, while that edit is
  // still unsaved. This is the exact user action GH #304 reported.
  await browser.execute((path) => {
    // The router's path-pinned entry point — the same one the sidebar and the
    // duplicate-day UI use.
    window.location.hash = `#/page-path/${encodeURIComponent(path)}`;
  }, "pages/archive/Note.md");
  await sleep(2000);

  // 1. The edit must still be here. This is the whole point.
  const afterText = await browser.$(".ls-block").getText();
  if (!afterText.includes("UNSAVED")) {
    throw new Error(`the unsaved edit was destroyed by the replacement: ${afterText}`);
  }
  if (afterText.includes("archived body")) {
    throw new Error("the archived file replaced the editor holding unsaved work");
  }

  // 2. The user must be told why they are not seeing the file they asked for.
  const toast = await browser.$(".toast, .toast-error, [role='alert']");
  const told = (await toast.isExisting()) ? await toast.getText() : "";
  if (!told.toLowerCase().includes("note")) {
    throw new Error(`no message naming the page holding the file back, saw: ${JSON.stringify(told)}`);
  }

  // 3. Non-wedging: resolve the incumbent, then the requested file must arrive.
  // A survival-only test would pass on an implementation that never shows it.
  await browser.keys(["Escape"]);
  await sleep(300);
  await browser.execute(() => window.dispatchEvent(new CustomEvent("tine:flush-all")));
  await sleep(2500);

  await browser.execute((path) => {
    window.location.hash = `#/page-path/${encodeURIComponent(path)}`;
  }, "pages/archive/Note.md");
  await sleep(2500);

  const arrived = await browser.$(".ls-block").getText();
  if (!arrived.includes("archived body")) {
    throw new Error(
      `after resolving the incumbent the requested file must arrive, got: ${JSON.stringify(arrived)}`,
    );
  }

  // 4. And the edit was written to the file it belonged to, not the other one.
  const canonical = read("pages/Note.md");
  const archived = read("pages/archive/Note.md");
  if (!canonical.includes("UNSAVED")) {
    throw new Error(`the edit did not reach its own file: ${JSON.stringify(canonical)}`);
  }
  if (archived.includes("UNSAVED")) {
    throw new Error(`the edit was written into the WRONG file: ${JSON.stringify(archived)}`);
  }

  console.log("PASS: unsaved edit survived, user was told why, requested file arrived after resolving");
} catch (error) {
  failure = error;
  console.error("FAIL:", error?.message ?? error);
  console.error("-- pages/Note.md:", JSON.stringify(read("pages/Note.md")));
  console.error("-- pages/archive/Note.md:", JSON.stringify(read("pages/archive/Note.md")));
  try {
    if (browser) fs.writeFileSync("/tmp/e2e-dirty-replacement.png", await browser.takeScreenshot(), "base64");
  } catch {}
} finally {
  try {
    if (browser) await browser.deleteSession();
  } catch {}
  try {
    process.kill(-td.pid, "SIGKILL");
  } catch {}
}

process.exit(failure ? 1 : 0);
