import { afterEach, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

// PDF5-SCROLL: pdf.js 5 gives every glyph span a CSS transform, an identity one
// for spans it never stretched or rotated. WebKit keeps each transformed span as
// its own layer and repaints them all on every scroll frame, which halved
// fast-scroll throughput. Those spans must stay untransformed (as in pdf.js 4)
// whenever the minimum-font scale is identity; stretched and rotated spans, and
// every span under a larger minimum font size, keep pdf.js's transform.

const css = ["node_modules/pdfjs-dist/web/pdf_viewer.css", "src/styles/pdf-workspace.css"]
  .map((path) => readFileSync(path, "utf8")).join("\n");

function layer(unitFont: boolean) {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.appendChild(style);
  const page = document.createElement("div");
  page.className = "pdf-page";
  page.innerHTML = `<div class="textLayer${unitFont ? " pdf-text-unit-font" : ""}" style="--min-font-size: 1;">`
    + `<span id="plain" style="left: 1%; top: 2%; --font-height: 9.96px; font-family: sans-serif;">x</span>`
    + `<span id="stretched" style="left: 1%; top: 3%; --font-height: 9.96px; --scale-x: 1.02;">word</span>`
    + `<span id="rotated" style="left: 1%; top: 4%; --font-height: 9.96px; --rotate: 90deg;">y</span>`
    + `<br id="eol">`
    + `<span class="markedContent"><span id="marked" style="--font-height: 9.96px;">z</span></span>`
    + `</div>`;
  document.body.appendChild(page);
  const transform = (id: string) => getComputedStyle(page.querySelector(`#${id}`)!).transform;
  return { transform, dispose: () => { page.remove(); style.remove(); } };
}

describe("pdf.js 5 text layer transforms", () => {
  let dispose = () => {};
  afterEach(() => dispose());

  it("leaves glyph spans pdf.js neither stretched nor rotated untransformed", () => {
    const l = layer(true);
    dispose = l.dispose;
    expect(l.transform("plain")).toBe("none");
    expect(l.transform("eol")).toBe("none");
    expect(l.transform("marked")).toBe("none");
  });

  it("keeps pdf.js's transform for stretched and rotated spans", () => {
    const l = layer(true);
    dispose = l.dispose;
    expect(l.transform("stretched")).toContain("scaleX(var(--scale-x))");
    expect(l.transform("rotated")).toContain("rotate(var(--rotate))");
  });

  it("keeps pdf.js's minimum-font scale when the browser enforces one", () => {
    const l = layer(false);
    dispose = l.dispose;
    expect(l.transform("plain")).toContain("scale(var(--min-font-size-inv))");
  });
});
