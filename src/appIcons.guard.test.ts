import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { IDENTITIES, deriveIdentityFiles } from "../scripts/lib/app-identity.mjs";
import { deriveFlatpak } from "../scripts/derive-flatpak-identity.mjs";

const ROOT = path.resolve(__dirname, "..");
const read = (file: string) => fs.readFileSync(path.join(ROOT, file));
const icons = ["32x32.png", "128x128.png", "128x128@2x.png", "icon.icns", "icon.ico"];

describe("identity icons", () => {
  it("resolves existing, distinct desktop and Android artwork for both identities", () => {
    for (const ship of ["release", "experiment"]) {
      const dir = ship === "release" ? "icons" : "icons-beta";
      expect(IDENTITIES[ship].iconDirectory).toBe(dir);
      const derived = deriveIdentityFiles(ROOT, ship);
      const conf = JSON.parse(derived["src-tauri/tauri.conf.json"]);
      expect(conf.bundle.icon).toEqual(icons.map((icon) => `${dir}/${icon}`));
      for (const icon of [...icons, "icon.png", "64x64.png"]) {
        expect(read(`src-tauri/${dir}/${icon}`).length).toBeGreaterThan(0);
        if (ship === "experiment") {
          expect(read(`src-tauri/${dir}/${icon}`).equals(read(`src-tauri/icons/${icon}`))).toBe(false);
        }
      }
      const gradle = derived["src-tauri/gen/android/app/build.gradle.kts"];
      expect(gradle).toContain(`val tineIconDirectory = "${dir}"`);
      for (const density of ["mdpi", "hdpi", "xhdpi", "xxhdpi", "xxxhdpi"]) {
        for (const name of ["ic_launcher", "ic_launcher_round", "ic_launcher_foreground"]) {
          const stable = read(`src-tauri/gen/android/app/src/main/res/mipmap-${density}/${name}.png`);
          const beta = read(`src-tauri/icons-beta/android/mipmap-${density}/${name}.png`);
          expect(beta.equals(stable)).toBe(false);
        }
      }
    }
  });

  it("keeps the release config byte-identical to the previous identity resolution", () => {
    expect(deriveIdentityFiles(ROOT, "release")["src-tauri/tauri.conf.json"])
      .toBe(read("src/fixtures/release-icon-config.json").toString());
  });

  it("changes only the background fill in the Beta SVG masters", () => {
    for (const suffix of ["", "-macos"]) {
      expect(read(`docs/app-icon-beta${suffix}.svg`).toString())
        .toBe(read(`docs/app-icon${suffix}.svg`).toString().replace("#10b981", "#e8890c"));
    }
  });

  it("selects Linux runtime icons and Android overlays through the same identity", () => {
    expect(read("src-tauri/src/linux_window_identity.rs").toString()).toContain('env!("TINE_ICON_DIRECTORY")');
    const gradle = read("src-tauri/gen/android/app/build.gradle.kts").toString();
    expect(gradle).toContain('if (tineIconDirectory != "icons")');
    for (const buildType of ["debug", "release"]) {
      expect(gradle).toContain(`getByName("${buildType}").res.srcDir`);
    }
    const adaptive = read("src-tauri/icons-beta/android/mipmap-anydpi-v26/ic_launcher.xml").toString();
    expect(adaptive).toContain("@mipmap/ic_launcher_foreground");
    expect(adaptive).toContain("@color/ic_launcher_background");
  });

  it("uses the selected artwork in the Flatpak install paths", () => {
    const ship = JSON.parse(read("src-tauri/app-identity.json").toString()).ship;
    const derived = deriveFlatpak({ root: ROOT, identity: ship, date: "2026-10-28", version: "0.9.0-beta.1" });
    expect(derived.files[derived.manifestPath]).toContain(`src-tauri/${IDENTITIES[ship].iconDirectory}/icon.png`);
  });
});
