// iOS distribution contract (transcribed from master's
// scripts/test-release-pipeline.mjs Apple block, c6b99809d). TestFlight is a
// manual, build-only-by-default lane; this pins its fail-closed shape, the
// tracked Apple inputs, and prepare-ios-project.mjs against a fixture.
// og addition: the identity switch (docs/app-identity.md) must select the
// stable identity before the lane can sign anything.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { IDENTITIES } from "./lib/app-identity.mjs";

const read = (file) => fs.readFileSync(path.join(process.cwd(), file), "utf8");
const iosTestFlightWorkflow = read(".github/workflows/ios-testflight.yml");
const iosIconVerifier = read("scripts/verify-ios-app-icon.mjs");
const iosConfig = JSON.parse(read("src-tauri/tauri.ios.conf.json"));
const iosInfoPlist = read("src-tauri/Info.ios.plist");
const iosEntitlements = read("src-tauri/Tine.ios.entitlements");
const iosPrivacyManifest = read("src-tauri/PrivacyInfo.xcprivacy");
const iosIconFixture = fs.readFileSync(path.join(process.cwd(), "src-tauri/icons/ios/AppIcon-512@2x.png"));
const aboutTab = read("src/components/AboutTab.tsx");

function yamlNamedStep(lines, name) {
  const marker = `- name: ${name}`;
  const start = lines.findIndex((line) => line.trimStart() === marker);
  assert.ok(start >= 0, `iOS workflow is missing step ${name}`);
  const indent = lines[start].length - lines[start].trimStart().length;
  let end = start + 1;
  while (end < lines.length) {
    const line = lines[end];
    if (line.trimStart().startsWith("- ") && line.length - line.trimStart().length === indent) break;
    end += 1;
  }
  return lines.slice(start, end);
}

assert.equal(IDENTITIES.release.identifier, "page.tine.Tine",
  "the TestFlight record, provisioning profile and iCloud container belong to page.tine.Tine");
assert.match(
  iosTestFlightWorkflow,
  /name: Require the stable app identity[\s\S]*?app-identity\.json[\s\S]*?s\.ship !== 'release'[\s\S]*?process\.exit\(1\)[\s\S]*?name: Require iOS distribution secrets/,
  "the iOS lane must refuse a Beta-identity checkout before it touches signing material"
);

