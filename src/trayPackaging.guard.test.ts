import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// GH #625 (OG-TRAY): the tray icon library (libappindicator-sys) dlopens
// libayatana-appindicator3 at runtime; nothing links it, so no bundler can see
// the need. tauri-bundler 2.9.2 (the version tauri-cli 2.11.2 locks) writes a
// deb `Depends`/`Recommends` and an rpm requires/recommends ONLY from this
// config, and its AppImage step (linuxdeploy) bundles only libraries found by
// ldd. The tray is optional and Tine degrades to "no tray" without the
// library, so the deb and rpm RECOMMEND it rather than depend on it.
// Exemplar: src/deepLinkPlatforms.guard.test.ts reads tauri.conf.json the same way.
describe("Linux packages recommend the tray library", () => {
  const config = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  it("the deb recommends libayatana-appindicator3-1", () => {
    expect(config.bundle.linux.deb.recommends).toContain("libayatana-appindicator3-1");
    expect(config.bundle.linux.deb.depends ?? []).not.toContain("libayatana-appindicator3-1");
  });
  it("the rpm recommends libayatana-appindicator-gtk3", () => {
    expect(config.bundle.linux.rpm.recommends).toContain("libayatana-appindicator-gtk3");
    expect(config.bundle.linux.rpm.depends ?? []).not.toContain("libayatana-appindicator-gtk3");
  });
});
