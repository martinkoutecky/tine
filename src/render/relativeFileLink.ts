// GH #457: a link to a file by plain path (`[x](../files/report.pdf)`,
// `[x](files/report.pdf)`, the Org `[[file:../files/a.txt][x]]`,
// `[x](<my file.txt>)`) opens that file from the graph folder. Read side only:
// nothing here rewrites a link when a page or file moves.
//
// Resolution is OG's (logseq 0.10.9, c67b8b5fa, unchanged in 1.0.0):
// frontend/components/block.cljs `search-link-cp` (:1082) sends a dest that
// has a `.` and no protocol to `relative-assets-path->absolute-path` (:1013),
// which joins `config.cljs:370 get-local-asset-absolute-path` — the dest with
// every leading `.` and `/` stripped — to the graph folder. So `files/x`,
// `./files/x` and `../files/x` all name `<graph>/files/x`, from a page in
// `pages/`, `journals/` or anywhere else; a dest with no `.` is a page
// reference.
//
// Intentional Tine difference (named in the test and the GH #457 receipt): a
// path that leaves the graph folder is never opened. OG opens an absolute
// `/x` or `C:\x` as is, and its path join lets `files/../../x` climb out of the
// graph; Tine shows both as broken links, because a link in a note is
// untrusted content (imported or malformed Markdown, a synced note). The native
// opener re-checks containment after symlinks.
//
// Links into `assets/` keep their own route (`assetLinkRel`), and a dest with a
// URL scheme or an anchor is not a file link.

/** Where a file link points: a graph-relative path, or outside the graph
 *  folder (an absolute path, or one that climbs out), which is shown as a
 *  broken link and never opened. */
export type FileLinkTarget =
  | { kind: "graph"; path: string }
  | { kind: "outside"; reason: "absolute" | "escapes" };

/** The file a link destination names, or null when it is not a plain file
 *  link: a URL scheme (`https:`, `zotero:`), an anchor (`#x`, `*x`), or a
 *  dest with no `.` (OG reads that as a page reference). `file:` without a
 *  slash after it reads as the path that follows (`file:../a.pdf`, the Org
 *  `[[file:…]]` form). Backslashes read as separators and percent escapes are
 *  decoded. O(dest bytes). */
export function fileLinkTarget(dest: string): FileLinkTarget | null {
  let path = dest.trim();
  if (/^file:(?![/\\])/i.test(path)) path = path.slice("file:".length);
  else if (/^[a-z][a-z0-9+.-]+:/i.test(path)) return null;
  if (!path || /^[#*]/.test(path) || !path.includes(".")) return null;
  path = path.replace(/\\/g, "/");
  try {
    path = decodeURIComponent(path);
  } catch {
    // A stray `%` is part of the file name.
  }
  if (path.startsWith("/") || /^[a-z]:\//i.test(path)) return { kind: "outside", reason: "absolute" };
  return graphRelative(path);
}

/** OG's rule: drop every leading `.` and `/`, then the rest is relative to the
 *  graph folder. A `..` later in the path that climbs above the graph folder is
 *  `outside`; the empty path is the graph folder itself. O(path segments). */
export function graphRelative(path: string): FileLinkTarget {
  const segments: string[] = [];
  for (const part of path.replace(/^[./]*/, "").split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (!segments.length) return { kind: "outside", reason: "escapes" };
      segments.pop();
    } else {
      segments.push(part);
    }
  }
  return { kind: "graph", path: segments.join("/") };
}
