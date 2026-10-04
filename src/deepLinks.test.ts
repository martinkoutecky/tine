import { describe, expect, it } from "vitest";
import { parseTineLink, pageLink, graphLink, blockLink } from "./deepLinks";
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
});
