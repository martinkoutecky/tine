import { filePathUrl, resolvedFilePath, type FileLinkTarget } from "./relativeFileLink";

/** Absolute file-URL PDF targets, without filesystem access. It does not read
 * relative or bare absolute spellings: `pdfFileLinkUrl` turns those into a file:
 * URL first. */
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

/** The absolute file: URL a scheme-less or `file:` PDF file link names (GH #732),
 * or null to keep its existing opener. The path is `resolvedFilePath`, the one
 * resolver of file links (graph root, parent steps kept; an absolute path such
 * as `/home/u/a.pdf` or `C:\dir\a.pdf` is itself, so it reads exactly as its
 * `file://` spelling, as in OG). A bare `x.pdf` stays with the graph-asset reader
 * (OG: assets-relative), a path that resolves into the graph's `assets/` keeps
 * its opener, and an unknown root cannot make a relative path absolute.
 * O(path segments). */
export function pdfFileLinkUrl(dest: string, target: FileLinkTarget, root: string): string | null {
  const absolute = target.kind === "outside" && target.reason === "absolute";
  if (!root && !absolute) return null;
  if (!/^file:|[/\\]/i.test(dest.trim())) return null;
  const path = resolvedFilePath(target, root);
  const assets = `${resolvedFilePath({ kind: "graph", path: "assets" }, root)}/`.toLowerCase();
  if (root && path.toLowerCase().startsWith(assets)) return null;
  const url = filePathUrl(path);
  return externalPdfPath(url).kind === "pdf" ? url : null;
}

/** Even an invalid/restored file: route must never reach graph annotation APIs. */
export const isExternalPdf = (filename: string): boolean => /^file:/i.test(filename);
