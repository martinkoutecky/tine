import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("onboards the video-paste preference, its default and the visible link-only outcome (GH #861)", () => {
  const tips = readFileSync("crates/tine-guide/src/templates/tips.md", "utf8");
  for (const detail of [
    "Settings → Editor → Auto-embed pasted video links", "on by default",
    "YouTube, Vimeo, Loom and Bilibili", "both editors, including Quick Capture",
    "never fetches video titles on paste", "keeps the URL instead of inserting a `{{video …}}` embed",
  ]) expect(tips).toContain(detail);
});
