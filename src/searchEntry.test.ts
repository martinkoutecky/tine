import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const root = path.resolve(import.meta.dirname, "..");
const app = fs.readFileSync(path.join(root, "src/App.tsx"), "utf8");
const sidebar = fs.readFileSync(path.join(root, "src/components/Sidebar.tsx"), "utf8");

describe("single visible search entry (GH #100)", () => {
  it("keeps one labelled Search in the shared desktop/mobile navigation group", () => {
    expect(app.match(/data-search-trigger/g)).toHaveLength(1);
    const start = app.indexOf("const NavigationActions =");
    const left = app.slice(start, app.indexOf("</div>", start));
    expect(left).toContain("data-search-trigger");
    expect(left).toContain('aria-label={t("navigation-search")}');
  });

  it("does not keep a duplicate readonly pseudo-input in the sidebar", () => {
    expect(sidebar).not.toContain('class="nav-search"');
    expect(sidebar).not.toContain('class="search-input"');
  });
});
