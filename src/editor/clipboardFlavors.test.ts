import { describe, expect, it } from "vitest";
import { classifyClipboardFlavors } from "./clipboardFlavors";

const classify = (uriList = "", plainText = "", types = ["text/uri-list"], eventFileCount = 0) =>
  classifyClipboardFlavors({ types, uriList, plainText, eventFileCount });

describe("clipboard file flavors", () => {
  it("keeps ordinary plain text authoritative for web URI lists", () => {
    expect(classify("https://tine.page", "Tine link")).toEqual({ hasFiles: false, text: "Tine link", canFallbackToText: true });
  });

  it("uses web URIs as text when plain text is absent, ignoring comments and blank CRLF lines", () => {
    expect(classify("# browser\r\nhttps://tine.page\r\n\r\nhttps://example.org\r\n")).toEqual({
      hasFiles: false, text: "https://tine.page\nhttps://example.org", canFallbackToText: true,
    });
  });

  it.each(["file:///tmp/a.pdf", "FILE:///Users/me/a.pdf", "content://media/image/1", "/tmp/a.pdf", "C:\\Users\\me\\a.pdf", "\\\\server\\share\\a.pdf", "smb://server/share/a.pdf", "relative/a.pdf", "https://tine.page\r\nfile:///tmp/a.pdf"])("claims file or ambiguous URI entries: %s", (uri) => {
    expect(classify(uri, "https://tine.page")).toMatchObject({ hasFiles: true, canFallbackToText: false });
  });

  it.each(["/Users/me/a.pdf", "C:\\Users\\me\\a.pdf", '"C:\\Users\\me\\a.pdf"', "\\\\server\\share\\a.pdf", "~/a.pdf", "file:///tmp/a.pdf"])("never recovers accompanying local path text: %s", (text) => {
    expect(classify("https://tine.page", text, ["Files", "text/uri-list"])).toMatchObject({ hasFiles: true, canFallbackToText: false });
  });

  it("keeps GNOME copied files and browser File objects authoritative", () => {
    expect(classify("https://tine.page", "https://tine.page", ["x-special/gnome-copied-files"])).toMatchObject({ hasFiles: true, canFallbackToText: false });
    expect(classify("https://tine.page", "https://tine.page", [], 1)).toMatchObject({ hasFiles: true, canFallbackToText: false });
  });

  it("allows a Files sentinel to recover a web URL but not a possible file name", () => {
    expect(classify("", "https://tine.page", ["Files"])).toMatchObject({ hasFiles: true, canFallbackToText: true });
    expect(classify("", "report.pdf", ["Files"])).toMatchObject({ hasFiles: true, canFallbackToText: false });
  });

  it("does not invent text for an empty or comments-only URI list", () => {
    expect(classify("# nothing\r\n")).toEqual({ hasFiles: false, text: "", canFallbackToText: false });
  });
});
