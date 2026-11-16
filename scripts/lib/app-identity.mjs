// The app identity front door for scripts. The ONE switch is
// src-tauri/app-identity.json (docs/app-identity.md): `ship` selects the
// experiment identity (page.tine.TineBeta, coexists with the released Tine) or
// the released one. Every identity-bearing file derives from it through
// `deriveIdentityFiles`; `node scripts/set-app-identity.mjs <ship>` writes them
// and src/appIdentity.guard.test.ts fails when any of them drifts. Journeys must
// read APP_ID from here rather than spell an identifier.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const SWITCH_FILE = "src-tauri/app-identity.json";

export function readSwitch(root = ROOT) {
  return JSON.parse(fs.readFileSync(path.join(root, SWITCH_FILE), "utf8"));
}

const SWITCH = readSwitch();
export const SHIP = SWITCH.ship;
export const IDENTITIES = SWITCH.identities;
export const IDENTITY = IDENTITIES[SHIP];
export const APP_ID = IDENTITY.identifier;

// Every committed native iOS identity literal belongs to this derived set.
export const IOS_IDENTITY_FILES = [
  "src-tauri/Info.ios.plist",
  "src-tauri/Tine.ios.entitlements",
  "src-tauri/tauri.ios.conf.json",
  "src-tauri/ios-app/ShareExtension/Info.plist",
  "src-tauri/ios-app/ShareExtension/TineShareExtension.entitlements",
  "src-tauri/ios-app/Shared/ShareInbox.swift",
  "src-tauri/ios-native-integrations/ios/Sources/NativeIntegrationsPlugin.swift",
  "src-tauri/ios-folder-picker-native/ios/Sources/GraphFolderPickerPlugin.swift",
];

export function iosIdentity(identity = IDENTITY) {
  return {
    ...identity.ios,
    identifier: identity.identifier,
    appGroup: `group.${identity.identifier}`,
    iCloudContainer: `iCloud.${identity.identifier}`,
    shareExtensionIdentifier: `${identity.identifier}.ShareExtension`,
    iconDirectory: `${identity.iconDirectory}/ios`,
  };
}

/**
 * The derived identity-bearing files for `ship`, as { relativePath: text },
 * computed from the files currently under `root`. Pure: writes nothing.
 */
