import { describe, expect, it } from "vitest";
import cases from "../tests/fixtures/deep-link-ids.json";
import { parseTineLink, parseAppRoute, pageLink, graphLink, blockLink } from "./deepLinks";
const id = "11111111-1111-4111-8111-111111111111";
describe("GH #181 external Tine addresses", () => {
  it("round-trips graph, Unicode and slash-containing page names, and page-free block ids", () => {
    expect(parseTineLink(graphLink(id))).toEqual({ graph: id });
    expect(parseTineLink(pageLink("日本 / A?#%", id))).toEqual({ graph: id, page: "日本 / A?#%" });
    expect(parseTineLink(pageLink("..", id))).toEqual({ graph: id, page: ".." });
    expect(parseTineLink(blockLink(id))).toEqual({ block: id });
  });
  it.each(["logseq://graph/A", "tine://capture/foo", "tine://page/A", "tine://graph/abc", "tine://block/abc", `tine://page/A?graph=${id}&capture=yes`, `tine://graph/${id}#x`, `tine://page/%ZZ?graph=${id}`])("refuses unsupported or malformed navigation: %s", (url) => {
    expect(() => parseTineLink(url)).toThrow();
  });
  it("keeps the authored spelling of a block id but folds a graph id (shared fixture)", () => {
    for (const text of cases.valid) {
      expect(parseTineLink(blockLink(text))).toEqual({ block: text });
      expect(parseTineLink(graphLink(text))).toEqual({ graph: text.toLowerCase() });
      expect(parseTineLink(`tine://block/${text}`)).toEqual({ block: text });
    }
    for (const text of cases.invalid) expect(() => blockLink(text)).toThrow();
  });
});
describe("current-graph app routes (ADR 0073)", () => {
  it("parses today, capture, search with an optional query, and a graph-less page", () => {
    expect(parseAppRoute("tine://today")).toEqual({ route: "today" });
    expect(parseAppRoute("tine://capture")).toEqual({ route: "capture" });
    expect(parseAppRoute("tine://search")).toEqual({ route: "search", query: "" });
    expect(parseAppRoute("tine://search?q=r%C3%A9sum%C3%A9+draft")).toEqual({ route: "search", query: "résumé draft" });
    expect(parseAppRoute("tine://page/%C3%9Cber%20Plan%2F2026")).toEqual({ route: "page", page: "Über Plan/2026" });
  });
  it("leaves known-graph addresses to parseTineLink", () => {
    expect(parseAppRoute(pageLink("A", id))).toBeNull();
    expect(parseAppRoute(graphLink(id))).toBeNull();
    expect(parseAppRoute(blockLink(id))).toBeNull();
  });
  it.each(["tine://capture/foo", "tine://today?x=1", "tine://search?q=a&x=b", "tine://search/x", "tine://page/", "tine://page/A#x", "tine://journal", "tine://today\n"])(
    "is not an app route: %s", (url) => {
      expect(parseAppRoute(url)).toBeNull();
    });
});
