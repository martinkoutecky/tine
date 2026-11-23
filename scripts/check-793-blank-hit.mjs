// The routed web app on the browser mock's `?blankhit` fixture pages
// (src/mock/flags/blankhit/) and the production stylesheet. Playwright sends
// actual pointer and keyboard input. The mock clipboard captures the two
// formats at the backend boundary.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import { openJournals, openPageByName } from "./lib/e2e-navigation.mjs";
import { idOf, mockBlockIds } from "./lib/mock-block-ids.mjs";

const block = (id, raw, children = [], collapsed = false) => ({ id, raw, children, collapsed });
// The fixture files' outlines; each block's id is the core's, read at startup.
const note = (path, name, blocks, kind = "page", pre_block = null) => ({ path, name, title: name, kind, pre_block, blocks });
const leaf = block("leaf", "Source child");
const ref = block("ref", "A link to [[Blank Hit]]", [leaf]);
const short = block("short", "Short outline", [block("short-child", "Short child")]);
const windowed = block("windowed", "Windowed outline", Array.from({ length: 85 }, (_, i) => block(`child-${i}`, `Child ${i}`)));
const shortPage = note("pages/Short Blank Hit.md", "Short Blank Hit", [short], "page", "Preamble text");
const longPage = note("pages/Blank Hit.md", "Blank Hit", [windowed, ...Array.from({ length: 4 }, (_, i) => block(`row-${i}`, `Row ${i}`)),
  ...Array.from({ length: 3 }, (_, i) => block(`tail-${i}`, `Tail ${i}`, [block(`tail-child-${i}`, `Tail child ${i}`)]))]);
const refPage = note("pages/Blank Hit Ref.md", "Blank Hit Ref", [ref, block("unrelated", "Unrelated source block")]);
const embedded = block("embedded", "Embedded source", [block("embedded-child", "Embedded child")]);
const embedSource = note("pages/Embedded Blank Hit.md", "Embedded Blank Hit", [embedded]);
const embedPage = note("pages/Embed Blank Hit.md", "Embed Blank Hit", [block("embed-host", "{{embed [[Embedded Blank Hit]]}}")]);
const firstDay = note("journals/2026_06_14.md", "Jun 14th, 2026", [block("day-one", "First day")], "journal");
const secondDay = note("journals/2026_06_13.md", "Jun 13th, 2026", [block("day-two", "Second day", [block("day-child", "Second day child")])], "journal");
const fixtures = [shortPage, longPage, refPage, embedSource, embedPage, firstDay, secondDay];

const root = process.cwd();
const baseline = process.argv.find(arg => arg.startsWith("--base="))?.slice(7);
const only = process.argv.find(arg => arg.startsWith("--case="))?.slice(7);
const originals = new Map();
if (baseline) {
  const git = (...args) => execFileSync("git", ["-C", root, ...args], { encoding: "utf8" });
  for (const file of git("diff", "--name-only", baseline, "--", "src").trim().split("\n").filter(Boolean)) {
    if (!file.includes(".test.")) originals.set(path.resolve(root, file), git("show", `${baseline}:${file}`));
  }
}

const server = await createServer({
  cacheDir: path.resolve("target/793-rule-vite"),
  server: { host: "127.0.0.1", port: 0, strictPort: false, watch: null, hmr: false },
  plugins: [{
    name: "blank-hit-baseline", enforce: "pre",
    load(id) { return originals.get(id); },
  }],
});

const flatten = blocks => blocks.flatMap(row => [row, ...flatten(row.children)]);
const expectedMarkdown = (blocks, depth = 0) => blocks.flatMap(row => [
  `${"\t".repeat(depth)}- ${row.raw}`, ...expectedMarkdown(row.children, depth + 1),
]);
const expectedHtml = blocks => blocks.map(row => [row.raw, expectedHtml(row.children)]);

async function blankPoint(locator) {
  await locator.waitFor();
  if (await locator.evaluate(el => el.offsetHeight < window.innerHeight)) await locator.scrollIntoViewIfNeeded();
  return locator.evaluate(el => {
    const rect = el.getBoundingClientRect();
    const hits = [];
    for (const y of [rect.top + rect.height / 2, rect.top + 0.25, rect.bottom - 1.25, rect.bottom - 0.75, rect.top + 2]) {
      for (const x of [rect.left + 3.25, rect.right - 2, rect.left + rect.width / 2, rect.left + 0.25, rect.left + 6]) {
        const target = document.elementFromPoint(x, y);
        if (target === el) return { x, y };
        hits.push([x, y, target?.tagName, target?.className, target?.closest(".ls-block")?.dataset.blockId]);
      }
    }
    throw new Error(`No blank pointer target on ${el.className}: ${JSON.stringify({ rect: rect.toJSON(), hits })}`);
  });
}

