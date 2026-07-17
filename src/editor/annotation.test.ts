import { describe, it, expect, afterEach } from "vitest";
import { setDoc } from "../store";
import { annotationInfoForBlock, pdfFileForPage, pdfFileFromPreBlock } from "./annotation";

// pdfFileForPage reduces an hls__ page's `file-path::` to a basename. A graph
// edited on Windows can carry backslash paths, so the split must handle BOTH
// separators (gh #61) — otherwise the whole path is handed to the asset reader.
describe("pdfFileForPage", () => {
  afterEach(() => setDoc("pages", []));
  const seed = (preBlock: string) =>
    setDoc("pages", [{ name: "hls__book", preBlock, roots: [], format: "markdown" } as any]);

  it("basenames a forward-slash relative path", () => {
    seed("file-path:: ../assets/book_123.pdf");
    expect(pdfFileForPage("hls__book")).toBe("book_123.pdf");
  });

  it("reads the OG Org #+FILE-PATH pre-block form", () => {
    seed("#+FILE: [[../assets/A Book.pdf][A Book]]\n#+FILE-PATH: ../assets/A Book.pdf");
    expect(pdfFileForPage("hls__book")).toBe("A Book.pdf");
  });

  it("basenames a Windows backslash relative path", () => {
    seed("file-path:: ..\\assets\\book_123.pdf");
    expect(pdfFileForPage("hls__book")).toBe("book_123.pdf");
  });

  it("basenames an absolute Windows path", () => {
    seed("file-path:: C:\\Users\\me\\graph\\assets\\book_123.pdf");
    expect(pdfFileForPage("hls__book")).toBe("book_123.pdf");
  });

  it("keeps spaces while reading the complete file-path property", () => {
    seed("file-path:: C:\\Users\\me\\graph\\assets\\A Book With Spaces.pdf");
    expect(pdfFileForPage("hls__book")).toBe("A Book With Spaces.pdf");
  });

  it("returns null when the page has no file-path", () => {
    seed("some:: other\n");
    expect(pdfFileForPage("hls__book")).toBeNull();
  });
});

describe("annotation block metadata", () => {
  it("uses parsed properties when present", () => {
    expect(annotationInfoForBlock({
      raw: "highlight",
      properties: [["ls-type", "annotation"], ["hl-page", "42"], ["hl-color", "red"]],
    })).toEqual({ color: "red", hlPage: 42 });
  });

  it("falls back to raw properties for older DTOs", () => {
    expect(annotationInfoForBlock({
      raw: "highlight\nhl-page:: 7\nhl-color:: blue\nls-type:: annotation",
    })).toEqual({ color: "blue", hlPage: 7 });
  });

  it("rejects ordinary blocks and malformed empty file paths", () => {
    expect(annotationInfoForBlock({ raw: "ordinary block" })).toBeNull();
    expect(pdfFileFromPreBlock("file-path::   ")).toBeNull();
  });
});
