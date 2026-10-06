import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// GH #625 (OG-TRAY): the tray icon library (libappindicator-sys) dlopens
// libayatana-appindicator3 at runtime; nothing links it, so no bundler can see
// the need. tauri-bundler 2.9.2 (the version tauri-cli 2.11.2 locks) writes the
// deb `Depends`/`Recommends` and the rpm requires/recommends from this config,
// and tauri-cli itself ALWAYS adds the webview/gtk packages (deb:
// libwebkit2gtk-4.1-0, libgtk-3-0; rpm equivalents). Its AppImage step
// (linuxdeploy) bundles only libraries found by ldd. The tray is optional and
// Tine degrades to "no tray" without the library, so the deb and rpm RECOMMEND
// it rather than depend on it.
//
// tauri-cli ALSO adds a hard `Depends` on libayatana-appindicator3 (and bundles
// the .so into the AppImage) when it finds the `tray-icon` feature in
// `[dependencies].tauri` or `app.trayIcon` in tauri.conf.json. Tine requests the
// feature from a `[target.'cfg(not(any(android, ios)))'.dependencies]` table,
// which the CLI does not read; the tests below fail if that placement changes.
// Exemplar: src/deepLinkPlatforms.guard.test.ts reads tauri.conf.json the same way.
const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

describe("Linux packages recommend the tray library", () => {
  const config = JSON.parse(read("../src-tauri/tauri.conf.json"));
  it("the deb recommends libayatana-appindicator3-1", () => {
    expect(config.bundle.linux.deb.recommends).toContain("libayatana-appindicator3-1");
    expect(config.bundle.linux.deb.depends ?? []).not.toContain("libayatana-appindicator3-1");
  });
  it("the rpm recommends libayatana-appindicator-gtk3", () => {
    expect(config.bundle.linux.rpm.recommends).toContain("libayatana-appindicator-gtk3");
    expect(config.bundle.linux.rpm.depends ?? []).not.toContain("libayatana-appindicator-gtk3");
  });
  it("tauri.conf.json declares no app.trayIcon (the CLI would add a hard Depends)", () => {
    expect(config.app?.trayIcon, "app.trayIcon makes tauri-cli depend on libayatana-appindicator3").toBeUndefined();
  });
  it("the tray-icon feature is requested only outside [dependencies] (tauri-cli reads that table)", () => {
    const cargo = read("../src-tauri/Cargo.toml");
    const section = (header: string) => {
      const start = cargo.indexOf(`\n[${header}]`);
      expect(start, `Cargo.toml has a [${header}] table`).toBeGreaterThanOrEqual(0);
      const rest = cargo.slice(start + 1);
      const next = rest.slice(header.length + 2).search(/^\[/m);
      return next < 0 ? rest : rest.slice(0, header.length + 2 + next);
    };
    const tauriLine = section("dependencies").split("\n").find((line) => /^tauri\s*=/.test(line));
    expect(tauriLine, "[dependencies] declares tauri").toBeDefined();
    expect(tauriLine, "tray-icon in [dependencies].tauri makes tauri-cli depend on and bundle libayatana-appindicator3").not.toContain("tray-icon");
    expect(section("target.'cfg(not(any(target_os = \"android\", target_os = \"ios\")))'.dependencies")).toContain("tray-icon");
  });
});

describe("the Flatpak manifest lets the tray reach the StatusNotifierWatcher", () => {
  it("finish-args talk to org.kde.StatusNotifierWatcher", () => {
    expect(read("../flatpak/page.tine.Tine.yml")).toContain("--talk-name=org.kde.StatusNotifierWatcher");
  });
});
