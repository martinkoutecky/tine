import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const root = path.resolve(import.meta.dirname, "..");
const app = fs.readFileSync(path.join(root, "src/styles/app.css"), "utf8");

describe("application overscroll containment (GH #177)", () => {
  it("terminates every descendant scroll chain at both the viewport and app shell", () => {
    expect(app).toMatch(/html\s*,\s*body\s*\{[^}]*overscroll-behavior:\s*none/s);
    expect(app).toMatch(/\.app-container\s*\{[^}]*overscroll-behavior:\s*none/s);
  });

  it("does not blanket-disable scrolling or touch gestures on nested scroll regions", () => {
    expect(app).not.toMatch(/\*\s*\{[^}]*overscroll-behavior/s);
    expect(app).not.toMatch(/\.(?:main-content|left-sidebar-scroll|right-sidebar|pdf-scroll|sheet-scroll)\s*\{[^}]*touch-action:\s*none/s);
  });
});
