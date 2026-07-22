import { describe, expect, it } from "vitest";
import { blockDtoExternalId } from "./blockIdentity";

describe("blockDtoExternalId", () => {
  it.each([
    ["Markdown id::", [["id", "markdown-authored"]], "markdown-authored"],
    ["Org :id:", [["id", "org-authored"]], "org-authored"],
    ["case-insensitive property key", [["ID", "case-authored"]], "case-authored"],
    ["empty authored value", [["Id", "   "]], "runtime-id"],
    ["id-less block", undefined, "runtime-id"],
  ] as const)("uses %s when available", (_case, properties, expected) => {
    expect(blockDtoExternalId({ id: "runtime-id", properties })).toBe(expected);
  });
});
