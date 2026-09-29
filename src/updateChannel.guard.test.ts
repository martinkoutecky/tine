// RULE (og preview identity): og is a separate app (`page.tine.TineOG`, version
// 0.6.x) that must NEVER learn about, offer, or install the shipped Tine. The
// shipped Tine's releases/latest carries a higher version number, so an og that
// read it would offer master, and installing it would replace og with master.
//
// The og update channel is the fixed-tag release `og-preview`, and it is named in
// exactly ONE machine-read place: the updater endpoint in src-tauri/tauri.conf.json.
// src/update.ts learns what the channel offers from the Tauri updater plugin's
// check() (a Rust-side request); it never fetch()es a channel URL, because GitHub
// release-asset downloads send no Access-Control-Allow-Origin and a webview fetch
// would fail silently forever. Exemplar: src/update.ts offeredVersion().
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const RULE =
  "og updater must read only the og-preview channel, and only through the updater plugin's check(): " +
  "the one channel URL is the tauri.conf.json updater endpoint " +
  "(https://github.com/martinkoutecky/tine/releases/download/og-preview/latest.json), never releases/latest " +
  "(the shipped Tine there is a newer version; installing it would replace og with master); " +
  "src/update.ts must not fetch() a channel URL (GitHub release assets send no CORS headers).";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
const stripComments = (source: string) =>
  source.split("\n").filter((l) => !/^\s*(\/\/|\*|\/\*)/.test(l)).join("\n");

describe("og update channel", () => {
  it("src/update.ts names no channel URL and never fetch()es one", () => {
    const code = stripComments(read("./update.ts"));
    expect(code, RULE).not.toMatch(/\bfetch\s*\(/);
    expect(code, RULE).not.toMatch(/releases\/latest/);
    expect(code, RULE).not.toMatch(/releases\/download|latest\.json|api\.github\.com/);
    // The only URL is the human-facing release page, and it is the og-preview one.
    const urls = code.match(/https?:\/\/[^\s"'`]+/g) ?? [];
    expect(urls, RULE).toEqual(["https://github.com/martinkoutecky/tine/releases/tag/og-preview"]);
    // The offered version comes from the updater plugin.
    expect(code, RULE).toMatch(/import\("@tauri-apps\/plugin-updater"\)/);
  });

  it("the Tauri updater endpoint is the og-preview manifest, never releases/latest", () => {
    const conf = JSON.parse(read("../src-tauri/tauri.conf.json"));
    // Identity is spelled only in the switch (src/appIdentity.guard.test.ts): this channel
    // exists because the running build is NOT the released identity.
    const sw = JSON.parse(read("../src-tauri/app-identity.json"));
    expect(sw.ship, "this guard is for the experiment identity; a release build ships releases/latest").not.toBe("release");
    const endpoints: string[] = conf.plugins.updater.endpoints;
    expect(endpoints, RULE).toEqual(["https://github.com/martinkoutecky/tine/releases/download/og-preview/latest.json"]);
    for (const endpoint of endpoints) expect(endpoint, RULE).not.toMatch(/releases\/latest/);
  });

  it("the updater's check() is permitted on every desktop platform", () => {
    // check() runs in the main window on Linux, macOS (notifier only) and Windows.
    const capability = JSON.parse(read("../src-tauri/capabilities/desktop.json"));
    expect([...capability.platforms].sort(), "updater:default must be granted on all desktop platforms").toEqual(["linux", "macOS", "windows"]);
    expect(capability.windows).toContain("main");
    expect(capability.permissions).toContain("updater:default");
  });
});