assert.doesNotMatch(iosTestFlightWorkflow, /\n\s+push:/, "TestFlight workflow must never run on push");
assert.match(iosTestFlightWorkflow, /workflow_dispatch:[\s\S]*?default: build-only[\s\S]*?- validate[\s\S]*?- upload/);
assert.match(iosTestFlightWorkflow, /permissions:\n\s+contents: read/);
assert.match(
  iosTestFlightWorkflow,
  /name: Require iOS distribution secrets[\s\S]*?IOS_CERTIFICATE[\s\S]*?IOS_MOBILE_PROVISION[\s\S]*?inputs\.action[^\n]*!= "build-only"[\s\S]*?APPLE_API_PRIVATE_KEY/,
  "the iOS workflow does not distinguish local signing secrets from optional App Store Connect actions"
);
assert.match(
  iosTestFlightWorkflow,
  /uses: swatinem\/rust-cache@v2[\s\S]*?cache-on-failure: true/,
  "the expensive iOS target cache is not preserved after post-build contract failures",
);
assert.match(
  iosTestFlightWorkflow,
  /name: Install iCloud entitlements and manual signing config[\s\S]*?npm run ios:prepare-project[\s\S]*?name: Build signed TestFlight IPA[\s\S]*?--export-method app-store-connect[\s\S]*?--build-number "\$\{GITHUB_RUN_NUMBER\}"/,
  "TestFlight builds do not use a unique build number and the App Store Connect export method"
);
assert.match(
  iosTestFlightWorkflow,
  /name: Install iOS signing materials[\s\S]*?security import "\$p12"[\s\S]*?-t cert -f pkcs12[\s\S]*?security set-key-partition-list[\s\S]*?security find-identity[\s\S]*?security cms -D -i "\$profile"[\s\S]*?Provisioning Profiles[\s\S]*?IOS_PROVISIONING_PROFILE_UUID/,
  "iOS signing must explicitly import PKCS#12 and the provisioning profile on macOS 26"
);
const iosWorkflowLines = iosTestFlightWorkflow.split(/\r?\n/);
const iosBuildStep = yamlNamedStep(iosWorkflowLines, "Build signed TestFlight IPA").join("\n");
assert.doesNotMatch(
  iosBuildStep,
  /IOS_(?:CERTIFICATE|CERTIFICATE_PASSWORD|MOBILE_PROVISION):/,
  "Tauri must not repeat its broken signing-input mutation after explicit Xcode project setup"
);
assert.match(
  iosTestFlightWorkflow,
  /name: Verify signed IPA contract[\s\S]*?expect_plist[\s\S]*?CFBundleIdentifier[\s\S]*?page\.tine\.Tine[\s\S]*?CFBundleDisplayName[\s\S]*?TineOutline[\s\S]*?root privacy manifest[\s\S]*?embedded provisioning profile[\s\S]*?com\.apple\.developer\.icloud-container-identifiers[\s\S]*?codesign --verify --deep --strict[\s\S]*?CloudDocuments/,
  "the signed IPA is not checked against Tine's identity, privacy, provisioning, and signature contract"
);
assert.match(iosTestFlightWorkflow, /name: Validate IPA with App Store Connect\n\s+if: inputs\.action != 'build-only'/);
assert.match(iosTestFlightWorkflow, /name: Upload IPA to TestFlight\n\s+if: inputs\.action == 'upload'/);
assert.match(
  iosTestFlightWorkflow,
  /AppIcon60x60@2x\.png[\s\S]*?pngcrush -q -revert-iphone-optimizations[\s\S]*?verify-ios-app-icon\.mjs[\s\S]*?src-tauri\/icons\/ios\/AppIcon-60x60@2x\.png/,
  "the signed TestFlight IPA is not checked against Tine's tracked primary icon"
);
assert.match(
  iosIconVerifier,
  /MAX_MEAN_ABSOLUTE_RGB_ERROR[\s\S]*?meanAbsoluteRgbError[\s\S]*?assert\.ok/,
  "the signed IPA icon verifier must tolerate packaging transforms while enforcing visual identity",
);
execFileSync(
  process.execPath,
  [
    path.join(process.cwd(), "scripts/verify-ios-app-icon.mjs"),
    path.join(process.cwd(), "src-tauri/icons/ios/AppIcon-60x60@2x.png"),
    path.join(process.cwd(), "src-tauri/icons/ios/AppIcon-60x60@2x.png"),
  ],
  { stdio: "pipe" }
);
assert.throws(
  () =>
    execFileSync(
      process.execPath,
      [
        path.join(process.cwd(), "scripts/verify-ios-app-icon.mjs"),
        path.join(process.cwd(), "src-tauri/icons/ios/AppIcon-60x60@2x.png"),
        path.join(process.cwd(), "src-tauri/icons/128x128.png"),
      ],
      { stdio: "pipe" }
    ),
  "the signed IPA icon verifier must reject different artwork"
);
assert.match(
  iosTestFlightWorkflow,
  /name: Remove Apple signing material\n\s+if: always\(\)[\s\S]*?security delete-keychain[\s\S]*?\.appstoreconnect\/private_keys/,
  "temporary iOS App Store Connect authentication is not cleaned after failures"
);
assert.doesNotMatch(
  iosTestFlightWorkflow,
  /contents:\s*write|git tag|git push|gh release|submit-for-review|release-to-users/i,
  "the TestFlight lane may publish source/releases or submit a production App Store release"
);

assert.equal(iosConfig.app.windows.length, 1, "iOS must retain its single-window contract");
assert.equal(iosConfig.app.windows[0].label, "main");
assert.equal(iosConfig.bundle.iOS.developmentTeam, "RQ5V4LK7N2");
// iOS WebKit is the OS's: below 15.4 the lsdoc wasm (reference types) and the
// ES2022 frontend cannot run, so the store must not offer Tine there (GH #572).
// The Swift package's lower `.iOS(.v14)` floor is compatible with this.
assert.equal(iosConfig.bundle.iOS.minimumSystemVersion, "15.4");
assert.equal(JSON.parse(fs.readFileSync(path.join(process.cwd(), "src-tauri/tauri.conf.json"), "utf8")).bundle.iOS.minimumSystemVersion, "15.4");
assert.equal(iosConfig.bundle.resources, undefined, "the privacy manifest must not be nested under Tauri's assets folder");
assert.match(iosInfoPlist, /<key>CFBundleDisplayName<\/key>\s*<string>TineOutline<\/string>/);
assert.match(iosInfoPlist, /<key>ITSAppUsesNonExemptEncryption<\/key>\s*<false\/>/);
assert.match(iosInfoPlist, /<key>NSUbiquitousContainers<\/key>[\s\S]*?iCloud\.page\.tine\.Tine[\s\S]*?NSUbiquitousContainerIsDocumentScopePublic[\s\S]*?<true\/>[\s\S]*?NSUbiquitousContainerName[\s\S]*?TineOutline/);
for (const entitlement of [
  "com.apple.developer.icloud-container-identifiers",
  "com.apple.developer.icloud-services",
  "CloudDocuments",
  "com.apple.developer.ubiquity-container-identifiers",
  "iCloud.page.tine.Tine",
]) {
  assert.ok(iosEntitlements.includes(entitlement), `iOS entitlements are missing ${entitlement}`);
}

