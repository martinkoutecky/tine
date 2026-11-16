// iOS distribution contract (transcribed from master's
// scripts/test-release-pipeline.mjs Apple block, c6b99809d). TestFlight is a
// manual, build-only-by-default lane; this pins its fail-closed shape, the
// tracked Apple inputs, and prepare-ios-project.mjs against a fixture.
// GH #890: Beta and stable keep separate app and storage identities.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { IDENTITIES, SHIP, readSwitch, deriveIdentityFiles, iosIdentity } from "./lib/app-identity.mjs";
import { selectIosDistribution } from "./ios-distribution-inputs.mjs";
import { assertOpaquePng } from "./lib/opaque-png.mjs";

const read = (file) => fs.readFileSync(path.join(process.cwd(), file), "utf8");
const iosTestFlightWorkflow = read(".github/workflows/ios-testflight.yml");
const iosIconVerifier = read("scripts/verify-ios-app-icon.mjs");
const iosConfig = JSON.parse(read("src-tauri/tauri.ios.conf.json"));
const iosInfoPlist = read("src-tauri/Info.ios.plist");
const iosEntitlements = read("src-tauri/Tine.ios.entitlements");
const iosPrivacyManifest = read("src-tauri/PrivacyInfo.xcprivacy");
const currentIos = iosIdentity();
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

// Exercise the selector that the workflow actually calls, including a branch
// build-only run. An identity or tag mismatch must fail before signing.
for (const [ship, identity] of Object.entries(IDENTITIES)) {
  const sw = { ...readSwitch(), ship };
  const version = ship === "experiment" ? "0.9.0-beta.8" : "0.9.0";
  const config = { version, identifier: identity.identifier, productName: identity.productName };
  const selected = selectIosDistribution(sw, config, `refs/tags/v${version}`, "321");
  const expectedIos = iosIdentity(identity);
  assert.equal(selected.ship, ship);
  assert.equal(selected.env.IOS_APP_IDENTIFIER, identity.identifier);
  assert.equal(selected.env.IOS_DISPLAY_NAME, identity.ios.displayName);
  assert.equal(selected.env.IOS_APP_GROUP, expectedIos.appGroup);
  assert.equal(selected.env.IOS_ICLOUD_CONTAINER, expectedIos.iCloudContainer);
  assert.equal(selected.env.IOS_EXTENSION_IDENTIFIER, expectedIos.shareExtensionIdentifier);
  assert.equal(selected.env.IOS_ICON_DIRECTORY, `${identity.iconDirectory}/ios`);
  assert.equal(selected.env.IOS_APP_STORE_APPLE_ID, ship === "experiment" ? "6832052325" : "6804403687");
  assert.equal(selected.profileSecret, ship === "experiment" ? "IOS_BETA_MOBILE_PROVISION" : "IOS_MOBILE_PROVISION");
  assert.equal(selected.extensionProfileSecret, ship === "experiment" ? "IOS_BETA_SHARE_EXTENSION_MOBILE_PROVISION" : "IOS_SHARE_EXTENSION_MOBILE_PROVISION");
  assert.equal(selected.env.IOS_MARKETING_VERSION, "0.9.0");
  assert.equal(selected.env.IOS_BUILD_VERSION, "0.9.0.321");
  assert.deepEqual(selectIosDistribution(sw, config, "refs/heads/release/0.9", "321"), selected);
  assert.throws(() => selectIosDistribution({ ...sw, ship: ship === "release" ? "experiment" : "release" }, config, "", "321"), /identity disagree/);
  assert.throws(() => selectIosDistribution(sw, { ...config, identifier: "wrong.id" }, "", "321"), /identity disagree/);
  assert.throws(() => selectIosDistribution(sw, config, "refs/tags/v0.8.4", "321"), /tag must match/);
  assert.throws(() => selectIosDistribution(sw, config, "", "0"), /positive Actions run number/);
}
assert.doesNotMatch(iosTestFlightWorkflow, /beta_as_release|set-app-identity\.mjs release/,
  "a Beta build must never replace the stable app via an identity fallback");
