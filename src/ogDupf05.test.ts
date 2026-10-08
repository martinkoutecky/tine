import { describe, expect, it } from "vitest";
import { pageIdentityKey } from "./pageIdentity";
import { canonicalGroupField, normalizeQueryDisplayDraft } from "./editor/queryViewProperties";

describe("OG-DUPF05 native Unicode name and group policy", () => {
  it("resolves frontend page identity with Rust whitespace, case, slashes and NFC", () => {
    expect(pageIdentityKey("\u0085/CAFÉ/\u0085")).toBe("café");
    expect(pageIdentityKey("\uFEFFFoo\uFEFF")).toBe("\uFEFFfoo\uFEFF");
    expect(pageIdentityKey(" ΟΣ ")).toBe("ος");
    expect(pageIdentityKey("/Cafe\u0301/")).toBe("café");
    expect(pageIdentityKey("//Foo//")).toBe("/foo/");
  });
  it("validates route grouping with the same policy as native readback", () => {
    expect(canonicalGroupField("\u0085state\u0085")).toBe("state");
    expect(normalizeQueryDisplayDraft({ group_by: "\u0085state\u0085" })?.group_by).toBe("\u0085state\u0085");
    expect(canonicalGroupField("\uFEFFstate\uFEFF")).toBeNull();
    expect(normalizeQueryDisplayDraft({ group_by: "\uFEFFstate\uFEFF" })).toBeNull();
    for (const token of ["state", "priority", "scheduled", "deadline", "tags", "page", "prop:state", "formula:x"])
      expect(canonicalGroupField(`  ${token}  `)).toBe(token);
    for (const token of ["", "prop:", "formula:", "other", "prop:x\ninside", "prop:x\0inside"])
      expect(canonicalGroupField(token)).toBeNull();
  });
});
