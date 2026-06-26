import { describe, it, expect } from "vitest";
import { isSoftwareRenderer } from "./gpu";

describe("isSoftwareRenderer", () => {
  it("flags Mesa software rasterizers", () => {
    expect(isSoftwareRenderer("llvmpipe (LLVM 15.0.7, 256 bits)")).toBe(true);
    expect(isSoftwareRenderer("Gallium 0.4 on llvmpipe")).toBe(true);
    expect(isSoftwareRenderer("softpipe")).toBe(true);
    expect(isSoftwareRenderer("swrast")).toBe(true);
  });
  it("flags SwiftShader and Windows WARP", () => {
    expect(isSoftwareRenderer("Google SwiftShader")).toBe(true);
    expect(isSoftwareRenderer("Microsoft Basic Render Driver")).toBe(true);
  });
  it("does NOT flag real GPUs", () => {
    expect(isSoftwareRenderer("Mesa Intel(R) UHD Graphics 620 (KBL GT2)")).toBe(false);
    expect(isSoftwareRenderer("NVIDIA GeForce RTX 3080/PCIe/SSE2")).toBe(false);
    expect(isSoftwareRenderer("AMD Radeon RX 6800 (RADV NAVI21)")).toBe(false);
    expect(isSoftwareRenderer("Apple M2")).toBe(false);
  });
  it("stays quiet when the renderer is unknown (no false alarm)", () => {
    expect(isSoftwareRenderer(null)).toBe(false);
    expect(isSoftwareRenderer("")).toBe(false);
  });
});
