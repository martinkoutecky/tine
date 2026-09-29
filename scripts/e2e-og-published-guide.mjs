// Browser smoke for the generated live Guide: reviewed permalink, baked query,
// read-only controls, and static fallback. Run after `npm run docs:build`.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const queryFixture = Boolean(process.env.TINE_PUBLISHED_ROOT);
const root = path.resolve(process.env.TINE_PUBLISHED_ROOT ??
  path.dirname(fileURLToPath(import.meta.url)), ...(queryFixture ? [] : ["../website/demo"]));
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css",
  ".json": "application/json", ".wasm": "application/wasm", ".svg": "image/svg+xml" };
const server = http.createServer(async (request, response) => {
  const pathname = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
  const relative = (pathname.endsWith("/") ? `${pathname}index.html` : pathname).replace(/^\/+/, "");
  const file = path.resolve(root, relative);
  if (!file.startsWith(root + path.sep)) { response.writeHead(403).end(); return; }
  try {
    const bytes = await fs.readFile(file);
    response.setHeader("Content-Type", mime[path.extname(file)] ?? "application/octet-stream");
    response.end(bytes);
  } catch { response.writeHead(404).end(); }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const address = server.address();
const origin = `http://127.0.0.1:${address.port}`;
let browser;
try {
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${origin}/#/page/${queryFixture ? "Selected%20tasks" : "Features%2FQueries"}`);
  await page.getByText(queryFixture ? "Selected tasks" : "A query in action").first().waitFor();
  assert.equal(new URL(page.url()).pathname, "/app/");
  assert.match(await page.locator("body").innerText(), queryFixture ? /Ship the M0 vertical slice/ : /Every open task in this graph/);
  assert.equal(await page.locator(".new-page-btn:visible").count(), 0);
  assert.equal(await page.getByTitle("Settings (t s)").count(), 0);
  assert.deepEqual(errors, []);
  await page.goto(`${origin}/index.html?static=1`);
  await page.getByText(queryFixture ? "Jun 14th, 2026" : "Welcome to Tine").first().waitFor();
  assert.equal(new URL(page.url()).pathname, "/index.html");
  console.log(`PASS: ${queryFixture ? "query" : "Guide"} permalink, baked query, read-only controls, static fallback`);
} finally {
  await browser?.close();
  server.close();
}
