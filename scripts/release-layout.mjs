import { IDENTITY } from "./lib/app-identity.mjs";
import { BETA_TAG, STABLE_CHANNEL, releaseVersion } from "./release-policy.mjs";
import fs from "node:fs";
import path from "node:path";

export const RELEASE_LANES = [
  "linux-x64",
  "linux-arm64",
  "macos-universal",
  "windows-x64",
  "windows-x86",
  "windows-arm64",
  "android",
];

export function assertReleaseVersion(version) {
  releaseVersion(version);
}

export function releaseLayout(version, identity = IDENTITY) {
  const product = identity.productName.replace(/\s+/g, "-");
  assertReleaseVersion(version);
  const lanes = {
    "linux-x64": {
      assets: [
        `${product}_${version}_amd64.AppImage`,
        `${product}_${version}_amd64.AppImage.sig`,
        `${product}_${version}_amd64.AppImage.zsync`,
        `${product}_${version}_amd64.deb`,
        `${product}_${version}_amd64.deb.sig`,
        `${product}-${version}-1.x86_64.rpm`,
        `${product}-${version}-1.x86_64.rpm.sig`,
      ],
      platforms: {
        "linux-x86_64": [`${product}_${version}_amd64.AppImage`, `${product}_${version}_amd64.AppImage.sig`],
        "linux-x86_64-appimage": [
          `${product}_${version}_amd64.AppImage`,
          `${product}_${version}_amd64.AppImage.sig`,
        ],
        "linux-x86_64-deb": [`${product}_${version}_amd64.deb`, `${product}_${version}_amd64.deb.sig`],
        "linux-x86_64-rpm": [
          `${product}-${version}-1.x86_64.rpm`,
          `${product}-${version}-1.x86_64.rpm.sig`,
        ],
      },
    },
    "linux-arm64": {
      assets: [
        `${product}_${version}_aarch64.AppImage`,
        `${product}_${version}_aarch64.AppImage.sig`,
        `${product}_${version}_aarch64.AppImage.zsync`,
        `${product}_${version}_arm64.deb`,
        `${product}_${version}_arm64.deb.sig`,
        `${product}-${version}-1.aarch64.rpm`,
        `${product}-${version}-1.aarch64.rpm.sig`,
      ],
      platforms: {
        "linux-aarch64": [
          `${product}_${version}_aarch64.AppImage`,
          `${product}_${version}_aarch64.AppImage.sig`,
        ],
        "linux-aarch64-appimage": [
          `${product}_${version}_aarch64.AppImage`,
          `${product}_${version}_aarch64.AppImage.sig`,
        ],
        "linux-aarch64-deb": [`${product}_${version}_arm64.deb`, `${product}_${version}_arm64.deb.sig`],
        "linux-aarch64-rpm": [
          `${product}-${version}-1.aarch64.rpm`,
          `${product}-${version}-1.aarch64.rpm.sig`,
        ],
      },
    },
    "macos-universal": {
      assets: [`${product}_${version}_universal.dmg`],
      platforms: {},
    },
    "windows-x64": {
      assets: [
        `${product}_${version}_x64-setup.exe`,
        `${product}_${version}_x64-setup.exe.sig`,
        `${product}_${version}_x64-portable.zip`,
      ],
      platforms: {
        "windows-x86_64": [
          `${product}_${version}_x64-setup.exe`,
          `${product}_${version}_x64-setup.exe.sig`,
        ],
        "windows-x86_64-nsis": [
          `${product}_${version}_x64-setup.exe`,
          `${product}_${version}_x64-setup.exe.sig`,
        ],
      },
    },
    "windows-x86": {
      assets: [
        `${product}_${version}_x86-setup.exe`,
        `${product}_${version}_x86-setup.exe.sig`,
        `${product}_${version}_x86-portable.zip`,
      ],
      // Experimental GH #275 lane: updates remain manual, as on stable.
      platforms: {},
    },
    "windows-arm64": {
      assets: [
        `${product}_${version}_arm64-setup.exe`,
        `${product}_${version}_arm64-setup.exe.sig`,
        `${product}_${version}_arm64-portable.zip`,
      ],
      platforms: {
        "windows-aarch64": [
          `${product}_${version}_arm64-setup.exe`,
          `${product}_${version}_arm64-setup.exe.sig`,
        ],
        "windows-aarch64-nsis": [
          `${product}_${version}_arm64-setup.exe`,
          `${product}_${version}_arm64-setup.exe.sig`,
        ],
      },
    },
    android: {
      assets: [`${product}_${version}_android-arm64.apk`],
      platforms: {},
    },
  };
  for (const spec of Object.values(lanes)) {
    // Tauri uses productName verbatim; our own zip/APK are already canonical.
    spec.sourceAssets = Object.fromEntries(spec.assets.map((name) => [name,
      name.endsWith("-portable.zip") || name.endsWith(".apk") ? name
        : identity.productName + name.slice(product.length),
    ]));
  }
  const platformAssets = RELEASE_LANES.flatMap((lane) => lanes[lane].assets);
  return {
    lanes,
    platformAssets,
    allAssets: ["latest.json", ...platformAssets],
    updaterPlatforms: Object.assign({}, ...RELEASE_LANES.map((lane) => lanes[lane].platforms)),
  };
}

