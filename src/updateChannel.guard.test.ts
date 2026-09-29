// RULE (og preview identity): og is a separate app (`page.tine.TineOG`, version
// 0.6.x) that must NEVER learn about, offer, or install the shipped Tine. The
// shipped Tine's releases/latest carries a higher version number, so an og that
// read it would offer master, and installing it would replace og with master.
// The og update channel is the fixed-tag release `og-preview` only. Both the
// notifier (src/update.ts) and the Tauri updater endpoint (tauri.conf.json)
// must name it; exemplar: src/update.ts PREVIEW_TAG.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const RULE =
  "og updater must read only the og-preview channel (releases/tags/og-preview, " +
  "releases/download/og-preview/latest.json), never releases/latest: the shipped Tine " +
  "there is a newer version and installing it would replace og with master.";

describe("og update channel", () => {
  it("src/update.ts never points at releases/latest", () => {
    const source = readFileSync(new URL("./update.ts", import.meta.url), "utf8");
    // Comments may name the forbidden endpoint to explain the rule.
    const code = source.split("\n").filter((l) => !/^\s*(\/\/|\*|\/\*)/.test(l)).join("\n");
    expect(code, RULE).not.toMatch(/releases\/latest/);
    expect(code, RULE).toMatch(/releases\/tags\/\$\{PREVIEW_TAG\}/);
    expect(code, RULE).toMatch(/PREVIEW_TAG = "og-preview"/);
  });

  it("the Tauri updater endpoint is the og-preview download, not releases/latest", () => {
    const conf = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
    // Identity is spelled only in the switch (src/appIdentity.guard.test.ts): this channel
    // exists because the running build is NOT the released identity.
    const sw = JSON.parse(readFileSync(new URL("../src-tauri/app-identity.json", import.meta.url), "utf8"));
    expect(sw.ship, "this guard is for the experiment identity; a release build ships releases/latest").not.toBe("release");
    const endpoints: string[] = conf.plugins.updater.endpoints;
    expect(endpoints.length, RULE).toBeGreaterThan(0);
    for (const endpoint of endpoints) {
      expect(endpoint, RULE).not.toMatch(/releases\/latest/);
      expect(endpoint, RULE).toBe(
        "https://github.com/martinkoutecky/tine/releases/download/og-preview/latest.json",
      );
    }
  });
});
