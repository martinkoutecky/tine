// One answer for release routing and version validation. App identity and updater
// channel are independent: the identity flip must not opt og into stable updates.
export const PREVIEW_TAG = "og-preview";
export const PREVIEW_ENDPOINT = "https://github.com/martinkoutecky/tine/releases/download/og-preview/latest.json";

export function releaseVersion(version) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-og\.([1-9]\d*))?$/.exec(version ?? "");
  if (!match) throw new Error(`invalid release version: ${version}`);
  const [major, minor, patch, sequence] = match.slice(1).map(Number);
  if (![major, minor, patch].every(Number.isSafeInteger) || minor >= 1000 || patch >= 1000) {
    throw new Error(`version exceeds Android versionCode component bounds: ${version}`);
  }
  // Reserve each minor's 1..999 codes for preview sequence numbers. Allowing
  // patches here would collide (0.7.1-og.1 and 0.7.0-og.2 both yield 7002).
  if (sequence && patch !== 0) throw new Error("og previews use X.Y.0-og.N to keep Android codes unique");
  const androidCode = major * 1_000_000 + minor * 1_000 + patch + (sequence || 0);
  if (!Number.isSafeInteger(androidCode) || androidCode > 2_100_000_000 || (sequence && (!Number.isSafeInteger(sequence) || sequence > 999))) {
    throw new Error(`version exceeds Android versionCode bounds: ${version}`);
  }
  return { major, minor, patch, sequence: sequence || null, androidCode };
}

export function releaseChannel(conf) {
  const endpoints = conf.plugins?.updater?.endpoints;
  if (JSON.stringify(endpoints) !== JSON.stringify([PREVIEW_ENDPOINT])) {
    throw new Error("og release builds must use only the og-preview updater endpoint");
  }
  return PREVIEW_TAG;
}

export function packagingProblems(conf, ship) {
  const version = releaseVersion(conf.version);
  const problems = [];
  if (version.sequence) {
    if (ship !== "experiment") problems.push("og prereleases require the separate experiment Android application id; stable Android codes remain unchanged");
    if (conf.bundle?.targets === "all" || conf.bundle?.targets?.includes("msi")) {
      problems.push("MSI cannot express -og.N; choose a numeric installer mapping or retain master's NSIS target before packaging");
    }
  }
  if (conf.bundle?.android?.versionCode !== version.androidCode) problems.push(`Android versionCode must be ${version.androidCode}`);
  return problems;
}

export function publicationPlan({ conf, mode, publish, tag }) {
  releaseChannel(conf);
  releaseVersion(conf.version);
  if (mode !== "build") throw new Error("og preview supports mode=build only; promotion is not implemented");
  if (publish !== true && publish !== false) throw new Error("publish must be an explicit boolean");
  if (publish && tag !== PREVIEW_TAG) throw new Error("og builds may publish only to og-preview, never a versioned/stable release");
  return { channel: PREVIEW_TAG, publish, prerelease: true, latest: false };
}

export function updaterAssetUrl(repository, asset, channel = "stable") {
  if (channel !== "stable" && channel !== PREVIEW_TAG) throw new Error(`unknown updater channel ${channel}`);
  const route = channel === PREVIEW_TAG ? `download/${PREVIEW_TAG}` : "latest/download";
  return `https://github.com/${repository}/releases/${route}/${asset}`;
}