assert.match(
  iosTestFlightWorkflow,
  /name: Select iOS distribution identity[\s\S]*?id: identity[\s\S]*?node scripts\/ios-distribution-inputs\.mjs[\s\S]*?name: Require iOS distribution secrets/,
  "the tested identity selector must run before signing material is read"
);
for (const step of ["Require iOS distribution secrets", "Install iOS signing materials"]) {
  const text = yamlNamedStep(iosTestFlightWorkflow.split(/\r?\n/), step).join("\n");
  assert.ok(text.includes("IOS_MOBILE_PROVISION: ${{ secrets[steps.identity.outputs.profile_secret] }}"));
  assert.ok(text.includes("IOS_SHARE_EXTENSION_MOBILE_PROVISION: ${{ secrets[steps.identity.outputs.extension_profile_secret] }}"));
}

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
  /uses: swatinem\/rust-cache@[0-9a-f]{40} # v2\.[0-9]+\.[0-9]+[\s\S]*?cache-on-failure: true/,
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
  /name: Verify signed IPA contract[\s\S]*?expect_plist[\s\S]*?CFBundleIdentifier "\$IOS_APP_IDENTIFIER"[\s\S]*?CFBundleDisplayName "\$IOS_DISPLAY_NAME"[\s\S]*?root privacy manifest[\s\S]*?embedded provisioning profile[\s\S]*?com\.apple\.developer\.icloud-container-identifiers[\s\S]*?codesign --verify --deep --strict[\s\S]*?CloudDocuments/,
  "the signed IPA is not checked against Tine's identity, privacy, provisioning, and signature contract"
);
// Native integrations (ADR 0073): the Share Extension's own profile is a
// required secret, checked for its identity and the shared App Group, and
// the signed IPA must embed the extension with the app's versions.
assert.match(
  iosTestFlightWorkflow,
  /name: Require iOS distribution secrets[\s\S]*?for name in [^\n]*IOS_SHARE_EXTENSION_MOBILE_PROVISION/,
  "the Share Extension provisioning profile must be a required secret",
);
assert.match(
  iosTestFlightWorkflow,
  /name: Install iOS signing materials[\s\S]*?application-groups[\s\S]*?\$IOS_APP_GROUP[\s\S]*?\$IOS_EXTENSION_IDENTIFIER[\s\S]*?IOS_SHARE_EXTENSION_PROFILE_UUID=/,
  "both profiles must be checked for the App Group and the extension profile exported to prepare-ios-project",
);
assert.match(
  iosTestFlightWorkflow,
  /name: Verify signed IPA contract[\s\S]*?PlugIns\/TineShareExtension\.appex[\s\S]*?\$IOS_EXTENSION_IDENTIFIER[\s\S]*?CFBundleVersion[\s\S]*?extension App Group[\s\S]*?UIApplicationShortcutItems/,
  "the signed IPA must embed the Share Extension with its identity, versions and App Group",
);
assert.match(iosTestFlightWorkflow, /name: Validate IPA with App Store Connect\n\s+if: inputs\.action != 'build-only'/);
assert.match(iosTestFlightWorkflow, /name: Upload IPA to TestFlight\n\s+if: inputs\.action == 'upload'/);
assert.match(iosTestFlightWorkflow, /--upload-package "\$TINE_IPA"[\s\S]*?--apple-id "\$IOS_APP_STORE_APPLE_ID"[\s\S]*?--bundle-id "\$IOS_APP_IDENTIFIER"/);
assert.match(
  iosTestFlightWorkflow,
  /AppIcon60x60@2x\.png[\s\S]*?pngcrush -q -revert-iphone-optimizations[\s\S]*?verify-ios-app-icon\.mjs[\s\S]*?src-tauri\/\$IOS_ICON_DIRECTORY\/AppIcon-60x60@2x\.png/,
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
assert.throws(() => execFileSync(process.execPath, [
  "scripts/verify-ios-app-icon.mjs",
  "src-tauri/icons-beta/ios/AppIcon-60x60@2x.png",
  "src-tauri/icons/ios/AppIcon-60x60@2x.png",
], { stdio: "pipe" }), "the Beta IPA icon verifier must reject the stable artwork at the same size");
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
assert.ok(iosInfoPlist.includes(`<string>${currentIos.displayName}</string>`));
assert.match(iosInfoPlist, /<key>ITSAppUsesNonExemptEncryption<\/key>\s*<false\/>/);
assert.ok(iosInfoPlist.includes(`<key>${currentIos.iCloudContainer}</key>`));
assert.match(iosInfoPlist, /<key>NSUbiquitousContainers<\/key>[\s\S]*?NSUbiquitousContainerIsDocumentScopePublic[\s\S]*?<true\/>[\s\S]*?NSUbiquitousContainerName/);
for (const entitlement of [
  "com.apple.developer.icloud-container-identifiers",
  "com.apple.developer.icloud-services",
  "CloudDocuments",
  "com.apple.developer.ubiquity-container-identifiers",
  currentIos.iCloudContainer,
  "com.apple.security.application-groups",
  currentIos.appGroup,
]) {
  assert.ok(iosEntitlements.includes(entitlement), `iOS entitlements are missing ${entitlement}`);
}

function testIosPreparation(ship, identity) {
  const ios = iosIdentity(identity);
  const derived = deriveIdentityFiles(process.cwd(), ship);
  const fixtureEntitlements = derived["src-tauri/Tine.ios.entitlements"];
  const iosPrepareFixture = fs.mkdtempSync(path.join(os.tmpdir(), "tine-ios-prepare-"));
try {
  const fixtureTauri = path.join(iosPrepareFixture, "src-tauri");
  const fixtureApple = path.join(fixtureTauri, "gen", "apple");
  const fixtureTarget = path.join(fixtureApple, "tine_iOS");
  const fixtureTrackedIcons = path.join(fixtureTauri, ios.iconDirectory);
  const fixtureGeneratedIcons = path.join(
    fixtureApple,
    "Assets.xcassets",
    "AppIcon.appiconset"
  );
  fs.mkdirSync(fixtureTarget, { recursive: true });
  fs.mkdirSync(fixtureTrackedIcons, { recursive: true });
  fs.mkdirSync(fixtureGeneratedIcons, { recursive: true });
  fs.writeFileSync(path.join(fixtureTauri, "app-identity.json"), derived["src-tauri/app-identity.json"]);
  fs.writeFileSync(path.join(fixtureTauri, "Tine.ios.entitlements"), fixtureEntitlements);
  fs.writeFileSync(path.join(fixtureTauri, "PrivacyInfo.xcprivacy"), iosPrivacyManifest);
  const trackedIconRoot = path.join(process.cwd(), "src-tauri", ios.iconDirectory);
  const stableIconNames = fs.readdirSync("src-tauri/icons/ios").filter((file) => file.endsWith(".png")).sort();
  const iconNames = fs.readdirSync(trackedIconRoot).filter((file) => file.endsWith(".png")).sort();
  assert.deepEqual(iconNames, stableIconNames, `${ship}: both identities need the complete iOS icon set`);
  for (const filename of iconNames) {
    const icon = path.join(trackedIconRoot, filename);
    assertOpaquePng(icon, `${ship} ${filename}`);
    fs.copyFileSync(icon, path.join(fixtureTrackedIcons, filename));
  }
  fs.writeFileSync(path.join(fixtureGeneratedIcons, "AppIcon-512@2x.png"), "tauri-icon");
  fs.writeFileSync(
    path.join(fixtureGeneratedIcons, "Contents.json"),
    JSON.stringify({ images: iconNames.map((filename) => ({ filename })) })
  );
  fs.writeFileSync(
    path.join(fixtureApple, "project.yml"),
    [
      "name: tine",
      "targets:",
      "  tine_iOS:",
      "    sources:",
      "      - path: Sources",
      "      - path: Assets.xcassets",
      "    info:",
      "      properties:",
      "        CFBundleShortVersionString: 0.7.1",
      '        CFBundleVersion: "0.7.1"',
      "    settings:",
      "      base:",
      "        ENABLE_BITCODE: false",
      "    dependencies:",
      "      - framework: libapp.a",
      "        embed: false",
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
      IOS_SHARE_EXTENSION_PROFILE_UUID: "11111111-2222-3333-4444-555555555555",
      TINE_XCODEGEN_BIN: fakeXcodegen,
    },
    stdio: "pipe",
  });

  const preparedProject = fs.readFileSync(path.join(fixtureApple, "project.yml"), "utf8");
  assert.match(preparedProject, /CODE_SIGN_STYLE: Manual/);
  assert.match(preparedProject, /- framework: libapp\.a\n\s+embed: false\n\s+- sdk: libz\.tbd\n\s+- sdk: libiconv\.tbd/);
  assert.match(preparedProject, /- path: PrivacyInfo\.xcprivacy\s+buildPhase: resources/);
  assert.match(preparedProject, /CODE_SIGN_IDENTITY: "Apple Distribution: Martin Koutecky \(RQ5V4LK7N2\)"/);
  assert.match(preparedProject, /PROVISIONING_PROFILE_SPECIFIER: "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"/);
  const exportOptions = fs.readFileSync(path.join(fixtureApple, "ExportOptions.plist"), "utf8");
  assert.match(exportOptions, /<key>signingStyle<\/key>\s*<string>manual<\/string>/);
  assert.ok(exportOptions.includes(`<key>${ios.identifier}</key>\n    <string>AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE</string>`));
  // Native integrations (ADR 0073): app-target Swift, the embedded Share
  // Extension with its own bundle id, versions and profile.
  assert.ok(exportOptions.includes(`<key>${ios.shareExtensionIdentifier}</key>\n    <string>11111111-2222-3333-4444-555555555555</string>`));
  assert.match(preparedProject, /- path: \.\.\/\.\.\/ios-app\/App\n\s+- path: \.\.\/\.\.\/ios-app\/Shared/);
  assert.match(preparedProject, /dependencies:\n\s+- target: TineShareExtension\n\s+embed: true\n\s+- sdk: AppIntents\.framework\n\s+weak: true/);
  const extensionTarget = preparedProject.split("\n  TineShareExtension:\n")[1] ?? "";
  assert.match(extensionTarget, /type: app-extension/);
  assert.ok(extensionTarget.includes(`PRODUCT_BUNDLE_IDENTIFIER: ${ios.shareExtensionIdentifier}\n`));
  assert.match(extensionTarget, /MARKETING_VERSION: "0\.7\.1"[\s\S]*?CURRENT_PROJECT_VERSION: "0\.7\.1"/);
  assert.match(extensionTarget, /CODE_SIGN_ENTITLEMENTS: \.\.\/\.\.\/ios-app\/ShareExtension\/TineShareExtension\.entitlements/);
  assert.match(extensionTarget, /PROVISIONING_PROFILE_SPECIFIER: "11111111-2222-3333-4444-555555555555"/);
  assert.equal(
    (preparedProject.match(/PROVISIONING_PROFILE_SPECIFIER: "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"/g) ?? []).length,
    1,
    "the app target keeps its own profile",
  );
  for (const tracked of [
    "src-tauri/ios-app/ShareExtension/Info.plist",
    "src-tauri/ios-app/ShareExtension/TineShareExtension.entitlements",
    "src-tauri/ios-app/ShareExtension/Sources/ShareViewController.swift",
    "src-tauri/ios-app/Shared/ShareInbox.swift",
    "src-tauri/ios-app/App/TineAppIntents.swift",
  ]) assert.ok(fs.existsSync(path.join(process.cwd(), tracked)), `missing ${tracked}`);
  assert.ok(derived["src-tauri/ios-app/ShareExtension/TineShareExtension.entitlements"].includes(ios.appGroup));
  assert.equal(
    fs.readFileSync(path.join(fixtureTarget, "tine_iOS.entitlements"), "utf8"),
    fixtureEntitlements,
  );
  assert.equal(
    fs.readFileSync(path.join(fixtureApple, "PrivacyInfo.xcprivacy"), "utf8"),
    iosPrivacyManifest,
  );
  for (const filename of iconNames) {
    assert.deepEqual(
      fs.readFileSync(path.join(fixtureGeneratedIcons, filename)),
      fs.readFileSync(path.join(trackedIconRoot, filename)),
      `${ship}: iOS project preparation must install the selected identity's ${filename}`,
    );
  }
} finally {
  fs.rmSync(iosPrepareFixture, { recursive: true, force: true });
}
}
for (const [ship, identity] of Object.entries(IDENTITIES)) testIosPreparation(ship, identity);

