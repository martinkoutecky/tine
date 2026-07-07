import { describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { AboutTab } from "./AboutTab";

// Guards the deliberate role-credit phrasing (GH #32 discussion): Martin is the
// author/director, Claude Code & Codex are collaborators — NOT "created by …"
// (erases him) nor "created with …" (reduces them to tools). If someone rewrites
// this line, this test makes them do it on purpose.
describe("AboutTab", () => {
  it("renders the role-based credits and project links", () => {
    const host = document.createElement("div");
    const dispose = render(() => <AboutTab />, host);
    try {
      const text = host.textContent ?? "";
      expect(text).toContain("Martin Koutecký");
      expect(text).toContain("direction, design, and authorship");
      expect(text).toContain("Claude Code & Codex");
      expect(text).toContain("engineering and analysis");
      // The three primary links #32 asked for + the phrasing must stay neutral.
      expect(text).toContain("tine.page");
      expect(text).toContain("GitHub");
      expect(text).toContain("Ko-fi");
      expect(text).not.toMatch(/created (by|with)/i);
    } finally {
      dispose();
    }
  });
});
