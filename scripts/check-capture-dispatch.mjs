// Source-served desktop Quick Capture dispatch regression.
//
// This deliberately loads /capture.html through Vite (not dist), leaves the
// production browser fallback in place (__TAURI_INTERNALS__ is absent), and
// drives real bubbling KeyboardEvents at the real title/editor nodes.  It owns
// the Vite server and Chromium lifecycle so it can be used as both the P1B
// fail-before recorder and the pass-after regression gate.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = Number(process.env.CAPTURE_DISPATCH_PORT || 5181);
const URL = `http://127.0.0.1:${PORT}/capture.html`;
const failBefore = process.argv.includes("--expect-fail-before");
const forceError = process.env.CAPTURE_DISPATCH_FORCE_ERROR === "1";
const viteBin = new globalThis.URL("../node_modules/vite/bin/vite.js", import.meta.url);
let server;
let serverExit;
let serverLog = "";

async function assertPortAvailable(stage) {
  const probe = createServer();
  try {
    await new Promise((resolve, reject) => {
      probe.once("error", reject);
      probe.listen({ host: "127.0.0.1", port: PORT, exclusive: true }, resolve);
    });
  } catch (error) {
    throw new Error(`${stage}: strict port ${PORT} is unavailable (${error.code || error.message})`);
  } finally {
    if (probe.listening) await new Promise((resolve, reject) => probe.close((error) => error ? reject(error) : resolve()));
  }
}

function startServer() {
  // Spawn Vite's local entrypoint directly in its own process group. This avoids
  // an npm/npx shell layer and lets cleanup signal every descendant at once.
  server = spawn(process.execPath, [viteBin.pathname, "--host", "127.0.0.1", "--port", String(PORT), "--strictPort", "--force"], {
    detached: process.platform !== "win32",
    stdio: "pipe",
  });
  server.stdout.on("data", (chunk) => { serverLog += chunk; });
  server.stderr.on("data", (chunk) => { serverLog += chunk; });
  serverExit = new Promise((resolve) => {
    server.once("error", (error) => resolve({ kind: "error", error }));
    server.once("exit", (code, signal) => resolve({ kind: "exit", code, signal }));
  });
}

async function waitForServer() {
  for (let i = 0; i < 80; i++) {
    if (server.exitCode !== null) {
      const outcome = await serverExit;
      throw new Error(`owned Vite exited before readiness (${outcome.kind === "error" ? outcome.error.message : `code ${outcome.code}, signal ${outcome.signal}`})\n${serverLog}`);
    }
    try {
      if ((await fetch(URL)).ok) {
        // The port was exclusively acquired immediately before this direct Vite
        // child was spawned; a live child plus its own readiness response is the
        // ownership proof. A foreign listener makes either preflight or strictPort fail.
        if (server.exitCode !== null) continue;
        return;
      }
    } catch {
      // Vite is still starting.
    }
    await sleep(125);
  }
  throw new Error(`Vite did not serve ${URL}\n${serverLog}`);
}

