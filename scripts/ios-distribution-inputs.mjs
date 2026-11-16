// TestFlight keeps the identity the checkout ships. Beta can never sign or
// upload as the App Store app (GH #890).
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import { ROOT, readSwitch, iosIdentity } from "./lib/app-identity.mjs";

export function selectIosDistribution(sw, config, ref, runNumber) {
  const version = /^(\d+\.\d+\.\d+)(-beta\.[1-9]\d{0,2})?$/.exec(config.version);
  if (!version) throw new Error("iOS distribution needs X.Y.Z or X.Y.Z-beta.N (N=1–999)");
  const ship = version[2] ? "experiment" : "release";
  const identity = sw.identities[ship];
  if (sw.ship !== ship || config.identifier !== identity.identifier || config.productName !== identity.productName) {
    throw new Error("iOS version and checkout identity disagree; refusing signing");
  }
  if (ref.startsWith("refs/tags/") && ref !== `refs/tags/v${config.version}`) {
    throw new Error("iOS tag must match the checkout version");
  }
  if (!/^[1-9]\d*$/.test(String(runNumber))) throw new Error("iOS build needs a positive Actions run number");
  const ios = iosIdentity(identity);
  return {
    ship,
    profileSecret: ios.provisioningProfileSecret,
    extensionProfileSecret: ios.shareExtensionProvisioningProfileSecret,
    env: {
      IOS_APP_IDENTIFIER: ios.identifier,
      IOS_DISPLAY_NAME: ios.displayName,
      IOS_EXTENSION_DISPLAY_NAME: identity.productName,
      IOS_APP_GROUP: ios.appGroup,
      IOS_ICLOUD_CONTAINER: ios.iCloudContainer,
      IOS_EXTENSION_IDENTIFIER: ios.shareExtensionIdentifier,
      IOS_ICON_DIRECTORY: ios.iconDirectory,
      IOS_APP_STORE_APPLE_ID: ios.appStoreAppleId,
      IOS_MARKETING_VERSION: version[1],
      // Tauri prefixes --build-number with the numeric version, as before.
      IOS_BUILD_VERSION: `${version[1]}.${runNumber}`,
    },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const configPath = path.join(ROOT, "src-tauri/tauri.conf.json");
  const config = JSON.parse(fs.readFileSync(configPath, "utf8"));
  const selected = selectIosDistribution(readSwitch(), config, process.env.GITHUB_REF ?? "", process.env.GITHUB_RUN_NUMBER);
  // Use the same derivation front door as desktop and Android, with no stable
  // fallback. A drifted checkout must fail before any signing material is read.
  execFileSync(process.execPath, [path.join(ROOT, "scripts/set-app-identity.mjs"), "--check"], { stdio: "inherit" });
  execFileSync(process.execPath, [path.join(ROOT, "scripts/set-app-identity.mjs"), selected.ship], { stdio: "inherit" });
  // Apple's marketing version is numeric; the source version and Beta sequence
  // remain recorded in the original tag/package and workflow log.
  const text = fs.readFileSync(configPath, "utf8").replace(
    /("version": )"[^"]*"/, `$1${JSON.stringify(selected.env.IOS_MARKETING_VERSION)}`,
  );
  fs.writeFileSync(configPath, text);
  fs.appendFileSync(process.env.GITHUB_ENV, Object.entries(selected.env).map(([key, value]) => `${key}=${value}\n`).join(""));
  fs.appendFileSync(process.env.GITHUB_OUTPUT,
    `profile_secret=${selected.profileSecret}\nextension_profile_secret=${selected.extensionProfileSecret}\n`);
  console.log(`${config.version}: ${selected.env.IOS_APP_IDENTIFIER}, App Store Connect ${selected.env.IOS_APP_STORE_APPLE_ID}, ` +
    `marketing ${selected.env.IOS_MARKETING_VERSION}, build ${selected.env.IOS_BUILD_VERSION}`);
}
