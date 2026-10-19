import propertyName from "../crates/tine-core/src/page_icon_property.txt?raw";
import { parserReady } from "./render/parse";
import { parse_inline_json } from "./render/wasm/lsdoc_wasm";
import type { Inline } from "./render/ast";
import { mediaKind } from "./media";

/** Shared with the store's icon index. Change this file's source policy once
 * to switch the property used by the title, menu, loaded and unloaded pages. */
export const PAGE_ICON_PROPERTIES = propertyName.trim().split(/\s+/);
export const PAGE_ICON_PROPERTY = PAGE_ICON_PROPERTIES[0];

/** Syntax classification stays separate from transport eligibility: a refused
 * image is still an image, never a short-text icon. Org exports ordinary link
 * nodes for images, so use the same image extension policy as inline media. */
export function pageIconImageLink(value: string, pagePath: string): Extract<Inline, { k: "link" }> | null {
  if ((!value.startsWith("!") && !value.startsWith("[[")) || !parserReady()) return null;
  const org = pagePath.toLowerCase().endsWith(".org");
  const nodes = JSON.parse(parse_inline_json(value, org)) as Inline[] | null;
  if (!nodes || nodes.length !== 1) return null;
  const node = nodes[0];
  if (node.k !== "link") return null;
  if (node.image) return node;
  if (!org) return null;
  const dest = node.url.type === "complex" ? node.url.link : node.url.v;
  return dest && mediaKind(dest) === "image" ? node : null;
}

/** Classify syntax with lsdoc, then resolve relative to the physical owner.
 * No URL is ever handed to the browser; only an assets-relative IPC name leaves
 * this boundary. Unknown physical owners cannot authorize a relative image. */
export function pageIconAsset(value: string, pagePath: string, node = pageIconImageLink(value, pagePath)): string | null {
  if (!node || !pagePath) return null;
  const org = pagePath.toLowerCase().endsWith(".org");
  const url = node.url;
  if (url.type !== "search" && !(org && (url.type === "file" || url.type === "page_ref"))) return null;
  let path = org && url.type === "file" && url.v.startsWith("file:") ? url.v.slice(5) : url.v;
  let malformed = false;
  try { path = decodeURIComponent(path); } catch (error) {
    // This is syntax validation, not an IO failure or an empty read answer.
    if (!(error instanceof URIError)) throw error;
    malformed = true;
  }
  if (malformed) return null;
  if (!path || path.startsWith("/") || path.includes(":") || path.includes("\\") || path.includes("\0") || path.includes("?") || path.includes("#")) return null;
  const parts = pagePath.split("/").slice(0, -1);
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (!parts.length) return null;
      parts.pop();
    } else parts.push(part);
  }
  if (parts[0] !== "assets" || parts.length < 2) return null;
  const rel = parts.slice(1).join("/");
  const ext = rel.slice(rel.lastIndexOf(".") + 1).toLowerCase();
  return ["png", "jpg", "jpeg"].includes(ext) ? rel : null;
}
