import { describe, expect, it } from "vitest";
import { installSystemInsetOwner, systemInsetOwner } from "./systemInsets";

describe("system inset ownership (GH #205)", () => {
  it.each([
    [true, "android", "native-viewport"],
    [false, "android", "css-viewport"],
    [true, "ios", "css-viewport"],
    [true, "desktop", "css-viewport"],
  ] as const)("selects the sole owner for native=%s platform=%s", (nativeHost, platform, expected) => {
    expect(systemInsetOwner(nativeHost, platform)).toBe(expected);
  });

  it("publishes the owner as the CSS contract", () => {
    const root = { dataset: {} } as HTMLElement;
    expect(installSystemInsetOwner(root, true, "android")).toBe("native-viewport");
    expect(root.dataset.systemInsets).toBe("native-viewport");
    expect(installSystemInsetOwner(root, false, "android")).toBe("css-viewport");
    expect(root.dataset.systemInsets).toBe("css-viewport");
  });
});
