import fs from "node:fs";
import { describe, expect, it } from "vitest";
import { msixVersion } from "../scripts/lib/msix-version.mjs";

describe("Store version mapping", () => {
  it.each([["0.7.0", "0.7.0.0"], ["1.2.3", "1.2.3.0"]])("maps %s", (input, output) => {
    expect(msixVersion(input)).toBe(output);
  });
  it("refuses prereleases and overflow with actionable errors", () => {
    expect(() => msixVersion("0.7.0-beta.3")).toThrow(/stable.*prereleases/);
    for (const version of ["65536.0.0", "1.65536.0", "1.0.65536"]) expect(() => msixVersion(version)).toThrow(/65535/);
    expect(() => msixVersion("1.1000.0")).toThrow(/Android/);
    expect(() => msixVersion("garbage")).toThrow(/invalid release version/);
  });
});

describe("MSIX manifest and workflow contract", () => {
  const manifest = fs.readFileSync("packaging/msix/AppxManifest.xml", "utf8");
  it("uses Partner Center identity, exactly two capabilities, and the existing protocol", () => {
    expect(manifest).toContain('Name="MartinKouteck.TineOutline"');
    expect(manifest).toContain('Publisher="CN=5B2AC29F-AAE7-4DAB-BA53-1AD73037C7F6"');
    expect(manifest).toContain("<PublisherDisplayName>Martin Koutecký</PublisherDisplayName>");
    expect(manifest).toContain("<DisplayName>TineOutline</DisplayName>");
    expect(manifest).toContain('DisplayName="TineOutline"');
    expect([...manifest.matchAll(/<rescap:Capability Name="([^"]+)"/g)].map((m) => m[1]).sort()).toEqual(["runFullTrust", "unvirtualizedResources"]);
    expect(manifest).toContain('<uap:Protocol Name="tine"');
    expect(manifest).toContain('Executable="tine.exe"');
    expect(manifest).toContain('MinVersion="10.0.18362.0"');
    for (const name of ["StoreLogo", "Square44x44Logo", "Square150x150Logo"]) {
      const png = fs.readFileSync(`src-tauri/icons/${name}.png`);
      const size = name === "StoreLogo" ? 50 : name === "Square44x44Logo" ? 44 : 150;
      expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([size, size]);
    }
  });
  it("enumerates every shipped platform for package detection", () => {
    const source = fs.readFileSync("src-tauri/src/platform.rs", "utf8");
    const detection = source.slice(source.indexOf('/// Package identity'), source.indexOf('/// Whether the process runs inside a Flatpak'));
    for (const os of ["windows", "linux", "macos", "ios", "android"]) expect(detection).toContain(`target_os = "${os}"`);
    expect(detection).not.toContain("cfg(not(");
    expect(detection.match(/unsafe \{/g)).toHaveLength(1);
  });
  it("only dispatches manually and pins all actions", () => {
    const workflow = fs.readFileSync(".github/workflows/msix.yml", "utf8");
    expect(workflow).toContain("workflow_dispatch:");
    expect(workflow).not.toMatch(/\n  (push|pull_request|release|schedule):/);
    for (const match of workflow.matchAll(/uses: (\S+)/g)) expect(match[1]).toMatch(/@[0-9a-f]{40}$/);
    expect(workflow).toContain("node scripts/set-app-identity.mjs release");
    expect(workflow).toContain("--no-sign");
  });
});
