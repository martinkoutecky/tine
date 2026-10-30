import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";
import { SHADOW, shadowPng } from "../../scripts/gen-pdf-page-shadow.mjs";

// PDF9-5: the page shadow must not be a CSS blur `box-shadow`. Without GPU
// compositing WebKit repaints the scroller on every scroll frame, and the
// page-sized blur cost ~20 ms of a 45 ms frame at DPR 2 (and, because pdf.js
// renders one 15 ms slice per animation frame, stretched every page render:
// ~1.7 s of fit-width fast-scroll blank). The shadow is the same look as a
// pre-rendered 9-slice image on `.pdf-page::before`.

const css = readFileSync("src/styles/pdf-workspace.css", "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

/** Declarations of every top-level rule whose selector list contains `selector` exactly. */
function declarations(selector: string): string[] {
  const out: string[] = [];
  for (const m of css.matchAll(/([^{}@;]+)\{([^{}]*)\}/g)) {
    if (m[1].split(",").some((part) => part.trim() === selector)) out.push(m[2]);
  }
  return out;
}

function alphaChannel(png: Buffer): { size: number; alpha: number[] } {
  expect([...png.subarray(0, 8)]).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
  const size = png.readUInt32BE(16);
  expect(png.readUInt32BE(20)).toBe(size);
  expect(png[24]).toBe(8); // bit depth
  expect(png[25]).toBe(6); // RGBA
  const data: Buffer[] = [];
  for (let at = 8; at < png.length;) {
    const length = png.readUInt32BE(at);
    if (png.toString("latin1", at + 4, at + 8) === "IDAT") data.push(png.subarray(at + 8, at + 8 + length));
    at += 12 + length;
  }
  const raw = inflateSync(Buffer.concat(data));
  const alpha: number[] = [];
  for (let y = 0; y < size; y++) {
    expect(raw[y * (size * 4 + 1)]).toBe(0); // filter: none
    for (let x = 0; x < size; x++) alpha.push(raw[y * (size * 4 + 1) + 1 + x * 4 + 3]);
  }
  return { size, alpha };
}

describe("PDF page shadow", () => {
  it("is not a blurred box-shadow on the page", () => {
    const rules = [".pdf-page", ".pdf-viewer .pdf-page", ".pdf-scroll .pdf-page"].flatMap(declarations);
    expect(rules.length).toBeGreaterThan(0);
    for (const body of rules) {
      for (const shadow of body.matchAll(/box-shadow\s*:\s*([^;]+)/g)) {
        // `none`, or a shadow without blur (offset-x offset-y 0 ...), never a real blur.
        const lengths = shadow[1].match(/-?\d*\.?\d+px|(?<![\w.])0(?![\w.])/g) ?? [];
        const blur = lengths[2] ?? "0";
        expect(shadow[1].trim() === "none" || parseFloat(blur) === 0, `blurred page shadow: ${shadow[0]}`).toBe(true);
      }
    }
  });

  it("is drawn from an image on .pdf-page::before that takes no pointer events", () => {
    const [body] = declarations(".pdf-page::before");
    expect(body).toBeDefined();
    expect(body).toMatch(/position:\s*absolute/);
    expect(body).toMatch(/pointer-events:\s*none/);
    const image = /border-image:\s*url\("data:image\/png;base64,[A-Za-z0-9+/=]+"\)\s+(\d+)\s*\/\s*(\d+)px\s*\/\s*(\d+)px\s+stretch/.exec(body);
    expect(image).not.toBeNull();
    const [slice, width, outset] = image!.slice(1).map(Number);
    expect(slice).toBe(SHADOW.slice);
    expect(width * SHADOW.scale).toBe(SHADOW.slice); // 2x image: the slice is drawn at half size
    expect(outset).toBe(width); // the shadow paints outside the page box ...
    expect(body).toMatch(/inset:\s*0\s*;/); // ... which is the pseudo-element's own box: no layout overflow
    expect(body).toMatch(new RegExp(`border:\\s*${width}px solid transparent`));
  });

  it("embeds the image scripts/gen-pdf-page-shadow.mjs generates, with the old shadow's offset", () => {
    const [body] = declarations(".pdf-page::before");
    const embedded = Buffer.from(/base64,([A-Za-z0-9+/=]+)/.exec(body)![1], "base64");
    const a = alphaChannel(embedded);
    const b = alphaChannel(shadowPng());
    expect(a.size).toBe(2 * SHADOW.slice + SHADOW.middle);
    expect(a.alpha).toEqual(b.alpha);
    // `0 2px`: the page is shadowed more below than above, and equally left and right.
    const mid = Math.floor(a.size / 2);
    const above = a.alpha[(SHADOW.slice - 1) * a.size + mid];
    const below = a.alpha[(SHADOW.slice + SHADOW.middle) * a.size + mid];
    expect(below).toBeGreaterThan(above);
    expect(a.alpha[mid * a.size + SHADOW.slice - 1]).toBe(a.alpha[mid * a.size + SHADOW.slice + SHADOW.middle]);
    // The page itself paints over the middle, so the shadow image must not darken the corners' outer edge.
    expect(a.alpha[0]).toBe(0);
  });
});
