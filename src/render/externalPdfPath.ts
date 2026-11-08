/** Absolute file-URL PDF targets, without filesystem access. Deliberately does
 * not interpret relative file: spellings or ../ links (GH #732). */
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

/** Even an invalid/restored file: route must never reach graph annotation APIs. */
export const isExternalPdf = (filename: string): boolean => /^file:/i.test(filename);
