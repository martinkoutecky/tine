// A1–A3 of the native-integrations batch (ADR 0073, GH #608): the Android
// share target, launcher shortcuts and Quick Settings tile are declared in the
// manifest, and every route they open is one the app actually handles. These
// are source facts the APK build cannot catch: a missing filter or a
// misspelled route compiles and ships as a silent no-op.
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { parseAppRoute } from "./deepLinks";

const ANDROID = path.resolve(__dirname, "../src-tauri/gen/android/app/src/main");
const read = (file: string) => fs.readFileSync(path.join(ANDROID, file), "utf8");
const manifest = read("AndroidManifest.xml");

/** The `<activity>` block declaring `name`, or the empty string. */
function activity(name: string): string {
  return [...manifest.matchAll(/<activity\b[\s\S]*?<\/activity>/g)].map((match) => match[0])
    .find((block) => block.includes(`android:name="${name}"`)) ?? "";
}

/** [action, mimeType] pairs of an element's intent filters. */
function filters(block: string): string[] {
  return [...block.matchAll(/<intent-filter>([\s\S]*?)<\/intent-filter>/g)].flatMap((match) => {
    const actions = [...match[1].matchAll(/<action android:name="([^"]+)"/g)].map((m) => m[1]);
    const types = [...match[1].matchAll(/android:mimeType="([^"]+)"/g)].map((m) => m[1]);
    return actions.flatMap((action) => (types.length ? types : [""]).map((type) => `${action} ${type}`.trim()));
  });
}

describe("Android native integrations (ADR 0073)", () => {
  const main = activity(".MainActivity");

  it("the main activity is the share target for text, links and images (GH #608)", () => {
    expect(main).not.toBe("");
    const declared = filters(main);
    for (const filter of [
      "android.intent.action.SEND text/*",
      "android.intent.action.SEND image/*",
      "android.intent.action.SEND_MULTIPLE image/*",
    ]) expect(declared, filter).toContain(filter);
    // The producer is the native plugin, and the Rust side registers it under its class name.
    expect(fs.existsSync(path.join(ANDROID, "java/page/tine/app/NativeIntegrationsPlugin.kt"))).toBe(true);
    const rust = fs.readFileSync(path.resolve(__dirname, "../src-tauri/src/native_integrations.rs"), "utf8");
    expect(rust).toContain('register_android_plugin("page.tine.app", "NativeIntegrationsPlugin")');
  });

  it("launcher shortcuts are declared and open only routes the app handles", () => {
    expect(main).toMatch(/android:name="android\.app\.shortcuts"\s+android:resource="@xml\/shortcuts"/);
    const shortcuts = read("res/xml/shortcuts.xml");
    const routes = [...shortcuts.matchAll(/android:data="([^"]+)"/g)].map((match) => match[1]);
    expect(routes.sort()).toEqual(["tine://capture", "tine://search", "tine://today"]);
    for (const route of routes) expect(parseAppRoute(route), route).not.toBeNull();
    for (const target of shortcuts.matchAll(/android:targetClass="([^"]+)"/g)) expect(target[1]).toBe("page.tine.app.MainActivity");
  });

  it("the Quick Settings tile is bound only by the system and opens the capture route", () => {
    const service = /<service\b[^>]*android:name="\.QuickCaptureTileService"[\s\S]*?<\/service>/.exec(manifest)?.[0] ?? "";
    expect(service).toContain('android:permission="android.permission.BIND_QUICK_SETTINGS_TILE"');
    expect(filters(service)).toContain("android.service.quicksettings.action.QS_TILE");
    const tile = read("java/page/tine/app/QuickCaptureTileService.kt");
    const route = /Uri\.parse\("([^"]+)"\)/.exec(tile)?.[1] ?? "";
    expect(route).toBe("tine://capture");
    expect(parseAppRoute(route)).toEqual({ route: "capture" });
  });
});
