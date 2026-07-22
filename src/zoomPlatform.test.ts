// @vitest-environment jsdom
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { applyAndroidInterfaceZoom } from "./zoom";

const root = path.resolve(import.meta.dirname, "..");
const zoom = fs.readFileSync(path.join(root, "src/zoom.ts"), "utf8");

describe("Android interface zoom contract (GH #133)", () => {
  it("does not rely on Wry's documented Android no-op zoom implementation", () => {
    expect(zoom).toContain('kind === "android"');
    expect(zoom).toContain("applyAndroidInterfaceZoom");
    expect(zoom).toContain("document.documentElement.style.zoom");
  });

  it("applies and resets the complete-document scale used by Android Chromium", () => {
    applyAndroidInterfaceZoom(1.25);
    expect(document.documentElement.style.zoom).toBe("1.25");
    applyAndroidInterfaceZoom(1);
    expect(document.documentElement.style.zoom).toBe("");
  });
});