async function separatorPoint(wrapper) {
  await wrapper.locator(".page-title").scrollIntoViewIfNeeded();
  return wrapper.evaluate(day => {
    const section = day.querySelector(".page-section");
    const band = [day, section].find(el => parseFloat(getComputedStyle(el).borderTopWidth) > 0);
    const rect = band.getBoundingClientRect();
    const css = getComputedStyle(band);
    const x = rect.left + rect.width / 2;
    const y = rect.top + parseFloat(css.borderTopWidth) + parseFloat(css.paddingTop) / 2;
    if (document.elementFromPoint(x, y) !== band) throw new Error("The separator band did not receive the pointer");
    return { x, y, target: band.className };
  });
}

await server.listen();
const address = `${server.resolvedUrls.local[0]}?blankhit`;
let browser;
const failures = [];
try {
  browser = await chromium.launch({ headless: true, args: ["--no-sandbox"] });
  {
    const probe = await browser.newPage();
    await probe.goto(address, { waitUntil: "commit" });
    await probe.locator(".main-content").first().waitFor();
    const ids = await mockBlockIds(probe, fixtures.map(note => note.path));
    for (const note of fixtures) for (const row of flatten(note.blocks)) row.id = idOf(ids[note.path], row.raw);
    await probe.close();
  }
const first = `[data-block-id="${short.id}"]`;
const wide = `[data-block-id="${windowed.id}"]`;
const childZero = `[data-block-id="${windowed.children[0].id}"]`;
const day = '.journal-feed-day[data-feed-day="Jun 13th, 2026"]';
const positive = [
  ["scroller gutter", shortPage, ".main-content", shortPage.blocks],
  ["scroller inner padding", shortPage, ".main-content-inner", shortPage.blocks],
  ["preamble row padding", shortPage, ".preamble-block > .block-main", shortPage.blocks],
  ["short row padding", shortPage, `${first} > .block-main`, shortPage.blocks],
  ["short child indentation", shortPage, `${first} > .block-children-container > .block-children`, shortPage.blocks],
  ["windowed row padding", longPage, `.outline-window ${childZero} > .block-main`, longPage.blocks],
  ["windowed child indentation", longPage, `${wide} > .block-children-container > .block-children`, longPage.blocks],
  ["reference card shell", longPage, ".reference-group", [ref]],
  ["reference card header", longPage, ".reference-group-header", [ref]],
  ["reference row padding", longPage, `[data-block-id="${ref.id}"] > .block-main`, [ref]],
  ["embed accent padding", embedPage, ".embed-page > .live-ref-group", [embedded]],
  ["second journal body", secondDay, `${day} .page-blocks`, secondDay.blocks],
  ["second journal row padding", secondDay, `${day} .page-blocks > .ls-block > .block-main`, secondDay.blocks],
  ["second journal indentation", secondDay, `${day} .block-children`, secondDay.blocks],
  ["zoom row padding", shortPage, ".zoomed-page .block-main", [short]],
  ["zoom child indentation", shortPage, ".zoomed-page .block-children", [short]],
];

  async function run(name, route, action, targets, getPoint, shouldSelect) {
    if (only && name !== only) return;
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    page.setDefaultTimeout(12_000);
    try {
      await page.goto(address, { waitUntil: "commit" });
      await page.locator(".main-content").first().waitFor();
      if (route.kind === "journal") await openJournals(page);
      else await openPageByName({
        execute: (fn, ...args) => page.evaluate(({ code, values }) => (0, eval)(`(${code})`)(...values), { code: fn.toString(), values: args }),
        keys: keys => page.keyboard.press(Array.isArray(keys) ? keys.join("+") : keys),
        $: selector => ({ waitForExist: options => page.locator(selector).waitFor(options), setValue: value => page.locator(selector).fill(value) }),
        waitUntil: async (condition, { timeout, timeoutMsg }) => {
          const until = Date.now() + timeout;
          while (!await condition()) assert.ok(Date.now() < until, timeoutMsg);
        },
      }, route.name);
      if (name.startsWith("zoom ")) await page.locator(`${first} > .block-main .bullet-container`).click();
      if (targets.includes(ref)) {
        await page.locator(".reference-group-header").first().scrollIntoViewIfNeeded();
        await page.locator(`.ls-block:not(.ref-block)[data-block-id="${ref.id}"]`).waitFor();
      }
      if (route === embedPage) await page.locator(`.ls-block:not(.ref-block)[data-block-id="${embedded.id}"]`).waitFor();
      if (name.startsWith("windowed ")) await page.locator(`.outline-window ${childZero}`).waitFor();
      if (route.kind === "journal") await page.locator(`${day} [data-block-id="${secondDay.blocks[0].id}"]`).waitFor();
      // Expose the intended outline hit on themes where adjacent rows cover
      // the last padding pixel or the guide button covers the indent strip.
      if (name.includes("indentation")) await page.addStyleTag({ content: ".block-children { padding-inline-start: 12px }" });
      if (name === "preamble row padding" || name === "windowed row padding") {
        await page.addStyleTag({ content: ".preamble-block > .block-main, .outline-window > .ls-block > .block-main { padding-block-end: 5px }" });
      }
      if (name === "second journal body") await page.addStyleTag({ content: ".journal-feed-day .page-blocks { padding-block-end: 5px }" });
      await page.evaluate(async () => {
        const { backend } = await import("/src/backend.ts");
        const { node, topSelected } = await import("/src/document/index.ts");
        const observed = { clipboard: null, events: [], read: ids => ids.map(id => node(id)?.raw ?? null), roots: topSelected };
        backend().writeRich = async (text, html) => { observed.clipboard = { text, html }; };
        window.addEventListener("keydown", event => queueMicrotask(() => observed.events.push([event.key, event.defaultPrevented])), true);
        window.gh793 = observed;
      });
      const removed = flatten(targets);
      const retained = route.kind === "journal" ? flatten([...firstDay.blocks, ...secondDay.blocks])
        : flatten(route.blocks);
      if (targets.includes(ref)) retained.push(refPage.blocks[1]);
      const source = [...new Map([...removed, ...retained].map(row => [row.id, row])).values()];
      const ids = source.map(row => row.id);
      assert.deepEqual(await page.evaluate(ids => window.gh793.read(ids), ids), source.map(row => row.raw));
      const hover = await getPoint(page);
      await page.mouse.move(hover.x, hover.y);
      const point = await getPoint(page);
      await page.mouse.click(point.x, point.y);
      await page.keyboard.press("Control+a");
      const roots = await page.evaluate(() => window.gh793.roots());
      await page.keyboard.press(action);
      const result = await page.evaluate(ids => {
        const { clipboard, events, read, roots } = window.gh793;
        function tree(list) {
          return [...list.children].map(li => [li.firstChild.textContent, li.querySelector(":scope > ul") ? tree(li.querySelector(":scope > ul")) : []]);
        }
        const html = clipboard && tree(new DOMParser().parseFromString(clipboard.html, "text/html").querySelector("body > ul"));
        return { events: events.filter(([key]) => key !== "Control"), source: read(ids), roots: roots(), text: clipboard?.text ?? null, html };
      }, ids);
      const copied = shouldSelect && action.startsWith("Control+");
      assert.deepEqual({ chosen: roots, handled: result.events.map(([, prevented]) => prevented), active: result.roots, source: result.source }, {
        chosen: shouldSelect ? targets.map(row => row.id) : [],
        handled: [shouldSelect, shouldSelect],
        active: [],
        source: source.map(row => shouldSelect && action !== "Control+c" && removed.includes(row) ? null : row.raw),
      }, `Pointer hit ${point.target ?? name}`);
      assert.equal(result.text, copied ? expectedMarkdown(targets).join("\n") : null);
      assert.deepEqual(result.html, copied ? expectedHtml(targets) : null);
      console.log(`PASS ${name}, ${action}${point.target ? ` (${point.target})` : ""}`);
    } catch (error) {
      console.error(`FAIL ${name}, ${action}`, error);
      failures.push(`${name}, ${action}`);
    } finally { await page.close(); }
  }
  for (const action of ["Delete", "Backspace", "Control+x"]) {
    await run("journal separator band", secondDay, action, [], page => separatorPoint(page.locator(day)), false);
  }
  if (!baseline) {
    for (const [name, route, selector, targets] of positive) {
      for (const action of ["Control+c", "Delete", "Backspace", "Control+x"]) {
        await run(name, route, action, targets, page => blankPoint(page.locator(selector).first()), true);
      }
    }
  }
} finally {
  await browser?.close();
  await server.close();
}
assert.deepEqual(failures, []);
