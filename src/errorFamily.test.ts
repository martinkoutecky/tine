import { expect, it } from "vitest";
import { errorFamily } from "./errorFamily";

it("recognizes fixed incomplete-transaction wire families with opaque recovery detail", () => {
  expect(errorFamily("rollback-incomplete: recovery: logseq/.tine-trash/a.md")).toBe("rollback-incomplete");
  expect(errorFamily(new Error("publication-incomplete: pages/a.md"))).toBe("publication-incomplete");
  expect(errorFamily("io:PermissionDenied")).toBe("io");
  expect(errorFamily("a rollback-incomplete operation happened")).toBe("unknown");
  expect(errorFamily("graph verification cancelled")).toBe("unknown");
});

it("recognizes the unreadable-owner creation refusal (R-CREATE-UNREADABLE-OWNER)", () => {
  expect(errorFamily(new Error("unreadable-owner"))).toBe("unreadable-owner");
  expect(errorFamily("an unreadable-owner happened")).toBe("unknown");
});

it("recognizes the exact not-found wire token for a missing graph asset, and never its prose", () => {
  expect(errorFamily("not-found")).toBe("not-found");
  expect(errorFamily("No such file or directory (os error 2)")).toBe("unknown");
});
