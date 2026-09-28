// Verifies a graph with a CUSTOM journal date format loads in the real app:
// seeds journals in `dd-MM-yyyy` filenames + a `dd.MM.yyyy` title format, launches
// the app, and checks the journal feed renders them (titled in the user's format).
//
// og batch 11 (journal filename proposals): a journal file named by its TITLE
// (`22.06.2026.md` under this config; a `yyyy-MM-dd` stem is
// already a date name to Tine, so the title format here must not be ISO) is only PROPOSED for renaming. Opening the
// graph leaves it byte-identical; Settings > Backups & recovery lists the exact
// `from -> to` pair; declining the native confirmation changes nothing; accepting
// renames it to its date name with identical bytes after a `pre-journal-rename`
// snapshot that still holds the original name.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";
import { FEED_LOAD_FAILURE, watchErrorToasts } from "./lib/e2e-toasts.mjs";

const TMP = process.env.E2E_TMP_DIR || `/tmp/tine-journal-format-e2e-${process.pid}`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;
const G = `${TMP}/graph`;
const XDG = `${TMP}/xdg`;
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4444);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4445);

const TITLE_NAMED = `${G}/journals/22.06.2026.md`;
const DATE_NAMED = `${G}/journals/22-06-2026.md`;
const TITLE_NAMED_BYTES = "- entry from the 22nd, named by its title\n";

fs.rmSync(TMP, { recursive: true, force: true });
fs.mkdirSync(`${G}/pages`, { recursive: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.mkdirSync(`${G}/logseq`, { recursive: true });
fs.writeFileSync(
  `${G}/logseq/config.edn`,
  '{:journal/file-name-format "dd-MM-yyyy" :journal/page-title-format "dd.MM.yyyy"}\n'
);
// Real journal files in the user's dd-MM-yyyy filename format.
fs.writeFileSync(`${G}/journals/24-06-2026.md`, "- entry from the 24th [[Pokus]]\n");
fs.writeFileSync(`${G}/journals/23-06-2026.md`, "- entry from the 23rd\n");
// A journal named by its title (left by a format change or another tool).
fs.writeFileSync(TITLE_NAMED, TITLE_NAMED_BYTES);
fs.writeFileSync(`${G}/pages/Pokus.md`, "- a page\n");

for (const d of ["data", "config", "cache"]) fs.mkdirSync(`${XDG}/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: `${XDG}/data`,
  XDG_CONFIG_HOME: `${XDG}/config`,
  XDG_CACHE_HOME: `${XDG}/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

fs.mkdirSync(ARTIFACTS, { recursive: true });
const tdLog = fs.openSync(path.join(ARTIFACTS, "journal-format-tauri-driver.log"), "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env,
  stdio: ["ignore", tdLog, tdLog],
  detached: true,
});
await sleep(3000);

const journalFiles = () => fs.readdirSync(`${G}/journals`).sort();
/** Every file under `dir` whose path has a directory containing `marker`. */
function snapshotCopies(dir, marker, name, inside = false, found = []) {
  let entries = [];
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return found; }
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) snapshotCopies(full, marker, name, inside || entry.name.includes(marker), found);
    else if (inside && entry.name === name) found.push(full);
  }
  return found;
}

