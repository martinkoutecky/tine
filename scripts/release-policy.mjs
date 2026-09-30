// One answer for release routing and version validation. App identity and updater
// channel are independent: the identity flip must not opt Beta into stable updates.
export const BETA_TAG = "beta";
export const BETA_ENDPOINT = "https://github.com/martinkoutecky/tine/releases/download/beta/latest.json";

export function releaseVersion(version) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-beta\.([1-9]\d*))?$/.exec(version ?? "");
  if (!match) throw new Error(`invalid release version: ${version}`);
  const [major, minor, patch, sequence] = match.slice(1).map(Number);
  if (![major, minor, patch].every(Number.isSafeInteger) || minor >= 1000 || patch >= 1000) {
    throw new Error(`version exceeds Android versionCode component bounds: ${version}`);
  }
  // Reserve each minor's 1..999 codes for Beta sequence numbers. Allowing
  // patches here would collide (0.7.1-beta.1 and 0.7.0-beta.2 both yield 7002).
  if (sequence && patch !== 0) throw new Error("Beta builds use X.Y.0-beta.N to keep Android codes unique");
  const androidCode = major * 1_000_000 + minor * 1_000 + patch + (sequence || 0);
  if (!Number.isSafeInteger(androidCode) || androidCode > 2_100_000_000 || (sequence && (!Number.isSafeInteger(sequence) || sequence > 999))) {
    throw new Error(`version exceeds Android versionCode bounds: ${version}`);
  }
  return { major, minor, patch, sequence: sequence || null, androidCode };
}

export function releaseChannel(conf) {
  const endpoints = conf.plugins?.updater?.endpoints;
  if (JSON.stringify(endpoints) !== JSON.stringify([BETA_ENDPOINT])) {
    throw new Error("Beta release builds must use only the beta updater endpoint");
  }
  return BETA_TAG;
}

export function packagingProblems(conf, ship) {
  const version = releaseVersion(conf.version);
  const problems = [];
  if (version.sequence) {
    if (ship !== "experiment") problems.push("Beta prereleases require the separate experiment Android application id; stable Android codes remain unchanged");
    if (conf.bundle?.targets === "all" || conf.bundle?.targets?.includes("msi")) {
      problems.push("MSI cannot express -beta.N; choose a numeric installer mapping or retain master's NSIS target before packaging");
    }
  }
  if (conf.bundle?.android?.versionCode !== version.androidCode) problems.push(`Android versionCode must be ${version.androidCode}`);
  return problems;
}

export function publicationPlan({ conf, mode, publish, tag }) {
  releaseChannel(conf);
  releaseVersion(conf.version);
  if (mode !== "build") throw new Error("Beta supports mode=build only; promotion is not implemented");
  if (publish !== true && publish !== false) throw new Error("publish must be an explicit boolean");
  if (publish && tag !== BETA_TAG) throw new Error("Beta builds may publish only to beta, never a versioned/stable release");
  return { channel: BETA_TAG, publish, prerelease: true, latest: false };
}

export function updaterAssetUrl(repository, asset, channel = "stable") {
  if (channel !== "stable" && channel !== BETA_TAG) throw new Error(`unknown updater channel ${channel}`);
  const route = channel === BETA_TAG ? `download/${BETA_TAG}` : "latest/download";
  return `https://github.com/${repository}/releases/${route}/${asset}`;
}
