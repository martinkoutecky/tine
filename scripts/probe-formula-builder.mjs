// Real-app probe for the Sheets visual formula builder.
// Spawns tauri-driver in its own process group and kills the group on teardown.
// Usage: DISPLAY=:99 node scripts/probe-formula-builder.mjs
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/formula-builder";
const APP = process.env.TINE_APP || `${repo}/target/release/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME && fs.existsSync(`${process.env.CARGO_HOME}/bin/tauri-driver`)
    ? `${process.env.CARGO_HOME}/bin/tauri-driver`
    : fs.existsSync(`${repo}/../.toolchain/cargo/bin/tauri-driver`)
      ? `${repo}/../.toolchain/cargo/bin/tauri-driver`
      : "tauri-driver");
const today = new Date();
const PORT = Number(process.env.TINE_PROBE_PORT || 4690 + Math.floor(Math.random() * 200));
const NATIVE_PORT = PORT + 1;
const JNAME = `${today.getFullYear()}_${String(today.getMonth() + 1).padStart(2, "0")}_${String(today.getDate()).padStart(2, "0")}`;
const JFILE = `${G}/journals/${JNAME}.md`;
const UPDATED = 'tine.formula.tier:: if(points <= 2, "big", "small")';

const MD = [
  "- Formula builder table",
  "  tine.view:: table",
  "  tine.fields:: status=text;points=number",
  '  tine.formula.tier:: if(points > 2, "big", "small")',
  "\t- Alpha",
  "\t  status:: todo",
  "\t  points:: 1",
  "\t- Beta",
  "\t  status:: done",
  "\t  points:: 3",
  "",
].join("\n");

fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.writeFileSync(JFILE, MD);
fs.rmSync("/tmp/formula-builder-xdg", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/formula-builder-xdg/${d}`, { recursive: true });

const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/formula-builder-xdg/data",
  XDG_CONFIG_HOME: "/tmp/formula-builder-xdg/config",
  XDG_CACHE_HOME: "/tmp/formula-builder-xdg/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/formula-builder-td.log", "w");
const td = spawn(TD, ["--port", String(PORT), "--native-port", String(NATIVE_PORT), "--native-driver", "/usr/bin/WebKitWebDriver"], {
  env,
  stdio: ["ignore", tdLog, tdLog],
  detached: true,
});
const killTree = () => {
  try {
    process.kill(-td.pid, "SIGKILL");
  } catch {
    /* already gone */
  }
};
await sleep(3000);

