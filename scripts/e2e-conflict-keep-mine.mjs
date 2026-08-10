// GH #254: the conflict banner at the real observation boundary.
//
// This is the sequence increment 2's adversarial rounds 3 and 4 EACH found
// broken, in different places, and which no test outside a mocked backend has
// ever driven: a banner is up, a SECOND external write lands underneath it, and
// the user clicks "Keep mine". The newer winner must survive that click — it is
// bytes the user was never shown — and the banner must come back live for the
// conflict they now actually have.
//
// The failure mode is silent. "Keep mine" appears to work; what is lost is
// someone else's write, which the user cannot see was ever there.
//
// Usage: node scripts/e2e-conflict-keep-mine.mjs
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import { ensureDisplay } from "./lib/e2e-display.mjs";

await ensureDisplay();

const G = "/tmp/tgraph-keep-mine";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4466);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4467);

function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  fs.writeFileSync(`${G}/pages/Note.md`, "- original body\n");
  fs.writeFileSync(`${G}/journals/2026_06_24.md`, "- opens [[Note]]\n");
}
const read = () => (fs.existsSync(`${G}/pages/Note.md`) ? fs.readFileSync(`${G}/pages/Note.md`, "utf8") : "(absent)");

seed();

fs.rmSync("/tmp/txdg-keepmine", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/txdg-keepmine/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg-keepmine/data",
  XDG_CONFIG_HOME: "/tmp/txdg-keepmine/config",
  XDG_CACHE_HOME: "/tmp/txdg-keepmine/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/td-keepmine.log", "w");
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

const bannerText = async (browser) => {
  const el = await browser.$(".conflict-banner");
  return (await el.isExisting()) ? await el.getText() : "";
};

// The visible editor text. A block being edited swaps its rendered div for a
// textarea, whose value is NOT part of the element's text, so reading the block
// alone reports an empty string for exactly the state these journeys care about.
const editorText = async (browser) => {
  const ta = await browser.$("textarea");
  if (await ta.isExisting()) {
    const value = await ta.getValue();
    if (value) return value;
  }
  return await browser.$(".ls-block").getText();
};

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
  await browser.keys(" MINE".split(""));

  // The first external winner must land while the edit is still PENDING — the
  // save debounce is 400 ms and restarts on each keystroke. Writing before the
  // edit instead lets the watcher reload the still-clean page and the save simply
  // succeeds, with no conflict to answer.
  fs.writeFileSync(`${G}/pages/Note.md`, "- first external winner\n");

  await browser.$(".conflict-banner").waitForExist({ timeout: 20000 });
  const first = await bannerText(browser);
  if (!first.toLowerCase().includes("note")) {
    throw new Error(`the banner must name the page, saw: ${JSON.stringify(first)}`);
  }

  // SECOND external winner, landing while that banner is still up. This is the
  // one the user has never been shown.
  fs.writeFileSync(`${G}/pages/Note.md`, "- second external winner\n");
  // A conflicted page is skipped by the ordinary save path, so the ONLY thing
  // that can mint authority for this newer winner is the re-observation the
  // watcher drives. Give it real time: if this needs longer than a user would
  // wait before clicking, that is itself the finding.
  await sleep(8000);

  // Answer the banner the user can see.
  const keep = await browser.$(".conflict-btn.keep");
  if (!(await keep.isExisting())) throw new Error("no \"Keep mine\" button on the banner");
  await keep.click();
  await sleep(3000);

  const onDisk = read();
  if (onDisk.includes("second external winner")) {
    // Correct: the click could not spend authority for a winner nobody saw.
    const again = await bannerText(browser);
    if (!again) {
      throw new Error(
        "the newer winner survived, but no banner came back — the user is left with an " +
          "unsaved edit and nothing to act on",
      );
    }
    console.log("PASS: the unseen winner survived and a live banner returned");
  } else if (onDisk.includes("MINE")) {
    throw new Error(
      "\"Keep mine\" overwrote an external write the user was never shown: " + JSON.stringify(onDisk),
    );
  } else {
    throw new Error(`unexpected file state after "Keep mine": ${JSON.stringify(onDisk)}`);
  }
} catch (error) {
  failure = error;
  console.error("FAIL:", error?.message ?? error);
  console.error("-- pages/Note.md:", JSON.stringify(read()));
  try {
    if (browser) fs.writeFileSync("/tmp/e2e-keep-mine.png", await browser.takeScreenshot(), "base64");
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
