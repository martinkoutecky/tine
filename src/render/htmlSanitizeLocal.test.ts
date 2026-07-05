import { describe, it, expect } from "vitest";
import { localImagePath, rawHtmlLocalImages } from "./htmlSanitize";

describe("localImagePath — classify a raw-HTML <img> src", () => {
  it("web / data / blob URLs are not local", () => {
    expect(localImagePath("https://x.com/a.png")).toBeNull();
    expect(localImagePath("http://x.com/a.png")).toBeNull();
    expect(localImagePath("data:image/png;base64,AAAA")).toBeNull();
    expect(localImagePath("blob:abcd")).toBeNull();
  });
  it("relative and protocol-relative paths are not local (nothing to resolve against)", () => {
    expect(localImagePath("img/rel.png")).toBeNull();
    expect(localImagePath("//host/share/a.png")).toBeNull();
  });
  it("absolute unix paths", () => {
    expect(localImagePath("/home/m/a.png")).toBe("/home/m/a.png");
  });
  it("file:// URLs are stripped to a filesystem path", () => {
    expect(localImagePath("file:///home/m/a.png")).toBe("/home/m/a.png");
  });
  it("percent-encoding is decoded", () => {
    expect(localImagePath("file:///home/m/a%20b.png")).toBe("/home/m/a b.png");
  });
  it("windows drive + UNC paths", () => {
    expect(localImagePath("C:\\pics\\a.png")).toBe("C:\\pics\\a.png");
    expect(localImagePath("file:///C:/pics/a.png")).toBe("C:/pics/a.png");
    expect(localImagePath("\\\\host\\share\\a.png")).toBe("\\\\host\\share\\a.png");
  });
});

describe("rawHtmlLocalImages — aligns 1:1 with <img> document order", () => {
  it("mixes external and local, preserving order and nulls", () => {
    const text = '<img src="https://x/a.png"/> mid <img src="/home/m/b.png"/> end <img src="data:image/png;base64,Z"/>';
    expect(rawHtmlLocalImages(text)).toEqual([null, "/home/m/b.png", null]);
  });
  it("no <img> → empty", () => {
    expect(rawHtmlLocalImages("<ins>x</ins> and <kbd>C</kbd>")).toEqual([]);
  });
  it("single-quoted and unquoted srcs", () => {
    expect(rawHtmlLocalImages("<img src='/a/b.png'>")).toEqual(["/a/b.png"]);
    expect(rawHtmlLocalImages("<img src=/a/b.png>")).toEqual(["/a/b.png"]);
  });
});
