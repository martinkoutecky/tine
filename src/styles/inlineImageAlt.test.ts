// GH #836: a markdown image whose file does not load shows the browser's broken-image icon with its
// alt text drawn inside the <img> box. That text inherits the wrapper's `line-height: 0` (which
// removes the descender gap under a loaded image) and was clipped to a sliver. The image itself must
// carry a real line-height; the wrapper keeps 0 so loaded images do not move.
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const css = fs.readFileSync(path.join(process.cwd(), "src/styles/app.css"), "utf8");

function ruleBodies(selector: string): string[] {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return [...css.matchAll(new RegExp(`^${escaped}\\s*\\{([^}]*)\\}`, "gm"))].map((match) => match[1]);
}

describe("GH #836 broken inline image alt text", () => {
  it("gives the image a nonzero line-height so its alt text is not clipped", () => {
    const values = ruleBodies(".inline-image").flatMap((body) => [...body.matchAll(/line-height:\s*([^;]+);/g)].map((m) => m[1].trim()));
    expect(values.length).toBeGreaterThan(0);
    expect(values.at(-1)).not.toMatch(/^0(\.0+)?(px|em|rem)?$/);
  });

  it("keeps the wrapper's line-height 0 so a loaded image has no descender gap", () => {
    expect(ruleBodies(".inline-image-wrap").join("\n")).toMatch(/line-height:\s*0;/);
  });
});
