import { describe, it, expect, vi } from "vitest";

// Capture draws the real editor before the wasm parser has loaded. Until then
// splitProps hides nothing, so a commit joins with empty hidden text and must
// not call into the wasm (outline-kernel M1 review F1).
describe("an editor commit before the parser is ready", () => {
  it("splits and joins without the wasm", async () => {
    vi.resetModules();
    const parse = await import("../render/parse");
    const { splitProps, joinProps, BUILTIN_HIDE } = await import("./properties");
    expect(parse.parserReady()).toBe(false);
    const raw = "hello\nid:: 6501b0f0-0000-4000-8000-000000000000";
    const { visible, hidden } = splitProps(raw, BUILTIN_HIDE);
    expect(hidden).toBe("");
    expect(joinProps(`${visible} edited`, hidden, "md")).toBe(`${raw} edited`);
    expect(joinProps("* org", hidden, "org")).toBe("* org");
  });
});
