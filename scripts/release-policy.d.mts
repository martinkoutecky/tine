/** Canonical release-version validation, including Beta sequencing and Android bounds. */
export function releaseVersion(version: string): {
  major: number; minor: number; patch: number; sequence: number | null; androidCode: number;
};
