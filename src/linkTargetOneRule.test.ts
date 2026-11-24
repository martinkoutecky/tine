// The scheme route has one rule, in core (crates/tine-core/src/link_scheme.rs);
// the TypeScript policy only names its answers. The media kind has one
// classifier, src/media.ts (wasm-free, so a stored asset's reference never
// waits on the parser); the link-target owner answers no media kind.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parseBlock } from "./render/parse";
import { localTarget } from "./localTarget";
import { mediaKind } from "./media";
import { anchorDecision, linkScheme, previewResourceDecision, rememberableLinkScheme } from "./linkSchemePolicy";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = (name: string) => readFileSync(path.join(root, name), "utf8");

describe("one media classifier", () => {
  it("is media.ts; core keeps no media inventory and exports none", () => {
    expect(source("src/media.ts")).toMatch(/VIDEO_EXTS = \[/);
    const owner = source("crates/tine-core/src/local_target.rs");
    for (const own of [/MEDIA_EXTS/, /fn media_kind/, /fn media_inventory/, /enum Media\b/]) expect(owner).not.toMatch(own);
    expect(source("crates/lsdoc-wasm/src/lib.rs")).not.toMatch(/media_inventory/);
  });

  it("the owner answers no media kind, so it cannot disagree with the renderer's", () => {
    const link = JSON.stringify(parseBlock("![x](https://example.test/clip.mKv)", false)).match(/"url":(\{[^}]*\})/);
    const answer = localTarget({ url: JSON.parse(link![1]) }, "md-embed");
    expect(answer).not.toHaveProperty("media");
    expect(answer.route.kind).toBe("web");
    expect(mediaKind("https://example.test/clip.mKv")).toBe("video");
  });
});

describe("one scheme-route rule", () => {
  it("linkSchemePolicy.ts parses no scheme and lists no route of its own", () => {
    const policy = source("src/linkSchemePolicy.ts");
    for (const own of [/["'](?:https?|mailto|data)["']/, /\\u0000/, /\.exec\(/, /\[a-z\]\):/]) {
      expect(policy).not.toMatch(own);
    }
  });

  it("names core's answers", () => {
    expect(anchorDecision(" \tHTTPS://x")).toBe("web");
    expect(anchorDecision("mailto:a@b")).toBe("mail");
    expect(anchorDecision("C:/x")).toBe("drive");
    expect(anchorDecision("tel:1")).toBe("defer");
    expect(anchorDecision("java\nscript:alert(1)")).toBe("drop");
    expect(anchorDecision("j ava script:x")).toBe("drop");
    expect(anchorDecision(" file:/x")).toBe("file");
    expect(anchorDecision("docs/a.png")).toBe("relative");
    expect(linkScheme("C:/x")).toBeNull();
    expect(linkScheme(" TEL:1")).toBe("tel");
    expect(previewResourceDecision("data:image/png;base64,x")).toBe("browser");
    expect(previewResourceDecision("C:/x.png")).toBe("local-candidate");
    expect(previewResourceDecision("tel:1")).toBe("drop");
    expect(["zotero", "http", "mailto", "file", "tine", "javascript"].map(rememberableLinkScheme))
      .toEqual([true, false, false, false, false, false]);
  });

  it("reads only the bounded prefix: long inputs keep their answer", () => {
    const query = "q=" + "a".repeat(20_000);
    expect(anchorDecision(`https://example.test/x?${query}`)).toBe("web");
    expect(previewResourceDecision(`data:image/png;base64,${"A".repeat(20_000)}`)).toBe("browser");
    expect(anchorDecision(`docs/a b.pdf?${query}`)).toBe("relative");
    expect(linkScheme(`zotero://select/${query}`)).toBe("zotero");
    // Undecided within the bound (a scheme hidden past it): refused, never opened.
    expect(anchorDecision(`${" ".repeat(600)}javascript:alert(1)`)).toBe("drop");
    expect(previewResourceDecision(`${" ".repeat(600)}javascript:alert(1)`)).toBe("drop");
  });
});