let browser;
let step = "launch";
const t0 = Date.now();
const mark = (name) => { step = name; console.log(`[${((Date.now() - t0) / 1000).toFixed(1)}s] ${name}`); };
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });
  mark("startup error toasts");
  const startupErrors = await watchErrorToasts(browser);
  if (startupErrors.some((text) => text.includes(FEED_LOAD_FAILURE)))
    throw new Error(`startup reported a journal feed failure: ${JSON.stringify(startupErrors)}`);
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });

  mark("custom-format feed");
  await sleep(2500);
  const text = await browser.execute(() => document.body.innerText);
  const has = (s) => text.includes(s);
  const checks = {
    "feed shows 24.06.2026 (custom title)": has("24.06.2026"),
    "feed shows 23.06.2026 (custom title)": has("23.06.2026"),
    "does not show raw filename 24-06-2026": !has("24-06-2026"),
    "entry content present": has("entry from the 24th"),
    "opening the graph did not rename the title-named journal":
      fs.existsSync(TITLE_NAMED) && fs.readFileSync(TITLE_NAMED, "utf8") === TITLE_NAMED_BYTES && !fs.existsSync(DATE_NAMED),
  };
  for (const [name, ok] of Object.entries(checks)) console.log(`${ok ? "PASS" : "FAIL"}: ${name}`);
  if (Object.values(checks).some((ok) => !ok)) throw new Error("custom journal format invariants failed");

  mark("proposals panel");
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5000 });
  await browser.$("//button[contains(concat(' ', normalize-space(@class), ' '), ' settings-nav-item ') and normalize-space(.)='Backups & recovery']").click();
  const proposal = () => browser.execute(() => [...document.querySelectorAll(".settings-modal .settings-block")]
    .map((node) => (node.textContent ?? "").replace(/\s+/g, " ").trim())
    .filter((line) => line.includes("→")));
  await browser.waitUntil(async () => (await proposal()).includes("22.06.2026.md → 22-06-2026.md"), {
    timeout: 10_000, interval: 200,
    timeoutMsg: "Backups & recovery did not propose 22.06.2026.md → 22-06-2026.md",
  });
  const listed = await proposal();
  if (listed.length !== 1) throw new Error(`expected exactly one proposal, got ${JSON.stringify(listed)}`);
  console.log("PASS: Backups & recovery proposes exactly 22.06.2026.md → 22-06-2026.md");
  const renameButton = () => browser.$("//div[contains(concat(' ', normalize-space(@class), ' '), ' settings-modal ')]//button[normalize-space(.)='Rename to date names']");

  mark("declined confirmation");
  await (await renameButton()).click();
  await answerNativeDialog("no", { env });
  await sleep(1000);
  if (JSON.stringify(journalFiles()) !== JSON.stringify(["22.06.2026.md", "23-06-2026.md", "24-06-2026.md"])
    || fs.readFileSync(TITLE_NAMED, "utf8") !== TITLE_NAMED_BYTES) {
    throw new Error(`declining the confirmation changed the journals: ${JSON.stringify(journalFiles())}`);
  }
  if (snapshotCopies(`${XDG}/data`, "pre-journal-rename", "22.06.2026.md").length) {
    throw new Error("declining the confirmation still took a pre-journal-rename snapshot");
  }
  console.log("PASS: declining the native confirmation renamed nothing");

  mark("accepted confirmation");
  await (await renameButton()).click();
  await answerNativeDialog("yes", { env });
  await browser.waitUntil(() => fs.existsSync(DATE_NAMED) && !fs.existsSync(TITLE_NAMED), {
    timeout: 10_000, timeoutMsg: `accepting did not rename the journal: ${JSON.stringify(journalFiles())}`,
  });
  if (fs.readFileSync(DATE_NAMED, "utf8") !== TITLE_NAMED_BYTES) {
    throw new Error(`renamed journal changed bytes: ${JSON.stringify(fs.readFileSync(DATE_NAMED, "utf8"))}`);
  }
  if (JSON.stringify(journalFiles()) !== JSON.stringify(["22-06-2026.md", "23-06-2026.md", "24-06-2026.md"])) {
    throw new Error(`unexpected journal files after rename: ${JSON.stringify(journalFiles())}`);
  }
  const copies = snapshotCopies(`${XDG}/data`, "pre-journal-rename", "22.06.2026.md");
  if (copies.length !== 1 || fs.readFileSync(copies[0], "utf8") !== TITLE_NAMED_BYTES) {
    throw new Error(`expected one pre-journal-rename snapshot holding the original name, found ${JSON.stringify(copies)}`);
  }
  await browser.waitUntil(async () => (await proposal()).length === 0, {
    timeout: 10_000, timeoutMsg: "the applied proposal was still listed",
  });
  console.log("PASS: accepting renamed it to 22-06-2026.md with identical bytes after a pre-journal-rename snapshot");
} catch (e) {
  console.log(`CHECK ERROR at ${step}:`, String(e).split("\n").slice(0, 3).join(" | "));
  try {
    console.log("settings text:", JSON.stringify(await browser.execute(() =>
      (document.querySelector(".settings-modal")?.textContent ?? "<no settings modal>").replace(/\s+/g, " ").slice(0, 3000))));
    console.log("toasts:", JSON.stringify(await browser.execute(() => [...document.querySelectorAll(".toast")].map((node) => node.textContent))));
  } catch {}
  console.log("journals:", JSON.stringify(journalFiles()));
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
}
