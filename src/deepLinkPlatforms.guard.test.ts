import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
describe("GH #181 five shipped platform URL boundary", () => {
  it("registers only tine in Linux desktop/Flatpak, Windows NSIS, macOS and iOS bundles, and Android", () => {
    const config = JSON.parse(read("src-tauri/tauri.conf.json"));
    for (const platform of ["macOS", "iOS"]) expect(config.bundle[platform].infoPlist).toBe("Info.plist");
    const plist = read("src-tauri/Info.plist");
    expect(plist).toMatch(/CFBundleURLSchemes<\/key><array><string>tine<\/string>/);
    expect(config.bundle.windows.nsis.installerHooks).toBe("tine-url-hooks.nsh");
    const hooks = read("src-tauri/tine-url-hooks.nsh");
    expect(hooks).toContain('Software\\Classes\\tine\\shell\\open\\command');
    expect(hooks).toContain('%1'); expect(hooks).toContain('NSIS_HOOK_PREUNINSTALL');
    for (const pkg of ["deb", "rpm"]) expect(config.bundle.linux[pkg].desktopTemplate).toBe("tine.desktop");
    for (const path of ["src-tauri/tine.desktop", "flatpak/page.tine.Tine.desktop", "src-tauri/src/linux_window_identity.rs"])
      expect(read(path), path).toContain("MimeType=x-scheme-handler/tine;");
    expect(read("src-tauri/gen/android/app/src/main/AndroidManifest.xml")).toMatch(/android.intent.action.VIEW[\s\S]*android.intent.category.BROWSABLE[\s\S]*android:scheme="tine"/);
    for (const source of [plist, hooks, read("src-tauri/gen/android/app/src/main/AndroidManifest.xml")])
      expect(source).not.toMatch(/(?:scheme=|<string>)"?logseq/);
  });
  it("queues native events and desktop argv until a window listener drains, including iOS and Android", () => {
    const native = read("src-tauri/src/lib.rs");
    expect(native).toContain('target_os = "macos", target_os = "ios", target_os = "android"');
    expect(native).toContain("tauri::RunEvent::Opened");
    expect(native).toContain("cli::LaunchRequest::Link(url) => deep_links::receive_url(app, url)");
    expect(native).toContain("deep_links::receive_url(app.handle(), url)");
    expect(native).toContain("deep_links::PendingLinks::default()");
    expect(read("src-tauri/src/platform.rs")).toContain("crate::deep_links::receive_url(&app, url)");
    expect(read("src/App.tsx")).toContain("__TINE_LINK_LAUNCH__ || injected");
  });
});