const iosPrepareFixture = fs.mkdtempSync(path.join(os.tmpdir(), "tine-ios-prepare-"));
try {
  const fixtureTauri = path.join(iosPrepareFixture, "src-tauri");
  const fixtureApple = path.join(fixtureTauri, "gen", "apple");
  const fixtureTarget = path.join(fixtureApple, "tine_iOS");
  const fixtureTrackedIcons = path.join(fixtureTauri, "icons", "ios");
  const fixtureGeneratedIcons = path.join(
    fixtureApple,
    "Assets.xcassets",
    "AppIcon.appiconset"
  );
  fs.mkdirSync(fixtureTarget, { recursive: true });
  fs.mkdirSync(fixtureTrackedIcons, { recursive: true });
  fs.mkdirSync(fixtureGeneratedIcons, { recursive: true });
  fs.writeFileSync(path.join(fixtureTauri, "Tine.ios.entitlements"), iosEntitlements);
  fs.writeFileSync(path.join(fixtureTauri, "PrivacyInfo.xcprivacy"), iosPrivacyManifest);
  fs.writeFileSync(path.join(fixtureTrackedIcons, "AppIcon-512@2x.png"), iosIconFixture);
  fs.writeFileSync(path.join(fixtureGeneratedIcons, "AppIcon-512@2x.png"), "tauri-icon");
  fs.writeFileSync(
    path.join(fixtureGeneratedIcons, "Contents.json"),
    JSON.stringify({ images: [{ filename: "AppIcon-512@2x.png" }] })
  );
  fs.writeFileSync(
    path.join(fixtureApple, "project.yml"),
    [
      "targets:",
      "  tine_iOS:",
      "    sources:",
      "      - path: Assets.xcassets",
      "    settings:",
      "      base:",
      "        ENABLE_BITCODE: false",
      "",
    ].join("\n"),
  );
  fs.writeFileSync(path.join(fixtureTarget, "tine_iOS.entitlements"), "stale");
  const fakeXcodegen = path.join(iosPrepareFixture, "xcodegen");
  fs.writeFileSync(fakeXcodegen, "#!/bin/sh\nexit 0\n", { mode: 0o700 });

  execFileSync(process.execPath, [path.join(process.cwd(), "scripts/prepare-ios-project.mjs")], {
    cwd: iosPrepareFixture,
    env: {
      ...process.env,
      APPLE_DEVELOPMENT_TEAM: "RQ5V4LK7N2",
      IOS_PROVISIONING_PROFILE_UUID: "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE",
      IOS_SIGNING_IDENTITY: "Apple Distribution: Martin Koutecky (RQ5V4LK7N2)",
      TINE_XCODEGEN_BIN: fakeXcodegen,
    },
    stdio: "pipe",
  });

  const preparedProject = fs.readFileSync(path.join(fixtureApple, "project.yml"), "utf8");
  assert.match(preparedProject, /CODE_SIGN_STYLE: Manual/);
  assert.match(preparedProject, /- path: PrivacyInfo\.xcprivacy\s+buildPhase: resources/);
  assert.match(preparedProject, /CODE_SIGN_IDENTITY: "Apple Distribution: Martin Koutecky \(RQ5V4LK7N2\)"/);
  assert.match(preparedProject, /PROVISIONING_PROFILE_SPECIFIER: "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"/);
  const exportOptions = fs.readFileSync(path.join(fixtureApple, "ExportOptions.plist"), "utf8");
  assert.match(exportOptions, /<key>signingStyle<\/key>\s*<string>manual<\/string>/);
  assert.match(exportOptions, /<key>page\.tine\.Tine<\/key>\s*<string>AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE<\/string>/);
  assert.equal(
    fs.readFileSync(path.join(fixtureTarget, "tine_iOS.entitlements"), "utf8"),
    iosEntitlements,
  );
  assert.equal(
    fs.readFileSync(path.join(fixtureApple, "PrivacyInfo.xcprivacy"), "utf8"),
    iosPrivacyManifest,
  );
  assert.deepEqual(
    fs.readFileSync(path.join(fixtureGeneratedIcons, "AppIcon-512@2x.png")),
    iosIconFixture,
    "iOS project preparation must replace Tauri's generated AppIcon with Tine's tracked icon"
  );
} finally {
  fs.rmSync(iosPrepareFixture, { recursive: true, force: true });
}

for (const declaration of [
  "NSPrivacyTracking",
  "NSPrivacyCollectedDataTypes",
  "NSPrivacyAccessedAPICategoryFileTimestamp",
  "C617.1",
  "3B52.1",
  "NSPrivacyAccessedAPICategorySystemBootTime",
  "35F9.1",
]) {
  assert.ok(iosPrivacyManifest.includes(declaration), `iOS privacy manifest is missing ${declaration}`);
}
assert.match(aboutTab, /const PRIVACY = "https:\/\/tine\.page\/privacy\.html"/);
assert.match(aboutTab, /const SUPPORT_EMAIL = "mailto:support@tine\.page"/);
assert.match(aboutTab, /nativePlatform\(\) === "desktop" \|\| nativePlatform\(\) === "android"[\s\S]*?KOFI/);

console.log("iOS distribution contract tests passed.");
