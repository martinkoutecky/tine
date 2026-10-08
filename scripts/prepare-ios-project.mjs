import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { execFileSync } from "node:child_process";
import { assertOpaquePng } from "./lib/opaque-png.mjs";

const root = process.cwd();
const source = path.join(root, "src-tauri", "Tine.ios.entitlements");
const privacySource = path.join(root, "src-tauri", "PrivacyInfo.xcprivacy");
const iconSource = path.join(root, "src-tauri", "icons", "ios");
const generatedRoot = path.join(root, "src-tauri", "gen", "apple");
const generatedIconCatalog = path.join(
  generatedRoot,
  "Assets.xcassets",
  "AppIcon.appiconset",
);

const signing = {
  identity: process.env.IOS_SIGNING_IDENTITY,
  profileUuid: process.env.IOS_PROVISIONING_PROFILE_UUID,
  teamId: process.env.APPLE_DEVELOPMENT_TEAM,
};

const signingValues = Object.values(signing).filter(Boolean);
if (signingValues.length > 0 && signingValues.length !== 3) {
  throw new Error(
    "iOS signing configuration requires IOS_SIGNING_IDENTITY, " +
      "IOS_PROVISIONING_PROFILE_UUID, and APPLE_DEVELOPMENT_TEAM together",
  );
}

function xml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

function yamlDoubleQuoted(value) {
  return `"${value.replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
}

if (!fs.existsSync(source)) {
  throw new Error(`missing tracked iOS entitlements: ${source}`);
}
if (!fs.existsSync(privacySource)) {
  throw new Error(`missing tracked iOS privacy manifest: ${privacySource}`);
}
if (!fs.existsSync(iconSource)) {
  throw new Error(`missing tracked iOS application icons: ${iconSource}`);
}
if (!fs.existsSync(generatedRoot)) {
  throw new Error("the generated iOS project is absent; run `npx tauri ios init --ci` first");
}

const generatedIconManifest = path.join(generatedIconCatalog, "Contents.json");
if (!fs.existsSync(generatedIconManifest)) {
  throw new Error(`generated iOS icon manifest is absent: ${generatedIconManifest}`);
}
const iconManifest = JSON.parse(fs.readFileSync(generatedIconManifest, "utf8"));
const iconFilenames = [
  ...new Set(
    (iconManifest.images ?? [])
      .map((image) => image.filename)
      .filter((filename) => typeof filename === "string" && filename.endsWith(".png")),
  ),
];
if (iconFilenames.length === 0) {
  throw new Error("generated iOS icon manifest does not reference any PNG files");
}
const referencedIcons = new Set(iconFilenames);
const trackedIcons = fs
  .readdirSync(iconSource, { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.endsWith(".png"))
  .map((entry) => entry.name)
  .sort();
const unreferencedIcons = trackedIcons.filter((filename) => !referencedIcons.has(filename));
if (unreferencedIcons.length > 0) {
  throw new Error(
    `tracked iOS icons are absent from the generated catalog manifest: ${unreferencedIcons.join(", ")}`,
  );
}
for (const filename of iconFilenames) {
  const trackedIcon = path.join(iconSource, filename);
  if (!fs.existsSync(trackedIcon)) {
    throw new Error(`generated iOS catalog requires missing tracked icon: ${trackedIcon}`);
  }
  assertOpaquePng(trackedIcon, `tracked iOS icon ${filename}`);
  fs.copyFileSync(trackedIcon, path.join(generatedIconCatalog, filename));
}
console.log(`installed ${iconFilenames.length} tracked Tine icons in the generated iOS catalog`);

const projectSpec = path.join(generatedRoot, "project.yml");
let project = fs.readFileSync(projectSpec, "utf8");
const assetMarker = "      - path: Assets.xcassets";
if (!project.includes("      - path: PrivacyInfo.xcprivacy")) {
  if (project.split(assetMarker).length !== 2) {
    throw new Error("generated iOS project does not contain the expected asset source entry");
  }
  project = project.replace(
    assetMarker,
    `${assetMarker}\n      - path: PrivacyInfo.xcprivacy\n        buildPhase: resources`,
  );
}

if (signingValues.length === 3) {
  if (!/^Apple Distribution: .+ \([A-Z0-9]{10}\)$/.test(signing.identity)) {
    throw new Error("IOS_SIGNING_IDENTITY is not an Apple Distribution identity");
  }
  if (!/^[A-F0-9]{8}(?:-[A-F0-9]{4}){3}-[A-F0-9]{12}$/i.test(signing.profileUuid)) {
    throw new Error("IOS_PROVISIONING_PROFILE_UUID is not a provisioning profile UUID");
  }
  if (!/^[A-Z0-9]{10}$/.test(signing.teamId)) {
    throw new Error("APPLE_DEVELOPMENT_TEAM is not an Apple team identifier");
  }

  const marker = "    settings:\n      base:\n        ENABLE_BITCODE: false";
  if (!project.includes("        CODE_SIGN_STYLE: Manual")) {
    if (project.split(marker).length !== 2) {
      throw new Error("generated iOS project does not contain the expected target settings block");
    }
    const configured = [
      "    settings:",
      "      base:",
      "        CODE_SIGN_STYLE: Manual",
      `        CODE_SIGN_IDENTITY: ${yamlDoubleQuoted(signing.identity)}`,
      `        DEVELOPMENT_TEAM: ${signing.teamId}`,
      `        PROVISIONING_PROFILE_SPECIFIER: ${yamlDoubleQuoted(signing.profileUuid)}`,
      "        ENABLE_BITCODE: false",
    ].join("\n");
    project = project.replace(marker, configured);
  }

  const exportOptions = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>signingStyle</key>
  <string>manual</string>
  <key>teamID</key>
  <string>${xml(signing.teamId)}</string>
  <key>signingCertificate</key>
  <string>Apple Distribution</string>
  <key>provisioningProfiles</key>
  <dict>
    <key>page.tine.Tine</key>
    <string>${xml(signing.profileUuid)}</string>
  </dict>
</dict>
</plist>
`;
  fs.writeFileSync(path.join(generatedRoot, "ExportOptions.plist"), exportOptions);

  console.log(`installed manual App Store signing config for profile ${signing.profileUuid}`);
}

fs.writeFileSync(projectSpec, project);
fs.copyFileSync(privacySource, path.join(generatedRoot, "PrivacyInfo.xcprivacy"));
execFileSync(process.env.TINE_XCODEGEN_BIN || "xcodegen", ["generate", "--spec", projectSpec], {
  cwd: generatedRoot,
  stdio: "inherit",
});
console.log("installed PrivacyInfo.xcprivacy at the iOS application bundle root");

const generated = [];
for (const entry of fs.readdirSync(generatedRoot, { withFileTypes: true })) {
  if (!entry.isDirectory() || !entry.name.endsWith("_iOS")) continue;
  const directory = path.join(generatedRoot, entry.name);
  for (const child of fs.readdirSync(directory, { withFileTypes: true })) {
    if (child.isFile() && child.name.endsWith("_iOS.entitlements")) {
      generated.push(path.join(directory, child.name));
    }
  }
}

if (generated.length !== 1) {
  throw new Error(`expected exactly one generated iOS entitlements file, found ${generated.length}`);
}

fs.copyFileSync(source, generated[0]);
console.log(`installed iCloud entitlements at ${path.relative(root, generated[0])}`);
