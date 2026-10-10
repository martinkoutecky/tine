// S2 (ADR 0073): the block a shared item becomes, transcribed from OG
// logseq/og 6e7afa8eb `frontend/mobile/intent.cljs` (`transform-args`,
// `extract-highlight`, `embed-asset-file`, `handle-payload`) and
// `frontend/quick_capture.cljs` (`quick-capture`).
import { describe, expect, it } from "vitest";
import { asOutlineBlock, captureTime, extractHighlight, isLink, OG_TEXT_TEMPLATE, shapeShare, transformArgs, type ShapeContext } from "./shareShape";

const md: ShapeContext = { time: "09:05", date: "Oct 10th, 2026", format: "md" };
const prefix = "**09:05** [[quick capture]]:";

describe("OG intent.cljs text helpers", () => {
  it("is-link matches a whole scheme URL only", () => {
    expect(isLink("https://example.com/a")).toBe(true);
    expect(isLink("look https://example.com")).toBe(false);
    expect(isLink("")).toBe(false);
  });
  it("extract-highlight splits a browser's highlighted text from its trailing URL", () => {
    expect(extractHighlight("\"A quote\" https://example.com/a")).toEqual(["A quote", "https://example.com/a"]);
    expect(extractHighlight("https://example.com/a")).toEqual([null, "https://example.com/a"]);
    expect(extractHighlight("just words")).toEqual(["just words", null]);
  });
  it("transform-args keeps a link and moves highlighted text into content", () => {
    expect(transformArgs({ url: "https://x.org", title: "X" })).toEqual({ url: "https://x.org", title: "X" });
    expect(transformArgs({ url: "note https://x.org", title: "X" })).toEqual({ url: "https://x.org", title: "X", content: "note" });
  });
});

describe("shapeShare per item type (Android: OG handle-result)", () => {
  it("plain text becomes the default template's text", () => {
    expect(shapeShare({ source: "android", text: "hello", assets: [] }, md)).toBe(`${prefix} hello`);
  });
  it("a titled link becomes a Markdown link, or an Org link on an Org journal", () => {
    expect(shapeShare({ source: "android", text: "https://x.org/p", title: "Page", assets: [] }, md)).toBe(`${prefix}  [Page](https://x.org/p)`);
    expect(shapeShare({ source: "android", text: "https://x.org/p", title: "Page", assets: [] }, { ...md, format: "org" })).toBe(`${prefix}  [[https://x.org/p][Page]]`);
  });
  it("Android's highlighted text plus link (EXTRA_TEXT) keeps both", () => {
    expect(shapeShare({ source: "android", text: "\"Quoted\" https://x.org/p", title: "Page", assets: [] }, md)).toBe(`${prefix} Quoted [Page](https://x.org/p)`);
  });
  it("an untitled link, or one whose title is the URL, stays a raw URL", () => {
    expect(shapeShare({ source: "android", text: "https://x.org/p", assets: [] }, md)).toBe(`${prefix}  https://x.org/p`);
    expect(shapeShare({ source: "android", text: "https://x.org/p", title: "https://x.org/p", assets: [] }, md)).toBe(`${prefix}  https://x.org/p`);
  });
  it("text shared with a page link (iOS handle-payload) puts the text before the link", () => {
    expect(shapeShare({ source: "ios", text: "worth reading", url: "https://x.org/p", assets: [] }, md)).toBe(`${prefix} worth reading https://x.org/p`);
  });
  it("video and tweet links become OG's embeds", () => {
    expect(shapeShare({ source: "android", text: "https://www.youtube.com/watch?v=dQw4w9WgXcQ", title: "Song", assets: [] }, md))
      .toBe(`${prefix}  Song {{video https://www.youtube.com/watch?v=dQw4w9WgXcQ}}`);
    expect(shapeShare({ source: "android", text: "https://x.com/someone/status/123", assets: [] }, md)).toBe(`${prefix}  {{twitter https://x.com/someone/status/123}}`);
  });
  it("a lone image uses the media template", () => {
    expect(shapeShare({ source: "android", assets: ["![](../assets/photo.png)"] }, md)).toBe(`${prefix} ![](../assets/photo.png)`);
    expect(shapeShare({ source: "android", assets: ["![](../assets/p.png)"] }, { ...md, mediaTemplate: "{date}: {url}" })).toBe("Oct 10th, 2026: ![](../assets/p.png)");
  });
  it("text with images, or several images, joins the rich parts one per line (handle-payload)", () => {
    expect(shapeShare({ source: "android", text: "two", assets: ["![](../assets/a.png)", "![](../assets/b.png)"] }, md))
      .toBe(`${prefix} two ![](../assets/a.png)\n![](../assets/b.png)`);
    expect(shapeShare({ source: "ios", url: "https://x.org", assets: ["![](../assets/a.png)"] }, md)).toBe(`${prefix}  https://x.org\n![](../assets/a.png)`);
  });
  it("a custom text template replaces every placeholder occurrence literally", () => {
    const context = { ...md, textTemplate: "{date} — {text} ({text}) {url}" };
    expect(shapeShare({ source: "android", text: "a $& b", assets: [] }, context)).toBe("Oct 10th, 2026 — a $& b (a $& b)");
  });
  it("the default template is OG's", () => {
    expect(OG_TEXT_TEMPLATE).toBe("**{time}** [[quick capture]]: {text} {url}");
  });
  it("an empty item yields nothing", () => {
    expect(shapeShare({ source: "android", text: "  ", assets: [] }, md)).toBeNull();
  });
});

describe("iOS share sheet (OG handle-payload)", () => {
  // Review round 1, finding 8: iOS items went through Android's transform-args.
  it("a URL shared as text survives OG's literal {text} template", () => {
    expect(shapeShare({ source: "ios", text: "https://example.com", assets: [] }, { ...md, textTemplate: "{text}" })).toBe("https://example.com");
  });
  it("a lone image uses the :text template, never :media", () => {
    expect(shapeShare({ source: "ios", assets: ["![](../assets/photo.png)"] }, { ...md, textTemplate: "inbox {url}", mediaTemplate: "camera {url}" }))
      .toBe("inbox ![](../assets/photo.png)");
  });
  it("a web link stays the raw URL: no titled link and no video or tweet embed", () => {
    expect(shapeShare({ source: "ios", url: "https://x.org/p", title: "Page", assets: [] }, md)).toBe(`${prefix}  https://x.org/p`);
    expect(shapeShare({ source: "ios", url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ", assets: [] }, md))
      .toBe(`${prefix}  https://www.youtube.com/watch?v=dQw4w9WgXcQ`);
  });
  it("text that looks like a highlight plus link is kept verbatim", () => {
    expect(shapeShare({ source: "ios", text: "\"Quoted\" https://x.org/p", assets: [] }, md)).toBe(`${prefix} "Quoted" https://x.org/p`);
  });
});

describe("helpers", () => {
  it("time is the locale's two-digit 24-hour clock (date/get-current-time)", () => {
    expect(captureTime(new Date(2026, 9, 10, 21, 7), "en-US")).toBe("21:07");
    expect(captureTime(new Date(2026, 9, 10, 9, 5), "en-US")).toBe("09:05");
  });
  it("multi-line content stays one block", () => {
    expect(asOutlineBlock("a\nb")).toBe("- a\n  b");
  });
});
