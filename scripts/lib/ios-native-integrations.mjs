// I1 of the native-integrations batch (ADR 0073): the generated Xcode project
// (`tauri ios init --ci`, regenerated on every CI run) gains Tine's committed
// app-target Swift sources and its Share Extension target. This patch is the
// committed, reviewable mechanism; scripts/prepare-ios-project.mjs applies it
// to the generated `project.yml` before xcodegen, so it survives regeneration.
//
// Paths are relative to src-tauri/gen/apple, where project.yml lives.
import { IDENTITIES } from "./app-identity.mjs";

export const APP_GROUP = `group.${IDENTITIES.release.identifier}`;
export const SHARE_EXTENSION_TARGET = "TineShareExtension";
export const SHARE_EXTENSION_IDENTIFIER = `${IDENTITIES.release.identifier}.ShareExtension`;
const SOURCES = "../../ios-app";
const SWIFT_VERSION = '"5.0"';

function yamlDoubleQuoted(value) {
  return `"${value.replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
}

function once(project, marker, what) {
  const parts = project.split(marker).length - 1;
  if (parts !== 1) throw new Error(`generated iOS project: expected one ${what}, found ${parts}`);
}

/** The app target's marketing and build versions, which an embedded extension
 * must repeat exactly (App Store validation). */
function appVersions(project) {
  const short = /^\s+CFBundleShortVersionString: "?([^"\n]+)"?$/m.exec(project)?.[1];
  const build = /^\s+CFBundleVersion: "?([^"\n]+)"?$/m.exec(project)?.[1];
  if (!short || !build) throw new Error("generated iOS project does not declare the app's bundle versions");
  return { short: short.trim(), build: build.trim() };
}

/**
 * Patch the generated project spec. `signing` is null for unsigned builds, or
 * `{ identity, teamId, extensionProfileUuid }` for the App Store export.
 * Idempotent.
 */
export function addNativeIntegrations(project, signing) {
  if (project.includes(`  ${SHARE_EXTENSION_TARGET}:`)) return project;
  const lines = project.split("\n");
  const topLevel = lines.filter((line) => /^[A-Za-z]/.test(line));
  if (topLevel.at(-1) !== "targets:") {
    throw new Error("generated iOS project: `targets:` is no longer the last top-level key");
  }

  // App target: Tine's App Intents and the shared inbox producer.
  const assetMarker = "      - path: Assets.xcassets";
  once(project, assetMarker, "app asset source entry");
  project = project.replace(
    assetMarker,
    `${assetMarker}\n      - path: ${SOURCES}/App\n      - path: ${SOURCES}/Shared`,
  );
  // The app target had no Swift of its own; name the language version.
  const bitcode = "        ENABLE_BITCODE: false";
  once(project, bitcode, "app target settings block");
  project = project.replace(bitcode, `${bitcode}\n        SWIFT_VERSION: ${SWIFT_VERSION}`);
  // Embed the extension in the app's PlugIns. App Intents exist from iOS 16
  // while Tine runs from 15.4, so the framework is linked weakly: a strong
  // (auto)link would stop the app from launching on iOS 15.
  const embed = [
    `      - target: ${SHARE_EXTENSION_TARGET}`,
    "        embed: true",
    "      - sdk: AppIntents.framework",
    "        weak: true",
  ].join("\n");
  once(project, "\n    dependencies:\n", "app target dependency list");
  project = project.replace("\n    dependencies:\n", `\n    dependencies:\n${embed}\n`);

  const { short, build } = appVersions(project);
  const settings = [
    `        PRODUCT_BUNDLE_IDENTIFIER: ${SHARE_EXTENSION_IDENTIFIER}`,
    `        PRODUCT_NAME: ${SHARE_EXTENSION_TARGET}`,
    `        INFOPLIST_FILE: ${SOURCES}/ShareExtension/Info.plist`,
    `        CODE_SIGN_ENTITLEMENTS: ${SOURCES}/ShareExtension/TineShareExtension.entitlements`,
    `        MARKETING_VERSION: ${yamlDoubleQuoted(short)}`,
    `        CURRENT_PROJECT_VERSION: ${yamlDoubleQuoted(build)}`,
    `        SWIFT_VERSION: ${SWIFT_VERSION}`,
    '        TARGETED_DEVICE_FAMILY: "1,2"',
    "        APPLICATION_EXTENSION_API_ONLY: true",
    "        SKIP_INSTALL: true",
  ];
  if (signing) {
    settings.push(
      "        CODE_SIGN_STYLE: Manual",
      `        CODE_SIGN_IDENTITY: ${yamlDoubleQuoted(signing.identity)}`,
      `        DEVELOPMENT_TEAM: ${signing.teamId}`,
      `        PROVISIONING_PROFILE_SPECIFIER: ${yamlDoubleQuoted(signing.extensionProfileUuid)}`,
    );
  }
  const target = [
    `  ${SHARE_EXTENSION_TARGET}:`,
    "    type: app-extension",
    "    platform: iOS",
    "    sources:",
    `      - path: ${SOURCES}/ShareExtension/Sources`,
    `      - path: ${SOURCES}/Shared`,
    "    settings:",
    "      base:",
    ...settings,
    "",
  ].join("\n");
  return `${project.trimEnd()}\n${target}`;
}
