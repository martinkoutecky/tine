// Linux real-app proof for GH #105. Exercises the pane-local overflow trigger,
// keyboard-accessible all-tabs overview, activation/reveal, close, and focus
// restoration in Tauri's WebKit shell.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4492);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4493);
const TMP = "/tmp/tine-tab-overflow-e2e";
const GRAPH = `${TMP}/graph`;
const ARTIFACT = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;
const PAGES = Array.from({ length: 10 }, (_, index) => `Readable tab title ${String(index + 1).padStart(2, "0")}`);

fs.rmSync(TMP, { recursive: true, force: true });
fs.mkdirSync(ARTIFACT, { recursive: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
for (const name of PAGES) fs.writeFileSync(`${GRAPH}/pages/${name}.md`, `- Content for [[${name}]]\n`);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Tab overflow native regression\n");

const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env: {
    ...process.env,
    TINE_GRAPH: GRAPH,
    XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
  },
  stdio: ["ignore", log, log], detached: true,
});

let browser;
try {
  await sleep(2500);
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.setWindowSize(1000, 720);
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });

  // Middle-click exact page results so the switcher stays open and every page
  // becomes a background tab through the production input path.
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5000 });
  for (const name of PAGES) {
    const before = await browser.$$(".tab-strip-scroll > .tab").length;
    await input.setValue(name);
    await browser.waitUntil(() => browser.execute((expected) =>
      [...document.querySelectorAll(".switcher-row")].some((candidate) =>
        candidate.querySelector(".switcher-kind")?.textContent?.trim() === "page"
        && candidate.querySelector(".switcher-name")?.textContent?.includes(expected)), name), {
      timeout: 5000, timeoutMsg: `page result did not render for ${name}`,
    });
    const opened = await browser.execute((expected) => {
      const row = [...document.querySelectorAll(".switcher-row")].find((candidate) =>
        candidate.querySelector(".switcher-kind")?.textContent?.trim() === "page"
        && candidate.querySelector(".switcher-name")?.textContent?.includes(expected));
      if (!row) return false;
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 1 }));
      return true;
    }, name);
    if (!opened) throw new Error(`page result missing for ${name}`);
    await browser.waitUntil(async () => (await browser.$$(".tab-strip-scroll > .tab")).length > before, {
      timeout: 5000, timeoutMsg: `${name} did not open as a background tab`,
    });
  }
  await browser.keys("Escape");

  const trigger = await browser.$("[data-tab-overview-trigger]");
  await trigger.waitForExist({ timeout: 5000 });
  const initial = await browser.execute(() => {
    const strip = document.querySelector(".tab-strip-scroll");
    const tabs = [...document.querySelectorAll(".tab-strip-scroll > .tab")];
    return {
      tabs: tabs.length,
      overflowing: !!strip && strip.scrollWidth > strip.clientWidth + 1,
      minWidth: Math.min(...tabs.map((tab) => tab.getBoundingClientRect().width)),
    };
  });
  if (!initial.overflowing || initial.tabs !== PAGES.length + 1 || initial.minWidth < 110) {
    throw new Error(`unexpected native tab geometry: ${JSON.stringify(initial)}`);
  }

  await browser.execute(() => document.querySelector("[data-tab-overview-trigger]")?.focus());
  await browser.keys("Enter");
  await browser.$("[role=listbox]").waitForExist({ timeout: 5000 });
  const rows = await browser.$$("[data-tab-overview-row]");
  if (rows.length !== initial.tabs) throw new Error(`${rows.length} overview rows for ${initial.tabs} tabs`);
  await browser.keys("End");
  const selectedId = await browser.execute(() => document.activeElement?.getAttribute("data-tab-id"));
  if (!selectedId) throw new Error("End did not focus the final overview row");
  await browser.keys("Enter");
  await browser.waitUntil(() => browser.execute((id) =>
    document.querySelector(".tab.active")?.getAttribute("data-tab-id") === id
    && !document.querySelector("[role=listbox]"), selectedId), {
    timeout: 5000, timeoutMsg: "overview selection did not activate and dismiss",
  });
  const reveal = await browser.execute(() => {
    const strip = document.querySelector(".tab-strip-scroll");
    const active = strip?.querySelector(".tab.active");
    if (!strip || !active) return false;
    const stripRect = strip.getBoundingClientRect();
    const activeRect = active.getBoundingClientRect();
    return activeRect.left >= stripRect.left - 1 && activeRect.right <= stripRect.right + 1;
  });
  if (!reveal) throw new Error("overview activation did not reveal the active tab");

  // Escape closes the portalled menu and returns keyboard focus to its trigger.
  await browser.execute(() => document.querySelector("[data-tab-overview-trigger]")?.focus());
  await browser.keys("Enter");
  await browser.keys("Escape");
  const escapeState = await browser.execute(() => ({
    triggerFocused: document.activeElement?.hasAttribute("data-tab-overview-trigger"),
    overviewOpen: !!document.querySelector("[role=listbox]"),
    paneSelected: !!document.querySelector(".pane-selected, .pane-select-hint"),
  }));
  if (!escapeState.triggerFocused || escapeState.overviewOpen || escapeState.paneSelected) {
    throw new Error(`Escape ownership failed: ${JSON.stringify(escapeState)}`);
  }

  // WebDriver W3C pointer actions retain a pressed pointer across commands. Hold
  // a real overview-row drag over another row, retire it with Escape, and only
  // then release the original pointer. A late native pointer-up must be inert.
  await trigger.click();
  await browser.$("[role=listbox]").waitForExist({ timeout: 5000 });
  const heldDrag = await browser.execute(() => {
    const rows = [...document.querySelectorAll("[data-tab-overview-row]")];
    const handle = rows[0]?.querySelector(".tab-overview-drag-handle");
    const target = rows[1];
    if (!(handle instanceof HTMLElement) || !(target instanceof HTMLElement)) return null;
    const start = handle.getBoundingClientRect();
    const end = target.getBoundingClientRect();
    return {
      before: rows.map((row) => row.getAttribute("data-tab-id")),
      sourceId: rows[0].getAttribute("data-tab-id"),
      targetId: target.getAttribute("data-tab-id"),
      start: { x: Math.round(start.left + start.width / 2), y: Math.round(start.top + start.height / 2) },
      end: { x: Math.round(end.left + end.width / 2), y: Math.round(end.top + end.height * 0.8) },
    };
  });
  if (!heldDrag?.sourceId || !heldDrag.targetId) throw new Error("overview rows did not expose a native drag path");
  await browser.execute(() => {
    window.__tabOverviewPointerTrace = [];
    const record = (event) => window.__tabOverviewPointerTrace.push({
      type: event.type,
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      target: event.target?.className ?? "",
    });
    document.addEventListener("pointerdown", record, true);
    window.addEventListener("pointermove", record, true);
  });
  let heldPointer = false;
  try {
    await browser.performActions([{
      type: "pointer",
      id: "p1e-o-held-overview-row",
      parameters: { pointerType: "mouse" },
      actions: [
        { type: "pointerMove", duration: 0, x: heldDrag.start.x, y: heldDrag.start.y },
        { type: "pointerDown", button: 0 },
        { type: "pointerMove", duration: 120, x: heldDrag.end.x, y: heldDrag.end.y },
      ],
    }]);
    heldPointer = true;
    try {
      await browser.waitUntil(() => browser.execute((sourceId, targetId) =>
        document.querySelector(`[data-tab-overview-row][data-tab-id="${sourceId}"]`)?.classList.contains("tab-overview-dragging")
        && document.querySelector(`[data-tab-overview-row][data-tab-id="${targetId}"]`)?.classList.contains("tab-overview-drop-after"), heldDrag.sourceId, heldDrag.targetId), {
        timeout: 5000, timeoutMsg: "native held overview-row drag did not arm a reorder session",
      });
    } catch (error) {
      const state = await browser.execute((start, end) => ({
        trace: window.__tabOverviewPointerTrace,
        startHit: document.elementFromPoint(start.x, start.y)?.className ?? "",
        endHit: document.elementFromPoint(end.x, end.y)?.className ?? "",
        dragging: document.querySelector(".tab-overview-dragging")?.getAttribute("data-tab-id") ?? null,
        drop: document.querySelector(".tab-overview-drop-before, .tab-overview-drop-after")?.className ?? null,
      }), heldDrag.start, heldDrag.end);
      throw new Error(`${String(error)}; state=${JSON.stringify(state)}`);
    }
    await browser.keys("Escape");
    await browser.$("[role=listbox]").waitForExist({ reverse: true, timeout: 5000 });
    await browser.performActions([{
      type: "pointer",
      id: "p1e-o-held-overview-row",
      parameters: { pointerType: "mouse" },
      actions: [{ type: "pointerUp", button: 0 }],
    }]);
    heldPointer = false;
  } finally {
    if (heldPointer) await browser.releaseActions();
  }
  const heldRetirement = await browser.execute((before) => ({
    order: [...document.querySelectorAll(".tab-strip-scroll > .tab")].map((tab) => tab.getAttribute("data-tab-id")),
    triggerFocused: document.activeElement?.hasAttribute("data-tab-overview-trigger"),
    overviewOpen: !!document.querySelector("[role=listbox]"),
    paneSelected: !!document.querySelector(".pane-selected, .pane-select-hint"),
    before,
  }), heldDrag.before);
  if (heldRetirement.overviewOpen || !heldRetirement.triggerFocused || heldRetirement.paneSelected
    || JSON.stringify(heldRetirement.order) !== JSON.stringify(heldRetirement.before)) {
    throw new Error(`held overview-row retirement failed: ${JSON.stringify(heldRetirement)}`);
  }

  // Delete follows the ordinary close path and keeps the overview usable.
  const beforeClose = await browser.$$(".tab-strip-scroll > .tab").length;
  await trigger.click();
  try {
    await browser.waitUntil(() => browser.execute(() =>
      !!document.querySelector("[role=listbox]") && document.activeElement?.hasAttribute("data-tab-overview-row")), {
      timeout: 5000, timeoutMsg: "overview did not focus a row before keyboard close",
    });
  } catch (error) {
    const state = await browser.execute(() => ({
      open: !!document.querySelector("[role=listbox]"),
      active: document.activeElement?.outerHTML,
      expanded: document.querySelector("[data-tab-overview-trigger]")?.getAttribute("aria-expanded"),
    }));
    throw new Error(`${String(error)}; state=${JSON.stringify(state)}`);
  }
  await browser.keys("End");
  await browser.keys("Delete");
  await browser.waitUntil(async () => (await browser.$$(".tab-strip-scroll > .tab")).length === beforeClose - 1, {
    timeout: 5000, timeoutMsg: "Delete did not close the focused overview tab",
  });
  const afterCloseRows = await browser.$$("[data-tab-overview-row]");
  if (afterCloseRows.length !== beforeClose - 1) throw new Error("overview did not update after close");

  // The control disappears once the remaining tab set fits; it is not
  // permanent toolbar clutter. Closing tabs is more reliable than asking a
  // headless Xvfb window manager for a size wider than its virtual screen.
  await browser.keys("Escape");
  for (let remaining = beforeClose - 1; remaining > 1; remaining--) {
    const geometry = await browser.execute(() => {
      const strip = document.querySelector(".tab-strip-scroll");
      return { overflowing: !!strip && strip.scrollWidth > strip.clientWidth + 1 };
    });
    if (!geometry.overflowing) break;
    const closed = await browser.execute(() => {
      const close = document.querySelector(".tab-strip-scroll > .tab:last-child .tab-close");
      if (!(close instanceof HTMLElement)) return false;
      close.click();
      return true;
    });
    if (!closed) throw new Error("could not close a tab while reducing overflow");
    await browser.waitUntil(async () => (await browser.$$(".tab-strip-scroll > .tab")).length === remaining - 1, {
      timeout: 5000, timeoutMsg: "tab strip did not shrink while reducing overflow",
    });
  }
  await browser.waitUntil(() => browser.execute(() => {
    const strip = document.querySelector(".tab-strip-scroll");
    return !!strip && strip.scrollWidth <= strip.clientWidth + 1 && !document.querySelector("[data-tab-overview-trigger]");
  }), { timeout: 5000, timeoutMsg: "overflow control remained after tabs fit" });

  console.log("PASS: native tab overflow overview, keyboard navigation, reveal, close, and resize behavior work in WebKit");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
