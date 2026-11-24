// The scheme route and the media kind each have one rule, in core
// (crates/tine-core/src/link_scheme.rs, local_target.rs). The TypeScript modules
// that name their answers keep no list or classifier of their own.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { media_inventory } from "./render/wasm/lsdoc_wasm.js";
import { mediaInventory, mediaKind } from "./media";
import { anchorDecision, linkScheme, previewResourceDecision, rememberableLinkScheme } from "./linkSchemePolicy";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = (name: string) => readFileSync(path.join(root, "src", name), "utf8");
const coreInventory = () => media_inventory().split(",").map((pair) => pair.split(":") as [string, string]);

describe("one media inventory", () => {
  it("media.ts keeps no extension list of its own", () => {
    // A single default (clipboard png, a recording's container) names a new
    // file's extension, not a media kind; a list of two or more is an inventory.
    const exts = new Set(coreInventory().map(([ext]) => ext));
    const lists = source("media.ts").match(/\[[^[\]]*\]|\|[^;{]*\|/g) ?? [];
    const own = lists.filter((list) => (list.match(/["'`]([a-z0-9]+)["'`]/g) ?? [])
      .filter((quoted) => exts.has(quoted.slice(1, -1))).length >= 2);
    expect(own).toEqual([]);
  });

  it("is core's table, read once", () => {
    expect([...mediaInventory()]).toEqual(coreInventory());
    expect(mediaInventory()).toBe(mediaInventory());
    expect(mediaKind("../assets/clip.MP4?v=1")).toBe("video");
  });
});

describe("one scheme-route rule", () => {
  it("linkSchemePolicy.ts parses no scheme and lists no route of its own", () => {
    const policy = source("linkSchemePolicy.ts");
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
});
