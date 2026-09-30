import { expect, it } from "vitest";
import { errorFamily } from "./errorFamily";

it("recognizes fixed incomplete-transaction wire families with opaque recovery detail", () => {
  expect(errorFamily("rollback-incomplete: recovery: logseq/.tine-trash/a.md")).toBe("rollback-incomplete");
  expect(errorFamily(new Error("publication-incomplete: pages/a.md"))).toBe("publication-incomplete");
  expect(errorFamily("io:PermissionDenied")).toBe("io");
  expect(errorFamily("a rollback-incomplete operation happened")).toBe("unknown");
  expect(errorFamily("graph verification cancelled")).toBe("unknown");
});
