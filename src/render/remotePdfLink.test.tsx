import { beforeAll, afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";

beforeAll(async () => { await initParser(); });
afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });

function openRenderedLink(source: string, format: "md" | "org" = "md") {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <InlineText text={source} format={format} />, host);
  return { host, dispose };
}

describe("PDF link destination", () => {
  for (const [source, url, format] of [
    ["[paper](https://example.org/paper.pdf)", "https://example.org/paper.pdf", "md"],
    ["![figure](https://example.org/figure.pdf)", "https://example.org/figure.pdf", "md"],
    ["[[http://example.org/REPORT.PDF][report]]", "http://example.org/REPORT.PDF", "org"],
  ] as const) {
    it(`opens ${source} externally`, () => {
      const external = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
      const view = openRenderedLink(source, format);
      try {
        expect(view.host.querySelector(".pdf-link")).toBeNull();
        const link = view.host.querySelector("a.external-link") as HTMLAnchorElement;
        expect(link).not.toBeNull();
        link.click();
        expect(external).toHaveBeenCalledWith(url);
      } finally { view.dispose(); }
    });
  }

  it("keeps a graph asset PDF in the reader", () => {
    const view = openRenderedLink("[paper](../assets/paper.pdf)");
    try { expect(view.host.querySelector(".pdf-link")).not.toBeNull(); }
    finally { view.dispose(); }
  });
});