export function releaseNotes(root, version) {
  const lines = fs.readFileSync(path.join(root, "CHANGELOG.md"), "utf8").split("\n");
  const start = lines.findIndex((line) => line.startsWith(`## [${version}]`));
  if (start === -1) throw new Error(`CHANGELOG.md has no ${version} release section`);
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i += 1) {
    if (lines[i].startsWith("## [") || /^\[[^\]]+\]:\s/.test(lines[i])) {
      end = i;
      break;
    }
  }
  return lines.slice(start + 1, end).join("\n").trim();
}

export function candidateProblems(directory, version, channel = "stable") {
  const layout = releaseLayout(version);
  const names = new Set(
    fs.readdirSync(directory, { withFileTypes: true }).filter((entry) => entry.isFile()).map((entry) => entry.name)
  );
  const problems = [];
  const missing = layout.allAssets.filter((name) => !names.has(name));
  const unexpected = [...names].filter((name) => !layout.allAssets.includes(name)).sort();
  if (missing.length) problems.push(`missing assets: ${missing.join(", ")}`);
  if (unexpected.length) problems.push(`unexpected assets: ${unexpected.join(", ")}`);
  const updaterPath = path.join(directory, "latest.json");
  if (!fs.existsSync(updaterPath)) return problems;
  let updater;
  try {
    updater = JSON.parse(fs.readFileSync(updaterPath, "utf8"));
  } catch (error) {
    problems.push(`latest.json is invalid JSON: ${error.message}`);
    return problems;
  }
  if (updater.version !== version) problems.push(`latest.json version is ${updater.version}, expected ${version}`);
  const expectedPlatforms = Object.keys(layout.updaterPlatforms).sort();
  const actualPlatforms = Object.keys(updater.platforms ?? {}).sort();
  const missingPlatforms = expectedPlatforms.filter((platform) => !actualPlatforms.includes(platform));
  const unexpectedPlatforms = actualPlatforms.filter((platform) => !expectedPlatforms.includes(platform));
  if (missingPlatforms.length) problems.push(`latest.json missing platforms: ${missingPlatforms.join(", ")}`);
  if (unexpectedPlatforms.length) problems.push(`latest.json has unexpected platforms: ${unexpectedPlatforms.join(", ")}`);
  for (const platform of expectedPlatforms) {
    const entry = updater.platforms?.[platform];
    const [asset] = layout.updaterPlatforms[platform];
    if (entry && channel === BETA_TAG) {
      let inBeta = false;
      try {
        const url = new URL(entry.url);
        const parts = url.pathname.split("/");
        inBeta = url.protocol === "https:" && url.hostname === "github.com"
          && parts.at(-3) === "download" && parts.at(-2) === BETA_TAG && parts.at(-1) === asset;
      } catch {
        inBeta = false;
      }
      if (!inBeta) problems.push(`latest.json ${platform} escapes beta`);
    }
    if (entry && channel === STABLE_CHANNEL) {
      let stable = false;
      try {
        const url = new URL(entry.url);
        const parts = url.pathname.split("/");
        stable = url.protocol === "https:" && url.hostname === "github.com"
          && parts.at(-4) === "releases" && parts.at(-3) === "latest" && parts.at(-2) === "download" && parts.at(-1) === asset;
      } catch {
        stable = false;
      }
      if (!stable) problems.push(`latest.json ${platform} is not a stable latest/download URL`);
    }
    if (entry && !entry.url?.endsWith(`/${asset}`)) problems.push(`latest.json ${platform} points at the wrong asset`);
    if (entry && (typeof entry.signature !== "string" || entry.signature.length === 0)) {
      problems.push(`latest.json ${platform} has no signature`);
    }
  }
  return problems;
}
