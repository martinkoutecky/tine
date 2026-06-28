import { describe, it, expect } from "vitest";
import { mediaKind, assetMarkdown, assetFileName } from "./media";

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

  it("assetFileName: <yyyymmdd-hhmmss>-<stem>.<ext>, sanitized; paste → <stamp>.png", () => {
    expect(assetFileName("My Holiday Clip.MP4")).toMatch(/^\d{8}-\d{6}-My_Holiday_Clip\.MP4$/);
    expect(assetFileName("a/b%c.png")).toMatch(/^\d{8}-\d{6}-a_b_c\.png$/);
    expect(assetFileName()).toMatch(/^\d{8}-\d{6}\.png$/);
    expect(assetFileName("noext")).toMatch(/^\d{8}-\d{6}-noext$/);
  });
});
