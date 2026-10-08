import { describe, it, expect, afterEach } from "vitest";
import { setDoc } from "../document/model";
import { annotationInfoForBlock, pdfFileForPage, pdfFileFromPreBlock, isAnnotationBlock } from "./annotation";

// pdfFileForPage reduces an hls__ page's `file-path::` to a basename. A graph
// edited on Windows can carry backslash paths, so the split must handle BOTH
// separators (gh #61) — otherwise the whole path is handed to the asset reader.
describe("pdfFileForPage", () => {
  afterEach(() => setDoc("pages", []));
  const seed = (preBlock: string) =>
    setDoc("pages", [{ name: "hls__book", preBlock, roots: [], format: preBlock.startsWith("#+") ? "org" : "md" } as any]);

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

  // UI-OG-C5-P6-PDFLINK: the hls page's file-path keeps the directory under assets/,
  // so reopening a nested PDF's highlight opens the nested PDF, not the root one.
  it("keeps the directory under assets/ so a nested PDF reopens as itself", () => {
    seed("file-path:: ../assets/nested/report.pdf");
    expect(pdfFileForPage("hls__book")).toBe("nested/report.pdf");
    seed("file-path:: ..\\assets\\nested\\report.pdf");
    expect(pdfFileForPage("hls__book")).toBe("nested/report.pdf");
    expect(pdfFileFromPreBlock("file-path:: ../assets/nested/report.pdf")).toBe("nested/report.pdf");
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

// OG-DUPD1 D08: an empty parsed result is authoritative.
describe("parser-owned annotation classification", () => {
  it("does not promote literal or rejected metadata", () => {
    for (const raw of ["```\nls-type:: annotation\nhl-page:: 42\n```", "ls-type::annotation"]) {
      expect(annotationInfoForBlock({ raw, properties: [] })).toBeNull();
      expect(annotationInfoForBlock({ raw })).toBeNull();
    }
  });
  it("does not find a PDF path inside a literal preamble", () => {
    expect(pdfFileFromPreBlock("```\nfile-path:: ../assets/hidden.pdf\n```")).toBeNull();
  });
});

it("recognizes accepted Org annotation drawers and excludes source directives", () => {
  const raw = "highlight\n:PROPERTIES:\n:ls-type: annotation\n:hl-page: 42\n:END:";
  expect(isAnnotationBlock(raw, "org")).toBe(true);
  expect(annotationInfoForBlock({ raw }, "org")).toEqual({ color: "yellow", hlPage: 42 });
  expect(pdfFileFromPreBlock("#+BEGIN_SRC\n#+FILE-PATH: hidden.pdf\n#+END_SRC", "org")).toBeNull();
});
