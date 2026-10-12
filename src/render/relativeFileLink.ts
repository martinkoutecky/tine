// GH #457: a relative link to a file (`[x](../files/report.pdf)`, the Org
// `[[file:../files/a.txt][x]]`, `[x](<my file.txt>)`) names a file relative to
// the page's own file, as a relative link does in any Markdown file, so a graph
// that moves between machines keeps its links. Read side only: nothing here
// rewrites a link when a page or file moves.
//
// OG (og-1.0.0 frontend/components/block.cljs search-link-cp :1112-1121 and
// relative-assets-path->absolute-path :1013) instead strips every leading `.`
// and `/` and joins the rest to the graph root. For a page in `pages/` or
// `journals/` the usual `../files/x` spelling names the same file both ways; a
// link with no leading `../` (`files/x`) or one that climbs above the graph
// differs, and is an intentional difference recorded in GH #457's receipt.
//
// Links into `assets/` keep their own route (`assetLinkRel`), and a dest with a
// scheme, an absolute path or no path shape (`Some Page`) is not a relative
// file link.

/** Where a relative file link points: a graph-relative path, or outside the
 *  graph folder (shown as a broken link and never opened). */
export type RelativeFileTarget = { kind: "graph"; path: string } | { kind: "outside" };

/** The relative path a link destination names, or null when it is not a
 *  relative file link: a URL scheme (`https:`, `zotero:`), an absolute or
 *  drive path, an anchor (`#x`, `*x`), or a bare word with no `/` or `.` (OG
 *  treats that as a page reference). `file:` without a slash after it is
 *  relative (`file:../a.pdf`). Backslashes read as separators and percent
 *  escapes are decoded. O(dest bytes). */
export function relativeFileLinkPath(dest: string): string | null {
  let path = dest.trim();
  if (/^file:(?![/\\])/i.test(path)) path = path.slice("file:".length);
  else if (/^[a-z][a-z0-9+.-]*:/i.test(path)) return null;
  path = path.replace(/\\/g, "/");
  if (!path || /^[/~#*]/.test(path) || /^[a-z]:/i.test(path)) return null;
  if (!/[/.]/.test(path)) return null;
  try {
    path = decodeURIComponent(path);
  } catch {
    // A stray `%` is part of the file name.
  }
  return path;
}

/** Resolve `relative` against the directory of `pageFile` (a graph-relative
 *  page path such as `pages/Plan.md`). A path that climbs above the graph root
 *  is `outside`; the native opener re-checks containment after symlinks. The
 *  empty path is the graph root itself. O(path segments). */
export function resolvePageRelative(pageFile: string, relative: string): RelativeFileTarget {
  const segments = pageFile.replace(/\\/g, "/").split("/").filter((part) => part && part !== ".");
  segments.pop();
  for (const part of relative.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (!segments.length) return { kind: "outside" };
      segments.pop();
    } else {
      segments.push(part);
    }
  }
  return { kind: "graph", path: segments.join("/") };
}
