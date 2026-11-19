import { anchorDecision } from "../linkSchemePolicy";

// GH #457: OG 0.10.9 block.cljs relative-assets-path->absolute-path joins
// paths to the graph root, not the page directory. config.cljs
// get-local-asset-absolute-path strips leading dots/slashes; Tine deliberately
// preserves parent traversal for non-asset file links (Martin, 2026-10-14).
// Asset links keep their separate, established OG route.
export type FileLinkTarget =
  | { kind: "graph"; path: string }
  | { kind: "outside"; reason: "absolute" | "escapes"; path: string };

/** Resolve relative paths and the policy's explicit file forms lexically.
 * Page-like destinations stay with their existing renderer. O(dest bytes). */
export function fileLinkTarget(dest: string, decision = anchorDecision(dest)): FileLinkTarget | null {
  if (decision !== "relative" && decision !== "drive" && decision !== "file") return null;
  let path = dest.trim();
  if (decision === "file") {
    if (!/^file:(?![/\\])/i.test(path)) return null;
    path = path.slice("file:".length);
  }
  if (!path || /^[#*]/.test(path)) return null;
  path = path.replace(/\\/g, "/");
  try { path = decodeURIComponent(path); } catch { /* A stray % is a filename character. */ }
  if (path.startsWith("/") || /^[a-z]:\//i.test(path)) {
    return { kind: "outside", reason: "absolute", path };
  }
  if (decision === "drive" || !path.includes(".")) return null;
  return graphRelative(path);
}

/** Normalize segments, retaining every parent step above the graph root.
 * This is lexical only: the OS opener owns existence and symlink behavior. */
export function graphRelative(path: string): FileLinkTarget {
  const segments: string[] = [];
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === ".." && segments.length && segments.at(-1) !== "..") segments.pop();
    else segments.push(part);
  }
  const normalized = segments.join("/");
  return segments[0] === ".."
    ? { kind: "outside", reason: "escapes", path: normalized }
    : { kind: "graph", path: normalized };
}

/** Resolve against a POSIX or Windows graph root without using this device's
 * URL base or page location. Cost: O(path segments), no filesystem probe. */
export function resolvedFilePath(target: FileLinkTarget, root: string): string {
  if (target.kind === "outside" && target.reason === "absolute") return target.path;
  const base = root.replace(/\\/g, "/");
  const prefix = base.startsWith("//") ? "//" : base.startsWith("/") ? "/" : "";
  const segments: string[] = [];
  for (const part of `${base}/${target.path}`.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (segments.length && !/^[a-z]:$/i.test(segments.at(-1)!)) segments.pop();
    } else segments.push(part);
  }
  return prefix + segments.join("/");
}

/** A path goes through the existing file: gate, never a custom-scheme route.
 * Encode literal percent/hash/question characters as well as spaces. */
export function filePathUrl(path: string): string {
  const encoded = path.split("/").map((part, i) => i === 0 && /^[a-z]:$/i.test(part) ? part : encodeURIComponent(part)).join("/");
  return path.startsWith("//") ? `file:${encoded}` : `file://${path.startsWith("/") ? "" : "/"}${encoded}`;
}

/** Display file: URL paths with the same WHATWG parsing used by the native
 * gate. The original URL still goes to that gate, including remote hosts. */
export function fileUrlTitle(dest: string): string | undefined {
  if (!/^file:/i.test(dest)) return undefined;
  try {
    const url = new URL(dest);
    const path = decodeURIComponent(url.pathname);
    return url.hostname ? `//${url.hostname}${path}` : path.replace(/^\/([a-z]:\/)/i, "$1");
  } catch { return dest; }
}
