// A stored asset gets its note reference whether or not the parser is ready:
// Quick Capture claims a pasted file and imports it while the parser is still
// starting (or after it failed), and the reference is built only after the
// file is already in assets/. The media kind is therefore answered without
// wasm. Each test makes every parser export throw, as an uninitialized module.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const wasm = vi.hoisted(() => ({ down: false }));
vi.mock("./render/wasm/lsdoc_wasm.js", async (importOriginal) => {
  const real = await importOriginal<Record<string, unknown>>();
  const out: Record<string, unknown> = {};
  for (const [name, value] of Object.entries(real)) {
    out[name] = typeof value === "function" && name !== "default" && name !== "initSync"
      ? (...args: unknown[]) => {
        if (wasm.down) throw new Error("lsdoc-wasm: reinit before init");
        return (value as (...a: unknown[]) => unknown)(...args);
      }
      : value;
  }
  return out;
});

import { assetMarkdown, mediaKind } from "./media";

beforeEach(() => { wasm.down = true; });
afterEach(() => { wasm.down = false; });

describe("a stored asset's reference needs no parser", () => {
  it("builds every kind of reference", () => {
    expect(assetMarkdown("clip.png")).toBe("![](../assets/clip.png)");
    expect(assetMarkdown("clip.mkv", { pagePath: "journals/2026_11_24.md" })).toBe("![](../assets/clip.mkv)");
    expect(assetMarkdown("note.pdf", { label: "Note" })).toBe("![Note](../assets/note.pdf)");
    expect(assetMarkdown("a.txt", { format: "org" })).toBe("[[../assets/a.txt][a.txt]]");
    expect(assetMarkdown("a.png", { format: "org" })).toBe("[[../assets/a.png]]");
  });

  it("answers the media kind, any extension case", () => {
    expect(mediaKind("https://example.test/clip.mKv")).toBe("video");
    expect(mediaKind("clip.MKV")).toBe("video");
    expect(mediaKind("../assets/a.Ogg?x=1")).toBe("audio");
    expect(mediaKind("a.txt")).toBeNull();
  });
});
