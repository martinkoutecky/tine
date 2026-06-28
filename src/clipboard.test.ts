import { describe, it, expect } from "vitest";
import { outlineToHtml } from "./clipboard";

describe("outlineToHtml (text/html clipboard flavor)", () => {
  it("builds nested <ul><li> from a tab-indented outline", () => {
    const md = "- a\n\t- b\n\t- c\n- d";
    expect(outlineToHtml(md)).toBe(
      "<ul><li>a<ul><li>b</li><li>c</li></ul></li><li>d</li></ul>"
    );
  });

  it("escapes HTML and folds continuation lines into the same <li>", () => {
    const md = "- a <x> & b\n  more";
    expect(outlineToHtml(md)).toBe("<ul><li>a &lt;x&gt; &amp; b<br>more</li></ul>");
  });

  it("empty input → empty string", () => {
    expect(outlineToHtml("")).toBe("");
  });
});
