import { describe, it, expect, vi } from "vitest";
import {
  mediaKind,
  assetMarkdown,
  assetFileName,
  formatAssetName,
  insertedAssetMarkdownTarget,
  replaceInsertedAssetMarkdown,
} from "./media";
import { DEFAULT_ASSET_NAME_FORMAT, STAMPED_ASSET_NAME_FORMAT } from "./assetSettings";

describe("media helpers", () => {
  it("mediaKind by extension (case-insensitive, query-tolerant)", () => {
    expect(mediaKind("a.png")).toBe("image");
    expect(mediaKind("../assets/clip.MP4")).toBe("video");
    expect(mediaKind("voice.m4a")).toBe("audio");
    expect(mediaKind("doc.pdf")).toBe(null);
    expect(mediaKind("../assets/x.webm?v=1")).toBe("video");
    expect(mediaKind("plain")).toBe(null);
  });

  it("assetMarkdown: ![] for image/video/audio, [] link for other", () => {
    expect(assetMarkdown("clip.mp4")).toBe("![](../assets/clip.mp4)");
    expect(assetMarkdown("song.mp3")).toBe("![](../assets/song.mp3)");
    expect(assetMarkdown("pic.png")).toBe("![](../assets/pic.png)");
    expect(assetMarkdown("paper.pdf")).toBe("[paper.pdf](../assets/paper.pdf)");
  });

  it("assetFileName: default = plain (sanitized) original name; paste → stamp.png", () => {
    // Default template is %assetname.%ext (the bare original name), ext case kept.
    expect(assetFileName("My Holiday Clip.MP4")).toBe("My_Holiday_Clip.MP4");
    expect(assetFileName("a/b%c.png")).toBe("a_b_c.png");
    expect(assetFileName()).toMatch(/^\d{8}-\d{6}-\d{3}-\d+\.png$/); // paste has no name → unique stamp
    expect(assetFileName("noext")).toBe("noext");
  });

  it("assetFileName: clipboard paste names are unique within the same second", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date(2030, 0, 2, 3, 4, 5, 6));
      const first = assetFileName();
      const second = assetFileName();
      expect(first).toMatch(/^20300102-030405-006-\d+\.png$/);
      expect(second).toMatch(/^20300102-030405-006-\d+\.png$/);
      expect(second).not.toBe(first);
    } finally {
      vi.useRealTimers();
    }
  });

  const D = new Date(2030, 0, 2, 3, 4, 5); // 2030-01-02 03:04:05 → every token distinct

  it("formatAssetName: default template = sanitized original name", () => {
    expect(formatAssetName(DEFAULT_ASSET_NAME_FORMAT, "Holiday Photo.JPG", D)).toBe("Holiday_Photo.JPG");
    // A paste (no original) → the %assetname falls back to a sortable stamp.
    expect(formatAssetName(DEFAULT_ASSET_NAME_FORMAT, undefined, D)).toBe("20300102-030405.png");
  });

  it("formatAssetName: stamped preset prefixes date+time", () => {
    expect(formatAssetName(STAMPED_ASSET_NAME_FORMAT, "Holiday Photo.JPG", D)).toBe(
      "20300102-030405-Holiday_Photo.JPG"
    );
  });

  it("formatAssetName: granular + combined tokens substitute", () => {
    expect(formatAssetName("%yyyy-%MM-%dd_%HH%mm%ss-%assetname.%ext", "cat.png", D)).toBe(
      "2030-01-02_030405-cat.png"
    );
    expect(formatAssetName("%yyyymmdd.%ext", "cat.png", D)).toBe("20300102.png");
    expect(formatAssetName("%yy", "cat.png", D)).toBe("30.png"); // omitted %ext → ext kept
  });

  it("formatAssetName: never drops the extension, sanitizes separators, no hidden/traversal", () => {
    // Template without %ext still keeps the real extension (else media won't render).
    expect(formatAssetName("%assetname", "movie.mp4", D)).toBe("movie.mp4");
    // No extension on the source → no extension forced.
    expect(formatAssetName("%assetname.%ext", "README", D)).toBe("README");
    // A path-traversal-ish source name can't yield separators, `..`, or a hidden
    // leading dot — it stays a single safe filename ending in the real extension.
    const danger = formatAssetName("%assetname.%ext", "../../etc/passwd.png", D);
    expect(danger).not.toMatch(/[/\\]/);
    expect(danger).not.toContain("..");
    expect(danger.startsWith(".")).toBe(false);
    expect(danger.endsWith(".png")).toBe(true);
  });

  it("replaceInsertedAssetMarkdown: repoints the tracked duplicate occurrence", () => {
    const candidate = "20300102-030405.png";
    const stored = "20300102-030405_1.png";
    const md = assetMarkdown(candidate);
    const raw = `${md} ${md}`;
    const secondOffset = md.length + 1;
    const target = insertedAssetMarkdownTarget(raw, md, secondOffset);

    expect(replaceInsertedAssetMarkdown(raw, candidate, stored, target)).toBe(
      `${md} ${assetMarkdown(stored)}`
    );
  });
});
