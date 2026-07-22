import { describe, expect, it } from "vitest";
import { blockDtoExternalId } from "./blockIdentity";

describe("blockDtoExternalId", () => {
  const cases: Array<[string, [string, string][] | undefined, string]> = [
    ["Markdown id::", [["id", "markdown-authored"]], "markdown-authored"],
    ["Org :id:", [["id", "org-authored"]], "org-authored"],
    ["case-insensitive property key", [["ID", "case-authored"]], "case-authored"],
    ["empty authored value", [["Id", "   "]], "runtime-id"],
    ["id-less block", undefined, "runtime-id"],
  ];

  it.each(cases)("uses %s when available", (_case, properties, expected) => {
    expect(blockDtoExternalId({ id: "runtime-id", properties })).toBe(expected);
  });
});
