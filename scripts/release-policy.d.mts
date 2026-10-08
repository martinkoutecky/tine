/** Canonical release-version validation, including Beta sequencing and Android bounds. */
export function releaseVersion(version: string): {
  major: number; minor: number; patch: number; sequence: number | null; androidCode: number;
};
/** The GitHub release tag a build publishes to: `beta`, or `v<version>` for stable. */
export function releaseTag(conf: { version: string; plugins?: { updater?: { endpoints?: string[] } } }): string;
