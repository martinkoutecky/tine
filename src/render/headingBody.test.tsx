import { beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { facetsOf } from "./facets";

beforeAll(async () => { await initParser(); });

function renderBody(raw: string): { host: HTMLDivElement; dispose: () => void } {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const hl = facetsOf(raw, "md").headingLevel;
  const dispose = render(() => <AstBody raw={raw} headingLevel={hl} />, host);
  return { host, dispose };
}

describe("multiline block heading rendering", () => {
  it("styles a heading on a continuation line at its own ATX size", () => {
    const { host, dispose } = renderBody("## foo\nbar\n### baz");
    try {
      // blocks[0] heading (from the facet path)
      expect(host.querySelector(".heading-text.h2")).toBeTruthy();
      // continuation-line `### baz` must render at h3 (was plain text before the fix)
      const h3 = host.querySelector(".heading-text.h3");
      expect(h3).toBeTruthy();
      expect(h3!.textContent).toContain("baz");
    } finally { dispose(); }
  });

  it("styles every heading in a stack of headings", () => {
    const { host, dispose } = renderBody("## A\n## B\n## C");
    try {
      const hs = host.querySelectorAll(".heading-text.h2");
      expect(hs.length).toBe(3);
    } finally { dispose(); }
  });

  it("styles a heading following a plain first line", () => {
    const { host, dispose } = renderBody("line1\n## heading2");
    try {
      const h2 = host.querySelector(".heading-text.h2");
      expect(h2).toBeTruthy();
      expect(h2!.textContent).toContain("heading2");
    } finally { dispose(); }
  });

  it("does not double-wrap a heading-kind blocks[0] (single heading-text span)", () => {
    const { host, dispose } = renderBody("## solo");
    try {
      // Exactly one heading-text span, not nested.
      expect(host.querySelectorAll(".heading-text").length).toBe(1);
      expect(host.querySelector(".heading-text .heading-text")).toBeFalsy();
    } finally { dispose(); }
  });
});
