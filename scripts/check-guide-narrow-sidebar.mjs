#!/usr/bin/env node
// The published Guide / HTML export at phone width: the page list folds behind
// the Pages toggle, so the page itself starts near the top of the screen, and
// the list is one tap away. At desktop width the list stays a sidebar.
// Usage: node scripts/check-guide-narrow-sidebar.mjs [site-dir]
//   (default target/guide/demo, built by scripts/build-guide-demo.mjs)
import { chromium } from "playwright";
import path from "node:path";
import { pathToFileURL } from "node:url";

const dir = path.resolve(process.argv[2] || "target/guide/demo");
const url = pathToFileURL(path.join(dir, "index.html")).href;
const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu"] });
const failures = [];
const check = (ok, what) => { if (!ok) failures.push(what); };
try {
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await phone.goto(url);
  await phone.waitForFunction(() => document.querySelectorAll("#tine-pages a").length > 0);
  const narrow = () => phone.evaluate(() => {
    const box = (sel) => document.querySelector(sel)?.getBoundingClientRect();
    const nav = box("#tine-pages");
    return {
      mainTop: box("main").top,
      navShown: !!nav && nav.height > 0,
      toggleShown: (box(".pages-toggle")?.height ?? 0) > 0,
      expanded: document.querySelector(".pages-toggle")?.getAttribute("aria-expanded"),
      scrollWidth: document.documentElement.scrollWidth,
    };
  });
  const closed = await narrow();
  await phone.screenshot({ path: path.join(dir, "..", "narrow-sidebar-390.png") });
  check(closed.mainTop < 844 / 3, `phone: page starts at ${closed.mainTop}px, below the first third`);
  check(!closed.navShown, "phone: page list shown before the toggle");
  check(closed.toggleShown, "phone: no Pages toggle");
  check(closed.scrollWidth <= 390, `phone: horizontal scroll (${closed.scrollWidth}px)`);
  if (closed.toggleShown) {
    await phone.click(".pages-toggle");
    const open = await narrow();
    check(open.navShown && open.expanded === "true", "phone: toggle does not open the page list");
    await phone.click(".pages-toggle");
    const shut = await narrow();
    check(!shut.navShown && shut.expanded === "false", "phone: toggle does not close the page list");
  }
  await phone.fill("#tine-search", "tine");
  const hits = await phone.evaluate(() => document.querySelectorAll("#tine-results a.res").length);
  check(hits > 0, "phone: search shows no results while the list is folded");

  const desk = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await desk.goto(url);
  await desk.waitForFunction(() => document.querySelectorAll("#tine-pages a").length > 0);
  const wide = await desk.evaluate(() => ({
    navShown: document.querySelector("#tine-pages").getBoundingClientRect().height > 0,
    toggleShown: (document.querySelector(".pages-toggle")?.getBoundingClientRect().height ?? 0) > 0,
  }));
  check(wide.navShown, "desktop: page list hidden");
  check(!wide.toggleShown, "desktop: Pages toggle shown");
} finally {
  await browser.close();
}
if (failures.length) {
  console.error(`FAIL ${url}\n- ${failures.join("\n- ")}`);
  process.exit(1);
}
console.log(`ok ${url}`);
