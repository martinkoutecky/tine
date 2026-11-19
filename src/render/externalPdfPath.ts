import { filePathUrl, resolvedFilePath, type FileLinkTarget } from "./relativeFileLink";

/** Absolute file-URL PDF targets, without filesystem access. It does not read
 * relative spellings: `relativePdfFile` resolves those from their file target. */
export function externalPdfPath(dest: string): { kind: "pdf"; path: string } | { kind: "other" } {
  if (!/^file:\/\//i.test(dest)) return { kind: "other" };
  try {
    const url = new URL(dest);
    if (url.protocol !== "file:" || url.search || url.hash) return { kind: "other" };
    const path = decodeURIComponent(url.pathname);
    if (path.includes("\0") || !/\.pdf$/i.test(path)) return { kind: "other" };
    return { kind: "pdf", path: url.hostname ? `//${url.hostname}${path}` : path.replace(/^\/([a-z]:\/)/i, "$1") };
  } catch { return { kind: "other" }; }
}

/** The absolute path and file: URL for a relative PDF link (GH #732), or null
 * to keep its existing opener. Uses `resolvedFilePath`, the one resolver of
 * relative file links (graph root, parent steps kept). A bare `x.pdf` stays with
 * the graph-asset reader (OG: assets-relative), a path that resolves into the
 * graph's `assets/` and an absolute plain path keep their openers, and an unknown
 * root cannot make a path absolute. O(path segments). */
export function relativePdfFile(target: FileLinkTarget, root: string): { path: string; url: string } | null {
  if (!root || (target.kind === "outside" && target.reason === "absolute")) return null;
  if (!/\.pdf$/i.test(target.path) || target.path.includes("\0")) return null;
  const path = resolvedFilePath(target, root);
  const assets = `${resolvedFilePath({ kind: "graph", path: "assets" }, root)}/`.toLowerCase();
  if (path.toLowerCase().startsWith(assets)) return null;
  return { path, url: filePathUrl(path) };
}

/** Even an invalid/restored file: route must never reach graph annotation APIs. */
export const isExternalPdf = (filename: string): boolean => /^file:/i.test(filename);
