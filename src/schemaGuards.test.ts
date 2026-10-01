import { expect, it } from "vitest";
import { schemaGuards } from "./schemaGuards";

it("keeps each schema's diagnostics, first unknown key and C0 policy", () => {
  class SchemaError extends Error {}
  const guards = (plainText: boolean) => schemaGuards(SchemaError, {
    object: (where) => `${where}: object`,
    string: (where, max) => `${where}: text ${max}`,
    plainText,
  });
  for (const plainText of [true, false]) {
    const guard = guards(plainText);
    for (const bad of [null, [], false, "object"]) expect(() => guard.record(bad, "x")).toThrow(new SchemaError("x: object"));
    const record = { okay: 1, first: 2, second: 3 };
    expect(guard.record(record, "x")).toBe(record);
    expect(() => guard.knownKeys(record, "x", ["okay"])).toThrow("x contains unknown field first");
    for (const bad of ["", 1, "long"]) expect(() => guard.text(bad, "x", 3)).toThrow(new SchemaError("x: text 3"));
    expect(guard.text("abc", "x", 3)).toBe("abc");
  }
  expect(guards(false).text("a\u0000", "x", 3)).toBe("a\u0000");
  expect(() => guards(true).text("a\u0000", "x", 3)).toThrow("x: text 3");
});
