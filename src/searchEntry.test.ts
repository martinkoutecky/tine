import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const root = path.resolve(import.meta.dirname, "..");
const app = fs.readFileSync(path.join(root, "src/App.tsx"), "utf8");
const sidebar = fs.readFileSync(path.join(root, "src/components/Sidebar.tsx"), "utf8");

describe("single visible search entry (GH #100)", () => {
  it("keeps one labelled trigger in the top-left navigation cluster", () => {
    expect(app.match(/data-search-trigger/g)).toHaveLength(1);
    const left = app.slice(app.indexOf('<div class="topbar-left">'), app.indexOf("</div>", app.indexOf('<div class="topbar-left">')));
    expect(left).toContain("data-search-trigger");
    expect(left).toContain('aria-label="Search"');
  });

  it("does not keep a duplicate readonly pseudo-input in the sidebar", () => {
    expect(sidebar).not.toContain('class="nav-search"');
    expect(sidebar).not.toContain('class="search-input"');
  });
});
