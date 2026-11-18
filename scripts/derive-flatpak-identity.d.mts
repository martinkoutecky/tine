export interface DerivedFlatpak {
  id: string;
  productName: string;
  manifestPath: string;
  /** Repo-relative output path to file text. */
  files: Record<string, string>;
}
export function deriveFlatpak(options: {
  root?: string;
  identity: string;
  date: string;
  version: string;
}): DerivedFlatpak;
export function resolveReleaseDate(options?: {
  explicit?: string;
  env?: Record<string, string | undefined>;
  gitCommitDate?: () => string | null;
}): string;
