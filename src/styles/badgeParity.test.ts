import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

// GH #843: a block's reference count and its margin comment count read as one pair of badges.
// The same-line half needs real layout: scripts/shot-gh843-badges.mjs.
const rule = (file: string, selector: string) => {
  const css = readFileSync(new URL(file, import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  const body = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].find(match => match[1].split(",").some(candidate => candidate.trim() === selector))?.[2];
  expect(body, `${file} defines ${selector}`).toBeDefined();
  return Object.fromEntries(body!.split(";").map((d) => d.split(":").map((p) => p.trim())).filter((d) => d.length === 2));
};

it("draws both count badges as the same pill", () => {
  const refs = rule("./app.css", ".block-refs-count");
  const marker = rule("./comments.css", ".margin-marker");
  for (const property of ["box-sizing", "min-width", "height", "padding", "border-radius", "font-size", "font-weight", "line-height"]) {
    expect([property, refs[property]]).toEqual([property, marker[property]]);
  }
});
