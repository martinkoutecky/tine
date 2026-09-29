import { describe, expect, it } from "vitest";
import { pageListLabels } from "./pages";
import type { PageEntry } from "./types";

const page = (name: string, path: string): PageEntry => ({
  name,
  kind: "page",
  date_key: null,
  path,
});

describe("pageListLabels", () => {
  it("leaves unique page names unchanged", () => {
    const pages = [page("foo", "pages/client-a/foo.md"), page("bar", "pages/client-b/bar.md")];
    expect(pageListLabels(pages)(pages[0])).toBe("foo");
  });

  it("adds the parent sub-path for colliding display names", () => {
    const pages = [page("foo", "pages/client-a/foo.md"), page("foo", "pages/client-b/foo.md")];
    expect(pageListLabels(pages)(pages[0])).toBe("foo — client-a/");
    expect(pageListLabels(pages)(pages[1])).toBe("foo — client-b/");
  });

  it("falls back to the full path when the parent sub-path is still ambiguous", () => {
    const pages = [page("foo", "pages/client-a/foo.md"), page("foo", "pages/client-a/foo.org")];
    expect(pageListLabels(pages)(pages[0])).toBe("foo — pages/client-a/foo.md");
    expect(pageListLabels(pages)(pages[1])).toBe("foo — pages/client-a/foo.org");
  });
});

// The label rule is unchanged; what changed is that deciding it no longer scans
// the whole list per row. A correctness test cannot see that, so pin the shape:
// one index build plus N lookups stays linear, not quadratic.
describe("pageListLabels scales with the list, not with list x rows", () => {
  const list = (n: number) => Array.from({ length: n }, (_, i) => page(`p${i}`, `pages/p${i}.md`));

  function costOfLabellingEveryRow(n: number): number {
    const pages = list(n);
    const started = performance.now();
    const label = pageListLabels(pages);
    for (const p of pages) label(p);
    return performance.now() - started;
  }

  it("stays roughly linear when the list grows tenfold", () => {
    costOfLabellingEveryRow(2_000); // warm the JIT
    const small = costOfLabellingEveryRow(1_000);
    const large = costOfLabellingEveryRow(10_000);
    // Quadratic would be ~100x; a loose ceiling because this is a shape check.
    expect(large).toBeLessThan(Math.max(small, 0.5) * 25);
  });

  it("still disambiguates correctly at scale", () => {
    const pages = [...list(5_000), page("p0", "pages/other/p0.md")];
    const label = pageListLabels(pages);
    expect(label(pages[0])).toBe("p0 — pages/");
    expect(label(pages[pages.length - 1])).toBe("p0 — other/");
    expect(label(pages[1])).toBe("p1");
  });

  it("does not confuse a name containing a space with a name plus a parent path", () => {
    const pages = [page("a b", "pages/c/a b.md"), page("a", "pages/b c/a.md"), page("a b", "pages/x/a b.md")];
    const label = pageListLabels(pages);
    expect(label(pages[1])).toBe("a");
  });
});
