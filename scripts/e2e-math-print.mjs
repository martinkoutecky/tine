// GH #915 paper diagnostic: print receives the app bundle too. A5 fitting is a pending product decision.
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { chromium } from "playwright";
import katex from "katex";
import { getDocument } from "pdfjs-dist/legacy/build/pdf.mjs";
import { mathOverflowEquations as equations } from "./lib/math-overflow-fixture.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "target/artifacts/fix-beta-math-wrap-915");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "print-"));
const cssFile = process.env.MATH_PRINT_CSS || path.join(root, "src/styles/math.css");
const css = fs.readFileSync(path.join(root, "src/styles/app.css"), "utf8")
  .replace('@import "./math.css";', fs.readFileSync(cssFile, "utf8"));
const html = `<!doctype html><meta charset="utf-8"><style>
${fs.readFileSync(path.join(root, "node_modules/katex/dist/katex.css"), "utf8")}
${fs.readFileSync(path.join(root, "crates/tine-graph-features/src/render_style.css"), "utf8")}
${css}
@page { size: A5; margin: 16mm; }
</style><ul>${equations.flatMap(tex => [false, true].map(displayMode =>
  `<li><span class="math${displayMode ? " math-display" : ""}">${katex.renderToString(tex, { displayMode })}</span></li>`)).join("")}</ul>`;
const server = http.createServer((request, response) => {
  if (request.url === "/") { response.setHeader("Content-Type", "text/html; charset=utf-8"); response.end(html); return; }
  const font = path.join(root, "node_modules/katex/dist/fonts", path.basename(request.url));
  if (request.url.startsWith("/fonts/") && fs.existsSync(font)) response.end(fs.readFileSync(font));
  else { response.statusCode = 404; response.end(); }
});
let browser;
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  cssFile, expectedOxygenAtoms: equations.reduce((count, tex) => count + [...tex].filter(char => char === "O").length, 0) * 2,
  pages: [] };
try {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 560, height: 900 } });
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.evaluate(() => document.fonts.ready);
  await page.emulateMedia({ media: "print" });
  proof.scrollports = await page.evaluate(() => [...document.querySelectorAll(".math, .math *")]
    .filter(element => element.scrollWidth > element.clientWidth + 1
      && ["auto", "scroll"].includes(getComputedStyle(element).overflowX)).length);
  const pdf = await page.pdf({ format: "A5", preferCSSPageSize: true, printBackground: true });
  fs.writeFileSync(path.join(out, "math.pdf"), pdf);
  const document = await getDocument({ data: new Uint8Array(pdf) }).promise;
  let text = "";
  try {
    for (let number = 1; number <= document.numPages; number++) {
      const printed = await document.getPage(number);
      const viewport = printed.getViewport({ scale: 1 });
      const content = await printed.getTextContent();
      const words = content.items.filter(item => "str" in item && item.str.trim());
      text += words.map(item => item.str).join("");
      proof.pages.push({ number, width: viewport.width, outside: words.filter(item =>
        item.transform[4] < -1 || item.transform[4] + item.width > viewport.width + 1)
        .map(item => ({ text: item.str, left: item.transform[4], right: item.transform[4] + item.width })) });
      const bitmap = printed.getViewport({ scale: 2 });
      const canvas = document.canvasFactory.create(Math.ceil(bitmap.width), Math.ceil(bitmap.height));
      try {
        await printed.render({ canvas: canvas.canvas, canvasContext: canvas.context, viewport: bitmap }).promise;
        fs.writeFileSync(path.join(out, `page-${number}.png`), canvas.canvas.toBuffer("image/png"));
      } finally { document.canvasFactory.destroy(canvas); }
    }
  } finally { await document.destroy(); }
  proof.oxygenAtoms = [...text].filter(char => char === "O").length;
  assert.equal(proof.scrollports, 0, "Printed formulas must not be clipped in interactive scrollports");
  assert.equal(proof.oxygenAtoms, proof.expectedOxygenAtoms, "Every equation retains all of its oxygen atoms in the PDF");
  assert.ok(proof.pages.every(page => page.outside.length === 0), "All printed math lies within the paper");
  proof.outcome = "PASS";
} catch (error) {
  proof.outcome = "FAIL";
  proof.expected = "Complete inline/display equations on narrow paper, without screen scrollport clipping";
  proof.observed = String(error.stack);
  proof.classification = error.code === "ERR_ASSERTION" ? "product" : "infrastructure";
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`${proof.outcome}: ${out}`);
  await browser?.close();
  if (server.listening) await new Promise(resolve => server.close(resolve));
}
