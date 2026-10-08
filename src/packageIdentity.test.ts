import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { isPackageId, isPackageVersion } from "./packageIdentity";

it.each(["dev.tine.demo", "a.b", "a..b", "a.-b", "a." + "b".repeat(62)])("accepts existing manifest ID syntax: %s", (value) => {
  expect(isPackageId(value)).toBe(true);
});
it.each(["abc", "a.", ".a", "a.b-", "A.b", "../a", "a." + "b".repeat(63)])("rejects manifest ID: %s", (value) => {
  expect(isPackageId(value)).toBe(false);
});
it.each(["0.1.0", "1.2.3-beta.1", "1.2.3-.", "1.2.3-beta.", "99999999999999999999.2.3"])("retains manifest version syntax: %s", (value) => {
  expect(isPackageVersion(value)).toBe(true);
});
it.each(["01.2.3", "1.2", "1.2.3.4", "1.2.3-", "1.2.3+build", "../1"])("rejects manifest version: %s", (value) => {
  expect(isPackageVersion(value)).toBe(false);
});
it.each(["src/plugins/manifest.ts", "src/themes/manifest.ts", "src/plugins/registry.ts"])("I-12: %s delegates identity syntax to packageIdentity.ts", (path) => {
  const source = readFileSync(path, "utf8");
  expect(source, "I-12: use isPackageId/isPackageVersion from src/packageIdentity.ts").toContain("isPackageId(id)");
  expect(source, "I-12: use isPackageVersion from src/packageIdentity.ts").toMatch(/isPackageVersion\((?:parsedVersion|version)\)/);
  expect(source, "I-12: package identity regexes belong only to src/packageIdentity.ts").not.toMatch(/\[a-z0-9\.\-\]|\(0\|\[1-9\]/);
});
