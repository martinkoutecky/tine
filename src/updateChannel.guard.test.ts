// RULE (og preview identity): og is a separate app (`page.tine.TineOG`, version
// 0.6.x) that must NEVER learn about, offer, or install the shipped Tine. The
// shipped Tine's releases/latest carries a higher version number, so an og that
// read it would offer master, and installing it would replace og with master.
// The og update channel is the fixed-tag release `og-preview` only. Both the
// notifier (src/update.ts) and the Tauri updater endpoint (tauri.conf.json)
// must name it, and update.ts names the channel URL in exactly ONE place (the
// notifier reads the same latest.json manifest the updater does). Exemplar:
// src/update.ts PREVIEW_TAG / MANIFEST_URL.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const RULE =
  "og updater must read only the og-preview channel (releases/download/og-preview/latest.json, " +
  "one URL source in update.ts), never releases/latest: the shipped Tine " +
  "there is a newer version and installing it would replace og with master.";

describe("og update channel", () => {
  it("src/update.ts never points at releases/latest", () => {
    const source = readFileSync(new URL("./update.ts", import.meta.url), "utf8");
    // Comments may name the forbidden endpoint to explain the rule.
    const code = source.split("\n").filter((l) => !/^\s*(\/\/|\*|\/\*)/.test(l)).join("\n");
    expect(code, RULE).not.toMatch(/releases\/latest/);
    expect(code, RULE).toMatch(/PREVIEW_TAG = "og-preview"/);
    // Exactly one URL source: one literal host reference, and every fetch reads MANIFEST_URL.
    expect(code.match(/https?:\/\//g) ?? [], RULE).toHaveLength(1);
    const fetches = [...code.matchAll(/\bfetch\(([^)]*)\)/g)].map((m) => m[1].trim());
    expect(fetches.length, RULE).toBeGreaterThan(0);
    for (const arg of fetches) expect(arg, RULE).toBe("MANIFEST_URL");
    expect(code, RULE).toMatch(/MANIFEST_URL = `\$\{RELEASES\}\/download\/\$\{PREVIEW_TAG\}\/latest\.json`/);
    expect(code, RULE).toMatch(/RELEASES = "https:\/\/github\.com\/martinkoutecky\/tine\/releases"/);
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
