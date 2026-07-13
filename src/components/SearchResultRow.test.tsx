import { describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { SearchResultRow, buildSearchExcerpt } from "./SearchResultRow";

describe("SearchResultRow (GH #98)", () => {
  it("keeps several distant matches visible within a fixed excerpt budget", () => {
    const text = `prefix ${"x".repeat(180)} alpha ${"y".repeat(220)} beta ${"z".repeat(500)}`;
    const alpha = text.indexOf("alpha");
    const beta = text.indexOf("beta");
    const segments = buildSearchExcerpt(text, [
      { start: alpha, end: alpha + 5 },
      { start: beta, end: beta + 4 },
    ]);
    expect(segments.filter((segment) => segment.marked).map((segment) => segment.text)).toEqual(["alpha", "beta"]);
    expect(segments.map((segment) => segment.text).join("").length).toBeLessThanOrEqual(216);
  });

  it("separates page context from the matched text and highlights every supplied span", () => {
    const root = document.createElement("div");
    const dispose = render(() => (
      <SearchResultRow
        page="Research"
        breadcrumb={["Parent", "Child"]}
        text="alpha and beta are both useful"
        spans={[{ start: 0, end: 5 }, { start: 10, end: 14 }]}
      />
    ), root);
    expect(root.querySelector(".search-result-context")?.textContent).toBe("Research › Parent › Child");
    expect([...root.querySelectorAll("mark")].map((mark) => mark.textContent)).toEqual(["alpha", "beta"]);
    expect(root.querySelector(".search-result-excerpt")?.textContent).not.toContain("Research");
    dispose();
  });
});
