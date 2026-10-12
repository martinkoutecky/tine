import { releaseVersion } from "../release-policy.mjs";

/** Store reserves the fourth component; retain the canonical release parser's bounds. */
export function msixVersion(version) {
  if (String(version).includes("-")) throw new Error("Microsoft Store packages require a stable X.Y.Z version; prereleases are refused");
  if (String(version).split(".").some((part) => /^\d+$/.test(part) && Number(part) > 65535)) {
    throw new Error("Microsoft Store version components must not exceed 65535");
  }
  const { major, minor, patch } = releaseVersion(version);
  return `${major}.${minor}.${patch}.0`;
}
