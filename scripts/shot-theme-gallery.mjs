// Built-in theme gallery acceptance shots. Runs against the built frontend and
// mock backend, saving full visual checks under subagent-tasks/notes/ plus small
// crops used by Settings card thumbnails.
// Usage (after `source scripts/env.sh && npm run build`):
//   node scripts/shot-theme-gallery.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5213;
const OUT = "subagent-tasks/notes";
const THUMBS = "public/theme-thumbnails";
const THEMES = ["nord", "solarized", "gruvbox"];
const MODES = ["light", "dark"];

mkdirSync(OUT, { recursive: true });
mkdirSync(THUMBS, { recursive: true });

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], {
  stdio: "ignore",
});

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      // still starting
    }
    await sleep(250);
  }
  throw new Error("server did not start");
}

async function openPage(browser, customCss = "") {
  const context = await browser.newContext({
    viewport: { width: 1280, height: 960 },
    deviceScaleFactor: 2,
  });
  if (customCss) {
    await context.addInitScript((css) => {
      globalThis.__tineMockCustomCss = css;
    }, customCss);
  }
  const page = await context.newPage();
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await page.waitForSelector(".page-ref", { timeout: 8000 });
  await page.waitForSelector(".tag", { timeout: 8000 });
  await page.waitForSelector(".inline-code", { timeout: 8000 });
  await sleep(350);
  return { context, page };
}

async function setMode(page, mode) {
  await page.evaluate((next) => {
    document.documentElement.setAttribute("data-theme", next);
  }, mode);
  await sleep(120);
}

async function setTheme(page, theme) {
  await page.evaluate((id) => {
    window.__tineApplyTheme?.(id);
  }, theme);
  await sleep(180);
}

async function metrics(page) {
  return page.evaluate(() => {
    const style = (el, prop) => getComputedStyle(el).getPropertyValue(prop);
    const body = document.body;
    const link = document.querySelector(".page-ref, a");
    const tag = document.querySelector(".tag");
    const code = document.querySelector(".inline-code, .code-block, code");
    const sidebar = document.querySelector(".left-sidebar");
    const theme = document.getElementById("tine-theme");
    const ids = Array.from(document.head.children).map((el) => el.id).filter(Boolean);
    return {
      bg: style(body, "background-color"),
      text: style(body, "color"),
      link: link ? style(link, "color") : "",
      tag: tag ? style(tag, "color") : "",
      border: sidebar ? style(sidebar, "border-right-color") : "",
      codeBg: code ? style(code, "background-color") : "",
      themeBytes: theme?.textContent?.length ?? 0,
      order: ids.filter((id) => id === "tine-ls-shim" || id === "tine-theme" || id === "tine-custom-css"),
    };
  });
}

function assertRecolored(theme, mode, got, base) {
  for (const key of ["bg", "text", "link", "border", "codeBg"]) {
    if (!got[key] || got[key] === base[key]) {
      throw new Error(`${theme}/${mode} did not recolor ${key}: ${got[key] || "(missing)"}`);
    }
  }
}

function assertManagedOrder(label, order) {
  const shim = order.indexOf("tine-ls-shim");
  const theme = order.indexOf("tine-theme");
  const custom = order.indexOf("tine-custom-css");
  if (shim === -1 || theme === -1) throw new Error(`${label}: missing managed style nodes (${order.join(" > ")})`);
  if (custom !== -1 && !(shim < theme && theme < custom)) {
    throw new Error(`${label}: bad style order (${order.join(" > ")})`);
  }
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({
    args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"],
  });

  const { context, page } = await openPage(browser);
  const defaultMetrics = {};

  for (const mode of MODES) {
    await setMode(page, mode);
    await setTheme(page, "");
    const got = await metrics(page);
    if (got.themeBytes !== 0) throw new Error(`Default/${mode} left #tine-theme non-empty`);
    assertManagedOrder(`Default/${mode}`, got.order);
    defaultMetrics[mode] = got;
    await page.screenshot({ path: `${OUT}/theme-gallery-default-${mode}.png` });
    if (mode === "light") {
      await page.screenshot({
        path: `${THUMBS}/default.png`,
        clip: { x: 246, y: 58, width: 640, height: 360 },
      });
    }
    console.log("OK   ", `default/${mode}`, got);
  }

  for (const theme of THEMES) {
    for (const mode of MODES) {
      await setMode(page, mode);
      await setTheme(page, theme);
      const got = await metrics(page);
      if (got.themeBytes === 0) throw new Error(`${theme}/${mode} did not populate #tine-theme`);
      assertManagedOrder(`${theme}/${mode}`, got.order);
      assertRecolored(theme, mode, got, defaultMetrics[mode]);
      await page.screenshot({ path: `${OUT}/theme-gallery-${theme}-${mode}.png` });
      if (mode === "light") {
        await page.screenshot({
          path: `${THUMBS}/${theme}.png`,
          clip: { x: 246, y: 58, width: 640, height: 360 },
        });
      }
      console.log("OK   ", `${theme}/${mode}`, got);
    }
  }
  await context.close();

  const { context: hotContext, page: hotPage } = await openPage(
    browser,
    'html[data-theme="light"] { --ls-primary-background-color: hotpink; }'
  );
  await setMode(hotPage, "light");
  await setTheme(hotPage, "nord");
  const hot = await metrics(hotPage);
  assertManagedOrder("hotpink cascade", hot.order);
  if (hot.bg !== "rgb(255, 105, 180)") {
    throw new Error(`hotpink custom.css did not win; body background is ${hot.bg}`);
  }
  await hotPage.screenshot({ path: `${OUT}/theme-gallery-hotpink-cascade.png` });
  console.log("OK   ", "hotpink cascade", hot);
  await hotContext.close();

  await browser.close();
  console.log("theme gallery screenshots written to", OUT);
} finally {
  server.kill("SIGTERM");
}
