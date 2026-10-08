// Verify rendered-copy fidelity end-to-end in the real ExportModal: a bare
// ((uuid)) block ref and a user {{macro}} resolve to what they render (not the
// uuid / literal) in the "Rendered" preview. Real frontend (Chromium + mock via
// vite preview). Kitchen-sink has the ref target + poem/hi macros.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5201;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) { try { if ((await fetch(url)).ok) return; } catch {} await sleep(250); }
  throw new Error("server did not start");
}
let fail = 0;
const check = (name, ok, extra = "") => { if (!ok) fail++; console.log(`${ok ? "PASS" : "FAIL"}  ${name}${extra ? "  " + extra : ""}`); };

async function exportPreviewFor(page, blockText) {
  // Right-click the target block, open Copy/export, read the Rendered preview.
  const block = page.locator(".ls-block .block-content", { hasText: blockText }).first();
  // Center the block in the viewport, then right-click its LEFT edge (plain text) —
  // NOT the center, which for the ref block lands on the .block-ref span (its own
  // context menu). Left edge hits the block row → the block context menu.
  await block.evaluate((el) => el.scrollIntoView({ block: "center" }));
  await sleep(250);
  await block.click({ button: "right", position: { x: 6, y: 8 } });
  await sleep(300);
  await page.getByText("Copy / export as").first().click();
  await page.waitForSelector(".export-modal", { timeout: 3000 });
  await sleep(250);
  const preview = await page.locator(".export-preview").inputValue();
  await page.keyboard.press("Escape"); // close modal
  await sleep(200);
  return preview;
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1000, height: 1000 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.evaluate(() => localStorage.removeItem("tine.exportOptions")); // default = Rendered
  // Navigate to kitchen-sink (has the ref target + macros).
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 3000 });
  await page.locator(".switcher-input").fill("kitchen");
  await sleep(400);
  await page.locator(".switcher-row").first().click();
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  // Let the async ((uuid)) resolution populate the sync cache (BlockRefView renders).
  await sleep(1200);

  // 1) Block-ref block → Rendered preview should show the referenced text, not the uuid.
  const refPreview = await exportPreviewFor(page, "Block reference (bare)");
  console.log("ref preview:", JSON.stringify(refPreview));
  check("ref resolves to referenced text", /Related Work section/.test(refPreview));
  check("ref does NOT show bare uuid", !/64b9c0e2/.test(refPreview));

  // 2) Macro block → Rendered preview should show the expansions.
  const macroPreview = await exportPreviewFor(page, "User macro (config.edn");
  console.log("macro preview:", JSON.stringify(macroPreview));
  check("poem macro expands", /Roses are red, violets are blue\./.test(macroPreview));
  check("hi macro expands (bold dropped)", /Hello, Martin! See/.test(macroPreview));
  check("macro does NOT show literal {{poem", !/\{\{poem/.test(macroPreview));

  console.log(errors.length ? "PAGE ERRORS:\n" + errors.join("\n") : "no page errors");
  console.log(fail ? `\n${fail} CHECK(S) FAILED` : "\nALL CHECKS PASSED");
  await browser.close();
  server.kill("SIGKILL");
  process.exit(fail ? 1 : 0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
