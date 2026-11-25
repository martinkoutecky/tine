// Visual regression for GH #88: a whole-block `{{embed ((uuid))}}` presents ONE
// bullet, the embedded root's, at the host's place (D-20261030-EMB restores 0.8's
// single-root rule after beta.1's GH #726 host bullet), with a heavier source
// descendant guide. GH #1002: that root bullet and guide keep the embed accent with
// "Color indent guides by level" on and off. GH #992: the embed takes no more room
// below its last row than the same outline on its own page. Only the host's own embed
// roots take the accent (a page embed or an embed in the host's own children stays
// ordinary), and Document mode and Bullet Threading still decide which marks show.
// Usage: npm run build:mock && node scripts/shot-block-embed.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5206;
const OUT = process.env.E2E_ARTIFACT_DIR || "artifacts/gh726-embeds/block-embed-accent";
const CASES = [
  { name: "default-light", theme: "", mode: "light" },
  { name: "default-dark", theme: "", mode: "dark" },
  { name: "nord-dark", theme: "nord", mode: "dark" },
  { name: "solarized-dark", theme: "solarized", mode: "dark" },
  { name: "gruvbox-dark", theme: "gruvbox", mode: "dark" },
];
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
let browser;
let failed = false;

async function waitForServer(url) {
  for (let i = 0; i < 60; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  mkdirSync(OUT, { recursive: true });
  await waitForServer(`http://localhost:${PORT}/?regressions`);
  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 900, height: 600 } });
  page.setDefaultTimeout(5000);
  const errors = [];
  page.on("console", (message) => message.type() === "error" && errors.push(message.text()));
  page.on("pageerror", (error) => errors.push(String(error)));

  await page.goto(`http://localhost:${PORT}/?regressions&big&blocks=120`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Block embed regression");
  await page.locator(".switcher-row").filter({ hasText: /^page\s*Block embed regression$/i }).first().click();

  const host = page.locator('.block-embed-host[data-block-id]');
  await host.getByText("Embedded grandchild", { exact: true }).waitFor({ timeout: 5000 });
  const bulletCount = await host.locator(".bullet-container").count();
  const hostControlCount = await host.locator(":scope > .block-main > .block-controls").count();
  const embeddedBulletCount = await host.locator(".embed-block .bullet-container").count();
  const rootToggle = host.locator(".embed-block .collapse-toggle.has-children").first();
  const rootToggleCount = await host.locator(".embed-block .collapse-toggle.has-children").count();
  // GH #992: below its last row the embed adds at most its 1px host edge, like 0.9;
  // the same outline on its own page adds none.
  const gapBelow = await host.evaluate((element) => {
    const rows = element.querySelectorAll(".embed-block .block-main");
    return element.getBoundingClientRect().bottom - rows[rows.length - 1].getBoundingClientRect().bottom;
  });
  if (gapBelow > 2) throw new Error(`embed has ${gapBelow}px below its last row (GH #992)`);

  // The embedded root is the embed's first row, whatever wraps the group's rows.
  const guideWidth = await host.evaluate((element) => getComputedStyle(
    element.querySelector(".embed-block .ls-block > .block-children-container > .block-children"),
  ).borderLeftWidth);

  async function accentMetrics() {
    return host.evaluate((element) => {
      const root = element.querySelector(".embed-block .ls-block");
      const child = root?.querySelector(":scope > .block-children-container > .block-children > .ls-block");
      const rootBullet = root?.querySelector(":scope > .block-main > .block-controls .bullet");
      const childBullet = child?.querySelector(":scope > .block-main > .block-controls .bullet");
      const rootGuide = root?.querySelector(":scope > .block-children-container > .block-children");
      const childGuide = child?.querySelector(":scope > .block-children-container > .block-children");
      if (!rootBullet || !childBullet || !rootGuide || !childGuide) {
        throw new Error("missing embedded root/child accent geometry");
      }
      // The accent as the theme resolves it here, read from a probe painted with the token.
      const probe = document.createElement("span");
      probe.style.background = "var(--tine-embed-accent, var(--block-embed-accent))";
      element.append(probe);
      const accent = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return {
        accent,
        rootBullet: getComputedStyle(rootBullet).backgroundColor,
        childBullet: getComputedStyle(childBullet).backgroundColor,
        rootGuide: getComputedStyle(rootGuide).borderLeftColor,
        childGuide: getComputedStyle(childGuide).borderLeftColor,
      };
    });
  }

  function assertAccent(label, metrics) {
    if (metrics.rootBullet !== metrics.accent) {
      throw new Error(`${label}: embedded root bullet ${metrics.rootBullet} is not the embed accent ${metrics.accent} (GH #1002)`);
    }
    if (metrics.rootBullet !== metrics.rootGuide) {
      throw new Error(`${label}: root bullet ${metrics.rootBullet} and guide ${metrics.rootGuide} drifted`);
    }
    if (metrics.rootBullet === metrics.childBullet) {
      throw new Error(`${label}: embedded root bullet did not differ from ordinary child bullet`);
    }
    if (metrics.rootGuide === metrics.childGuide) {
      throw new Error(`${label}: embedded root guide did not differ from ordinary child guide`);
    }
  }

  if (bulletCount !== 3) {
    // Source root (standing for the host), child and grandchild.
    throw new Error(`expected 3 outline bullets, got ${bulletCount}`);
  }
  if (hostControlCount !== 0) throw new Error(`macro host has ${hostControlCount} control column(s)`);
  if (embeddedBulletCount !== 3) throw new Error(`embedded outline has ${embeddedBulletCount} bullets`);
  if (guideWidth !== "2px") throw new Error(`expected a 2px embedded-root guide, got ${guideWidth}`);
  if (rootToggleCount < 1) {
    throw new Error(`embedded live outline has no collapse control: ${await host.locator(".embed-block").innerHTML()}`);
  }
  const child = host.getByText("Embedded child", { exact: true });
  await rootToggle.click();
  await child.waitFor({ state: "detached", timeout: 3000 });
  await rootToggle.click();
  await host.getByText("Embedded grandchild", { exact: true }).waitFor({ timeout: 3000 });
  if (errors.length) throw new Error(`browser errors:\n${errors.join("\n")}`);

  for (const testCase of CASES) {
    for (const single of [false, true]) {
      await page.evaluate(({ theme, mode, single }) => {
        document.documentElement.setAttribute("data-theme", mode);
        document.documentElement.classList.toggle("tine-guide-colors-single", single);
        window.__tineApplyTheme?.(theme);
      }, { ...testCase, single });
      await sleep(180);
      const name = `${testCase.name}${single ? "-single-guide-color" : ""}`;
      const metrics = await accentMetrics();
      assertAccent(name, metrics);
      await host.screenshot({ path: `${OUT}/${name}.png` });
      console.log(JSON.stringify({ case: name, ...metrics }));
    }
  }
  await page.evaluate(() => document.documentElement.classList.remove("tine-guide-colors-single"));

  const previousMetrics = await accentMetrics();
  const custom = await page.evaluate(() => {
    const style = document.createElement("style");
    style.id = "block-embed-accent-probe";
    style.textContent = 'html[data-theme="dark"] { --accent: rgb(210, 70, 90); --bullet-color: rgb(90, 100, 110); }';
    document.head.appendChild(style);
    return true;
  });
  if (!custom) throw new Error("could not install custom accent probe");
  const customMetrics = await accentMetrics();
  assertAccent("custom accent", customMetrics);
  if (customMetrics.rootBullet === previousMetrics.rootBullet) {
    throw new Error("custom accent tokens did not recolor the block-embed accent");
  }

  await page.evaluate(() => {
    const style = document.getElementById("block-embed-accent-probe");
    if (style) style.textContent += "\n:root { --block-embed-accent: rgb(1, 2, 3); }";
  });
  const overrideMetrics = await accentMetrics();
  if (overrideMetrics.rootBullet !== "rgb(1, 2, 3)" || overrideMetrics.rootGuide !== "rgb(1, 2, 3)") {
    throw new Error(`custom --block-embed-accent override did not win: ${JSON.stringify(overrideMetrics)}`);
  }
  await page.evaluate(() => document.getElementById("block-embed-accent-probe")?.remove());

  // The mode answers which marks show; the accent only recolours what it shows.
  const modeMarks = () => host.evaluate((element) => {
    const root = element.querySelector(".embed-block .ls-block");
    const guide = root.querySelector(":scope > .block-children-container > .block-children");
    const before = getComputedStyle(guide, "::before");
    return {
      bulletOpacity: getComputedStyle(root.querySelector(":scope > .block-main .bullet-container")).opacity,
      bullet: getComputedStyle(root.querySelector(":scope > .block-main > .block-controls .bullet")).backgroundColor,
      guideColor: getComputedStyle(guide).borderLeftColor,
      guideWidth: getComputedStyle(guide).borderLeftWidth,
      beforeColor: before.content === "none" ? null : before.borderLeftColor,
    };
  });
  const clear = "rgba(0, 0, 0, 0)";
  await page.evaluate(() => document.documentElement.classList.add("document-mode"));
  const documentMode = await modeMarks();
  await page.evaluate(() => document.documentElement.classList.remove("document-mode"));
  if (documentMode.bulletOpacity !== "0" || documentMode.guideColor !== clear) {
    throw new Error(`Document mode shows the embedded root's marks: ${JSON.stringify(documentMode)}`);
  }
  // Bullet Threading as the host decorates every row (src/components/block/rowChrome.tsx).
  const threading = (width) => host.evaluate((element, width) => {
    for (const row of element.querySelectorAll(".ls-block")) {
      row.classList.add("plugin-thread-lines");
      row.classList.toggle("plugin-thread-lines-hidden", width === 0);
      row.style.setProperty("--thread-width", `${width}px`);
      row.style.setProperty("--thread-color", "rgb(200, 0, 200)");
    }
  }, width);
  for (const width of [1, 0]) {
    await threading(width);
    const marks = await modeMarks();
    if (marks.guideWidth !== "0px" || marks.beforeColor !== clear) {
      throw new Error(`Bullet Threading (thickness ${width}) gets a solid embed guide over its connector: ${JSON.stringify(marks)}`);
    }
  }
  await threading(1);
  await host.evaluate((element) => {
    const content = element.querySelector(".embed-block .ls-block > .block-main .block-content");
    content.tabIndex = 0;
    content.focus();
  });
  const focused = await modeMarks();
  if (focused.bullet !== "rgb(200, 0, 200)") {
    throw new Error(`Bullet Threading's active bullet lost to the embed accent: ${focused.bullet}`);
  }

  // Which rows are the embed's root: the host's own embed's, nothing else (GH #125).
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Block embed nesting regression");
  await page.locator(".switcher-row").filter({ hasText: /^page\s*Block embed nesting regression$/i }).first().click();
  const nestingHost = page.locator(".main-content .block-embed-host").first();
  const nested = nestingHost.locator(".embed-block .block-embed-host").first();
  await nestingHost.getByText("Nesting root", { exact: true }).locator("visible=true").first().waitFor();
  await nested.scrollIntoViewIfNeeded();
  await nested.getByText("Embedded grandchild", { exact: true }).first().waitFor();
  const nesting = await nestingHost.evaluate((element) => {
    const rows = (scope, text) => [...scope.querySelectorAll(".ls-block")].filter((row) => !row.classList.contains("block-embed-host")
      && row.querySelector(":scope > .block-main .block-content")?.textContent.trim() === text);
    const probe = document.createElement("span");
    probe.style.background = "var(--tine-embed-accent, var(--block-embed-accent))";
    element.append(probe);
    const accent = getComputedStyle(probe).backgroundColor;
    probe.remove();
    const look = (row) => {
      const guide = row.querySelector(":scope > .block-children-container > .block-children");
      return {
        bullet: getComputedStyle(row.querySelector(":scope > .block-main > .block-controls .bullet")).backgroundColor === accent,
        guide: guide ? getComputedStyle(guide).borderLeftColor === accent && getComputedStyle(guide).borderLeftWidth === "2px" : null,
      };
    };
    const embed = element.querySelector(":scope > .block-main .embed-block");
    const own = element.querySelector(":scope > .block-children-container");
    return {
      "host root": look(rows(embed, "Nesting root")[0]),
      "nested whole-block embed root": look(rows(embed, "Embedded root")[0]),
      "nested page embed root": look(rows(embed, "Source root one")[0]),
      "page embed in a host child": look(rows(own, "Source root one")[0]),
      "embed with text in a host child": look(rows(own, "Embedded root")[0]),
    };
  });
  for (const [cell, look] of Object.entries(nesting)) {
    const want = cell.endsWith("host root") || cell.startsWith("nested whole-block");
    if (look.bullet !== want || look.guide !== want) {
      throw new Error(`${cell}: accent ${JSON.stringify(look)}, expected ${want} (GH #125)`);
    }
  }
  console.log(JSON.stringify({ out: OUT, bulletCount, hostControlCount, embeddedBulletCount, guideWidth, gapBelow, documentMode, focused, nesting }));
} catch (error) {
  console.error(String(error));
  failed = true;
} finally {
  await browser?.close();
  server.kill("SIGKILL");
}
if (failed) process.exit(1);
