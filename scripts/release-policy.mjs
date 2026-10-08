// One answer for release routing and version validation. App identity and updater
// channel are independent: the identity flip must not opt Beta into stable updates.
export const BETA_TAG = "beta";
export const BETA_ENDPOINT = "https://github.com/martinkoutecky/tine/releases/download/beta/latest.json";
export const STABLE_CHANNEL = "stable";
export const STABLE_ENDPOINT = "https://github.com/martinkoutecky/tine/releases/latest/download/latest.json";

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
  const endpoints = JSON.stringify(conf.plugins?.updater?.endpoints);
  const { sequence } = releaseVersion(conf.version);
  if (endpoints === JSON.stringify([BETA_ENDPOINT])) return BETA_TAG;
  if (endpoints === JSON.stringify([STABLE_ENDPOINT])) {
    if (sequence) throw new Error("a -beta.N version must use only the beta updater endpoint");
    return STABLE_CHANNEL;
  }
  throw new Error("release builds must use exactly the beta or the stable updater endpoint");
}

/** The release name in AppImage zsync update information: `beta`, or GitHub's `latest` keyword. */
export function zsyncReleaseName(conf) {
  return releaseChannel(conf) === BETA_TAG ? BETA_TAG : "latest";
}

/** The GitHub release a channel publishes to: the moving `beta` tag, or `v<version>`. */
export function releaseTag(conf) {
  return releaseChannel(conf) === BETA_TAG ? BETA_TAG : `v${conf.version}`;
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
  const channel = releaseChannel(conf);
  releaseVersion(conf.version);
  if (mode !== "build") throw new Error("release supports mode=build only; promotion is not implemented");
  if (publish !== true && publish !== false) throw new Error("publish must be an explicit boolean");
  if (publish && tag !== releaseTag(conf)) {
    throw new Error(`a ${channel} build may publish only to ${releaseTag(conf)}, not ${tag}`);
  }
  const stable = channel === STABLE_CHANNEL;
  return { channel, publish, prerelease: !stable, latest: stable };
}

export function updaterAssetUrl(repository, asset, channel = "stable") {
  if (channel !== STABLE_CHANNEL && channel !== BETA_TAG) throw new Error(`unknown updater channel ${channel}`);
  const route = channel === BETA_TAG ? `download/${BETA_TAG}` : "latest/download";
  return `https://github.com/${repository}/releases/${route}/${asset}`;
}