let browser;
let failures = 0;
let checks = 0;
const check = (name, ok, extra = "") => {
  checks++;
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok ? "" : "  " + extra}`);
  if (!ok) failures++;
};

async function waitForDiskLine(line) {
  for (let i = 0; i < 30; i += 1) {
    if (fs.readFileSync(JFILE, "utf8").includes(line)) return true;
    await sleep(250);
  }
  return false;
}

try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: PORT,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });
  await browser.$(".ls-block").waitForExist({ timeout: 20000 });
  await browser.$(".sheet-table").waitForExist({ timeout: 10000 });

  const headerTagged = await browser.execute(() => {
    const header = [...document.querySelectorAll(".sheet-field-header")]
      .find((el) => (el.textContent || "").includes("tier"));
    if (!header) return false;
    header.setAttribute("data-probe", "tier-header");
    header.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 120, clientY: 120 }));
    return true;
  });
  check("formula header present and context menu opened", headerTagged);
  await sleep(350);
  const editorOpened = await browser.execute(() => {
    const item = [...document.querySelectorAll(".ctx-item")]
      .find((el) => /Edit formula/.test(el.textContent || ""));
    item?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    return !!item;
  });
  check("Edit formula menu item clicked", editorOpened);
  await browser.$(".formula-editor").waitForExist({ timeout: 5000 });

  // Post-#161 closure: a value picker is a child of the FormulaEditor, so one
  // Escape peels only the picker and returns to its face without changing the
  // unsaved formula draft or closing the outer editor.
  const formulaDraftBeforePicker = await browser.execute(() =>
    document.querySelector(".formula-editor")?.textContent ?? "",
  );
  await browser.$(".formula-builder-value-face").click();
  await browser.$(".formula-builder-picker").waitForExist({ timeout: 5000 });
  await browser.keys(["Escape"]);
  await browser.$(".formula-builder-picker").waitForExist({ reverse: true, timeout: 5000 });
  const pickerDismissal = await browser.execute(() => ({
    editorOpen: !!document.querySelector(".formula-editor"),
    activeIsFace: document.activeElement?.classList.contains("formula-builder-value-face") ?? false,
    text: document.querySelector(".formula-editor")?.textContent ?? "",
  }));
  check(
    "Escape closes only the formula value picker and restores its face",
    pickerDismissal.editorOpen && pickerDismissal.activeIsFace
      && pickerDismissal.text.replace(/\s+/g, " ").trim() === formulaDraftBeforePicker.replace(/\s+/g, " ").trim(),
    JSON.stringify(pickerDismissal),
  );

  const face = await browser.execute(() => {
    const ifFace = document.querySelector(".formula-builder-if");
    const keyword = document.querySelector(".formula-builder-keyword");
    const builder = document.querySelector(".formula-builder");
    const op = document.querySelector(".formula-builder-operator");
    if (!ifFace || !keyword || !builder || !op) return null;
    const keywordStyle = getComputedStyle(keyword);
    const builderStyle = getComputedStyle(builder);
    return {
      text: ifFace.textContent || "",
      op: op.value,
      keywordWeight: keywordStyle.fontWeight,
      builderBorder: builderStyle.borderTopStyle,
      builderBackground: builderStyle.backgroundColor,
    };
  });
  check("IF/THEN/ELSE builder face renders", !!face && face.text.includes("IF") && face.text.includes("THEN") && face.text.includes("ELSE"), JSON.stringify(face));
  check("builder has styled border/background", !!face && face.builderBorder === "solid" && face.builderBackground !== "rgba(0, 0, 0, 0)", JSON.stringify(face));
  check("comparison operator starts as >", !!face && face.op === ">", JSON.stringify(face));

  const flipped = await browser.execute(() => {
    const op = document.querySelector(".formula-builder-operator");
    if (!op) return false;
    op.value = "<=";
    op.dispatchEvent(new Event("change", { bubbles: true, cancelable: true }));
    return true;
  });
  check("operator dropdown flipped to <=", flipped);
  await sleep(250);

  const saved = await browser.execute(() => {
    const save = [...document.querySelectorAll(".formula-editor-btn")]
      .find((el) => (el.textContent || "").trim() === "Save");
    save?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    return !!save && !save.disabled;
  });
  check("formula editor saved", saved);

  const diskChanged = await waitForDiskLine(UPDATED);
  check("formula property changed on disk", diskChanged, fs.readFileSync(JFILE, "utf8"));

  const values = await browser.execute(() => {
    const headers = [...document.querySelectorAll(".sheet-header-cell")].map((h) => (h.textContent || "").trim());
    const tierCol = headers.findIndex((h) => h.includes("tier"));
    const byRow = [0, 1].map((row) => {
      const cell = document.querySelector(`.sheet-field-cell[data-row="${row}"][data-col="${tierCol}"]`);
      return (cell?.textContent || "").trim();
    });
    return { tierCol, byRow };
  });
  check("formula column re-evaluated after save", values.tierCol >= 0 && values.byRow[0] === "big" && values.byRow[1] === "small", JSON.stringify(values));

  console.log(failures === 0 ? `\nALL PASS (${checks} checks)` : `\n${failures} FAILED of ${checks}`);
} catch (e) {
  console.error("PROBE ERROR:", e);
  failures++;
} finally {
  if (browser) await browser.deleteSession().catch(() => {});
  killTree();
}
process.exit(failures === 0 ? 0 : 1);
