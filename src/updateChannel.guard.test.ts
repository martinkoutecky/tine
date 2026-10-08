// RULE (channel identity): stable Tine (`page.tine.Tine`) and Tine Beta
// (`page.tine.TineBeta`) are separate apps, and each must NEVER offer or install
// the other's build: installing it would replace one app with the other.
//
// Each build's channel is named in exactly ONE machine-read place: the updater
// endpoint in src-tauri/tauri.conf.json (stable: releases/latest; Beta: the
// fixed-tag release `beta`), chosen by the identity switch. src/update.ts learns
// what the channel offers from the Tauri updater plugin's check() (a Rust-side
// request); it never fetch()es a channel URL, because GitHub release-asset
// downloads send no Access-Control-Allow-Origin and a webview fetch would fail
// silently forever. Exemplar: src/update.ts offeredVersion().
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const RULE =
  "the updater must read only this build's channel, and only through the updater plugin's check(): " +
  "the one channel URL is the tauri.conf.json updater endpoint chosen by the identity switch " +
  "(stable: releases/latest; Beta: releases/download/beta), never the other channel's " +
  "(installing it would replace one app with the other); " +
  "src/update.ts must not fetch() a channel URL (GitHub release assets send no CORS headers).";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
const stripComments = (source: string) =>
  source.split("\n").filter((l) => !/^\s*(\/\/|\*|\/\*)/.test(l)).join("\n");

describe("update channel", () => {
  it("src/update.ts names no channel URL and never fetch()es one", () => {
    const code = stripComments(read("./update.ts"));
    expect(code, RULE).not.toMatch(/\bfetch\s*\(/);
    expect(code, RULE).not.toMatch(/releases\/latest\/download/);
    expect(code, RULE).not.toMatch(/releases\/download|latest\.json|api\.github\.com/);
    // The only URLs are the two channels' human-facing release pages.
    const urls = code.match(/https?:\/\/[^\s"'`]+/g) ?? [];
    expect(urls, RULE).toEqual([
      "https://github.com/martinkoutecky/tine/releases/latest",
      "https://github.com/martinkoutecky/tine/releases/tag/beta",
    ]);
    // The offered version comes from the updater plugin.
    expect(code, RULE).toMatch(/import\("@tauri-apps\/plugin-updater"\)/);
  });

  it("the Tauri updater endpoint is the switch's channel manifest", () => {
    const conf = JSON.parse(read("../src-tauri/tauri.conf.json"));
    // Identity is spelled only in the switch (src/appIdentity.guard.test.ts): the released
    // identity reads releases/latest, the experiment reads only the beta release.
    const sw = JSON.parse(read("../src-tauri/app-identity.json"));
    const endpoints: string[] = conf.plugins.updater.endpoints;
    expect(endpoints, RULE).toEqual([sw.ship === "release"
      ? "https://github.com/martinkoutecky/tine/releases/latest/download/latest.json"
      : "https://github.com/martinkoutecky/tine/releases/download/beta/latest.json"]);
  });

  it("I-4/I-12: notification and installation acquire updates through checkedChannelUpdate (src/update.ts)", () => {
    const code = stripComments(read("./update.ts"));
    expect(code.match(/await check\(\)/g), RULE).toHaveLength(1);
    expect(code.match(/await checkedChannelUpdate\(\)/g), RULE).toHaveLength(2);
    expect(code, RULE).toContain("releaseVersion(update.version).sequence");
  });

  it("the updater's check() is permitted on every desktop platform", () => {
    // check() runs in the main window on Linux, macOS (notifier only) and Windows.
    const capability = JSON.parse(read("../src-tauri/capabilities/desktop.json"));
    expect([...capability.platforms].sort(), "updater:default must be granted on all desktop platforms").toEqual(["linux", "macOS", "windows"]);
    expect(capability.windows).toContain("main");
    expect(capability.permissions).toContain("updater:default");
  });
});
