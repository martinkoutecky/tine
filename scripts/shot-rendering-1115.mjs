// GH #959, #963 and #129 comment 16, on a routed page with synthetic notes.
import { chromium } from "playwright";
import { createServer } from "vite";
import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { hasPaleThreadFork, sampleThreadInk } from "./lib/thread-ink.mjs";

const out = resolve(process.argv[2] ?? "target/artifacts/rendering-1115/after");
mkdirSync(out, { recursive: true });
const server = await createServer({ configFile: "vite.config.ts", cacheDir: `${out}/vite-cache`,
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("rendering-1115"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, strictPort: false, fs: { allow: [process.cwd(), realpathSync("node_modules"), dirname(realpathSync("node_modules/inter-ui"))] } } });
await server.listen();
let browser;
const evidence = [], failures = [];
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const cm6 of [false, true]) for (const zoom of [1, 1.25]) for (const display of ["all", "active"]) {
    const name = `${cm6 ? "cm6" : "classic"}-${zoom}-${display}`;
    const page = await browser.newPage({ viewport: { width: 1800, height: 1500 }, deviceScaleFactor: 1.5 });
    page.on("pageerror", error => console.error(`${name}: ${error}`));
    await page.goto(`${server.resolvedUrls.local[0]}?regressions`, { waitUntil: "commit" });
    await page.locator("[data-search-trigger]").waitFor();
    if (process.env.RENDER_BASELINE_CSS) await page.evaluate(css => {
      document.querySelector('style[data-vite-dev-id$="/src/styles/app.css"]').textContent = css;
    }, readFileSync(process.env.RENDER_BASELINE_CSS, "utf8"));
    await page.evaluate(async ({ cm6, zoom, display }) => {
      const { pluginManager } = await import("/src/plugins/manager.ts");
      const settings = { display, thickness: 2, color: "#e05070", "color-by-level": false };
      pluginManager.hasDeclarativeDecoration = kind => kind === "thread-lines";
      pluginManager.declarativeDecorationSetting = (_kind, key) => settings[key];
      (await import("/src/editor/cm6Setting.ts")).setCm6EditorEnabled(cm6);
      document.documentElement.dataset.theme = "light";
      document.documentElement.style.zoom = String(zoom);
      (await import("/src/router.ts")).openPage("Rendering regression");
    }, { cm6, zoom, display });
    const numbered = page.locator(".page-blocks .bullet-container.ordered");
    await numbered.nth(11).waitFor();
    await page.locator(".margin-column .comment-card").first().waitFor();
    await page.evaluate(() => document.fonts.ready);
    // Activate a numbered row through the same content target in each editor.
    await numbered.first().locator("xpath=../..").locator(".block-content-wrapper").click();
    await page.locator(cm6 ? '[data-block-editor="cm6"]' : "textarea.block-editor").waitFor();
    await page.mouse.move(1700, 1000);
    const facts = await page.evaluate(({ zoom }) => {
      const painted = (el, pseudo) => {
        const s = getComputedStyle(el, pseudo);
        return s.display !== "none" && s.content !== "none" && parseFloat(s.borderLeftWidth) > 0 && s.borderLeftColor !== "rgba(0, 0, 0, 0)";
      };
      const markers = [...document.querySelectorAll(".page-blocks .bullet-container.ordered")].map(el => {
        const ring = el.getBoundingClientRect(), text = el.querySelector(".bullet-order").getBoundingClientRect();
        return { label: el.textContent, ring: { x: ring.x, y: ring.y, width: ring.width, height: ring.height },
          text: { x: text.x, y: text.y, width: text.width, height: text.height },
          dx: text.x + text.width / 2 - ring.x - ring.width / 2,
          dy: text.y + text.height / 2 - ring.y - ring.height / 2 };
      });
      const comments = [...document.querySelectorAll(".comment-card, .comment-in-thread")].map(row => {
        const main = row.querySelector(":scope > .block-main");
        return { text: main?.textContent, strayStub: main && painted(main, "::after"), strayElbow: main && painted(main, "::before") };
      });
      // All nested outlines: downward extension that follows child-level indentation.
      const branches = [...document.querySelectorAll(".plugin-thread-lines:not(.collapsed):not(.comment-in-thread)")].map(row => {
        const parent = row.querySelector(":scope > .block-main");
        const child = row.querySelector(":scope > .block-children-container > .block-children > .ls-block > .block-main");
        if (!child) return null;
        const stub = getComputedStyle(parent, "::after"), elbow = getComputedStyle(child, "::before");
        const continuation = getComputedStyle(child.parentElement, "::after");
        const rail = continuation.display !== "none" && continuation.content !== "none" ? continuation : elbow;
        return { parent: parent.textContent, drawn: painted(parent, "::after"),
          railDelta: child.getBoundingClientRect().left + zoom * parseFloat(rail.left) - parent.getBoundingClientRect().left - zoom * parseFloat(stub.left),
          gap: child.getBoundingClientRect().top + zoom * parseFloat(elbow.top) - parent.getBoundingClientRect().bottom };
      }).filter(Boolean);
      return { markers, comments, branches };
    }, { zoom });
    await page.screenshot({ path: `${out}/${name}.png`, scale: "css" });
    // Isolate the painted ring from text and incoming threading for its pixel mask.
    // Neither the number's metrics nor the circle's own paint is changed.
    const maskStyle = await page.addStyleTag({ content: ".bullet-order { color: transparent !important; } .block-main:has(> .block-controls > .ordered)::before { visibility: hidden !important; }" });
    const shot = await page.screenshot({ path: `${out}/${name}-marker-mask.png`, scale: "css" });
    await maskStyle.evaluate(el => el.remove());
    facts.markerInk = await page.evaluate(async ({ src, markers }) => {
      const image = new Image(); image.src = src; await image.decode();
      const canvas = document.createElement("canvas"); canvas.width = image.width; canvas.height = image.height;
      const ctx = canvas.getContext("2d"); ctx.drawImage(image, 0, 0);
      return markers.map(({ label, ring }) => {
        const pixels = ctx.getImageData(Math.floor(ring.x), Math.floor(ring.y), Math.ceil(ring.width), Math.ceil(ring.height));
        let left = pixels.width, right = -1, top = pixels.height, bottom = -1;
        for (let y = 0; y < pixels.height; y++) for (let x = 0; x < pixels.width; x++) {
          const i = 4 * (y * pixels.width + x);
          if (Math.max(pixels.data[i], pixels.data[i + 1], pixels.data[i + 2]) >= 250) continue;
          left = Math.min(left, x); right = Math.max(right, x); top = Math.min(top, y); bottom = Math.max(bottom, y);
        }
        return { label, width: right - left + 1, height: bottom - top + 1,
          dx: Math.floor(ring.x) + (left + right + 1) / 2 - ring.x - ring.width / 2,
          dy: Math.floor(ring.y) + (top + bottom + 1) / 2 - ring.y - ring.height / 2 };
      });
    }, { src: `data:image/png;base64,${shot.toString("base64")}`, markers: facts.markers });
    // A circle can acquire one extra raster row/column at a fractional origin.
    for (const ink of facts.markerInk) {
      if (ink.width <= 0 || ink.height <= 0 || Math.abs(ink.width - ink.height) > 1) failures.push(`${name}: ${ink.label} screenshot shows a stretched or absent marker (${ink.width}×${ink.height})`);
      if (Math.abs(ink.dx) > 1 || Math.abs(ink.dy) > 1) failures.push(`${name}: ${ink.label} painted marker is off center (${ink.dx}, ${ink.dy})`);
    }
    for (const marker of facts.markers) {
      if (Math.abs(marker.dx) > .1 || Math.abs(marker.dy) > .1) failures.push(`${name}: ${marker.label} marker off center (${marker.dx}, ${marker.dy})`);
    }
    for (const comment of facts.comments) if (comment.strayStub || comment.strayElbow) failures.push(`${name}: stray connector inside ${comment.text}`);
    for (const branch of facts.branches) if (display === "all" && (!branch.drawn || Math.abs(branch.railDelta) > .1 || branch.gap > .1)) failures.push(`${name}: broken child-level connector ${JSON.stringify(branch)}`);
    if (!facts.comments.length || !facts.branches.length) throw new Error(`${name}: fixture missing comments or nested outline`);
    if (display === "all") {
      // Sample the branch at device resolution, independently of the CSS-pixel marker checks.
      const blocks = page.locator(".page-blocks .block-content");
      await blocks.getByText("Deep leaf", { exact: true }).click();
      const { row, dpr } = await blocks.getByText("First outline child", { exact: true }).evaluate((el, zoom) => {
        const main = el.closest(".block-main");
        const box = main.getBoundingClientRect();
        const arm = getComputedStyle(main, "::before"), rail = getComputedStyle(main.parentElement, "::after");
        const bullet = main.querySelector(".bullet").getBoundingClientRect();
        return { dpr: devicePixelRatio, row: { x: box.left + zoom * parseFloat(rail.left), joinY: box.top,
          top: box.top + zoom * parseFloat(arm.top), bulletY: bullet.top + bullet.height / 2,
          width: zoom * parseFloat(rail.borderLeftWidth), radius: parseFloat(arm.borderBottomLeftRadius),
          continuationVisible: rail.display !== "none" && rail.content !== "none" } };
      }, zoom);
      const junction = await page.screenshot({ path: `${out}/${name}-thread-join.png` });
      const [ink] = await page.evaluate(sampleThreadInk,
        { src: `data:image/png;base64,${junction.toString("base64")}`, rows: [row], dpr, zoom });
      facts.threadFork = ink.fork;
      if (!ink.above || !ink.fork?.arm || hasPaleThreadFork(ink.fork)) failures.push(`${name}: pale or missing routed T junction ${JSON.stringify(ink.fork)}`);
    }
    if (!cm6 && zoom === 1 && display === "all") {
      await page.keyboard.press("Escape");
      await page.evaluate(async () => (await import("/src/ui.ts")).openSettings("editor"));
      await page.locator(".settings-search-input").fill("Show brackets");
      await page.locator('[data-setting-target="settings-ui-show-brackets"]').click();
      const toggle = page.locator('[data-setting-id="settings-ui-show-brackets"]').getByRole("switch");
      if (await toggle.getAttribute("aria-checked") !== "true") throw new Error("brackets fixture must start enabled");
      await toggle.click();
      await page.keyboard.press("Escape");
      const labels = await page.locator(".ns-hier-row .page-ref").allTextContents();
      if (labels.length !== 2 || labels.some(label => label.includes("[[") || label.includes("]]"))) failures.push(`GH #959: brackets-off Hierarchy labels ${labels}`);
      if ((await page.locator(".page-blocks .page-ref").first().textContent()).includes("[[")) failures.push("GH #959: brackets-off inline link");
      await page.screenshot({ path: `${out}/959-brackets-off.png` });
      facts.bracketsOff = labels;
    }
    evidence.push({ name, ...facts });
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify({ evidence, failures }, null, 2) + "\n");
  console.log(JSON.stringify({ cases: evidence.length, failures }, null, 2));
  if (failures.length) process.exitCode = 1;
} finally { await browser?.close(); await server.close(); }