async function stopServer() {
  if (!server) return;
  const exited = async (timeout) => {
    const outcome = await Promise.race([serverExit, sleep(timeout).then(() => null)]);
    return outcome;
  };
  const signalTree = (signal) => {
    try {
      if (process.platform !== "win32") process.kill(-server.pid, signal);
      else server.kill(signal);
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  };
  const treeAlive = () => {
    if (process.platform === "win32") return server.exitCode === null;
    try {
      process.kill(-server.pid, 0);
      return true;
    } catch (error) {
      if (error.code === "ESRCH") return false;
      throw error;
    }
  };
  const waitForTreeExit = async (timeout) => {
    const deadline = Date.now() + timeout;
    while (treeAlive()) {
      if (Date.now() >= deadline) return false;
      await sleep(50);
    }
    return true;
  };
  // Always address the process group, even when Vite has already exited: a
  // crash must not strand an esbuild or other descendant behind its leader.
  signalTree("SIGTERM");
  if (server.exitCode === null && !await exited(4000)) {
    signalTree("SIGKILL");
    if (!await exited(2000)) throw new Error(`owned Vite process did not exit (pid ${server.pid})`);
  }
  if (!await waitForTreeExit(1000)) {
    signalTree("SIGKILL");
    if (!await waitForTreeExit(2000)) throw new Error(`owned Vite process tree did not exit (pgid ${server.pid})`);
  }
  await assertPortAvailable("Vite cleanup");
}

function fail(message) {
  throw new Error(message);
}

async function newCapture(browser, pageErrors) {
  const page = await browser.newPage({ viewport: { width: 760, height: 620 } });
  page.on("console", (m) => {
    if (m.type() === "error") pageErrors.push(`console: ${m.text()}`);
  });
  page.on("pageerror", (e) => pageErrors.push(`pageerror: ${String(e)}`));
  await page.goto(URL, { waitUntil: "networkidle" });
  if (await page.evaluate(() => "__TAURI_INTERNALS__" in window)) {
    fail("browser harness unexpectedly has Tauri internals; the production plain-browser fallback is required");
  }
  await page.waitForSelector(".capture-title");
  await page.waitForSelector(".capture-shell textarea.block-editor");
  return page;
}

async function dispatchKey(locator, init) {
  return locator.evaluate((target, eventInit) => {
    const event = new KeyboardEvent("keydown", eventInit);
    // Chromium accepts isComposing in the constructor. keyCode is legacy and
    // must be made explicit because keyboard.press() cannot synthesize 229.
    if (eventInit.keyCode !== undefined) {
      Object.defineProperty(event, "keyCode", { configurable: true, value: eventInit.keyCode });
    }
    target.dispatchEvent(event);
    return { bubbles: event.bubbles, cancelable: event.cancelable, defaultPrevented: event.defaultPrevented };
  }, init);
}

async function expectDrafts(page, block, title, { mounted = true } = {}) {
  const editor = page.locator(".capture-shell textarea.block-editor");
  if (mounted) {
    if (await editor.count() !== 1) fail("the production capture Block textarea is not mounted");
    if (await editor.inputValue() !== block) fail(`block draft changed: expected ${JSON.stringify(block)}, got ${JSON.stringify(await editor.inputValue())}`);
  } else {
    const text = await page.locator(".capture-shell .page-blocks").innerText();
    if (!text.includes(block)) fail(`rendered Block draft changed: expected to retain ${JSON.stringify(block)}, got ${JSON.stringify(text)}`);
  }
  const titleInput = page.locator(".capture-title");
  if (await titleInput.inputValue() !== title) fail(`title draft changed: expected ${JSON.stringify(title)}, got ${JSON.stringify(await titleInput.inputValue())}`);
}

async function expectTitleImeDrafts(page, block, title, event, { preserved }) {
  if (!event.bubbles || !event.cancelable) fail("title IME event was not explicitly bubbling and cancelable");
  const observed = await page.evaluate(({ expectedBlock }) => ({
    title: document.querySelector(".capture-title")?.value,
    blocks: document.querySelector(".capture-shell .page-blocks")?.innerText || "",
    expectedBlock,
  }), { expectedBlock: block });
  const hasBlock = observed.blocks.includes(block);
  const hasTitle = observed.title === title;
  if (preserved) {
    if (!hasBlock || !hasTitle) {
      fail(`title IME guard did not preserve drafts after dispatched handler event: block retained=${hasBlock}, title=${JSON.stringify(observed.title)}`);
    }
    if (event.defaultPrevented) fail("title IME guard unexpectedly prevented the dispatched event");
  } else {
    // On the pre-production source, this exact bubbling/cancelable event reaches
    // Solid's delegated title handler: its Escape branch prevents default and
    // synchronously runs the real cancellation path, clearing both witnesses.
    if (!event.defaultPrevented) fail("pre-production title handler did not prevent the bubbling/cancelable IME Escape");
    if (hasBlock || hasTitle) {
      fail(`pre-production title IME fallthrough did not clear both drafts: block retained=${hasBlock}, title=${JSON.stringify(observed.title)}`);
    }
  }
}

async function regainEditorFromTitle(page, block, title) {
  const titleInput = page.locator(".capture-title");
  await titleInput.press("Enter");
  await page.waitForSelector(".capture-shell textarea.block-editor", { state: "attached", timeout: 1500 });
  const live = page.locator(".capture-shell textarea.block-editor");
  await live.evaluate((node) => {
    if (document.activeElement !== node) throw new Error("title Enter did not focus the real capture textarea");
    if (node.selectionStart !== node.value.length || node.selectionEnd !== node.value.length) {
      throw new Error(`title Enter caret mismatch: ${node.selectionStart}/${node.selectionEnd} for ${node.value.length}`);
    }
  });
  await expectDrafts(page, block, title);
}

async function enterTitleAndRegainEditor(page, block = "bullet draft", title = "title draft") {
  const editor = page.locator(".capture-shell textarea.block-editor");
  await editor.fill(block);
  const titleInput = page.locator(".capture-title");
  await titleInput.focus();
  await titleInput.fill(title);
  await regainEditorFromTitle(page, block, title);
}

async function openScheduled(page, prefix) {
  const editor = page.locator(".capture-shell textarea.block-editor");
  await editor.focus();
  await editor.fill(`${prefix} /scheduled`);
  await page.waitForSelector(".autocomplete", { timeout: 3000 });
  await editor.press("Enter");
  await page.waitForSelector(".date-picker", { timeout: 3000 });
}

async function expectCancelled(page) {
  await page.waitForSelector(".capture-shell textarea.block-editor");
  const editor = page.locator(".capture-shell textarea.block-editor");
  if (await editor.inputValue() !== "") fail("ordinary capture cancellation did not clear the Block draft");
  if (await page.locator(".capture-title").inputValue() !== "") fail("ordinary capture cancellation did not clear the title draft");
}

async function runFixedMatrix(browser, pageErrors) {
  {
    const page = await newCapture(browser, pageErrors);
    await enterTitleAndRegainEditor(page);
    await page.close();
  }

  for (const init of [
    { key: "Escape", isComposing: true },
    { key: "Escape", keyCode: 229 },
  ]) {
    const page = await newCapture(browser, pageErrors);
    const editor = page.locator(".capture-shell textarea.block-editor");
    await editor.fill("ime block");
    const title = page.locator(".capture-title");
    await title.focus();
    await title.fill("ime title");
    const event = await dispatchKey(title, { bubbles: true, cancelable: true, ...init });
    await expectTitleImeDrafts(page, "ime block", "ime title", event, { preserved: true });
    await regainEditorFromTitle(page, "ime block", "ime title");
    await page.close();
  }

  {
    const page = await newCapture(browser, pageErrors);
    await openScheduled(page, "title date draft");
    const title = page.locator(".capture-title");
    await title.focus();
    await title.fill("title date");
    await title.press("Escape");
    if (await page.locator(".date-picker").count()) fail("first title Escape did not close the real DatePicker");
    await expectDrafts(page, "title date draft ", "title date", { mounted: false });
    await title.press("Escape");
    await expectCancelled(page);
    await page.close();
  }

  {
    const page = await newCapture(browser, pageErrors);
    await openScheduled(page, "textarea date draft");
    const editor = page.locator(".capture-shell textarea.block-editor");
    await editor.press("Escape");
    if (await page.locator(".date-picker").count()) fail("first textarea Escape did not close the real DatePicker");
    if (await editor.inputValue() !== "textarea date draft ") fail("DatePicker dismissal changed the textarea draft");
    await editor.press("Escape");
    await expectCancelled(page);
    await page.close();
  }

  for (const init of [
    { key: "Escape", isComposing: true },
    { key: "Escape", keyCode: 229 },
  ]) {
    const page = await newCapture(browser, pageErrors);
    const editor = page.locator(".capture-shell textarea.block-editor");
    await editor.fill("completion ime ");
    await editor.pressSequentially("[[H");
    await page.waitForSelector(".autocomplete", { timeout: 3000 });
    await dispatchKey(editor, { bubbles: true, cancelable: true, ...init });
    if (!await page.locator(".autocomplete").count()) fail("IME Escape closed the real autocomplete popup");
    if (!(await editor.inputValue()).startsWith("completion ime")) fail("IME Escape changed the Block draft");
    await page.close();
  }

  for (const targetKind of ["title", "textarea"]) {
    for (const init of [
      { key: "Escape", isComposing: true },
      { key: "Escape", keyCode: 229 },
    ]) {
      const page = await newCapture(browser, pageErrors);
      await openScheduled(page, `${targetKind} picker draft`);
      const target = targetKind === "title" ? page.locator(".capture-title") : page.locator(".capture-shell textarea.block-editor");
      if (targetKind === "title") await target.focus();
      await dispatchKey(target, { bubbles: true, cancelable: true, ...init });
      if (!await page.locator(".date-picker").count()) fail(`${targetKind} IME Escape closed DatePicker`);
      await expectDrafts(page, `${targetKind} picker draft `, "", { mounted: targetKind === "textarea" });
      await page.close();
    }
  }

  {
    const page = await newCapture(browser, pageErrors);
    const editor = page.locator(".capture-shell textarea.block-editor");
    await editor.fill("completion draft ");
    await editor.pressSequentially("[[H");
    await page.waitForSelector(".autocomplete", { timeout: 3000 });
    await editor.press("Escape");
    if (await page.locator(".autocomplete").count()) fail("first completion Escape did not close the real autocomplete popup");
    if (!(await editor.inputValue()).startsWith("completion draft")) fail("completion dismissal changed the Block draft");
    await editor.press("Escape");
    await expectCancelled(page);
    await page.close();
  }

  for (const targetKind of ["title", "textarea"]) {
    const page = await newCapture(browser, pageErrors);
    const editor = page.locator(".capture-shell textarea.block-editor");
    await editor.fill(`${targetKind} cancel block`);
    const target = targetKind === "title" ? page.locator(".capture-title") : editor;
    if (targetKind === "title") {
      await target.focus();
      await target.fill("cancel title");
    }
    await target.press("Escape");
    await expectCancelled(page);
    await page.close();
  }
}

async function runFailBefore(browser, pageErrors) {
  const failures = [];
  {
    const page = await newCapture(browser, pageErrors);
    try {
      await enterTitleAndRegainEditor(page);
      failures.push("title Enter unexpectedly regained the real textarea");
    } catch (error) {
      failures.push(`title Enter lifecycle fails: ${String(error.message || error)}`);
    }
    await page.close();
  }
  for (const init of [
    { key: "Escape", isComposing: true, label: "isComposing" },
    { key: "Escape", keyCode: 229, label: "keyCode-229" },
  ]) {
    const page = await newCapture(browser, pageErrors);
    const editor = page.locator(".capture-shell textarea.block-editor");
    await editor.fill("ime block");
    const title = page.locator(".capture-title");
    await title.focus();
    await title.fill("ime title");
    try {
      const event = await dispatchKey(title, { bubbles: true, cancelable: true, ...init });
      await expectTitleImeDrafts(page, "ime block", "ime title", event, { preserved: false });
      failures.push(`${init.label} title fallthrough: bubbling/cancelable event reached the real title handler and synchronously cleared both drafts`);
    } catch (error) {
      failures.push(`${init.label} title fallthrough did not reproduce: ${String(error.message || error)}`);
    }
    await page.close();
  }
  if (failures.some((line) => line.includes("unexpectedly") || line.includes("did not reproduce"))) fail(failures.join("\n"));
  console.log(`FAIL-BEFORE reproduced\n${failures.join("\n")}`);
}

let browser;
let runError;
let cleanupError;
try {
  await assertPortAvailable("Vite startup");
  startServer();
  await waitForServer();
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const pageErrors = [];
  if (failBefore) await runFailBefore(browser, pageErrors);
  else await runFixedMatrix(browser, pageErrors);
  if (pageErrors.length) fail(`page/module errors:\n${pageErrors.join("\n")}`);
  if (forceError) fail("forced capture-dispatch error after source-served matrix");
} catch (error) {
  runError = error;
} finally {
  try {
    await browser?.close();
  } catch (error) {
    cleanupError = error;
  }
  try {
    await stopServer();
  } catch (error) {
    cleanupError = cleanupError || error;
  }
}
if (runError && cleanupError) throw new AggregateError([runError, cleanupError], "capture-dispatch run and cleanup both failed");
if (runError) throw runError;
if (cleanupError) throw cleanupError;
console.log(failBefore ? "FAIL-BEFORE evidence recorded" : "PASS desktop Quick Capture dispatch matrix");
