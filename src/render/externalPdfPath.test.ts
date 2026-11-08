import { expect, it } from "vitest";
import { externalPdfPath } from "./externalPdfPath";

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
