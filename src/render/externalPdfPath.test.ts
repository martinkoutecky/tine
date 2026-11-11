import { expect, it } from "vitest";
import { externalPdfPath, relativePdfFileUrl } from "./externalPdfPath";
import { fileLinkTarget } from "./relativeFileLink";

it.each([
  ["file:///tmp/paper.pdf", "/tmp/paper.pdf"],
  ["file:///home/a/文%20件.PDF", "/home/a/文 件.PDF"],
  ["file:///C:/My%20papers/文.pdf", "C:/My papers/文.pdf"],
  ["file:///C:\\My%20papers\\paper.pdf", "C:/My papers/paper.pdf"],
  ["file://server/share/paper.pdf", "//server/share/paper.pdf"],
  ["file:///tmp/a%2520b%23c%3Fd.pdf", "/tmp/a%20b#c?d.pdf"],
  ["file:///tmp/paper%2Epdf", "/tmp/paper.pdf"],
])("parses an absolute PDF URL without reading: %s", (url, path) => {
  expect(externalPdfPath(url)).toEqual({ kind: "pdf", path });
});

it.each(["file:paper.pdf", "file:../paper.pdf", "file:/tmp/paper.pdf", "../paper.pdf",
  "https://example.org/paper.pdf", "file:///tmp/paper.txt", "file:///tmp/a%00.pdf",
  "file:///tmp/paper.pdf#section", "file:///tmp/paper.pdf?query", "file:///tmp/a%zz.pdf"])(
  "leaves unsupported/ambiguous links with their existing opener: %s", (url) => {
    expect(externalPdfPath(url)).toEqual({ kind: "other" });
  });

// GH #732 follow-up: a relative file link resolves with the one graph-root
// resolver (relativeFileLink.ts), then reads as the absolute file: URL it names.
const rel = (dest: string, root = "/home/user/graph") => {
  const target = fileLinkTarget(dest); // null: a scheme URL or page name, never a file link
  return target && relativePdfFileUrl(dest, target, root);
};

it.each([
  ["file:../papers/a%20b.pdf", "file:///home/user/papers/a%20b.pdf"],
  ["../papers/a b.pdf", "file:///home/user/papers/a%20b.pdf"],
  ["..\\papers\\a%20b.PDF", "file:///home/user/papers/a%20b.PDF"],
  ["../../../../../x.pdf", "file:///x.pdf"],
  ["./docs/a.pdf", "file:///home/user/graph/docs/a.pdf"],
  ["docs/a%2520b%23c.pdf", "file:///home/user/graph/docs/a%2520b%23c.pdf"],
  ["file:docs/文.pdf", "file:///home/user/graph/docs/%E6%96%87.pdf"],
  ["file:a.pdf", "file:///home/user/graph/a.pdf"],
])("resolves the relative PDF link %s from the graph root", (dest, url) => {
  expect(rel(dest)).toBe(url);
  // The reader parses its own answer back to the same path, whatever the spelling.
  expect(externalPdfPath(url!).kind).toBe("pdf");
});

it.each([
  ["../文.pdf", "C:\\Users\\me\\graph", "file:///C:/Users/me/%E6%96%87.pdf"],
  ["file:../a.pdf", "C:/Users/me/graph", "file:///C:/Users/me/a.pdf"],
  ["../a.pdf", "//server/share/graph", "file://server/share/a.pdf"],
])("resolves %s against the Windows root %s", (dest, root, url) => {
  expect(rel(dest, root)).toBe(url);
  expect(externalPdfPath(url!).kind).toBe("pdf");
});

it.each(["paper.pdf", "/tmp/paper.pdf", "file:assets/a.pdf", "file:docs/../Assets/a.pdf", "file:../graph/assets/sub/a.pdf", "C:\\papers\\paper.pdf", "../notes.txt", "file:../notes.pdf.txt",
  "../paper.pdf#page=2", "../paper.pdf?x=1", "https://example.org/paper.pdf", "file:///tmp/paper.pdf", "Some Page.pdf"])(
  "leaves %s with its existing opener", (dest) => {
    expect(rel(dest)).toBeNull();
  });

it("needs a graph root to make a relative link absolute", () => {
  expect(rel("../a.pdf", "")).toBeNull();
});
