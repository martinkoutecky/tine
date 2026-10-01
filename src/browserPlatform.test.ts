import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { browserPlatform } from "./browserPlatform";

it.each([
  ["Android Windows", "", true, false, true, false],
  ["iPhone", "", false, true, false, true],
  ["Macintosh; Intel Mac OS X", "MacIntel", false, false, false, true],
  ["Linux", "Linux", false, false, false, false],
] as const)("browser hints preserve existing consumers: %s", (ua, platform, android, ios, windows, appleEmoji) => {
  expect(browserPlatform(ua, platform)).toMatchObject({ android, ios, windows, appleEmoji });
});
it("keeps keyboard, window chrome, and updater installation as named policies", () => {
  expect(browserPlatform("iPad", "MacIntel")).toMatchObject({ ios: true, macKeyboard: true, manualDesktopUpdate: false });
  expect(browserPlatform("Macintosh", "")).toMatchObject({ macDesktop: true, macKeyboard: false, manualDesktopUpdate: true });
  expect(browserPlatform("", "macintel")).toMatchObject({ macDesktop: true, macKeyboard: false });
});
it.each(["src/nativeChrome.ts", "src/editableEmoji.ts", "src/update.ts", "src/keybindings.ts"])("I-12: %s uses browserPlatform.ts for browser hints", (path) => {
  const source = readFileSync(path, "utf8");
  expect(source, "I-12: browser platform hints belong to src/browserPlatform.ts").toContain("browserPlatform(");
  expect(source, "I-12: use browserPlatform.ts instead of a second UA/platform regex").not.toMatch(/\/(?:Android|Windows|iPhone|Mac|\\bMac|\(Macintosh)[^\n]*\/i?\.test\(/);
});