export function deriveIdentityFiles(root, ship) {
  const sw = readSwitch(root);
  const identity = sw.identities[ship];
  if (!identity) throw new Error(`unknown ship "${ship}" (expected one of ${Object.keys(sw.identities).join(", ")})`);
  const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
  const out = {};

  out[SWITCH_FILE] = JSON.stringify({ ...sw, ship }, null, 2) + "\n";

  // Targeted line edits keep the file's hand formatting (a JSON round trip would not).
  const confFile = "src-tauri/tauri.conf.json";
  let conf = read(confFile);
  const edits = [
    [/^(  "identifier": )"[^"]*"/m, identity.identifier],
    [/^(  "productName": )"[^"]*"/m, identity.productName],
    [/("label": "main",\s*"title": )"[^"]*"/, identity.productName],
  ];
  for (const [pattern, value] of edits) {
    if (!pattern.test(conf)) throw new Error(`${confFile}: no match for ${pattern}`);
    conf = conf.replace(pattern, `$1${JSON.stringify(value)}`);
  }
  conf = conf.replace(/("icon": \[)([\s\S]*?)(\n    \])/,
    (_, start, entries, end) => start + entries.replace(/"icons(?:-beta)?\//g, `"${identity.iconDirectory}/`) + end);
  out[confFile] = conf;

  const gradleFile = "src-tauri/gen/android/app/build.gradle.kts";
  const gradle = read(gradleFile);
  const applicationId = /^(\s*applicationId = )"[^"]*"$/m;
  if (!applicationId.test(gradle)) throw new Error(`${gradleFile}: no applicationId line`);
  const iconDirectory = /^(val tineIconDirectory = )"[^"]*"$/m;
  if (!iconDirectory.test(gradle)) throw new Error(`${gradleFile}: no tineIconDirectory line`);
  out[gradleFile] = gradle.replace(applicationId, `$1"${identity.androidApplicationId}"`)
    .replace(iconDirectory, `$1"${identity.iconDirectory}"`);

  // Launcher shortcuts name their target package explicitly (ADR 0073).
  const shortcutsFile = "src-tauri/gen/android/app/src/main/res/xml/shortcuts.xml";
  const shortcuts = read(shortcutsFile);
  const targetPackage = /(android:targetPackage=)"[^"]*"/g;
  if (!targetPackage.test(shortcuts)) throw new Error(`${shortcutsFile}: no android:targetPackage`);
  out[shortcutsFile] = shortcuts.replace(targetPackage, `$1"${identity.androidApplicationId}"`);

  // Names in native picker messages follow the iOS display name; the extension
  // keeps stable's existing share-sheet name. Derive values by their keys so
  // --check also detects an unknown/wrong identifier, not just the other ship.
  const escape = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const displayNames = new RegExp(Object.values(sw.identities)
    .map((value) => value.ios.displayName).sort((a, b) => b.length - a.length).map(escape).join("|"), "g");
  const ios = iosIdentity(identity);
  const plistString = (key) => new RegExp(`(<key>${escape(key)}</key>\\s*<string>)[^<]*(</string>)`);
  const entitlement = (key) => new RegExp(`(<key>${escape(key)}</key>\\s*<array>\\s*<string>)[\\s\\S]*?(</string>\\s*</array>)`);
  const swiftString = (name) => new RegExp(`(\\b(?:let|var) ${name} = ")[^"]*(")`);
  for (const file of IOS_IDENTITY_FILES) {
    let text = read(file);
    const edits = [];
    if (file.endsWith(".entitlements")) {
      edits.push([entitlement("com.apple.security.application-groups"), ios.appGroup]);
      if (file === "src-tauri/Tine.ios.entitlements") {
        edits.push([entitlement("com.apple.developer.icloud-container-identifiers"), ios.iCloudContainer]);
        edits.push([entitlement("com.apple.developer.ubiquity-container-identifiers"), ios.iCloudContainer]);
      }
    } else if (file === "src-tauri/Info.ios.plist") {
      edits.push([plistString("CFBundleDisplayName"), ios.displayName]);
      edits.push([plistString("NSUbiquitousContainerName"), ios.displayName]);
      edits.push([/(<key>NSUbiquitousContainers<\/key>\s*<dict>\s*<key>)[^<]*(<\/key>)/, ios.iCloudContainer]);
      let shortcuts = 0;
      text = text.replace(/(<key>UIApplicationShortcutItemType<\/key>\s*<string>)[^<]*(<\/string>)/g,
        (_, start, end) => start + `${identity.identifier}.${["today", "capture", "search"][shortcuts++]}` + end);
      if (shortcuts !== 3) throw new Error(`${file}: expected three Home Screen shortcuts`);
    } else if (file.endsWith("Shared/ShareInbox.swift")) {
      edits.push([swiftString("appGroup"), ios.appGroup]);
      edits.push([swiftString("changedNotification"), `${identity.identifier}.share-inbox-changed`]);
    } else if (file.endsWith("NativeIntegrationsPlugin.swift")) {
      edits.push([swiftString("tineAppGroup"), ios.appGroup]);
      edits.push([swiftString("tineInboxChangedNotification"), `${identity.identifier}.share-inbox-changed`]);
      edits.push([swiftString("tineSpotlightDomain"), `${identity.identifier}.pages`]);
    } else if (file.endsWith("GraphFolderPickerPlugin.swift")) {
      edits.push([swiftString("iCloudContainerIdentifier"), ios.iCloudContainer]);
    }
    for (const [pattern, value] of edits) {
      if (!pattern.test(text)) throw new Error(`${file}: no match for ${pattern}`);
      text = text.replace(pattern, (_, start, end) => start + value + end);
    }
    if (file.endsWith("ShareExtension/Info.plist")) {
      text = text.replace(/(<key>CFBundleDisplayName<\/key>\s*<string>)[^<]*(<\/string>)/,
        (_, start, end) => start + identity.productName + end);
    } else if (file.endsWith("tauri.ios.conf.json")) {
      text = text.replace(/("title": )"[^"]*"/, `$1${JSON.stringify(identity.productName)}`);
    } else {
      text = text.replace(displayNames, () => identity.ios.displayName);
    }
    out[file] = text;
  }
  return out;
}