// Administration must address the same app after upload. Synthetic credentials
// and an in-process fetch stub ensure these probes never contact Apple.
const adminModule = pathToFileURL(path.resolve("scripts/app-store-connect-testflight.mjs")).href;
for (const [ship, identity] of Object.entries(IDENTITIES)) {
  for (const wrongId of [false, true]) {
    const probe = `
      import assert from "node:assert/strict";
      import { generateKeyPairSync } from "node:crypto";
      const { privateKey } = generateKeyPairSync("ec", { namedCurve: "prime256v1" });
      process.env.APPLE_API_PRIVATE_KEY = privateKey.export({ type: "pkcs8", format: "pem" });
      process.env.APPLE_API_KEY = "SYNTHETIC";
      process.env.APPLE_API_ISSUER = "synthetic-issuer";
      let calls = 0;
      globalThis.fetch = async (url, options) => {
        const parsed = new URL(url);
        assert.equal(options.method, "GET");
        if (++calls === 1) {
          assert.equal(parsed.pathname, "/v1/apps");
          assert.equal(parsed.searchParams.get("filter[bundleId]"), ${JSON.stringify(identity.identifier)});
          return new Response(JSON.stringify({ data: [{ id: ${JSON.stringify(wrongId ? "wrong-app" : identity.ios.appStoreAppleId)}, attributes: {} }] }));
        }
        assert.equal(parsed.pathname, "/v1/builds");
        assert.equal(parsed.searchParams.get("filter[app]"), ${JSON.stringify(identity.ios.appStoreAppleId)});
        throw new Error("selected-app-verified");
      };
      await import(${JSON.stringify(adminModule)});
    `;
    assert.throws(() => execFileSync(process.execPath, ["--input-type=module", "-e", probe], {
      env: { ...process.env, APP_BUNDLE_ID: ship === SHIP ? "" : identity.identifier },
      stdio: "pipe",
    }), (error) => error.stderr.toString().includes(wrongId
      ? "App Store Connect app ID disagrees with app-identity.json"
      : "selected-app-verified"), `${ship}: administration must use the selected bundle/app ID`);
  }
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
