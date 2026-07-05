import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const lsShimCss = readFileSync(
  fileURLToPath(new URL("./styles/ls-shim.css", import.meta.url)),
  "utf8"
);

const mappings: Array<[string, string]> = [
  ["--bg-primary", "--ls-primary-background-color"],
  ["--bg-secondary", "--ls-secondary-background-color"],
  ["--bg-tertiary", "--ls-tertiary-background-color"],
  ["--bg-quaternary", "--ls-quaternary-background-color"],
  ["--text-primary", "--ls-page-inline-code-color"],
  ["--text-secondary", "--ls-secondary-text-color"],
  ["--text-title", "--ls-title-text-color"],
  ["--link-color", "--ls-block-ref-link-text-color"],
  ["--link-hover", "--ls-link-text-hover-color"],
  ["--accent", "--ls-block-bullet-active-color"],
  ["--block-select-bg", "--ls-a-chosen-bg"],
  ["--border-color", "--ls-secondary-border-color"],
  ["--guide-color", "--ls-guideline-color"],
  ["--block-highlight", "--ls-block-highlight-color"],
  ["--bullet-color", "--ls-block-bullet-color"],
  ["--selection-bg", "--ls-selection-background-color"],
  ["--code-bg", "--ls-page-inline-code-bg-color"],
  ["--mark-bg", "--ls-page-mark-bg-color"],
  ["--mark-text", "--ls-page-mark-color"],
  ["--tag-color", "--ls-tag-text-color"],
];

describe("OG --ls-* shim", () => {
  it("routes Tine tokens through OG variables", () => {
    for (const [tineToken, ogToken] of mappings) {
      expect(lsShimCss).toContain(`${tineToken}: var(${ogToken},`);
    }
  });

  it("seeds OG variables without reading back from Tine tokens", () => {
    for (const [, ogToken] of mappings) {
      expect(lsShimCss).not.toMatch(new RegExp(`${ogToken}:\\s*var\\(--(bg|text|link|accent|block|border|guide|bullet|selection|code|mark|tag)`));
    }
  });

  it("keeps secondary OG aliases chained so overriding any alias reaches Tine", () => {
    expect(lsShimCss).toContain("--ls-link-ref-text-color: var(--ls-link-text-color)");
    expect(lsShimCss).toContain("--ls-block-ref-link-text-color: var(--ls-link-ref-text-color)");
    expect(lsShimCss).toContain("--ls-block-bullet-active-color: var(--ls-active-primary-color)");
    expect(lsShimCss).toContain("--ls-secondary-border-color: var(--ls-border-color)");
    expect(lsShimCss).toContain("--ls-page-inline-code-color: var(--ls-primary-text-color)");
  });
});
