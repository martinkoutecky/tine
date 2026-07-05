import { describe, expect, it } from "vitest";
import { pageListLabel } from "./pages";
import type { PageEntry } from "./types";

const page = (name: string, path: string): PageEntry => ({
  name,
  kind: "page",
  date_key: null,
  path,
});

describe("pageListLabel", () => {
  it("leaves unique page names unchanged", () => {
    const pages = [page("foo", "pages/client-a/foo.md"), page("bar", "pages/client-b/bar.md")];
    expect(pageListLabel(pages[0], pages)).toBe("foo");
  });

  it("adds the parent sub-path for colliding display names", () => {
    const pages = [page("foo", "pages/client-a/foo.md"), page("foo", "pages/client-b/foo.md")];
    expect(pageListLabel(pages[0], pages)).toBe("foo — client-a/");
    expect(pageListLabel(pages[1], pages)).toBe("foo — client-b/");
  });

  it("falls back to the full path when the parent sub-path is still ambiguous", () => {
    const pages = [page("foo", "pages/client-a/foo.md"), page("foo", "pages/client-a/foo.org")];
    expect(pageListLabel(pages[0], pages)).toBe("foo — pages/client-a/foo.md");
    expect(pageListLabel(pages[1], pages)).toBe("foo — pages/client-a/foo.org");
  });
});
